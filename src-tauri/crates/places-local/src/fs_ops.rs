use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::is_combining_mark;

#[cfg(target_os = "linux")]
use places::copy_counted;
use places::{CANCELLED, Progress, Refusal, cancelled_io, is_cancel};

use crate::rename;

/// The hidden folder Keep Versions keeps a folder's history in (D10):
/// every listing, search and count leaves it out, even with Show Hidden
/// Files. Keep Versions itself stays in the app (`versions`), which names
/// it from here.
pub const VERSIONS_DIR: &str = ".versions";

/// True when any component is the reserved `.versions` directory (D10).
pub fn is_internal_path(path: &Path) -> bool {
    path.components()
        .any(|part| part.as_os_str().to_string_lossy() == VERSIONS_DIR)
}

pub fn make_dir(path: String) -> Result<(), String> {
    let p = Path::new(&path);
    if p.exists() {
        return Err(format!("Path already exists: {}", path));
    }
    std::fs::create_dir(p).map_err(|e| format!("Failed to create directory: {}", e))
}

/// Rename an item in its folder, never over another one: the rename itself
/// refuses a taken name (`rename::rename_in_place`), so an item that
/// took it after the view was listed, or a link whose target is gone, stays.
pub fn rename_entry(from: String, to: String) -> Result<(), String> {
    rename::rename_in_place(Path::new(&from), Path::new(&to)).map_err(|e| match e.kind() {
        std::io::ErrorKind::AlreadyExists => format!("Target already exists: {}", to),
        _ => format!("Failed to rename: {}", e),
    })
}

/// A listing row; shared by every place through the `places` crate.
pub use places::Entry;

pub fn read_entries(path: &Path, show_hidden: bool) -> Result<Vec<Entry>, String> {
    let read_dir =
        std::fs::read_dir(path).map_err(|e| format!("Failed to read directory: {}", e))?;

    let mut entries = Vec::new();
    for dir_entry in read_dir.flatten() {
        let name = dir_entry.file_name().to_string_lossy().to_string();
        // The history directory is always hidden, even with Show Hidden Files.
        if name == VERSIONS_DIR {
            continue;
        }
        if name.starts_with('.') && !show_hidden {
            continue;
        }
        if let Some(entry) = make_entry(&dir_entry.path(), name) {
            entries.push(entry);
        }
    }
    Ok(entries)
}

/// The entry for a single path, or `None` if nothing is there.
pub fn entry_for(path: &Path) -> Option<Entry> {
    let name = path.file_name()?.to_string_lossy().to_string();
    make_entry(path, name)
}

/// Entries for a list of paths from anywhere (Starred), skipping ones that no
/// longer exist and, unless `show_hidden`, hidden ones.
pub fn entries_for(paths: &[String], show_hidden: bool) -> Vec<Entry> {
    paths
        .iter()
        .filter(|p| !is_internal_path(Path::new(p)))
        .filter_map(|p| entry_for(Path::new(p)))
        .filter(|e| show_hidden || !e.is_hidden)
        .collect()
}

fn make_entry(entry_path: &Path, name: String) -> Option<Entry> {
    use std::time::UNIX_EPOCH;

    let secs = |t: std::io::Result<std::time::SystemTime>| {
        t.ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
    };

    // Use symlink_metadata so broken symlinks don't error the whole listing
    let symlink_meta = std::fs::symlink_metadata(entry_path).ok()?;
    let is_symlink = symlink_meta.file_type().is_symlink();

    // For is_dir and size, follow the link if it's a symlink; a broken link
    // keeps what the link itself reports.
    let meta = if is_symlink {
        std::fs::metadata(entry_path).unwrap_or(symlink_meta)
    } else {
        symlink_meta
    };

    Some(Entry {
        is_hidden: name.starts_with('.'),
        name,
        path: entry_path.to_string_lossy().to_string(),
        is_dir: meta.is_dir(),
        size: meta.len(),
        modified: secs(meta.modified()),
        is_symlink,
        cloud: None,
    })
}

// Functions that touch many files are exposed by `lib.rs` as commands that
// run on Tauri's blocking-thread pool, so big listings or copies never stall
// the window or other commands.
pub fn list_dir(path: String, show_hidden: bool) -> Result<Vec<Entry>, String> {
    read_entries(Path::new(&path), show_hidden)
}

/// Change the three permission triples, keeping any setuid/setgid/sticky bits.
pub fn set_permissions(path: String, mode: u32) -> Result<(), String> {    if mode & !0o777 != 0 {
        return Err("Invalid permissions.".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let meta = std::fs::metadata(&path)
            .map_err(|e| format!("Failed to read permissions: {e}"))?;
        let mut permissions = meta.permissions();
        permissions.set_mode((permissions.mode() & 0o7000) | mode);
        std::fs::set_permissions(&path, permissions)
            .map_err(|e| format!("Failed to change permissions: {e}"))
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Err("Changing permissions is not supported on this platform.".into())
    }
}

/// Create an empty file, refusing to replace anything.
pub fn create_file(path: String) -> Result<(), String> {
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map(|_| ())
        .map_err(|e| format!("Could not create the file: {e}"))
}

/// Create a symlink beside `path`, named "Link to <name>" (or a numbered
/// variant), pointing at the original absolute path.
pub fn create_link(path: String) -> Result<String, String> {
    let src = Path::new(&path);
    let parent = src
        .parent()
        .ok_or_else(|| "This item has no parent folder.".to_string())?;
    let name = src
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .ok_or_else(|| "This item has no name.".to_string())?;
    if std::fs::symlink_metadata(src).is_err() {
        return Err(format!("“{name}” no longer exists."));
    }

    for n in 1..=10_000 {
        let label = if n == 1 {
            format!("Link to {name}")
        } else {
            format!("Link to {name} {n}")
        };
        let dest = parent.join(&label);
        if std::fs::symlink_metadata(&dest).is_ok() {
            continue;
        }
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(src, &dest)
                .map_err(|e| format!("Could not create the link: {e}"))?;
            return Ok(dest.to_string_lossy().to_string());
        }
        #[cfg(not(unix))]
        {
            let _ = dest;
            return Err("Creating links is not supported on this platform.".into());
        }
    }
    Err("Could not find a free name for the link.".into())
}

#[derive(Debug, Serialize)]
pub struct TextPreview {
    pub text: String,
    pub truncated: bool,
}

/// Read the start of a UTF-8 text file for the preview panel.
pub fn preview_text(path: String, limit: usize) -> Result<TextPreview, String> {
    let limit = limit.clamp(1, 1_048_576);
    let mut file =
        std::fs::File::open(&path).map_err(|e| format!("Failed to open file: {e}"))?;
    let mut bytes = Vec::new();
    file.by_ref()
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("Failed to read file: {e}"))?;
    let truncated = bytes.len() > limit;
    bytes.truncate(limit);
    let text = String::from_utf8(bytes)
        .map_err(|_| "This file can't be previewed as text.".to_string())?;
    Ok(TextPreview { text, truncated })
}

/// A whole file's bytes for the preview (3D models), refused when it is
/// larger than `limit` so a huge file never fills the webview's memory.
pub fn preview_bytes(path: String, limit: u64) -> Result<Vec<u8>, String> {
    let mut file =
        std::fs::File::open(&path).map_err(|e| format!("Failed to open file: {e}"))?;
    let meta = file.metadata().map_err(|e| format!("Failed to read file: {e}"))?;
    if !meta.is_file() {
        return Err("Only files can be previewed.".into());
    }
    if meta.len() > limit {
        return Err("This file is too large to preview.".into());
    }
    let mut bytes = Vec::with_capacity(meta.len() as usize);
    // At most `limit`, even if the file grew since it was measured.
    file.by_ref()
        .take(limit)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("Failed to read file: {e}"))?;
    Ok(bytes)
}

#[derive(Debug, Deserialize)]
pub struct RenameItem {
    pub from: String,
    pub to: String,
}

/// Rename several items in place, all or nothing. Every target is checked
/// before the first rename, and each rename refuses a name taken since
/// (`rename::rename_in_place`); if one fails, the completed renames are
/// rolled back the same way, so none lands on an item that took an old name.
pub fn rename_many(items: Vec<RenameItem>) -> Result<(), String> {
    rename_many_by(items, &rename::rename_in_place)
}

type InPlaceFn<'a> = &'a dyn Fn(&Path, &Path) -> std::io::Result<()>;

/// `rename_many` with the rename given, so tests can take a name meanwhile.
fn rename_many_by(items: Vec<RenameItem>, rename: InPlaceFn) -> Result<(), String> {
    if items.is_empty() {
        return Ok(());
    }
    let mut seen_from = HashSet::new();
    let mut seen_to = HashSet::new();
    let mut work: Vec<(PathBuf, PathBuf)> = Vec::new();

    for item in items {
        let from = PathBuf::from(&item.from);
        let to = PathBuf::from(&item.to);
        if from == to {
            continue;
        }
        if from.parent() != to.parent() {
            return Err("Items can only be renamed, not moved.".into());
        }
        if to.file_name().is_none() {
            return Err("A name can't be empty.".into());
        }
        if std::fs::symlink_metadata(&from).is_err() {
            return Err(format!("“{}” no longer exists.", item.from));
        }
        if !seen_from.insert(from.clone()) || !seen_to.insert(to.clone()) {
            return Err("Two items would have the same name.".into());
        }
        // Only a change of case on a disk that ignores it may find `from`
        // itself at `to`; a link to `to`, or a hard link of the same file
        // under that exact name, is another item.
        if std::fs::symlink_metadata(&to).is_ok() && !rename::folded_onto_itself(&from, &to) {
            return Err(already_named(&to));
        }
        work.push((from, to));
    }

    let mut done: Vec<(PathBuf, PathBuf)> = Vec::new();
    for (from, to) in work {
        if let Err(e) = rename(&from, &to) {
            let failed = match e.kind() {
                std::io::ErrorKind::AlreadyExists => already_named(&to),
                _ => format!("Failed to rename “{}”: {e}", from.display()),
            };
            return Err(failed + &roll_back(&done, rename));
        }
        done.push((from, to));
    }
    Ok(())
}

fn already_named(to: &Path) -> String {
    let name = to.file_name().unwrap_or_default().to_string_lossy();
    format!("A file or folder named “{name}” already exists.")
}

/// Put the completed renames back, last first, never over an item that
/// took an old name meanwhile; says which stayed renamed, if any.
fn roll_back(done: &[(PathBuf, PathBuf)], rename: InPlaceFn) -> String {
    let mut kept = String::new();
    for (from, to) in done.iter().rev() {
        if let Err(e) = rename(to, from) {
            let reason = match e.kind() {
                std::io::ErrorKind::AlreadyExists => "another item is there now".to_string(),
                _ => e.to_string(),
            };
            kept += &format!(" “{}” could not be renamed back to “{}”: {reason}.", to.display(), from.display());
        }
    }
    kept
}

/// Lowercase with accents removed, so "cafe" finds "Café". Matches the
/// frontend's `fold` (`src/search.ts`), which filters the folder before these
/// results arrive; both are tested against `tests/fold-cases.json`.
pub fn fold(s: &str) -> String {
    s.nfd()
        .filter(|c| !is_combining_mark(*c))
        .collect::<String>()
        .to_lowercase()
}

/// Why a search ended before looking everywhere.
#[derive(Serialize, Debug, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum SearchStop {
    /// It found `limit` matches.
    Results,
    /// It looked through `MAX_SEARCH_SCANNED` items.
    Scanned,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct SearchSummary {
    /// None when the search covered the whole folder (or was cancelled).
    pub stopped: Option<SearchStop>,
    pub scanned: usize,
}

/// How many items one search looks at before giving up.
pub const MAX_SEARCH_SCANNED: usize = 100_000;
/// How often found entries are handed to `found` while a search runs.
const SEARCH_BATCH_EVERY: Duration = Duration::from_millis(100);

/// Recursively find entries whose name contains `query`, ignoring case and
/// accents. Matches go to `found` in batches as the search runs, so the first
/// ones show up before it ends; `cancelled` is checked for every item.
/// Symlinked folders are returned but not descended into, so a link loop can't
/// make search run forever; unreadable subfolders are skipped. Stops after
/// `limit` results (the UI asks for a bounded number).
pub fn search_dir(
    path: &str,
    query: &str,
    show_hidden: bool,
    limit: usize,
    found: &mut dyn FnMut(Vec<Entry>),
    cancelled: &dyn Fn() -> bool,
) -> Result<SearchSummary, String> {
    let root = Path::new(path);
    let needle = fold(query.trim());
    let mut walk = SearchWalk {
        batch: Vec::new(),
        sent: 0,
        scanned: 0,
        last_sent: Instant::now(),
        found,
    };
    if needle.is_empty() {
        return Ok(walk.summary(None));
    }
    // A queue, not a stack: every folder at one depth is read before any
    // deeper one, so results come nearest first and the limits cut the
    // farthest folders.
    let mut queue = std::collections::VecDeque::from([root.to_path_buf()]);
    while let Some(dir) = queue.pop_front() {
        let read_dir = match std::fs::read_dir(&dir) {
            Ok(read_dir) => read_dir,
            Err(e) if dir == root => {
                return Err(format!("Failed to read directory: {e}"));
            }
            Err(_) => continue,
        };
        for item in read_dir.flatten() {
            if cancelled() {
                return Ok(walk.summary(None));
            }
            walk.tick();
            if let Some(stop) = walk.stop(limit) {
                return Ok(walk.summary(Some(stop)));
            }
            walk.scanned += 1;
            let name = item.file_name().to_string_lossy().to_string();
            if name == VERSIONS_DIR || (name.starts_with('.') && !show_hidden) {
                continue;
            }
            let child = item.path();
            let Some(entry) = make_entry(&child, name) else {
                continue;
            };
            if entry.is_dir && !entry.is_symlink {
                queue.push_back(child);
            }
            if fold(&entry.name).contains(&needle) {
                walk.add(entry);
            }
        }
    }
    Ok(walk.summary(None))
}

/// The running state of `search_dir`: counts and the batch not yet sent.
struct SearchWalk<'a> {
    batch: Vec<Entry>,
    sent: usize,
    scanned: usize,
    last_sent: Instant,
    found: &'a mut dyn FnMut(Vec<Entry>),
}

impl SearchWalk<'_> {
    fn stop(&self, limit: usize) -> Option<SearchStop> {
        if self.sent + self.batch.len() >= limit {
            Some(SearchStop::Results)
        } else if self.scanned >= MAX_SEARCH_SCANNED {
            Some(SearchStop::Scanned)
        } else {
            None
        }
    }

    fn add(&mut self, entry: Entry) {
        self.batch.push(entry);
        self.tick();
    }

    /// Send the waiting batch once it's due. Called for every item scanned,
    /// so a lone match doesn't wait for a later one, or for the walk to end.
    fn tick(&mut self) {
        if !self.batch.is_empty() && self.last_sent.elapsed() >= SEARCH_BATCH_EVERY {
            self.flush();
        }
    }

    fn flush(&mut self) {
        self.last_sent = Instant::now();
        if self.batch.is_empty() {
            return;
        }
        self.sent += self.batch.len();
        (self.found)(std::mem::take(&mut self.batch));
    }

    fn summary(&mut self, stopped: Option<SearchStop>) -> SearchSummary {
        self.flush();
        SearchSummary {
            stopped,
            scanned: self.scanned,
        }
    }
}

/// What the "N items" column needs about a folder: how many entries it has,
/// and whether it keeps versions (an owned `.versions/` inside). The flag is
/// noticed while counting, so the version badge costs no extra directory
/// read; only a folder that has `.versions` gets its marker read (D14).
#[derive(Debug, Serialize, PartialEq)]
pub struct FolderSummary {
    pub count: u32,
    pub versioned: bool,
}

/// Whether a folder's `.versions` is Keep Versions' own (its marker, D14).
/// Keep Versions stays in the app (D7), which passes its check in.
pub type OwnsHistory = fn(&Path) -> bool;

/// Summary of the folder at `path`, or `None` if it can't be read; `owns`
/// is asked only when the folder has a `.versions`.
pub fn folder_summary(path: &Path, show_hidden: bool, owns: OwnsHistory) -> Option<FolderSummary> {
    let read_dir = std::fs::read_dir(path).ok()?;
    let mut count = 0;
    let mut has_versions = false;
    for entry in read_dir.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name == VERSIONS_DIR {
            has_versions = true;
        } else if show_hidden || !name.starts_with('.') {
            count += 1;
        }
    }
    let versioned = has_versions && owns(path);
    Some(FolderSummary { count, versioned })
}

/// Number of entries directly inside `path`, or `None` if it can't be read.
pub fn count_items(path: &Path, show_hidden: bool) -> Option<u32> {
    // The count alone: whose `.versions` it is doesn't change it.
    folder_summary(path, show_hidden, |_| false).map(|summary| summary.count)
}

/// Summaries for several folders at once (the "12 items" size column and the
/// version badge on folder icons), the badge from `owns`.
pub fn item_counts(paths: Vec<String>, show_hidden: bool, owns: OwnsHistory) -> Vec<Option<FolderSummary>> {
    paths
        .iter()
        .map(|p| folder_summary(Path::new(p), show_hidden, owns))
        .collect()
}

// ---------------------------------------------------------------------------
// Copy / move / trash
//
// Nothing here ever deletes data outright except the source of a finished
// cross-volume move. A replaced target goes to the Trash, and the new item is
// staged under a temporary name first so a failed copy leaves the target alone.
// ---------------------------------------------------------------------------

/// True if something (even a broken symlink) exists at `path`.
pub fn path_exists(path: String) -> bool {
    std::fs::symlink_metadata(&path).is_ok()
}

pub use places::PathInfo;

#[cfg(unix)]
fn device_id(meta: &std::fs::Metadata) -> u64 {
    use std::os::unix::fs::MetadataExt;
    meta.dev()
}

#[cfg(not(unix))]
fn device_id(_: &std::fs::Metadata) -> u64 {
    0
}

/// Kind and filesystem of each path (following symlinks); `None` if it's gone.
/// Used for drag and drop: files dropped from Finder arrive as bare paths.
pub fn path_info(paths: Vec<String>) -> Vec<Option<PathInfo>> {
    paths
        .iter()
        .map(|p| {
            std::fs::metadata(p).ok().map(|m| PathInfo {
                is_dir: m.is_dir(),
                device: device_id(&m),
            })
        })
        .collect()
}

/// Absolute, symlink-resolved form of `path`, which may not exist yet.
pub fn resolve(path: &Path) -> PathBuf {
    if let Ok(p) = path.canonicalize() {
        return p;
    }
    match (path.parent(), path.file_name()) {
        (Some(parent), Some(name)) => resolve(parent).join(name),
        _ => path.to_path_buf(),
    }
}

/// True when `child` is `ancestor` itself or lies anywhere inside it.
pub fn is_subpath(child: &Path, ancestor: &Path) -> bool {
    resolve(child).starts_with(resolve(ancestor))
}

/// Copy a file, symlink or whole directory tree. Symlinks are recreated, not
/// followed, so a link pointing at a parent can't cause an endless copy.
/// Every file, link and folder counts as an item; a cancel stops between
/// entries (and, on Linux, between slices of a file).
pub fn copy_recursive(from: &Path, to: &Path, progress: &Progress) -> std::io::Result<()> {
    if progress.is_cancelled() {
        return Err(cancelled_io());
    }
    let meta = std::fs::symlink_metadata(from)?;
    if meta.file_type().is_symlink() {
        #[cfg(unix)]
        std::os::unix::fs::symlink(std::fs::read_link(from)?, to)?;
        #[cfg(not(unix))]
        std::fs::copy(from, to)?;
    } else if meta.is_dir() {
        std::fs::create_dir(to)?;
        copy_children(from, to, &meta, progress)?;
    } else {
        copy_file_progress(from, to, progress)?;
    }
    progress.add_item();
    Ok(())
}

/// Copy what is in folder `from` into `to`, a folder just made for it, then
/// give `to` the folder's permissions (last, so a read-only one still fills).
pub fn copy_children(
    from: &Path,
    to: &Path,
    meta: &std::fs::Metadata,
    progress: &Progress,
) -> std::io::Result<()> {
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        copy_recursive(&entry.path(), &to.join(entry.file_name()), progress)?;
    }
    std::fs::set_permissions(to, meta.permissions())
}

/// Make `dest`, the copy of folder `from`, refusing a name that is taken.
pub fn make_folder(from: &Path, dest: &Path, progress: &Progress) -> std::io::Result<std::fs::Metadata> {
    if progress.is_cancelled() {
        return Err(cancelled_io());
    }
    let meta = std::fs::symlink_metadata(from)?;
    std::fs::create_dir(dest)?;
    Ok(meta)
}

/// The error of a fill that wrote nothing: a name taken after the check
/// reads as the refusal the check would have made.
pub fn fill_error(verb: &str, dest: &Path, error: std::io::Error) -> String {
    if error.kind() == std::io::ErrorKind::AlreadyExists {
        return Refusal::Exists(dest.display().to_string()).into();
    }
    copy_error(verb, error)
}

/// Whether `path` is a folder itself, not a link to one.
pub fn is_folder(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|meta| meta.is_dir())
}

/// Copy one file in slices; std turns each `File` → `File` slice into
/// `copy_file_range`, so btrfs still reflinks and a cancel works mid-file.
#[cfg(target_os = "linux")]
fn copy_file_progress(from: &Path, to: &Path, progress: &Progress) -> std::io::Result<()> {
    let mut input = std::fs::File::open(from)?;
    let mut output = std::fs::File::create(to)?;
    copy_counted(&mut input, &mut output, progress)?;
    std::fs::set_permissions(to, input.metadata()?.permissions())
}

/// Copy one file with `fs::copy`, which keeps extended attributes and clones
/// on APFS, and count bytes by watching the copy grow. A cancel takes effect
/// before or after the file, not during it.
#[cfg(not(target_os = "linux"))]
fn copy_file_progress(from: &Path, to: &Path, progress: &Progress) -> std::io::Result<()> {
    let base = progress.snapshot().bytes;
    let (src, dst) = (from.to_path_buf(), to.to_path_buf());
    let copy = std::thread::spawn(move || std::fs::copy(src, dst));
    while !copy.is_finished() {
        std::thread::sleep(std::time::Duration::from_millis(100));
        let len = std::fs::metadata(to).map(|m| m.len()).unwrap_or(0);
        progress.set_bytes(base + len);
    }
    let copied = copy
        .join()
        .map_err(|_| std::io::Error::other("the copy stopped unexpectedly"))??;
    progress.set_bytes(base + copied);
    if progress.is_cancelled() {
        return Err(cancelled_io());
    }
    Ok(())
}

pub use places::Measure;

/// Apparent sizes of everything under `paths`. Symlinks are counted, not
/// followed; folders that can't be read are skipped, not reported.
pub fn measure(paths: &[PathBuf]) -> Measure {
    let mut total = Measure::default();
    for path in paths {
        measure_into(path, &mut total);
    }
    total
}

fn measure_into(path: &Path, total: &mut Measure) {
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return;
    };
    total.items += 1;
    if meta.is_dir() {
        for entry in std::fs::read_dir(path).into_iter().flatten().flatten() {
            measure_into(&entry.path(), total);
        }
    } else if meta.is_file() {
        total.bytes += meta.len();
    }
}

/// A copy error as the user sees it; a cancel stays exactly "Cancelled.".
pub fn copy_error(verb: &str, error: std::io::Error) -> String {
    if is_cancel(&error) {
        CANCELLED.to_string()
    } else {
        format!("Failed to {verb}: {error}")
    }
}

pub fn is_cross_device(e: &std::io::Error) -> bool {
    e.kind() == std::io::ErrorKind::CrossesDevices || e.raw_os_error() == Some(18) // EXDEV
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn local_entries_do_not_serialize_a_cloud_field() {
        let entry = Entry {
            name: "a.txt".into(),
            path: "/tmp/a.txt".into(),
            is_dir: false,
            size: 1,
            modified: 0,
            is_hidden: false,
            is_symlink: false,
            cloud: None,
        };
        let json = serde_json::to_value(&entry).unwrap();
        assert!(json.get("cloud").is_none());
    }

    #[test]
    fn entries_for_skips_missing_and_hidden_paths() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("a.txt"), "hello").unwrap();
        fs::create_dir(dir.path().join("Folder")).unwrap();
        fs::write(dir.path().join(".secret"), "x").unwrap();
        let p = |n: &str| dir.path().join(n).to_string_lossy().to_string();
        let paths = [p("a.txt"), p("gone.txt"), p("Folder"), p(".secret")];

        let names = |list: Vec<Entry>| {
            list.into_iter()
                .map(|e| (e.name, e.is_dir))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            names(entries_for(&paths, false)),
            [("a.txt".into(), false), ("Folder".into(), true)]
        );
        assert_eq!(entries_for(&paths, true).len(), 3);
        assert_eq!(entries_for(&paths, false)[0].size, 5);
    }

    /// Run a search to the end, returning the batches it sent and its summary.
    fn search(
        dir: &Path,
        query: &str,
        show_hidden: bool,
        limit: usize,
    ) -> (Vec<Vec<Entry>>, SearchSummary) {
        let mut batches = Vec::new();
        let summary = search_dir(
            &dir.to_string_lossy(),
            query,
            show_hidden,
            limit,
            &mut |batch| batches.push(batch),
            &|| false,
        )
        .unwrap();
        (batches, summary)
    }

    fn found_names(batches: Vec<Vec<Entry>>) -> Vec<String> {
        let mut names: Vec<String> = batches.into_iter().flatten().map(|e| e.name).collect();
        names.sort();
        names
    }

    #[test]
    fn search_is_recursive_and_skips_hidden_or_internal_folders() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("Alpha.txt"), "a").unwrap();
        fs::create_dir(dir.path().join("sub")).unwrap();
        fs::write(dir.path().join("sub/alpha-two.txt"), "b").unwrap();
        fs::create_dir(dir.path().join("sub/deeper")).unwrap();
        fs::write(dir.path().join("sub/deeper/ALPHA-three.txt"), "c").unwrap();
        fs::create_dir(dir.path().join(".hidden")).unwrap();
        fs::write(dir.path().join(".hidden/alpha-secret.txt"), "d").unwrap();
        fs::create_dir(dir.path().join(".versions")).unwrap();
        fs::write(dir.path().join(".versions/alpha-old.txt"), "e").unwrap();

        let (batches, summary) = search(dir.path(), "alpha", false, 100);
        assert_eq!(
            found_names(batches),
            ["ALPHA-three.txt", "Alpha.txt", "alpha-two.txt"]
        );
        assert_eq!(summary.stopped, None);

        let with_hidden = found_names(search(dir.path(), "alpha", true, 100).0);
        assert!(with_hidden.contains(&"alpha-secret.txt".to_string()));
        assert!(!with_hidden.contains(&"alpha-old.txt".to_string()));
    }

    /// How many folders below `root` an entry is: 1 for one directly inside.
    fn depth(root: &Path, entry: &Entry) -> usize {
        Path::new(&entry.path).strip_prefix(root).unwrap().components().count()
    }

    fn write_all(root: &Path, files: &[&str]) {
        for file in files {
            let path = root.join(file);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, "x").unwrap();
        }
    }

    /// Nearest first: every folder at one depth is read before any deeper
    /// one, so results arrive in depth order whatever order folders list in.
    #[test]
    fn search_reads_each_level_before_the_next() {
        let dir = tempdir().unwrap();
        write_all(dir.path(), &["a/note-1a.txt", "a/b/note-2a.txt", "z/note-1z.txt", "z/y/note-2z.txt", "note-0.txt"]);
        let found: Vec<usize> = search(dir.path(), "note", false, 100)
            .0
            .into_iter()
            .flatten()
            .map(|entry| depth(dir.path(), &entry))
            .collect();
        assert_eq!(found, [1, 2, 2, 3, 3]);
    }

    /// The result limit cuts the farthest folders, not whichever came first.
    #[test]
    fn search_limit_keeps_shallow_matches() {
        let dir = tempdir().unwrap();
        write_all(dir.path(), &["a/note.txt", "a/s/t/note.txt", "b/note.txt", "b/s/t/note.txt", "c/note.txt", "c/s/t/note.txt"]);
        let (batches, summary) = search(dir.path(), "note", false, 3);
        let depths: Vec<usize> = batches.into_iter().flatten().map(|entry| depth(dir.path(), &entry)).collect();
        assert_eq!(depths, [2, 2, 2]);
        assert_eq!(summary.stopped, Some(SearchStop::Results));
    }

    #[test]
    fn search_ignores_accents_and_case_both_ways() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("Café Menu.pdf"), "a").unwrap();
        // macOS file systems may hand back names already decomposed.
        fs::write(dir.path().join("Re\u{301}sume\u{301}.txt"), "b").unwrap();
        fs::write(dir.path().join("cafe-notes.txt"), "c").unwrap();
        assert_eq!(
            found_names(search(dir.path(), "CAFE", false, 100).0),
            ["Café Menu.pdf", "cafe-notes.txt"]
        );
        assert_eq!(found_names(search(dir.path(), "café", false, 100).0).len(), 2);
        assert_eq!(found_names(search(dir.path(), "résumé", false, 100).0).len(), 1);
        assert_eq!(fold("Ångström"), "angstrom");
    }

    #[test]
    fn search_stops_at_the_limit_and_says_so() {
        let dir = tempdir().unwrap();
        for i in 0..20 {
            fs::write(dir.path().join(format!("match-{i}.txt")), "x").unwrap();
        }
        let (batches, summary) = search(dir.path(), "match", false, 5);
        assert_eq!(found_names(batches).len(), 5);
        assert_eq!(summary.stopped, Some(SearchStop::Results));
        // Finding everything there is is not a stop, even at exactly the limit.
        let (_, all) = search(dir.path(), "match", false, 20);
        assert_eq!(all.stopped, None);
        assert_eq!(all.scanned, 20);
    }

    #[test]
    fn search_stops_when_cancelled_and_ignores_an_empty_query() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("a.txt"), "x").unwrap();
        let mut sent = 0;
        let summary = search_dir(
            &dir.path().to_string_lossy(),
            "a",
            false,
            10,
            &mut |batch| sent += batch.len(),
            &|| true,
        )
        .unwrap();
        assert_eq!((sent, summary.stopped, summary.scanned), (0, None, 0));
        let (batches, empty) = search(dir.path(), "  ", false, 10);
        assert!(batches.is_empty());
        assert_eq!(empty, SearchSummary { stopped: None, scanned: 0 });
    }

    #[test]
    fn search_of_an_unreadable_folder_is_an_error() {
        let dir = tempdir().unwrap();
        let missing = dir.path().join("gone");
        let result = search_dir(&missing.to_string_lossy(), "a", false, 10, &mut |_| {}, &|| false);
        assert!(result.unwrap_err().starts_with("Failed to read directory"));
    }

    #[test]
    fn search_sends_matches_in_batches_while_it_runs() {
        let mut got = Vec::new();
        let mut walk = SearchWalk {
            batch: Vec::new(),
            sent: 0,
            scanned: 0,
            last_sent: Instant::now() - SEARCH_BATCH_EVERY,
            found: &mut |batch: Vec<Entry>| got.push(batch.len()),
        };
        let entry = |name: &str| Entry {
            name: name.into(),
            path: format!("/tmp/{name}"),
            is_dir: false,
            size: 1,
            modified: 0,
            is_hidden: false,
            is_symlink: false,
            cloud: None,
        };
        walk.add(entry("a")); // due: sent at once
        walk.add(entry("b")); // waits for the next batch
        assert_eq!(walk.stop(2), Some(SearchStop::Results));
        walk.scanned = MAX_SEARCH_SCANNED;
        assert_eq!(walk.stop(10), Some(SearchStop::Scanned));
        assert_eq!(walk.summary(None).scanned, MAX_SEARCH_SCANNED);
        assert_eq!(got, [1, 1]);
    }

    #[test]
    fn a_lone_early_match_is_sent_while_the_walk_goes_on() {
        use std::cell::Cell;
        // The match is at the top; the subfolder, walked after it, has no
        // matches, so no later match would push the waiting batch out.
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("needle.txt"), "x").unwrap();
        fs::create_dir(dir.path().join("hay")).unwrap();
        for i in 0..50 {
            fs::write(dir.path().join(format!("hay/straw-{i}")), "x").unwrap();
        }
        let items = Cell::new(0);
        let sent_at = Cell::new(None);
        let summary = search_dir(
            &dir.path().to_string_lossy(),
            "needle",
            false,
            10,
            &mut |_| {
                if sent_at.get().is_none() {
                    sent_at.set(Some(items.get()));
                }
            },
            &|| {
                items.set(items.get() + 1);
                // Partway through the haystack, more than a batch interval passes.
                if items.get() == 10 {
                    std::thread::sleep(SEARCH_BATCH_EVERY + Duration::from_millis(20));
                }
                false
            },
        )
        .unwrap();
        assert_eq!(summary.scanned, 52);
        let sent_at = sent_at.get().expect("the match was sent");
        assert!(sent_at < 52, "sent only when the walk ended (after item {sent_at})");
    }

    #[test]
    fn text_preview_stops_at_the_limit_and_rejects_binary() {
        let dir = tempdir().unwrap();
        let text = dir.path().join("a.txt");
        fs::write(&text, "hello world").unwrap();
        let preview = preview_text(text.to_string_lossy().to_string(), 5).unwrap();
        assert_eq!(preview.text, "hello");
        assert!(preview.truncated);

        let binary = dir.path().join("a.bin");
        fs::write(&binary, [0xff, 0xfe, 0xfd]).unwrap();
        assert!(preview_text(binary.to_string_lossy().to_string(), 10).is_err());
    }

    #[test]
    fn preview_bytes_reads_the_whole_file_and_refuses_one_over_the_limit() {
        let dir = tempdir().unwrap();
        let model = dir.path().join("cube.stl");
        fs::write(&model, [1u8, 2, 3, 4, 5]).unwrap();
        let path = model.to_string_lossy().to_string();
        assert_eq!(preview_bytes(path.clone(), 5).unwrap(), vec![1, 2, 3, 4, 5]);
        assert_eq!(preview_bytes(path, 4).unwrap_err(), "This file is too large to preview.");

        let folder = dir.path().to_string_lossy().to_string();
        assert_eq!(preview_bytes(folder, 100).unwrap_err(), "Only files can be previewed.");
        let missing = dir.path().join("gone.stl").to_string_lossy().to_string();
        assert!(preview_bytes(missing, 100).unwrap_err().starts_with("Failed to open file"));
    }

    #[test]
    fn batch_rename_is_all_or_nothing() {
        let dir = tempdir().unwrap();
        for name in ["a.txt", "b.txt", "taken.txt"] {
            fs::write(dir.path().join(name), name).unwrap();
        }
        let p = |name: &str| dir.path().join(name).to_string_lossy().to_string();
        rename_many(vec![
            RenameItem { from: p("a.txt"), to: p("x.txt") },
            RenameItem { from: p("b.txt"), to: p("y.txt") },
        ])
        .unwrap();
        assert!(dir.path().join("x.txt").exists());
        assert!(dir.path().join("y.txt").exists());
        assert!(!dir.path().join("a.txt").exists());

        let err = rename_many(vec![
            RenameItem { from: p("x.txt"), to: p("z.txt") },
            RenameItem { from: p("y.txt"), to: p("taken.txt") },
        ])
        .unwrap_err();
        assert!(err.contains("already exists"));
        assert!(dir.path().join("x.txt").exists());
        assert!(dir.path().join("y.txt").exists());

        let err = rename_many(vec![RenameItem { from: p("x.txt"), to: p("../moved.txt") }])
            .unwrap_err();
        assert!(err.contains("only be renamed"));
    }

    #[test]
    fn empty_files_are_created_but_never_replaced() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("new.txt");
        create_file(file.to_string_lossy().to_string()).unwrap();
        assert_eq!(fs::read_to_string(&file).unwrap(), "");
        assert!(create_file(file.to_string_lossy().to_string()).is_err());
        assert_eq!(fs::read_to_string(&file).unwrap(), "");
    }

    #[cfg(unix)]
    #[test]
    fn creating_links_uses_free_names() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("a.txt");
        fs::write(&file, "x").unwrap();
        let first = create_link(file.to_string_lossy().to_string()).unwrap();
        assert_eq!(Path::new(&first).file_name().unwrap(), "Link to a.txt");
        assert_eq!(fs::read_link(&first).unwrap(), file);

        let second = create_link(file.to_string_lossy().to_string()).unwrap();
        assert_eq!(Path::new(&second).file_name().unwrap(), "Link to a.txt 2");
    }

    #[cfg(unix)]
    #[test]
    fn permissions_can_be_changed_and_refuse_unknown_bits() {        use std::os::unix::fs::PermissionsExt;
        let dir = tempdir().unwrap();
        let file = dir.path().join("a.txt");
        fs::write(&file, "x").unwrap();

        set_permissions(file.to_string_lossy().to_string(), 0o600).unwrap();
        assert_eq!(fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o600);

        set_permissions(file.to_string_lossy().to_string(), 0o755).unwrap();
        assert_eq!(fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o755);

        assert!(set_permissions(file.to_string_lossy().to_string(), 0o1000).is_err());
        assert_eq!(fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o755);
    }

    #[test]
    fn versions_directory_is_hidden_even_with_show_hidden() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("a.txt"), "a").unwrap();
        fs::create_dir(dir.path().join(".versions")).unwrap();
        fs::write(dir.path().join(".versions/index.sqlite"), "x").unwrap();

        for show_hidden in [false, true] {
            let entries = read_entries(dir.path(), show_hidden).unwrap();
            assert!(!entries.iter().any(|e| e.name == ".versions"));
            assert_eq!(count_items(dir.path(), show_hidden), Some(1));
        }
        let inside = dir.path().join(".versions/index.sqlite");
        assert!(entries_for(&[inside.to_string_lossy().to_string()], true).is_empty());
    }

    #[test]
    fn lists_file_and_subdir() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("hello.txt");
        fs::write(&file_path, "world").unwrap();
        let sub_path = dir.path().join("subdir");
        fs::create_dir(&sub_path).unwrap();

        let entries = read_entries(dir.path(), true).unwrap();

        let file_entry = entries
            .iter()
            .find(|e| e.name == "hello.txt")
            .expect("hello.txt not found");
        assert!(!file_entry.is_dir);
        assert_eq!(file_entry.size, 5);

        let dir_entry = entries
            .iter()
            .find(|e| e.name == "subdir")
            .expect("subdir not found");
        assert!(dir_entry.is_dir);
    }

    #[test]
    fn dotfile_excluded_when_hidden_false() {
        let dir = tempdir().unwrap();
        let dotfile = dir.path().join(".hidden");
        fs::write(&dotfile, "secret").unwrap();

        let entries = read_entries(dir.path(), false).unwrap();
        assert!(!entries.iter().any(|e| e.name == ".hidden"));
    }

    #[test]
    fn dotfile_included_when_hidden_true_and_marked_hidden() {
        let dir = tempdir().unwrap();
        let dotfile = dir.path().join(".hidden");
        fs::write(&dotfile, "secret").unwrap();

        let entries = read_entries(dir.path(), true).unwrap();
        let entry = entries
            .iter()
            .find(|e| e.name == ".hidden")
            .expect(".hidden not found");
        assert!(entry.is_hidden);
    }

    #[test]
    fn nonexistent_path_returns_err() {
        let result = read_entries(Path::new("/no/such/path/exists/here"), false);
        assert!(result.is_err());
    }

    #[test]
    fn broken_symlink_included_as_symlink() {
        let dir = tempdir().unwrap();
        let target = dir.path().join("missing_target");
        let link = dir.path().join("broken_link");

        #[cfg(unix)]
        std::os::unix::fs::symlink(&target, &link).unwrap();

        #[cfg(unix)]
        {
            let entries = read_entries(dir.path(), true).unwrap();
            let entry = entries
                .iter()
                .find(|e| e.name == "broken_link")
                .expect("broken_link not found");
            assert!(entry.is_symlink);
        }
    }

    // ----- item_counts tests -----

    #[test]
    fn count_items_respects_hidden_flag() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("a.txt"), "a").unwrap();
        fs::create_dir(dir.path().join("sub")).unwrap();
        fs::write(dir.path().join(".hidden"), "h").unwrap();

        assert_eq!(count_items(dir.path(), false), Some(2));
        assert_eq!(count_items(dir.path(), true), Some(3));
    }

    #[test]
    fn item_counts_returns_none_for_unreadable_path() {
        let dir = tempdir().unwrap();
        let counts = item_counts(
            vec![
                dir.path().to_string_lossy().to_string(),
                "/no/such/path/exists/here".to_string(),
            ],
            false,
            |_| false,
        );
        assert_eq!(
            counts,
            vec![
                Some(FolderSummary {
                    count: 0,
                    versioned: false
                }),
                None
            ]
        );
    }

    // ----- make_dir tests -----

    #[test]
    fn make_dir_creates_directory() {
        let dir = tempdir().unwrap();
        let new_dir = dir.path().join("new_folder");
        let path_str = new_dir.to_string_lossy().to_string();

        assert!(!new_dir.exists());
        make_dir(path_str).unwrap();
        assert!(new_dir.exists());
        assert!(new_dir.is_dir());
    }

    #[test]
    fn make_dir_errors_when_already_exists() {
        let dir = tempdir().unwrap();
        let existing = dir.path().join("existing_folder");
        fs::create_dir(&existing).unwrap();
        let path_str = existing.to_string_lossy().to_string();

        let result = make_dir(path_str);
        assert!(result.is_err());
        let msg = result.unwrap_err();
        assert!(msg.contains("already exists"), "unexpected error: {}", msg);
    }

    // ----- rename_entry tests -----

    #[test]
    fn rename_entry_renames_file() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("original.txt");
        let dst = dir.path().join("renamed.txt");
        fs::write(&src, "hello").unwrap();

        rename_entry(
            src.to_string_lossy().to_string(),
            dst.to_string_lossy().to_string(),
        )
        .unwrap();

        assert!(!src.exists());
        assert!(dst.exists());
    }

    #[test]
    fn rename_entry_errors_when_target_exists() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("source.txt");
        let dst = dir.path().join("target.txt");
        fs::write(&src, "hello").unwrap();
        fs::write(&dst, "world").unwrap();

        let result = rename_entry(
            src.to_string_lossy().to_string(),
            dst.to_string_lossy().to_string(),
        );
        assert!(result.is_err());
        let msg = result.unwrap_err();
        assert!(msg.contains("already exists"), "unexpected error: {}", msg);
    }

    /// A link whose target is gone still takes its name: `exists()` follows
    /// it and says the name is free, and `std::fs::rename` replaced it. The
    /// view doesn't list it (hidden), so only the backend can refuse.
    #[test]
    fn rename_entry_never_replaces_a_dangling_link() {
        let dir = tempdir().unwrap();
        let (src, link) = (dir.path().join("a.txt"), dir.path().join(".hidden"));
        fs::write(&src, "mine").unwrap();
        std::os::unix::fs::symlink(dir.path().join("gone"), &link).unwrap();

        let err = rename_entry(s(&src), s(&link)).unwrap_err();

        assert!(err.contains("already exists"), "{err}");
        assert!(fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
        assert_eq!(fs::read_to_string(&src).unwrap(), "mine");
    }

    /// A link to `b` renamed to `b` is not `b` itself: a check that follows
    /// links took it for a rename to the same item and replaced `b`.
    #[test]
    fn batch_rename_never_replaces_the_item_a_link_points_to() {
        let dir = tempdir().unwrap();
        let (link, target) = (dir.path().join("a"), dir.path().join("b"));
        fs::write(&target, "kept").unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();

        let err = rename_many(vec![RenameItem { from: s(&link), to: s(&target) }]).unwrap_err();

        assert!(err.contains("already exists"), "{err}");
        assert!(fs::symlink_metadata(&target).unwrap().file_type().is_file());
        assert_eq!(fs::read_to_string(&target).unwrap(), "kept");
        assert!(fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
    }

    /// An item that takes a target's name after the check stays; the items
    /// already renamed go back.
    #[test]
    fn batch_rename_never_replaces_an_item_that_took_the_name_meanwhile() {
        let dir = tempdir().unwrap();
        let p = |name: &str| dir.path().join(name);
        fs::write(p("a"), "a").unwrap();
        fs::write(p("b"), "b").unwrap();
        let racing = |from: &Path, to: &Path| {
            if to == p("y") {
                fs::write(p("y"), "theirs").unwrap();
            }
            rename::rename_in_place(from, to)
        };
        let items = vec![
            RenameItem { from: s(&p("a")), to: s(&p("x")) },
            RenameItem { from: s(&p("b")), to: s(&p("y")) },
        ];

        let err = rename_many_by(items, &racing).unwrap_err();

        assert_eq!(err, "A file or folder named “y” already exists.");
        assert_eq!(fs::read_to_string(p("y")).unwrap(), "theirs");
        assert_eq!(fs::read_to_string(p("a")).unwrap(), "a");
        assert_eq!(fs::read_to_string(p("b")).unwrap(), "b");
        assert!(!p("x").exists());
    }

    /// The rollback never lands on an item that took an old name: that
    /// rename stays, and the error says so.
    #[test]
    fn batch_rename_rollback_never_replaces_an_item_that_took_an_old_name() {
        let dir = tempdir().unwrap();
        let p = |name: &str| dir.path().join(name);
        fs::write(p("a"), "a").unwrap();
        fs::write(p("b"), "b").unwrap();
        let failing = |from: &Path, to: &Path| {
            if to == p("y") {
                fs::write(p("a"), "theirs").unwrap();
                return Err(std::io::ErrorKind::PermissionDenied.into());
            }
            rename::rename_in_place(from, to)
        };
        let items = vec![
            RenameItem { from: s(&p("a")), to: s(&p("x")) },
            RenameItem { from: s(&p("b")), to: s(&p("y")) },
        ];

        let err = rename_many_by(items, &failing).unwrap_err();

        assert!(err.starts_with(&format!("Failed to rename “{}”", p("b").display())), "{err}");
        let kept = format!("“{}” could not be renamed back to “{}”: another item is there now.", p("x").display(), p("a").display());
        assert!(err.ends_with(&kept), "{err}");
        assert_eq!(fs::read_to_string(p("a")).unwrap(), "theirs");
        assert_eq!(fs::read_to_string(p("x")).unwrap(), "a");
        assert_eq!(fs::read_to_string(p("b")).unwrap(), "b");
    }

    // ----- path tests -----

    fn s(p: &Path) -> String {
        p.to_string_lossy().to_string()
    }

    #[test]
    fn is_subpath_detects_nesting() {
        let dir = tempdir().unwrap();
        let a = dir.path().join("a");
        fs::create_dir(&a).unwrap();
        assert!(is_subpath(&a.join("b/c"), &a));
        assert!(is_subpath(&a, &a));
        assert!(!is_subpath(&dir.path().join("ab"), &a));
        assert!(!is_subpath(dir.path(), &a));
    }

    #[test]
    fn path_info_reports_kind_device_and_missing() {
        let dir = tempdir().unwrap();
        let f = dir.path().join("f.txt");
        fs::write(&f, "x").unwrap();
        let sub = dir.path().join("sub");
        fs::create_dir(&sub).unwrap();

        let info = path_info(vec![s(&f), s(&sub), s(&dir.path().join("missing"))]);
        let file = info[0].as_ref().unwrap();
        let folder = info[1].as_ref().unwrap();
        assert!(!file.is_dir);
        assert!(folder.is_dir);
        assert_eq!(file.device, folder.device);
        assert!(info[2].is_none());
    }

    #[cfg(unix)]
    #[test]
    fn measure_counts_and_skips_unreadable_folders() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempdir().unwrap();
        let root = dir.path().join("a");
        fs::create_dir_all(root.join("locked")).unwrap();
        fs::write(root.join("x"), vec![0u8; 10]).unwrap();
        fs::write(root.join("locked/y"), vec![0u8; 5]).unwrap();
        fs::set_permissions(root.join("locked"), fs::Permissions::from_mode(0o000)).unwrap();
        if fs::read_dir(root.join("locked")).is_ok() {
            fs::set_permissions(root.join("locked"), fs::Permissions::from_mode(0o755)).unwrap();
            return; // running as root: permissions can't make it unreadable
        }
        let measured = measure(&[root.clone()]);
        fs::set_permissions(root.join("locked"), fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(measured, Measure { bytes: 10, items: 3 });
    }
}
