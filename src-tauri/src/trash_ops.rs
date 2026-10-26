//! Undo, Restore From Trash, Delete Permanently and Empty Trash.
//!
//! Items are trashed with the `trash` crate, which on macOS asks Finder to do
//! it so Finder's "Put Back" keeps working. Neither reports where the item
//! went, so the Trash folder is compared before and after to find it; that
//! path and the original one are kept in `trash_log.json` for Undo and
//! Restore. Items trashed elsewhere are restored from whatever record the
//! platform keeps: Finder's "Put Back" entries in the Trash's `.DS_Store` on
//! macOS, the freedesktop `.trashinfo` sidecar files on Linux.
//!
//! The two layouts differ, so every function that needs the Trash's shape
//! asks here rather than assuming macOS:
//!
//! | | macOS | Linux (freedesktop) |
//! | --- | --- | --- |
//! | root | `~/.Trash` | `$XDG_DATA_HOME/Trash` |
//! | items | directly in the root | `files/` |
//! | origin records | `.DS_Store` | `info/<name>.trashinfo` |
//! | other volumes | `.Trashes/<uid>` | `.Trash-<uid>` |
//!
//! Permanent deletion is the one place this app removes files for good, and
//! only for items inside a Trash folder.

use crate::progress::Progress;
use std::collections::{HashMap, HashSet};
use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::fs_ops::is_subpath;
use crate::platform::percent_decode;

/// Most items the log remembers.
const MAX_LOGGED: usize = 2000;

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct TrashLog {
    /// Path in the Trash → where it came from.
    #[serde(default)]
    pub items: HashMap<String, LoggedItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LoggedItem {
    pub origin: String,
    /// When it was trashed (unix seconds).
    pub time: i64,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct TrashInfo {
    /// Where the item came from, if known.
    pub origin: Option<String>,
    /// When it was trashed (unix seconds).
    pub trashed: i64,
}

pub fn read_log(path: &Path) -> TrashLog {
    fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Remember where each `(trashed, origin)` item now lives, forgetting items
/// that have since left the Trash.
pub fn record(log: &mut TrashLog, moves: &[(PathBuf, PathBuf)], now: i64) {
    log.items.retain(|p, _| fs::symlink_metadata(p).is_ok());
    for (trashed, origin) in moves {
        log.items.insert(
            trashed.to_string_lossy().to_string(),
            LoggedItem {
                origin: origin.to_string_lossy().to_string(),
                time: now,
            },
        );
    }
    if log.items.len() > MAX_LOGGED {
        let mut times: Vec<i64> = log.items.values().map(|i| i.time).collect();
        times.sort_unstable_by(|a, b| b.cmp(a));
        let keep_from = times[MAX_LOGGED - 1];
        log.items.retain(|_, i| i.time >= keep_from);
    }
}

#[cfg(unix)]
fn dev_of(path: &Path) -> Option<u64> {
    use std::os::unix::fs::MetadataExt;
    fs::symlink_metadata(path).ok().map(|m| m.dev())
}

#[cfg(not(unix))]
fn dev_of(_: &Path) -> Option<u64> {
    Some(0)
}

/// Topmost folder above `path` on the same filesystem, i.e. its mount point.
fn mount_root(path: &Path) -> PathBuf {
    let dev = dev_of(path);
    let mut root = path.to_path_buf();
    for parent in path.ancestors().skip(1) {
        if dev_of(parent) != dev {
            break;
        }
        root = parent.to_path_buf();
    }
    root
}

/// The Trash folder an item at `path` goes to when trashed.
pub fn trash_dir_for(path: &Path, home: &Path) -> PathBuf {
    let same_disk = dev_of(path.parent().unwrap_or(path)) == dev_of(home);
    if cfg!(target_os = "macos") {
        if same_disk {
            return home.join(".Trash");
        }
        return mount_root(path)
            .join(".Trashes")
            .join(uid(home).to_string());
    }
    // freedesktop.org layout (Linux), for development and tests.
    if same_disk {
        return home_trash(home).join("files");
    }
    mount_root(path)
        .join(format!(".Trash-{}", uid(home)))
        .join("files")
}

#[cfg(unix)]
fn uid(home: &Path) -> u32 {
    use std::os::unix::fs::MetadataExt;
    fs::metadata(home).map(|m| m.uid()).unwrap_or(0)
}

#[cfg(not(unix))]
fn uid(_: &Path) -> u32 {
    0
}

/// The Trash that items on the home filesystem go to, as the *root*: on macOS
/// the folder holding the items, on Linux the folder holding `files/` and
/// `info/`.
pub fn home_trash(home: &Path) -> PathBuf {
    if cfg!(target_os = "macos") {
        home.join(".Trash")
    } else {
        crate::platform::data_home(home).join("Trash")
    }
}

/// The folder whose *contents* are the Trash listing: the home Trash itself on
/// macOS, its `files/` subfolder on Linux. This is the path the UI opens.
pub fn items_root(home: &Path) -> PathBuf {
    if cfg!(target_os = "macos") {
        home.join(".Trash")
    } else {
        home_trash(home).join("files")
    }
}

/// `items_root`, created first on Linux: a fresh account has no
/// `Trash/files` until something is trashed, and the Trash view must show
/// "Trash is Empty" rather than an error. Creating `files/` and `info/` is
/// what any freedesktop implementation does on first use. `~/.Trash` is left
/// to macOS.
pub fn ensure_items_root(home: &Path) -> PathBuf {
    if !cfg!(target_os = "macos") {
        create_freedesktop_trash(&home_trash(home));
    }
    items_root(home)
}

/// Create `files/` and `info/` under a freedesktop Trash root if missing.
fn create_freedesktop_trash(trash_root: &Path) {
    let _ = fs::create_dir_all(trash_root.join("files"));
    let _ = fs::create_dir_all(trash_root.join("info"));
}

// ---------------------------------------------------------------------------
// freedesktop `.trashinfo` sidecars
//
// Every item in a freedesktop Trash has `info/<same name>.trashinfo` beside
// it holding the original path, so Linux needs no log for Restore. The
// format is an ini file:
//
//     [Trash Info]
//     Path=/home/me/notes.txt      (percent-encoded)
//     DeletionDate=2026-10-24T10:11:12
// ---------------------------------------------------------------------------

/// The `info/` folder that belongs to a freedesktop trash *files* folder.
fn info_dir_of(trash_dir: &Path) -> Option<PathBuf> {
    if cfg!(target_os = "macos") {
        return None;
    }
    // Only the `files/` subfolder has an `info/` sibling next to it.
    if trash_dir.file_name()? != "files" {
        return None;
    }
    Some(trash_dir.parent()?.join("info"))
}

/// Whether `a` and `b` are the same existing folder.
fn same_dir(a: &Path, b: &Path) -> bool {
    match (fs::canonicalize(a), fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

/// The `.trashinfo` describing `trashed`, if this platform keeps one.
///
/// Only items directly in `trash_dir` (the Trash's `files/` folder) have
/// one. Deeper down, `files/` and `info/` folders are just the contents of a
/// trashed folder: their files are user data, never records to read or delete.
fn sidecar_of(trashed: &Path, trash_dir: &Path) -> Option<PathBuf> {
    if !same_dir(trashed.parent()?, trash_dir) {
        return None;
    }
    let info = info_dir_of(trash_dir)?;
    let mut file = trashed.file_name()?.to_os_string();
    file.push(".trashinfo");
    Some(info.join(file))
}

/// The `Path=` line of a `.trashinfo` file.
fn parse_trashinfo(text: &str) -> Option<PathBuf> {
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("Path=") {
            let path = PathBuf::from(percent_decode(rest));
            return path.is_absolute().then_some(path);
        }
    }
    None
}

/// Where a trashed item came from, according to its `.trashinfo` sidecar.
pub fn sidecar_origin(trashed: &Path, trash_dir: &Path) -> Option<PathBuf> {
    let text = fs::read_to_string(sidecar_of(trashed, trash_dir)?).ok()?;
    parse_trashinfo(&text)
}

/// Forget an item's sidecar, so a stale `info/` entry can't outlive the item.
fn drop_sidecar(trashed: &Path, trash_dir: &Path) {
    if let Some(sidecar) = sidecar_of(trashed, trash_dir) {
        let _ = fs::remove_file(sidecar);
    }
}

fn names_in(dir: &Path) -> HashSet<OsString> {
    fs::read_dir(dir)
        .map(|rd| rd.flatten().map(|e| e.file_name()).collect())
        .unwrap_or_default()
}

/// The entry that appeared in `dir` since `before` was taken and looks like
/// the trashed `original_name`. `None` when it can't be told apart.
fn find_new(
    dir: &Path,
    before: &HashSet<OsString>,
    original_name: &str,
    is_dir: bool,
) -> Option<PathBuf> {
    locate_all(dir, before, &[(OsStr::new(original_name), is_dir)])
        .pop()
        .flatten()
}

/// Where each `(name, is_dir)` item went in `dir`, judged by the entries
/// that appeared since `before`: one with the item's own name, else the one
/// look-alike that no other item could be (name clashes in the Trash get
/// renamed: "a 2.txt", Finder: "a 10.21.33.txt"). `None` when it can't be
/// told apart.
fn locate_all(
    dir: &Path,
    before: &HashSet<OsString>,
    items: &[(&OsStr, bool)],
) -> Vec<Option<PathBuf>> {
    let mut fresh: Vec<(OsString, bool)> = names_in(dir)
        .into_iter()
        .filter(|n| !before.contains(n))
        .filter_map(|n| {
            let is_dir = fs::symlink_metadata(dir.join(&n)).ok()?.is_dir();
            Some((n, is_dir))
        })
        .collect();
    // Exact names first, so a look-alike can't take another item's entry.
    let mut found: Vec<Option<OsString>> = items
        .iter()
        .map(|&(name, is_dir)| {
            let at = fresh.iter().position(|(n, d)| n == name && *d == is_dir)?;
            Some(fresh.swap_remove(at).0)
        })
        .collect();
    let look_alikes: Vec<Vec<usize>> = items
        .iter()
        .zip(&found)
        .map(|(&(name, is_dir), found)| {
            if found.is_some() {
                return Vec::new();
            }
            let stem = Path::new(name).file_stem().unwrap_or(name).to_string_lossy();
            (0..fresh.len())
                .filter(|&i| fresh[i].1 == is_dir && fresh[i].0.to_string_lossy().starts_with(&*stem))
                .collect()
        })
        .collect();
    for (k, candidates) in look_alikes.iter().enumerate() {
        if let [one] = candidates[..] {
            if look_alikes.iter().filter(|c| c.contains(&one)).count() == 1 {
                found[k] = Some(fresh[one].0.clone());
            }
        }
    }
    found.into_iter().map(|f| f.map(|n| dir.join(n))).collect()
}

/// Move `path` to the Trash and return where it ended up, if that could be
/// worked out (it's needed for Undo, not for trashing itself).
pub fn trash_and_locate(path: &Path, home: &Path) -> Result<Option<PathBuf>, String> {
    let meta =
        fs::symlink_metadata(path).map_err(|_| format!("“{}” no longer exists", path.display()))?;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let dir = trash_dir_for(path, home);
    let before = names_in(&dir);
    trash::delete(path).map_err(|e| format!("Failed to move to Trash: {}", e))?;
    Ok(find_new(
        &dir,
        &before,
        &name,
        meta.is_dir() && !meta.file_type().is_symlink(),
    ))
}

// ---------------------------------------------------------------------------
// Trashing a batch
// ---------------------------------------------------------------------------

/// Most items trashed in one call. On macOS each call is one Finder request:
/// one sound, and nothing can stop it once it runs. This bounds both the
/// sounds of a huge selection and how long a Cancel waits.
pub const TRASH_CHUNK: usize = 100;

/// How often the items of a running call are checked, for progress.
const WATCH_EVERY: std::time::Duration = std::time::Duration::from_millis(50);

/// What a batch Move to Trash says about one path: where it went, when that
/// could be worked out (for Undo), or why it is still there. Paths a Cancel
/// stopped before end with `progress::CANCELLED`.
pub type TrashResult = Result<Option<PathBuf>, String>;

struct Item {
    index: usize,
    path: PathBuf,
    name: OsString,
    is_dir: bool,
}

/// Items that go to the same Trash folder in one call.
struct Chunk {
    dir: PathBuf,
    items: Vec<Item>,
}

/// Move `paths` to the Trash in as few calls as possible: one per Trash
/// folder and `TRASH_CHUNK` items, so a selection is one Finder request (and
/// one sound) on macOS instead of one per item. Progress counts each item as
/// it leaves its folder; a Cancel stops before the next call. Returns one
/// result per path, in order.
pub fn trash_many(paths: &[PathBuf], home: &Path, progress: &Progress) -> Vec<TrashResult> {
    let delete = |batch: &[PathBuf]| {
        trash::delete_all(batch).map_err(|e| format!("Failed to move to Trash: {}", e))
    };
    trash_many_with(paths, |p| trash_dir_for(p, home), &delete, progress)
}

/// One Move to Trash over `paths`: each path `elsewhere` takes (a refusal,
/// or another Trash such as Drive's) gets its result from there; the rest go
/// to `local` together, as one batch. Returns one result per path (the
/// place in the Trash as text, for the frontend) and the `(trashed, origin)`
/// pairs of the local items that could be found, for the log.
#[allow(clippy::type_complexity)]
pub fn trash_routed(
    paths: &[String],
    mut elsewhere: impl FnMut(&str) -> Option<Result<Option<String>, String>>,
    local: impl FnOnce(&[PathBuf]) -> Vec<TrashResult>,
) -> (Vec<Result<Option<String>, String>>, Vec<(PathBuf, PathBuf)>) {
    let mut results: Vec<Result<Option<String>, String>> = Vec::with_capacity(paths.len());
    let mut origins = Vec::new();
    let mut at = Vec::new();
    for (i, path) in paths.iter().enumerate() {
        match elsewhere(path) {
            Some(result) => results.push(result),
            None => {
                results.push(Err(crate::progress::CANCELLED.into()));
                origins.push(PathBuf::from(path));
                at.push(i);
            }
        }
    }
    let mut moves = Vec::new();
    for ((i, origin), result) in at.into_iter().zip(origins.iter()).zip(local(&origins)) {
        if let Ok(Some(trashed)) = &result {
            moves.push((trashed.clone(), origin.clone()));
        }
        results[i] = result.map(|place| place.map(|p| p.to_string_lossy().to_string()));
    }
    (results, moves)
}

/// `trash_many` with the Trash folder and the trashing itself passed in, so
/// tests can stand in for Finder.
fn trash_many_with(
    paths: &[PathBuf],
    dir_for: impl Fn(&Path) -> PathBuf,
    delete: &(impl Fn(&[PathBuf]) -> Result<(), String> + Sync),
    progress: &Progress,
) -> Vec<TrashResult> {
    let mut results: Vec<TrashResult> = paths
        .iter()
        .map(|_| Err(crate::progress::CANCELLED.into()))
        .collect();
    let mut items = Vec::new();
    for (index, path) in paths.iter().enumerate() {
        match fs::symlink_metadata(path) {
            Ok(meta) => items.push(Item {
                index,
                path: path.clone(),
                name: path.file_name().map(OsString::from).unwrap_or_default(),
                is_dir: meta.is_dir(),
            }),
            Err(_) => results[index] = Err(format!("“{}” no longer exists", path.display())),
        }
    }
    for chunk in chunks(items, dir_for) {
        if progress.is_cancelled() {
            break;
        }
        for (index, result) in trash_chunk(&chunk, delete, progress) {
            results[index] = result;
        }
    }
    results
}

/// Group `items` by Trash folder, in order, into calls of at most
/// `TRASH_CHUNK` items with no two of the same name: those land in the Trash
/// renamed, and apart they can still be told apart for Undo.
fn chunks(items: Vec<Item>, dir_for: impl Fn(&Path) -> PathBuf) -> Vec<Chunk> {
    let mut chunks: Vec<(Chunk, HashSet<OsString>)> = Vec::new();
    for item in items {
        let dir = dir_for(&item.path);
        let open = chunks.iter_mut().find(|(chunk, names)| {
            chunk.dir == dir && chunk.items.len() < TRASH_CHUNK && !names.contains(&item.name)
        });
        match open {
            Some((chunk, names)) => {
                names.insert(item.name.clone());
                chunk.items.push(item);
            }
            None => {
                let names = HashSet::from([item.name.clone()]);
                chunks.push((Chunk { dir, items: vec![item] }, names));
            }
        }
    }
    chunks.into_iter().map(|(chunk, _)| chunk).collect()
}

/// Trash one chunk in one call and find where its items went. If the call
/// fails, the items it left behind are tried one by one, so one bad item
/// doesn't keep the others and each gets its own error.
fn trash_chunk(
    chunk: &Chunk,
    delete: &(impl Fn(&[PathBuf]) -> Result<(), String> + Sync),
    progress: &Progress,
) -> Vec<(usize, TrashResult)> {
    let before = names_in(&chunk.dir);
    let outcome = delete_counted(&chunk.items, delete, progress);
    let (gone, left): (Vec<&Item>, Vec<&Item>) = match &outcome {
        Ok(()) => (chunk.items.iter().collect(), Vec::new()),
        Err(_) => chunk
            .items
            .iter()
            .partition(|item| fs::symlink_metadata(&item.path).is_err()),
    };
    let wanted: Vec<(&OsStr, bool)> = gone.iter().map(|i| (i.name.as_os_str(), i.is_dir)).collect();
    let located = locate_all(&chunk.dir, &before, &wanted);
    let mut results: Vec<(usize, TrashResult)> = gone
        .iter()
        .zip(located)
        .map(|(item, place)| (item.index, Ok(place)))
        .collect();
    match (outcome, &left[..]) {
        (Err(error), [only]) if chunk.items.len() == 1 => results.push((only.index, Err(error))),
        _ => {
            for item in left {
                let result = match progress.check() {
                    Err(cancelled) => Err(cancelled),
                    Ok(()) => {
                        let alone = Chunk {
                            dir: chunk.dir.clone(),
                            items: vec![Item { name: item.name.clone(), path: item.path.clone(), ..*item }],
                        };
                        trash_chunk(&alone, delete, progress).remove(0).1
                    }
                };
                results.push((item.index, result));
            }
        }
    }
    results
}

/// Run `delete` on `items` while counting each one that leaves its folder.
fn delete_counted(
    items: &[Item],
    delete: &(impl Fn(&[PathBuf]) -> Result<(), String> + Sync),
    progress: &Progress,
) -> Result<(), String> {
    let paths: Vec<PathBuf> = items.iter().map(|item| item.path.clone()).collect();
    let (running, ended) = std::sync::mpsc::channel::<()>();
    std::thread::scope(|scope| {
        scope.spawn(move || {
            let mut counted = vec![false; items.len()];
            loop {
                let last = !matches!(
                    ended.recv_timeout(WATCH_EVERY),
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout)
                );
                for (counted, item) in counted.iter_mut().zip(items) {
                    if !*counted && fs::symlink_metadata(&item.path).is_err() {
                        *counted = true;
                        progress.add_item();
                    }
                }
                if last {
                    break;
                }
            }
        });
        let outcome = delete(&paths);
        drop(running);
        outcome
    })
}

/// Where a trashed item came from: its `.trashinfo` sidecar first, then this
/// app's log, then Finder's "Put Back" entry. The sidecar is written together
/// with the item by whoever trashed it, so it is always about *this* item;
/// a log entry can outlive its item (restored by another app) and then
/// describe a later one that reused the name.
/// `trash_dir` is the Trash folder `trashed` sits in (see `trash_dir_for`).
pub fn origin_of(
    trashed: &Path,
    trash_dir: &Path,
    log: &TrashLog,
    put_back: &HashMap<String, PathBuf>,
) -> Option<PathBuf> {
    if let Some(origin) = sidecar_origin(trashed, trash_dir) {
        return Some(origin);
    }
    if let Some(item) = log.items.get(&*trashed.to_string_lossy()) {
        return Some(PathBuf::from(&item.origin));
    }
    let name = trashed.file_name()?.to_string_lossy();
    put_back.get(&*name).cloned()
}

/// Origin and trash date for each path in the Trash.
pub fn info(
    paths: &[String],
    home: &Path,
    log: &TrashLog,
    put_back: &HashMap<String, PathBuf>,
) -> Vec<TrashInfo> {
    paths
        .iter()
        .map(|p| {
            let path = Path::new(p);
            let trash_dir = trash_dir_for(path, home);
            // Like the origin, the log's date only counts without a sidecar.
            let logged = match sidecar_origin(path, &trash_dir) {
                Some(_) => None,
                None => log.items.get(p).map(|i| i.time),
            };
            TrashInfo {
                origin: origin_of(path, &trash_dir, log, put_back)
                    .map(|o| o.to_string_lossy().to_string()),
                trashed: logged.unwrap_or_else(|| changed_time(path)),
            }
        })
        .collect()
}

/// When the item was last moved or changed, which for a trashed item is
/// when it went to the Trash.
#[cfg(unix)]
fn changed_time(path: &Path) -> i64 {
    use std::os::unix::fs::MetadataExt;
    fs::symlink_metadata(path).map(|m| m.ctime()).unwrap_or(0)
}

#[cfg(not(unix))]
fn changed_time(_: &Path) -> i64 {
    0
}

/// Move `trashed`, which sits in `trash_dir`, back to `origin`, recreating
/// missing parent folders. Never overwrites: an existing item at `origin` is
/// an error.
pub fn restore(trashed: &Path, origin: &Path, trash_dir: &Path) -> Result<(), String> {
    if fs::symlink_metadata(trashed).is_err() {
        return Err(format!("“{}” is no longer in the Trash", trashed.display()));
    }
    if fs::symlink_metadata(origin).is_ok() {
        let folder = origin
            .parent()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "/".into());
        let name = origin
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        return Err(format!(
            "An item named “{}” already exists in “{}”",
            name, folder
        ));
    }
    if let Some(parent) = origin.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("Cannot recreate “{}”: {}", parent.display(), e))?;
    }
    fs::rename(trashed, origin).map_err(|e| format!("Cannot restore: {}", e))?;
    drop_sidecar(trashed, trash_dir);
    Ok(())
}

fn remove_tree(path: &Path) -> std::io::Result<()> {
    // remove_dir_all doesn't follow symlinks, so only the link itself goes.
    if fs::symlink_metadata(path)?.is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
}

/// Delete `path` for good. Refused unless it lies inside `trash_dir`.
pub fn delete_permanently(path: &Path, trash_dir: &Path) -> Result<(), String> {
    if fs::symlink_metadata(path).is_err() {
        return Err(format!("“{}” no longer exists", path.display()));
    }
    let inside = path
        .parent()
        .is_some_and(|parent| is_subpath(parent, trash_dir));
    if !inside {
        return Err("Only items in the Trash can be deleted permanently".into());
    }
    remove_tree(path).map_err(|e| format!("Cannot delete “{}”: {}", path.display(), e))?;
    drop_sidecar(path, trash_dir);
    Ok(())
}

/// Delete everything in a Trash. `trash_root` is the folder `home_trash`
/// returns, so on Linux both `files/` and `info/` are emptied.
/// Returns the error for the first item that couldn't be deleted, after
/// trying all of them.
pub fn empty(trash_root: &Path, progress: &Progress) -> Result<(), String> {
    let files = trash_root.join("files");
    // The layout is the one on disk: a Trash without `files/` keeps its
    // items directly inside, as macOS does.
    let freedesktop = info_dir_of(&files).filter(|_| files.is_dir());
    let (items, info) = match freedesktop {
        // freedesktop layout: keep `files/` and `info/` themselves.
        Some(info) => (files, Some(info)),
        None => (trash_root.to_path_buf(), None),
    };
    let mut first_error = None;
    for entry in fs::read_dir(&items).into_iter().flatten().flatten() {
        progress.check()?;
        let path = entry.path();
        match remove_tree(&path) {
            Ok(()) => drop_sidecar(&path, &items),
            Err(e) => {
                first_error.get_or_insert(format!(
                    "Cannot delete “{}”: {}",
                    entry.file_name().to_string_lossy(),
                    e
                ));
            }
        }
        progress.add_item();
    }
    // `.trashinfo` files whose item is already gone.
    for entry in info.iter().flat_map(fs::read_dir).flatten().flatten() {
        let _ = remove_tree(&entry.path());
    }
    first_error.map_or(Ok(()), Err)
}

// ---------------------------------------------------------------------------
// Finder's "Put Back" records
//
// A Trash's .DS_Store holds, for each item Finder trashed, `ptbL` (the
// original folder, relative to the volume root) and `ptbN` (the original
// name). The file is a B-tree inside a "buddy allocator" container; this
// reads just enough of it, and gives up quietly on anything unexpected.
// ---------------------------------------------------------------------------

/// Name in the Trash → original path, from `ds_store` on a volume mounted at `volume_root`.
pub fn read_put_back(ds_store: &Path, volume_root: &Path) -> HashMap<String, PathBuf> {
    let Ok(data) = fs::read(ds_store) else {
        return HashMap::new();
    };
    let mut folders: HashMap<String, String> = HashMap::new();
    let mut names: HashMap<String, String> = HashMap::new();
    for (file, code, value) in ds_records(&data).unwrap_or_default() {
        match &code {
            b"ptbL" => folders.insert(file, value),
            b"ptbN" => names.insert(file, value),
            _ => None,
        };
    }
    folders
        .into_iter()
        .map(|(file, folder)| {
            let name = names.get(&file).cloned().unwrap_or_else(|| file.clone());
            (
                file,
                volume_root.join(folder.trim_start_matches('/')).join(name),
            )
        })
        .collect()
}

fn be32(d: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(
        d.get(at..at.checked_add(4)?)?.try_into().ok()?,
    ))
}

fn utf16be(bytes: &[u8]) -> Option<String> {
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|c| u16::from_be_bytes([c[0], c[1]]))
        .collect();
    String::from_utf16(&units).ok()
}

/// Every string-valued record as (file name, code, value).
fn ds_records(d: &[u8]) -> Option<Vec<(String, [u8; 4], String)>> {
    if be32(d, 0)? != 1 || d.get(4..8)? != b"Bud1" {
        return None;
    }
    // Offsets in the file are relative to byte 4.
    let mut p = be32(d, 8)? as usize + 4;
    let count = be32(d, p)? as usize;
    if count > d.len() / 4 {
        return None;
    }
    p += 8;
    let addrs: Vec<u32> = (0..count)
        .map(|i| be32(d, p + 4 * i))
        .collect::<Option<_>>()?;
    p += 4 * count.div_ceil(256) * 256;

    // Table of contents: (name, block id) pairs; "DSDB" is the B-tree's master block.
    let mut master = None;
    let toc = be32(d, p)?;
    p += 4;
    for _ in 0..toc {
        let len = *d.get(p)? as usize;
        let name = d.get(p + 1..p + 1 + len)?;
        p += 1 + len;
        let id = be32(d, p)?;
        p += 4;
        if name == b"DSDB" {
            master = Some(id);
        }
    }
    let block =
        |id: u32| -> Option<usize> { Some((*addrs.get(id as usize)? & !0x1f) as usize + 4) };
    let root = be32(d, block(master?)?)?;
    let mut out = Vec::new();
    walk_node(d, &block, root, &mut out, 0)?;
    Some(out)
}

fn walk_node(
    d: &[u8],
    block: &dyn Fn(u32) -> Option<usize>,
    id: u32,
    out: &mut Vec<(String, [u8; 4], String)>,
    depth: u32,
) -> Option<()> {
    if depth > 16 {
        return None;
    }
    let mut p = block(id)?;
    let rightmost = be32(d, p)?;
    let count = be32(d, p + 4)?;
    p += 8;
    for _ in 0..count {
        if rightmost != 0 {
            walk_node(d, block, be32(d, p)?, out, depth + 1)?;
            p += 4;
        }
        p = read_record(d, p, out)?;
    }
    if rightmost != 0 {
        walk_node(d, block, rightmost, out, depth + 1)?;
    }
    Some(())
}

fn read_record(d: &[u8], mut p: usize, out: &mut Vec<(String, [u8; 4], String)>) -> Option<usize> {
    let len = be32(d, p)? as usize;
    p += 4;
    let file = utf16be(d.get(p..p.checked_add(2 * len)?)?)?;
    p += 2 * len;
    let code: [u8; 4] = d.get(p..p + 4)?.try_into().ok()?;
    let kind = d.get(p + 4..p + 8)?;
    p += 8;
    match kind {
        b"bool" => p += 1,
        b"long" | b"shor" | b"type" => p += 4,
        b"comp" | b"dutc" => p += 8,
        b"blob" => p += 4 + be32(d, p)? as usize,
        b"ustr" => {
            let n = be32(d, p)? as usize;
            let value = utf16be(d.get(p + 4..(p + 4).checked_add(2 * n)?)?)?;
            p += 4 + 2 * n;
            out.push((file, code, value));
        }
        _ => return None,
    }
    Some(p)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::progress::Progress;
    use tempfile::tempdir;

    #[test]
    fn find_new_spots_the_trashed_item() {
        let dir = tempdir().unwrap();
        let t = dir.path();
        fs::write(t.join("a.txt"), "old").unwrap();
        let before = names_in(t);

        // Renamed because "a.txt" was taken.
        fs::write(t.join("a 10.21.33.txt"), "new").unwrap();
        assert_eq!(
            find_new(t, &before, "a.txt", false),
            Some(t.join("a 10.21.33.txt"))
        );
        // Wrong kind.
        assert_eq!(find_new(t, &before, "a.txt", true), None);

        // Exact name wins over look-alikes.
        let before = names_in(t);
        fs::write(t.join("b.txt"), "").unwrap();
        fs::write(t.join("b 2.txt"), "").unwrap();
        assert_eq!(find_new(t, &before, "b.txt", false), Some(t.join("b.txt")));

        // Two look-alikes and no exact match: can't tell.
        let before = names_in(t);
        fs::write(t.join("c 2.txt"), "").unwrap();
        fs::write(t.join("c 3.txt"), "").unwrap();
        assert_eq!(find_new(t, &before, "c.txt", false), None);
    }

    #[test]
    fn restore_moves_back_and_recreates_folders() {
        let dir = tempdir().unwrap();
        let trashed = dir.path().join("trash/report.pdf");
        fs::create_dir_all(trashed.parent().unwrap()).unwrap();
        fs::write(&trashed, "pdf").unwrap();
        let origin = dir.path().join("gone/deeper/report.pdf");

        restore(&trashed, &origin, trashed.parent().unwrap()).unwrap();
        assert_eq!(fs::read_to_string(&origin).unwrap(), "pdf");
        assert!(!trashed.exists());
    }

    #[test]
    fn restore_never_overwrites() {
        let dir = tempdir().unwrap();
        let trashed = dir.path().join("t.txt");
        let origin = dir.path().join("o.txt");
        fs::write(&trashed, "trashed").unwrap();
        fs::write(&origin, "keep me").unwrap();

        let err = restore(&trashed, &origin, dir.path()).unwrap_err();
        assert!(err.contains("already exists"), "{}", err);
        assert_eq!(fs::read_to_string(&origin).unwrap(), "keep me");
        assert!(trashed.exists());
        assert!(
            restore(
                &dir.path().join("missing"),
                &dir.path().join("x"),
                dir.path()
            )
            .is_err()
        );
    }

    // --- layout helpers --------------------------------------------------

    /// The folder trashed items actually sit in, for this platform's layout:
    /// the Trash root itself on macOS, its `files/` subfolder on Linux.
    fn items_dir(trash_root: &Path) -> PathBuf {
        if cfg!(target_os = "macos") {
            trash_root.to_path_buf()
        } else {
            fs::create_dir_all(trash_root.join("files")).unwrap();
            fs::create_dir_all(trash_root.join("info")).unwrap();
            trash_root.join("files")
        }
    }

    #[test]
    fn delete_permanently_only_inside_the_trash() {
        let dir = tempdir().unwrap();
        let root = dir.path().join("Trash");
        let trash = items_dir(&root);
        fs::create_dir_all(trash.join("folder/sub")).unwrap();
        fs::write(trash.join("folder/sub/f.txt"), "x").unwrap();
        fs::write(trash.join("file.txt"), "x").unwrap();
        let outside = dir.path().join("precious.txt");
        fs::write(&outside, "keep").unwrap();

        assert!(delete_permanently(&outside, &trash).is_err());
        assert!(outside.exists());
        assert!(
            delete_permanently(&trash, &trash).is_err(),
            "the Trash itself"
        );
        assert!(trash.exists());
        let sneaky = trash.join("../precious.txt");
        assert!(delete_permanently(&sneaky, &trash).is_err());
        assert!(outside.exists());

        delete_permanently(&trash.join("file.txt"), &trash).unwrap();
        delete_permanently(&trash.join("folder"), &trash).unwrap();
        assert_eq!(fs::read_dir(&trash).unwrap().count(), 0);
    }

    #[cfg(unix)]
    #[test]
    fn deleting_a_link_keeps_its_target() {
        let dir = tempdir().unwrap();
        let trash = items_dir(&dir.path().join("Trash"));
        fs::create_dir_all(&trash).unwrap();
        let target = dir.path().join("target");
        fs::create_dir(&target).unwrap();
        fs::write(target.join("data.txt"), "keep").unwrap();
        std::os::unix::fs::symlink(&target, trash.join("link")).unwrap();

        delete_permanently(&trash.join("link"), &trash).unwrap();
        assert!(target.join("data.txt").exists());
        std::os::unix::fs::symlink(&target, trash.join("link2")).unwrap();
        empty(&dir.path().join("Trash"), &Progress::default()).unwrap();
        assert!(target.join("data.txt").exists());
    }

    #[test]
    fn empty_removes_everything_in_the_trash_only() {
        let dir = tempdir().unwrap();
        let root = dir.path().join("Trash");
        let trash = items_dir(&root);
        fs::create_dir_all(trash.join("a/b")).unwrap();
        fs::write(trash.join("a/b/c.txt"), "x").unwrap();
        fs::write(trash.join(".DS_Store"), "x").unwrap();
        fs::write(dir.path().join("outside.txt"), "x").unwrap();

        empty(&root, &Progress::default()).unwrap();
        assert!(root.exists());
        assert_eq!(fs::read_dir(&trash).unwrap().count(), 0);
        assert!(dir.path().join("outside.txt").exists());
        empty(&dir.path().join("no-trash-here"), &Progress::default()).unwrap();
    }

    #[test]
    fn log_records_and_forgets_items_that_left() {
        let dir = tempdir().unwrap();
        let a = dir.path().join("a");
        fs::write(&a, "").unwrap();
        let mut log = TrashLog::default();
        let gone = dir.path().join("gone");
        record(
            &mut log,
            &[(a.clone(), "/Users/me/a".into()), (gone, "/Users/me/gone".into())],
            100,
        );
        assert_eq!(log.items.len(), 2);
        record(&mut log, &[(a.clone(), "/Users/me/a".into())], 102);
        assert_eq!(log.items.len(), 1, "the missing item is dropped");

        let info = info(
            &[a.to_string_lossy().to_string()],
            dir.path(),
            &log,
            &HashMap::new(),
        );
        assert_eq!(
            info[0],
            TrashInfo {
                origin: Some("/Users/me/a".into()),
                trashed: 102
            }
        );
    }

    /// Stands in for Finder: moves each path into `trash` (renaming a clash
    /// "name 2.ext"), stopping with an error at `fail_on`, and logs the calls.
    fn fake_finder<'a>(
        trash: &'a Path,
        calls: &'a std::sync::Mutex<Vec<Vec<String>>>,
        fail_on: Option<&'a str>,
    ) -> impl Fn(&[PathBuf]) -> Result<(), String> + Sync + 'a {
        move |paths: &[PathBuf]| {
            let names = paths.iter().map(|p| p.file_name().unwrap().to_string_lossy().to_string());
            calls.lock().unwrap().push(names.collect());
            for path in paths {
                let name = path.file_name().unwrap().to_string_lossy().to_string();
                if fail_on == Some(name.as_str()) {
                    return Err(format!("Failed to move to Trash: {name} is locked"));
                }
                let mut target = trash.join(&name);
                if target.exists() {
                    let p = Path::new(&name);
                    let stem = p.file_stem().unwrap().to_string_lossy();
                    let ext = p.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
                    target = trash.join(format!("{stem} 2{ext}"));
                }
                fs::rename(path, target).unwrap();
            }
            Ok(())
        }
    }

    fn files(dir: &Path, names: &[&str]) -> Vec<PathBuf> {
        fs::create_dir_all(dir).unwrap();
        names
            .iter()
            .map(|name| {
                let path = dir.join(name);
                fs::write(&path, *name).unwrap();
                path
            })
            .collect()
    }

    #[test]
    fn a_batch_is_one_call_and_finds_every_item() {
        let dir = tempdir().unwrap();
        let trash = dir.path().join("Trash");
        fs::create_dir(&trash).unwrap();
        fs::write(trash.join("a.txt"), "older").unwrap();
        let mut paths = files(&dir.path().join("src"), &["a.txt", "b.txt"]);
        fs::create_dir(dir.path().join("src/folder")).unwrap();
        paths.push(dir.path().join("src/folder"));
        let calls = std::sync::Mutex::new(Vec::new());
        let progress = Progress::default();

        let results =
            trash_many_with(&paths, |_| trash.clone(), &fake_finder(&trash, &calls, None), &progress);

        assert_eq!(calls.lock().unwrap().len(), 1, "one call, so one Finder sound");
        assert_eq!(
            results,
            vec![
                Ok(Some(trash.join("a 2.txt"))),
                Ok(Some(trash.join("b.txt"))),
                Ok(Some(trash.join("folder"))),
            ]
        );
        assert_eq!(progress.snapshot().items, 3);
    }

    #[test]
    fn same_names_and_other_trash_folders_go_in_separate_calls() {
        let dir = tempdir().unwrap();
        let (home_trash, usb_trash) = (dir.path().join("Trash"), dir.path().join("usb/.Trashes"));
        fs::create_dir(&home_trash).unwrap();
        fs::create_dir_all(&usb_trash).unwrap();
        let mut paths = files(&dir.path().join("one"), &["same.txt", "other.txt"]);
        paths.extend(files(&dir.path().join("two"), &["same.txt"]));
        paths.extend(files(&dir.path().join("usb"), &["stick.txt"]));
        let dir_for = |p: &Path| {
            if p.starts_with(dir.path().join("usb")) { usb_trash.clone() } else { home_trash.clone() }
        };
        let calls = std::sync::Mutex::new(Vec::new());
        // One fake Finder per Trash folder would be truer; one is enough to
        // count calls, so the USB item lands in the home "Trash" too.
        let finder = fake_finder(&home_trash, &calls, None);

        let results = trash_many_with(&paths, dir_for, &finder, &Progress::default());

        assert_eq!(
            *calls.lock().unwrap(),
            vec![vec!["same.txt", "other.txt"], vec!["same.txt"], vec!["stick.txt"]],
        );
        assert_eq!(results[0], Ok(Some(home_trash.join("same.txt"))));
        assert_eq!(results[2], Ok(Some(home_trash.join("same 2.txt"))), "apart, both are found");
        assert!(paths.iter().all(|p| !p.exists()));
    }

    #[test]
    fn a_failed_call_retries_what_it_left_one_by_one() {
        let dir = tempdir().unwrap();
        let trash = dir.path().join("Trash");
        fs::create_dir(&trash).unwrap();
        let paths = files(&dir.path().join("src"), &["a.txt", "locked.txt", "c.txt"]);
        let calls = std::sync::Mutex::new(Vec::new());
        let progress = Progress::default();

        let results = trash_many_with(
            &paths,
            |_| trash.clone(),
            &fake_finder(&trash, &calls, Some("locked.txt")),
            &progress,
        );

        assert_eq!(
            *calls.lock().unwrap(),
            vec![vec!["a.txt", "locked.txt", "c.txt"], vec!["locked.txt"], vec!["c.txt"]],
        );
        assert_eq!(results[0], Ok(Some(trash.join("a.txt"))));
        assert_eq!(results[1], Err("Failed to move to Trash: locked.txt is locked".into()));
        assert_eq!(results[2], Ok(Some(trash.join("c.txt"))));
        assert!(paths[1].exists(), "the locked item stays where it was");
        assert_eq!(progress.snapshot().items, 2);
    }

    #[test]
    fn a_vanished_item_is_reported_and_never_passed_on() {
        let dir = tempdir().unwrap();
        let trash = dir.path().join("Trash");
        fs::create_dir(&trash).unwrap();
        let mut paths = files(&dir.path().join("src"), &["here.txt"]);
        paths.insert(0, dir.path().join("src/gone.txt"));
        let calls = std::sync::Mutex::new(Vec::new());

        let results =
            trash_many_with(&paths, |_| trash.clone(), &fake_finder(&trash, &calls, None), &Progress::default());

        assert!(matches!(&results[0], Err(e) if e.contains("no longer exists")));
        assert_eq!(results[1], Ok(Some(trash.join("here.txt"))));
        assert_eq!(*calls.lock().unwrap(), vec![vec!["here.txt"]]);
    }

    #[test]
    fn cancel_stops_before_the_next_call() {
        let dir = tempdir().unwrap();
        let trash = dir.path().join("Trash");
        fs::create_dir(&trash).unwrap();
        let names: Vec<String> = (0..TRASH_CHUNK + 1).map(|i| format!("f{i}.txt")).collect();
        let names: Vec<&str> = names.iter().map(String::as_str).collect();
        let paths = files(&dir.path().join("src"), &names);
        let calls = std::sync::Mutex::new(Vec::new());
        let progress = Progress::default();
        let finder = fake_finder(&trash, &calls, None);
        // Cancelled while the first call runs: that call still finishes.
        let cancelling = |batch: &[PathBuf]| {
            progress.cancel();
            finder(batch)
        };

        let results = trash_many_with(&paths, |_| trash.clone(), &cancelling, &progress);

        assert_eq!(calls.lock().unwrap().len(), 1);
        assert!(results[..TRASH_CHUNK].iter().all(|r| matches!(r, Ok(Some(_)))));
        assert_eq!(results[TRASH_CHUNK], Err(crate::progress::CANCELLED.into()));
        assert!(paths[TRASH_CHUNK].exists());
        assert_eq!(progress.snapshot().items, TRASH_CHUNK as u64);
    }

    #[test]
    fn routed_batches_send_local_paths_together_and_keep_the_order() {
        let paths: Vec<String> = ["/a", "gdrive://x", "/b", "share://s"].map(String::from).into();
        let elsewhere = |path: &str| match path {
            "gdrive://x" => Some(Ok(Some("gdrive://x".to_string()))),
            "share://s" => Some(Err("Shared folders are read only.".to_string())),
            _ => None,
        };
        let mut batches = Vec::new();
        let local = |origins: &[PathBuf]| {
            batches.push(origins.to_vec());
            vec![Ok(Some(PathBuf::from("/Trash/a"))), Err("locked".to_string())]
        };

        let (results, moves) = trash_routed(&paths, elsewhere, local);

        assert_eq!(batches, vec![vec![PathBuf::from("/a"), PathBuf::from("/b")]]);
        assert_eq!(
            results,
            vec![
                Ok(Some("/Trash/a".into())),
                Ok(Some("gdrive://x".into())),
                Err("locked".into()),
                Err("Shared folders are read only.".into()),
            ]
        );
        assert_eq!(moves, vec![(PathBuf::from("/Trash/a"), PathBuf::from("/a"))]);
    }

    #[test]
    fn look_alikes_that_fit_two_items_stay_unknown() {
        let dir = tempdir().unwrap();
        let t = dir.path();
        let before = names_in(t);
        fs::write(t.join("a 2.txt"), "").unwrap();
        fs::write(t.join("b.txt"), "").unwrap();
        // "a 2.txt" could be either "a.txt" or "a.md": can't tell.
        let items = [(OsStr::new("a.txt"), false), (OsStr::new("a.md"), false), (OsStr::new("b.txt"), false)];
        assert_eq!(locate_all(t, &before, &items), vec![None, None, Some(t.join("b.txt"))]);
    }

    #[test]
    fn trash_many_moves_items_to_the_real_trash() {
        let dir = tempdir().unwrap();
        let paths = files(dir.path(), &["batch-1.txt", "batch-2.txt"]);
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| dir.path().to_path_buf());

        let results = trash_many(&paths, &home, &Progress::default());
        assert!(paths.iter().all(|p| !p.exists()));
        for (result, name) in results.into_iter().zip(["batch-1.txt", "batch-2.txt"]) {
            if let Some(p) = result.unwrap() {
                assert_eq!(fs::read_to_string(&p).unwrap(), name);
                let _ = fs::remove_file(p);
            }
        }
    }

    #[test]
    fn trash_and_locate_moves_the_item_out() {
        let dir = tempdir().unwrap();
        let f = dir.path().join("trash-me.txt");
        fs::write(&f, "bye").unwrap();
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| dir.path().to_path_buf());

        let found = trash_and_locate(&f, &home).unwrap();
        assert!(!f.exists());
        if let Some(p) = found {
            assert_eq!(fs::read_to_string(&p).unwrap(), "bye");
            let _ = fs::remove_file(p);
        }
        assert!(trash_and_locate(&f, &home).is_err());
    }

    // --- .DS_Store -------------------------------------------------------

    fn utf16(s: &str) -> Vec<u8> {
        s.encode_utf16().flat_map(|u| u.to_be_bytes()).collect()
    }

    fn ustr_record(file: &str, code: &[u8; 4], value: &str) -> Vec<u8> {
        let mut r = (file.encode_utf16().count() as u32).to_be_bytes().to_vec();
        r.extend(utf16(file));
        r.extend(code);
        r.extend(b"ustr");
        r.extend((value.encode_utf16().count() as u32).to_be_bytes());
        r.extend(utf16(value));
        r
    }

    fn long_record(file: &str, code: &[u8; 4]) -> Vec<u8> {
        let mut r = (file.encode_utf16().count() as u32).to_be_bytes().to_vec();
        r.extend(utf16(file));
        r.extend(code);
        r.extend(b"long");
        r.extend(7u32.to_be_bytes());
        r
    }

    fn node(rightmost: u32, items: &[(Option<u32>, Vec<u8>)]) -> Vec<u8> {
        let mut n = rightmost.to_be_bytes().to_vec();
        n.extend((items.len() as u32).to_be_bytes());
        for (child, rec) in items {
            if let Some(c) = child {
                n.extend(c.to_be_bytes());
            }
            n.extend(rec);
        }
        n
    }

    /// A .DS_Store in the layout Finder writes: blocks at 32-byte-aligned
    /// offsets, block 0 the master record, the rest B-tree nodes.
    fn ds_store(nodes: &[Vec<u8>], root: u32) -> Vec<u8> {
        let mut d = vec![0u8; 0x8000];
        let put = |d: &mut Vec<u8>, at: usize, bytes: &[u8]| {
            d[at..at + bytes.len()].copy_from_slice(bytes)
        };
        let mut addrs = vec![];
        let master_at = 0x20usize;
        addrs.push(master_at as u32 | 5);
        let mut master = root.to_be_bytes().to_vec();
        master.extend([0u8; 16]);
        put(&mut d, master_at + 4, &master);
        for (i, n) in nodes.iter().enumerate() {
            let at = 0x1000 * (i + 1);
            addrs.push(at as u32 | 12);
            put(&mut d, at + 4, n);
        }
        let alloc_at = 0x6000usize;
        let mut a = (addrs.len() as u32).to_be_bytes().to_vec();
        a.extend([0u8; 4]);
        for x in &addrs {
            a.extend(x.to_be_bytes());
        }
        a.extend(vec![0u8; 4 * (256 - addrs.len())]);
        a.extend(1u32.to_be_bytes());
        a.push(4);
        a.extend(b"DSDB");
        a.extend(0u32.to_be_bytes());
        put(&mut d, alloc_at + 4, &a);
        put(&mut d, 0, &1u32.to_be_bytes());
        put(&mut d, 4, b"Bud1");
        put(&mut d, 8, &(alloc_at as u32).to_be_bytes());
        put(&mut d, 12, &0x800u32.to_be_bytes());
        put(&mut d, 16, &(alloc_at as u32).to_be_bytes());
        d
    }

    #[test]
    fn reads_put_back_records_from_a_leaf() {
        let dir = tempdir().unwrap();
        let leaf = node(
            0,
            &[
                (None, long_record("notes 10.21.33.txt", b"Iloc")),
                (
                    None,
                    ustr_record("notes 10.21.33.txt", b"ptbL", "Users/me/Documents/"),
                ),
                (
                    None,
                    ustr_record("notes 10.21.33.txt", b"ptbN", "notes.txt"),
                ),
                (None, ustr_record("Photos", b"ptbL", "Users/me/Pictures/")),
            ],
        );
        let path = dir.path().join(".DS_Store");
        fs::write(&path, ds_store(&[leaf], 1)).unwrap();

        let map = read_put_back(&path, Path::new("/"));
        assert_eq!(
            map.get("notes 10.21.33.txt"),
            Some(&PathBuf::from("/Users/me/Documents/notes.txt"))
        );
        assert_eq!(
            map.get("Photos"),
            Some(&PathBuf::from("/Users/me/Pictures/Photos"))
        );
        assert_eq!(map.len(), 2);
    }

    #[test]
    fn walks_internal_nodes() {
        let dir = tempdir().unwrap();
        let left = node(0, &[(None, ustr_record("a.txt", b"ptbL", "Users/me/"))]);
        let right = node(
            0,
            &[(None, ustr_record("c.txt", b"ptbL", "Users/me/Desktop/"))],
        );
        let root = node(
            2,
            &[(Some(1), ustr_record("b.txt", b"ptbL", "Users/me/Music/"))],
        );
        let path = dir.path().join(".DS_Store");
        fs::write(&path, ds_store(&[left, right, root], 3)).unwrap();

        let map = read_put_back(&path, Path::new("/"));
        assert_eq!(map.len(), 3);
        assert_eq!(map["c.txt"], PathBuf::from("/Users/me/Desktop/c.txt"));
        assert_eq!(map["b.txt"], PathBuf::from("/Users/me/Music/b.txt"));
    }

    #[test]
    fn bad_ds_store_files_are_ignored() {
        let dir = tempdir().unwrap();
        let path = dir.path().join(".DS_Store");
        assert!(read_put_back(&path, Path::new("/")).is_empty());
        fs::write(&path, b"\x00\x00\x00\x01Bud1\xff\xff\xff\xff").unwrap();
        assert!(read_put_back(&path, Path::new("/")).is_empty());
        let mut good = ds_store(
            &[node(0, &[(None, ustr_record("x", b"ptbL", "Users/"))])],
            1,
        );
        good.truncate(0x1010);
        fs::write(&path, &good).unwrap();
        assert!(read_put_back(&path, Path::new("/")).is_empty());
    }

    // --- freedesktop .trashinfo ------------------------------------------

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn trashinfo_gives_the_original_path() {
        let dir = tempdir().unwrap();
        let info = dir.path().join("info");
        let files = dir.path().join("files");
        fs::create_dir_all(&info).unwrap();
        fs::create_dir_all(&files).unwrap();
        let trashed = files.join("notes.txt");
        fs::write(
            info.join("notes.txt.trashinfo"),
            "[Trash Info]\nPath=/home/me/My%20Notes.txt\nDeletionDate=2026-10-24T10:11:12\n",
        )
        .unwrap();

        assert_eq!(
            sidecar_origin(&trashed, &files),
            Some(PathBuf::from("/home/me/My Notes.txt"))
        );
        // A relative or missing Path is not usable.
        fs::write(
            info.join("notes.txt.trashinfo"),
            "[Trash Info]\nPath=relative.txt\n",
        )
        .unwrap();
        assert_eq!(sidecar_origin(&trashed, &files), None);
        fs::remove_file(info.join("notes.txt.trashinfo")).unwrap();
        assert_eq!(sidecar_origin(&trashed, &files), None);
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn origin_prefers_the_sidecar_over_the_log() {
        let dir = tempdir().unwrap();
        let info = dir.path().join("info");
        let files = dir.path().join("files");
        fs::create_dir_all(&info).unwrap();
        fs::create_dir_all(&files).unwrap();
        let trashed = files.join("a.txt");
        fs::write(
            info.join("a.txt.trashinfo"),
            "[Trash Info]\nPath=/home/me/a.txt\n",
        )
        .unwrap();

        // A stale log entry, left behind when another app restored an earlier
        // `a.txt`, must not send this one to the wrong folder.
        let mut log = TrashLog::default();
        log.items.insert(
            trashed.to_string_lossy().to_string(),
            LoggedItem {
                origin: "/home/me/stale/a.txt".into(),
                time: 5,
            },
        );
        assert_eq!(
            origin_of(&trashed, &files, &log, &HashMap::new()),
            Some(PathBuf::from("/home/me/a.txt"))
        );
        // Without a sidecar the log answers.
        fs::remove_file(info.join("a.txt.trashinfo")).unwrap();
        assert_eq!(
            origin_of(&trashed, &files, &log, &HashMap::new()),
            Some(PathBuf::from("/home/me/stale/a.txt"))
        );
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn restore_and_delete_drop_the_sidecar() {
        let dir = tempdir().unwrap();
        let root = dir.path().join("Trash");
        let files = items_dir(&root);
        let info = root.join("info");

        // Restore: the item goes back and its sidecar is gone.
        let trashed = files.join("a.txt");
        fs::write(&trashed, "hi").unwrap();
        fs::write(
            info.join("a.txt.trashinfo"),
            "[Trash Info]\nPath=/home/me/a.txt\n",
        )
        .unwrap();
        let origin = dir.path().join("back/a.txt");
        restore(&trashed, &origin, &files).unwrap();
        assert_eq!(fs::read_to_string(&origin).unwrap(), "hi");
        assert!(
            !info.join("a.txt.trashinfo").exists(),
            "sidecar removed on restore"
        );

        // Delete permanently: same, so `info/` can't collect orphans.
        let trashed = files.join("b.txt");
        fs::write(&trashed, "bye").unwrap();
        fs::write(
            info.join("b.txt.trashinfo"),
            "[Trash Info]\nPath=/home/me/b.txt\n",
        )
        .unwrap();
        delete_permanently(&trashed, &files).unwrap();
        assert!(!trashed.exists());
        assert!(
            !info.join("b.txt.trashinfo").exists(),
            "sidecar removed on delete"
        );
    }

    /// A trashed project with its own `files/` and `info/` folders, like the
    /// Trash's layout one level down.
    #[cfg(not(target_os = "macos"))]
    fn trashed_lookalike(root: &Path) -> (PathBuf, PathBuf) {
        let files = items_dir(root);
        let project = files.join("project");
        fs::create_dir_all(project.join("files")).unwrap();
        fs::create_dir_all(project.join("info")).unwrap();
        fs::write(project.join("files/a.txt"), "x").unwrap();
        fs::write(
            project.join("info/a.txt.trashinfo"),
            "[Trash Info]\nPath=/etc/a.txt\n",
        )
        .unwrap();
        (files, project)
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn deleting_inside_a_trashed_folder_keeps_its_other_files() {
        let dir = tempdir().unwrap();
        let (files, project) = trashed_lookalike(&dir.path().join("Trash"));

        delete_permanently(&project.join("files/a.txt"), &files).unwrap();
        assert!(!project.join("files/a.txt").exists());
        assert_eq!(
            fs::read_to_string(project.join("info/a.txt.trashinfo")).unwrap(),
            "[Trash Info]\nPath=/etc/a.txt\n",
            "a file that only looks like a record is user data"
        );
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn restoring_from_inside_a_trashed_folder_keeps_its_other_files() {
        let dir = tempdir().unwrap();
        let (files, project) = trashed_lookalike(&dir.path().join("Trash"));

        let origin = dir.path().join("back/a.txt");
        restore(&project.join("files/a.txt"), &origin, &files).unwrap();
        assert!(origin.exists());
        assert!(project.join("info/a.txt.trashinfo").exists());
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn lookalike_records_inside_a_trashed_folder_are_not_read() {
        let dir = tempdir().unwrap();
        let (files, project) = trashed_lookalike(&dir.path().join("Trash"));
        let nested = project.join("files/a.txt");

        // Its "record" would send Restore to /etc/a.txt.
        assert_eq!(sidecar_origin(&nested, &files), None);
        assert_eq!(
            origin_of(&nested, &files, &TrashLog::default(), &HashMap::new()),
            None
        );
        // Asked about the wrong Trash folder, a real record is ignored too.
        fs::write(
            dir.path().join("Trash/info/top.txt.trashinfo"),
            "[Trash Info]\nPath=/home/me/top.txt\n",
        )
        .unwrap();
        fs::write(files.join("top.txt"), "x").unwrap();
        assert_eq!(sidecar_origin(&files.join("top.txt"), &project), None);
        assert_eq!(
            sidecar_origin(&files.join("top.txt"), &files),
            Some(PathBuf::from("/home/me/top.txt"))
        );
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn emptying_keeps_the_files_and_info_folders() {
        let dir = tempdir().unwrap();
        let root = dir.path().join("Trash");
        let files = items_dir(&root);
        let info = root.join("info");
        fs::write(files.join("a.txt"), "x").unwrap();
        fs::write(info.join("a.txt.trashinfo"), "x").unwrap();

        empty(&root, &Progress::default()).unwrap();
        assert!(files.is_dir(), "files/ survives so the Trash still works");
        assert!(info.is_dir(), "info/ survives too");
        assert_eq!(fs::read_dir(&files).unwrap().count(), 0);
        assert_eq!(fs::read_dir(&info).unwrap().count(), 0);
    }

    #[test]
    fn a_missing_freedesktop_trash_is_created_without_touching_contents() {
        let dir = tempdir().unwrap();
        let root = dir.path().join("Trash");
        create_freedesktop_trash(&root);
        assert!(root.join("files").is_dir());
        assert!(root.join("info").is_dir());
        fs::write(root.join("files/keep.txt"), "x").unwrap();
        fs::write(root.join("info/keep.txt.trashinfo"), "x").unwrap();
        create_freedesktop_trash(&root);
        assert!(root.join("files/keep.txt").exists());
        assert!(root.join("info/keep.txt.trashinfo").exists());
    }

    #[test]
    fn home_trash_matches_the_platform_layout() {
        let home = Path::new("/home/me");
        let root = home_trash(home);
        if cfg!(target_os = "macos") {
            assert_eq!(root, PathBuf::from("/home/me/.Trash"));
        } else {
            assert!(root.ends_with("Trash"));
            assert!(!root.ends_with("Trash/files"));
        }
    }

    /// A freedesktop Trash (`files/` + `info/`) holding a, b and c.
    fn freedesktop_with_three(root: &Path) {
        fs::create_dir_all(root.join("files")).unwrap();
        fs::create_dir_all(root.join("info")).unwrap();
        for name in ["a", "b", "c"] {
            fs::write(root.join("files").join(name), "x").unwrap();
            fs::write(root.join("info").join(format!("{name}.trashinfo")), "x").unwrap();
        }
    }

    /// A macOS-style Trash (items directly inside) holding a, b and c.
    fn macos_with_three(root: &Path) {
        fs::create_dir_all(root).unwrap();
        for name in ["a", "b", "c"] {
            fs::write(root.join(name), "x").unwrap();
        }
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn empty_counts_items_freedesktop() {
        let dir = tempdir().unwrap();
        let root = dir.path().join("Trash");
        freedesktop_with_three(&root);
        let progress = Progress::default();
        empty(&root, &progress).unwrap();
        assert_eq!(progress.snapshot().items, 3);
        assert_eq!(fs::read_dir(root.join("files")).unwrap().count(), 0);
        assert_eq!(fs::read_dir(root.join("info")).unwrap().count(), 0);
    }

    #[test]
    fn empty_counts_items_macos_layout() {
        let dir = tempdir().unwrap();
        let root = dir.path().join(".Trash");
        macos_with_three(&root);
        let progress = Progress::default();
        empty(&root, &progress).unwrap();
        assert_eq!(progress.snapshot().items, 3);
        assert_eq!(fs::read_dir(&root).unwrap().count(), 0);
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn cancelled_empty_keeps_the_rest_freedesktop() {
        let dir = tempdir().unwrap();
        let root = dir.path().join("Trash");
        freedesktop_with_three(&root);
        let progress = Progress::default();
        progress.cancel();
        assert_eq!(empty(&root, &progress), Err("Cancelled.".to_string()));
        assert_eq!(fs::read_dir(root.join("files")).unwrap().count(), 3);
        assert_eq!(fs::read_dir(root.join("info")).unwrap().count(), 3);
    }

    #[test]
    fn cancelled_empty_keeps_the_rest_macos_layout() {
        let dir = tempdir().unwrap();
        let root = dir.path().join(".Trash");
        macos_with_three(&root);
        let progress = Progress::default();
        progress.cancel();
        assert_eq!(empty(&root, &progress), Err("Cancelled.".to_string()));
        assert_eq!(fs::read_dir(&root).unwrap().count(), 3);
    }
}
