use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::is_combining_mark;

#[cfg(target_os = "linux")]
use crate::progress::copy_counted;
use crate::placement::{self, Stage, Staged, Target, TrashFn, Unfilled, remove, remove_any, to_trash};
use crate::progress::{CANCELLED, Progress, cancelled_io, is_cancel};

pub fn make_dir(path: String) -> Result<(), String> {
    let p = Path::new(&path);
    if p.exists() {
        return Err(format!("Path already exists: {}", path));
    }
    std::fs::create_dir(p).map_err(|e| format!("Failed to create directory: {}", e))
}

pub fn rename_entry(from: String, to: String) -> Result<(), String> {
    let to_path = Path::new(&to);
    if to_path.exists() {
        return Err(format!("Target already exists: {}", to));
    }
    std::fs::rename(&from, &to).map_err(|e| format!("Failed to rename: {}", e))
}

#[derive(Debug, Serialize)]
pub struct Entry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub size: u64,
    pub modified: i64,
    pub is_hidden: bool,
    pub is_symlink: bool,
    /// Present only for an entry from a cloud location. Local entries omit the
    /// key entirely, so existing consumers see exactly the same JSON.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cloud: Option<crate::cloud::CloudEntry>,
}

pub fn read_entries(path: &Path, show_hidden: bool) -> Result<Vec<Entry>, String> {
    let read_dir =
        std::fs::read_dir(path).map_err(|e| format!("Failed to read directory: {}", e))?;

    let mut entries = Vec::new();
    for dir_entry in read_dir.flatten() {
        let name = dir_entry.file_name().to_string_lossy().to_string();
        // The history directory is always hidden, even with Show Hidden Files.
        if name == crate::versions::VERSIONS_DIR {
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
        .filter(|p| !crate::versions::is_internal_path(Path::new(p)))
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

#[derive(Debug, Deserialize)]
pub struct RenameItem {
    pub from: String,
    pub to: String,
}

/// Rename several items in place, all or nothing. Every target is checked
/// before the first rename; if one still fails, the completed renames are
/// rolled back.
pub fn rename_many(items: Vec<RenameItem>) -> Result<(), String> {
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
        if std::fs::symlink_metadata(&to).is_ok() && !same_file(&from, &to) {
            let name = to.file_name().unwrap_or_default().to_string_lossy();
            return Err(format!("A file or folder named “{name}” already exists."));
        }
        work.push((from, to));
    }

    let mut done: Vec<(PathBuf, PathBuf)> = Vec::new();
    for (from, to) in work {
        if let Err(e) = std::fs::rename(&from, &to) {
            for (old_from, old_to) in done.iter().rev() {
                let _ = std::fs::rename(old_to, old_from);
            }
            return Err(format!("Failed to rename “{}”: {e}", from.display()));
        }
        done.push((from, to));
    }
    Ok(())
}

#[cfg(unix)]
fn same_file(a: &Path, b: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    match (std::fs::metadata(a), std::fs::metadata(b)) {
        (Ok(ma), Ok(mb)) => ma.dev() == mb.dev() && ma.ino() == mb.ino(),
        _ => false,
    }
}

#[cfg(not(unix))]
fn same_file(_a: &Path, _b: &Path) -> bool {
    false
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
            if name == crate::versions::VERSIONS_DIR || (name.starts_with('.') && !show_hidden) {
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

/// Summary of the folder at `path`, or `None` if it can't be read.
pub fn folder_summary(path: &Path, show_hidden: bool) -> Option<FolderSummary> {
    let read_dir = std::fs::read_dir(path).ok()?;
    let mut count = 0;
    let mut has_versions = false;
    for entry in read_dir.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name == crate::versions::VERSIONS_DIR {
            has_versions = true;
        } else if show_hidden || !name.starts_with('.') {
            count += 1;
        }
    }
    let versioned = has_versions && crate::versions::read_marker(path).is_ok();
    Some(FolderSummary { count, versioned })
}

/// Number of entries directly inside `path`, or `None` if it can't be read.
pub fn count_items(path: &Path, show_hidden: bool) -> Option<u32> {
    folder_summary(path, show_hidden).map(|summary| summary.count)
}

/// Summaries for several folders at once (the "12 items" size column and the
/// version badge on folder icons).
pub fn item_counts(paths: Vec<String>, show_hidden: bool) -> Vec<Option<FolderSummary>> {
    paths
        .iter()
        .map(|p| folder_summary(Path::new(p), show_hidden))
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

#[derive(Debug, Serialize, PartialEq)]
pub struct PathInfo {
    pub is_dir: bool,
    /// Filesystem the path lives on; equal ids mean a move is a cheap rename.
    pub device: u64,
}

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
pub(crate) fn resolve(path: &Path) -> PathBuf {
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
fn copy_children(
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

/// Fill `dest` with a copy of `from`, for a copy or a move across volumes.
/// A folder fills in place (`Stage::by_kind`), so it is made first: until
/// it is, a failure is no partial copy of ours, and whatever took the name
/// after the check is left alone. Anything else goes to a staging name.
fn copy_fill<'a>(
    from: &Path,
    dest: &Path,
    folder: bool,
    progress: &Progress,
    verb: &str,
) -> Result<Staged<'a>, Unfilled> {
    let partial = |e| Unfilled::Partial(copy_error(verb, e));
    if !folder {
        return copy_recursive(from, dest, progress).map(|()| Staged::Copy).map_err(partial);
    }
    let meta = make_folder(from, dest, progress).map_err(|e| Unfilled::Nothing(fill_error(verb, dest, e)))?;
    copy_children(from, dest, &meta, progress).map_err(partial)?;
    progress.add_item();
    Ok(Staged::Copy)
}

/// Make `dest`, the copy of folder `from`, refusing a name that is taken.
fn make_folder(from: &Path, dest: &Path, progress: &Progress) -> std::io::Result<std::fs::Metadata> {
    if progress.is_cancelled() {
        return Err(cancelled_io());
    }
    let meta = std::fs::symlink_metadata(from)?;
    std::fs::create_dir(dest)?;
    Ok(meta)
}

/// The error of a fill that wrote nothing: a name taken after the check
/// reads as the refusal the check would have made.
fn fill_error(verb: &str, dest: &Path, error: std::io::Error) -> String {
    if error.kind() == std::io::ErrorKind::AlreadyExists {
        return placement::Refusal::Exists(dest.display().to_string()).into();
    }
    copy_error(verb, error)
}

/// Whether `path` is a folder itself, not a link to one.
fn is_folder(path: &Path) -> bool {
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

/// Sizes and item counts of `paths` before a copy, for the progress total.
#[derive(Serialize, Default, PartialEq, Eq, Debug, Clone, Copy)]
pub struct Measure {
    pub bytes: u64,
    pub items: u64,
}

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
fn copy_error(verb: &str, error: std::io::Error) -> String {
    if is_cancel(&error) {
        CANCELLED.to_string()
    } else {
        format!("Failed to {verb}: {error}")
    }
}

fn is_cross_device(e: &std::io::Error) -> bool {
    e.kind() == std::io::ErrorKind::CrossesDevices || e.raw_os_error() == Some(18) // EXDEV
}

/// Copy `from` to `to` (recursively for folders).
pub fn copy_entry(
    from: String,
    to: String,
    overwrite: bool,
    progress: &Progress,
) -> Result<(), String> {
    copy_entry_with(from, to, overwrite, progress, &to_trash)
}

fn copy_entry_with(
    from: String,
    to: String,
    overwrite: bool,
    progress: &Progress,
    trash: TrashFn,
) -> Result<(), String> {
    let (from, to) = (Path::new(&from), Path::new(&to));
    let target = placement::check_transfer(from, to, overwrite)?;
    copy_to(&target, from, progress, trash)
}

/// Copy `from` to a target that passed the refusals.
fn copy_to(target: &Target, from: &Path, progress: &Progress, trash: TrashFn) -> Result<(), String> {
    let folder = is_folder(from);
    let fill = |dest: &Path| copy_fill(from, dest, folder, progress, "copy");
    target.place(Stage::by_kind(folder, &remove), fill, trash)?;
    Ok(())
}

/// Move `from` to `to`: a rename on the same volume, copy-then-delete across volumes.
pub fn move_entry(
    from: String,
    to: String,
    overwrite: bool,
    progress: &Progress,
) -> Result<(), String> {
    move_entry_with(from, to, overwrite, progress, &to_trash)
}

fn move_entry_with(
    from: String,
    to: String,
    overwrite: bool,
    progress: &Progress,
    trash: TrashFn,
) -> Result<(), String> {
    move_entry_by(from, to, overwrite, progress, trash, MOVE_RENAME)
}

/// Renames an item. A move takes it as a parameter so tests can make it fail
/// as it does across volumes.
type RenameFn<'a> = &'a dyn Fn(&Path, &Path) -> std::io::Result<()>;

/// How a move renames its item: never over one that took the name.
const MOVE_RENAME: RenameFn<'static> = &placement::rename_new;

fn move_entry_by(
    from: String,
    to: String,
    overwrite: bool,
    progress: &Progress,
    trash: TrashFn,
    rename: RenameFn,
) -> Result<(), String> {
    let (from, to) = (Path::new(&from), Path::new(&to));
    let target = placement::check_transfer(from, to, overwrite)?;
    move_to(&target, from, progress, trash, rename)
}

/// Move `from` to a target that passed the refusals.
fn move_to(
    target: &Target,
    from: &Path,
    progress: &Progress,
    trash: TrashFn,
    rename: RenameFn,
) -> Result<(), String> {
    let folder = is_folder(from);
    let fill = |dest: &Path| move_or_copy(from, dest, folder, progress, rename);
    let placed = target.place(Stage::by_kind(folder, &remove), fill, trash)?;
    if placed == Staged::Copy {
        // Across volumes the source goes only now, with its copy in place.
        // No progress is checked from the end of the copy on, so a cancel
        // arriving now can't leave both items or neither.
        remove_any(from).map_err(|e| format!("Moved, but could not remove the original: {}", e))?;
    }
    Ok(())
}

/// Fill `dest` with `from`: renamed (the user's original is there now) or,
/// across volumes, copied (the original stays at `from` until the end).
fn move_or_copy<'a>(
    from: &'a Path,
    dest: &Path,
    folder: bool,
    progress: &Progress,
    rename: RenameFn,
) -> Result<Staged<'a>, Unfilled> {
    match rename(from, dest) {
        Ok(()) => {
            progress.add_item();
            Ok(Staged::Original(from))
        }
        Err(e) if is_cross_device(&e) => copy_fill(from, dest, folder, progress, "move"),
        Err(e) => Err(Unfilled::Nothing(fill_error("move", dest, e))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::progress::Progress;
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

    #[test]
    fn a_folder_with_an_owned_history_is_versioned() {
        let dir = tempdir().unwrap();
        let owned = dir.path().join("Owned");
        fs::create_dir(&owned).unwrap();
        drop(crate::versions::history::History::create(&owned).unwrap());
        fs::write(owned.join("a.txt"), "a").unwrap();
        // A `.versions` another tool made doesn't count (D14).
        let foreign = dir.path().join("Foreign");
        fs::create_dir_all(foreign.join(".versions")).unwrap();
        fs::write(foreign.join(".versions/meta.json"), r#"{"app":"other"}"#).unwrap();
        let plain = dir.path().join("Plain");
        fs::create_dir(&plain).unwrap();

        let summary = |p: &Path, hidden| folder_summary(p, hidden).unwrap();
        assert_eq!(
            summary(&owned, true),
            FolderSummary {
                count: 1,
                versioned: true
            }
        );
        assert_eq!(
            summary(&foreign, true),
            FolderSummary {
                count: 0,
                versioned: false
            }
        );
        assert_eq!(
            summary(&plain, false),
            FolderSummary {
                count: 0,
                versioned: false
            }
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

    // ----- copy / move / trash tests -----

    fn s(p: &Path) -> String {
        p.to_string_lossy().to_string()
    }

    #[test]
    fn copy_entry_copies_file() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("a.txt");
        let dst = dir.path().join("b.txt");
        fs::write(&src, "hello").unwrap();

        copy_entry(s(&src), s(&dst), false, &Progress::default()).unwrap();
        assert_eq!(fs::read_to_string(&dst).unwrap(), "hello");
        assert!(src.exists());
    }

    #[test]
    fn copy_entry_copies_dir_tree() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("tree");
        fs::create_dir_all(src.join("sub/deeper")).unwrap();
        fs::write(src.join("top.txt"), "1").unwrap();
        fs::write(src.join("sub/deeper/leaf.txt"), "2").unwrap();
        let dst = dir.path().join("copy");

        copy_entry(s(&src), s(&dst), false, &Progress::default()).unwrap();
        assert_eq!(fs::read_to_string(dst.join("top.txt")).unwrap(), "1");
        assert_eq!(
            fs::read_to_string(dst.join("sub/deeper/leaf.txt")).unwrap(),
            "2"
        );
    }

    #[cfg(unix)]
    #[test]
    fn copy_entry_recreates_symlinks_instead_of_following() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("tree");
        fs::create_dir(&src).unwrap();
        // A link back to its own parent would recurse forever if followed.
        std::os::unix::fs::symlink(&src, src.join("loop")).unwrap();
        let dst = dir.path().join("copy");

        copy_entry(s(&src), s(&dst), false, &Progress::default()).unwrap();
        let meta = fs::symlink_metadata(dst.join("loop")).unwrap();
        assert!(meta.file_type().is_symlink());
    }

    #[test]
    fn copy_entry_refuses_existing_target_without_overwrite() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("a.txt");
        let dst = dir.path().join("b.txt");
        fs::write(&src, "new").unwrap();
        fs::write(&dst, "old").unwrap();

        let err = copy_entry(s(&src), s(&dst), false, &Progress::default()).unwrap_err();
        assert!(err.contains("already exists"), "unexpected error: {}", err);
        assert_eq!(fs::read_to_string(&dst).unwrap(), "old");
    }

    #[test]
    fn copy_entry_overwrites_when_asked() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("a.txt");
        let dst = dir.path().join("b.txt");
        fs::write(&src, "new").unwrap();
        fs::write(&dst, "old").unwrap();

        copy_entry(s(&src), s(&dst), true, &Progress::default()).unwrap();
        assert_eq!(fs::read_to_string(&dst).unwrap(), "new");
        // No staging leftovers in the folder.
        let names: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names.len(), 2, "unexpected entries: {:?}", names);
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
    fn copy_and_move_refuse_into_itself() {
        let dir = tempdir().unwrap();
        let a = dir.path().join("a");
        fs::create_dir(&a).unwrap();
        let inside = a.join("a");

        assert!(copy_entry(s(&a), s(&inside), false, &Progress::default()).is_err());
        assert!(move_entry(s(&a), s(&inside), false, &Progress::default()).is_err());
        assert!(!inside.exists());
    }

    #[test]
    fn copy_and_move_refuse_same_path() {
        let dir = tempdir().unwrap();
        let f = dir.path().join("f.txt");
        fs::write(&f, "x").unwrap();

        assert!(copy_entry(s(&f), s(&f), true, &Progress::default()).is_err());
        assert!(move_entry(s(&f), s(&f), true, &Progress::default()).is_err());
        assert_eq!(fs::read_to_string(&f).unwrap(), "x");
    }

    #[test]
    fn replacing_a_folder_with_its_own_child_is_refused() {
        let dir = tempdir().unwrap();
        let parent = dir.path().join("p");
        fs::create_dir_all(parent.join("child")).unwrap();

        assert!(copy_entry(s(&parent.join("child")), s(&parent), true, &Progress::default()).is_err());
        assert!(move_entry(s(&parent.join("child")), s(&parent), true, &Progress::default()).is_err());
        assert!(parent.join("child").exists());
    }

    #[test]
    fn move_entry_moves_file_and_removes_source() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("a.txt");
        fs::create_dir(dir.path().join("dest")).unwrap();
        let dst = dir.path().join("dest/a.txt");
        fs::write(&src, "hello").unwrap();

        move_entry(s(&src), s(&dst), false, &Progress::default()).unwrap();
        assert!(!src.exists());
        assert_eq!(fs::read_to_string(&dst).unwrap(), "hello");
    }

    #[test]
    fn move_entry_refuses_existing_target_without_overwrite() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("a.txt");
        let dst = dir.path().join("b.txt");
        fs::write(&src, "new").unwrap();
        fs::write(&dst, "old").unwrap();

        assert!(move_entry(s(&src), s(&dst), false, &Progress::default()).is_err());
        assert!(src.exists());
        assert_eq!(fs::read_to_string(&dst).unwrap(), "old");
    }

    #[test]
    fn move_entry_overwrites_when_asked() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("a.txt");
        let dst = dir.path().join("b.txt");
        fs::write(&src, "new").unwrap();
        fs::write(&dst, "old").unwrap();

        move_entry(s(&src), s(&dst), true, &Progress::default()).unwrap();
        assert!(!src.exists());
        assert_eq!(fs::read_to_string(&dst).unwrap(), "new");
    }

    fn failing_trash(_: &Path) -> Result<Option<PathBuf>, String> {
        Err("no Trash on this volume".to_string())
    }

    fn names_in(dir: &Path) -> Vec<std::ffi::OsString> {
        let mut names: Vec<_> = fs::read_dir(dir).unwrap().flatten().map(|e| e.file_name()).collect();
        names.sort();
        names
    }

    #[test]
    fn replace_move_keeps_the_original_when_the_trash_fails() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("a.txt");
        let dst = dir.path().join("b.txt");
        fs::write(&src, "mine").unwrap();
        fs::write(&dst, "old").unwrap();

        let err = move_entry_with(s(&src), s(&dst), true, &Progress::default(), &failing_trash)
            .unwrap_err();
        assert!(err.contains("Could not move the existing item to the Trash"), "{err}");
        assert_eq!(fs::read_to_string(&src).unwrap(), "mine");
        assert_eq!(fs::read_to_string(&dst).unwrap(), "old");
        assert_eq!(names_in(dir.path()), ["a.txt", "b.txt"]);
    }

    #[test]
    fn replace_folder_move_keeps_the_original_when_the_trash_fails() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("mine");
        fs::create_dir_all(src.join("sub")).unwrap();
        fs::write(src.join("sub/x"), "data").unwrap();
        let dst = dir.path().join("dest/mine");
        fs::create_dir_all(&dst).unwrap();

        assert!(move_entry_with(s(&src), s(&dst), true, &Progress::default(), &failing_trash).is_err());
        assert_eq!(fs::read_to_string(src.join("sub/x")).unwrap(), "data");
        assert!(dst.is_dir());
        assert_eq!(names_in(&dir.path().join("dest")), ["mine"]);
    }

    #[test]
    fn replace_copy_removes_only_its_copy_when_the_trash_fails() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("a.txt");
        let dst = dir.path().join("b.txt");
        fs::write(&src, "mine").unwrap();
        fs::write(&dst, "old").unwrap();

        assert!(copy_entry_with(s(&src), s(&dst), true, &Progress::default(), &failing_trash).is_err());
        assert_eq!(fs::read_to_string(&src).unwrap(), "mine");
        assert_eq!(fs::read_to_string(&dst).unwrap(), "old");
        assert_eq!(names_in(dir.path()), ["a.txt", "b.txt"]);
    }

    #[test]
    fn a_staged_original_never_overwrites_what_took_its_old_place() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("a.txt");
        let dst = dir.path().join("b.txt");
        fs::write(&src, "mine").unwrap();
        fs::write(&dst, "old").unwrap();
        // Something new appears at the original's old path, then the Trash fails.
        let trash = |_: &Path| {
            fs::write(&src, "newer").unwrap();
            Err("no Trash on this volume".to_string())
        };

        let err = move_entry_with(s(&src), s(&dst), true, &Progress::default(), &trash).unwrap_err();
        assert_eq!(fs::read_to_string(&src).unwrap(), "newer");
        assert_eq!(fs::read_to_string(&dst).unwrap(), "old");
        let kept: Vec<_> = names_in(dir.path())
            .into_iter()
            .filter(|n| n.to_string_lossy().starts_with(".b.txt.filebrowser-"))
            .collect();
        assert_eq!(kept.len(), 1, "the original stays at its staging name");
        let staged = dir.path().join(&kept[0]);
        assert_eq!(fs::read_to_string(&staged).unwrap(), "mine");
        assert!(err.contains(&staged.display().to_string()), "{err}");
    }

    #[test]
    fn replace_move_puts_the_original_back_when_the_last_rename_fails() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("a.txt");
        let dst = dir.path().join("b.txt");
        fs::write(&src, "mine").unwrap();
        fs::write(&dst, "old").unwrap();
        let bin = dir.path().join("bin");
        fs::create_dir(&bin).unwrap();
        // Trashes the target, then something else takes its name: a folder
        // with content, which a file can't be renamed onto.
        let trash = |p: &Path| {
            fs::rename(p, bin.join("b.txt")).unwrap();
            fs::create_dir(p).unwrap();
            fs::write(p.join("x"), "other").unwrap();
            Ok(Some(bin.join("b.txt")))
        };

        let err = move_entry_with(s(&src), s(&dst), true, &Progress::default(), &trash).unwrap_err();
        assert!(err.contains("Failed to replace"), "{err}");
        // The old item can't come back over the folder: the error says where it is.
        assert!(err.contains(&format!("in the Trash at “{}”", bin.join("b.txt").display())), "{err}");
        assert_eq!(fs::read_to_string(&src).unwrap(), "mine");
        assert_eq!(fs::read_to_string(bin.join("b.txt")).unwrap(), "old");
        assert_eq!(names_in(dir.path()), ["a.txt", "b.txt", "bin"]);
    }

    #[test]
    fn replace_trashes_through_the_given_function() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("a.txt");
        let dst = dir.path().join("b.txt");
        fs::write(&src, "mine").unwrap();
        fs::write(&dst, "old").unwrap();
        let bin = dir.path().join("bin");
        fs::create_dir(&bin).unwrap();
        let trash = |p: &Path| {
            fs::rename(p, bin.join("b.txt")).map_err(|e| e.to_string())?;
            Ok(Some(bin.join("b.txt")))
        };

        move_entry_with(s(&src), s(&dst), true, &Progress::default(), &trash).unwrap();
        assert!(!src.exists());
        assert_eq!(fs::read_to_string(&dst).unwrap(), "mine");
        assert_eq!(fs::read_to_string(bin.join("b.txt")).unwrap(), "old");
        assert_eq!(names_in(dir.path()), ["b.txt", "bin"]);
    }

    // ----- what the placement refactor must keep -----

    type Op = fn(String, String, bool, &Progress) -> Result<(), String>;
    type OpWith = fn(String, String, bool, &Progress, TrashFn) -> Result<(), String>;

    /// Each refusal's exact words, and which one wins when several apply.
    #[test]
    fn refusals_keep_their_wording_and_order() {
        let dir = tempdir().unwrap();
        let a = dir.path().join("a");
        fs::create_dir_all(a.join("child")).unwrap();
        fs::write(a.join("child/f.txt"), "x").unwrap();
        let gone = dir.path().join("gone");
        let cases = [
            // A vanished source wins over the same path.
            (gone.clone(), gone.clone(), true, format!("“{}” no longer exists", gone.display())),
            // The same path wins over "into itself" (a path is inside itself).
            (a.clone(), a.clone(), true, "The source and destination are the same".to_string()),
            // "Into itself" wins over an existing target, with or without Replace.
            (a.clone(), a.join("child"), false, "Cannot copy or move a folder into itself".to_string()),
            (a.clone(), a.join("child"), true, "Cannot copy or move a folder into itself".to_string()),
            // Without Replace, an existing target wins over "an item inside it".
            (a.join("child"), a.clone(), false, format!("Target already exists: {}", a.display())),
            (a.join("child"), a.clone(), true, "Cannot replace a folder with an item inside it".to_string()),
        ];
        let ops: [(&str, Op); 2] = [("copy", copy_entry), ("move", move_entry)];
        for (name, op) in ops {
            for (from, to, overwrite, want) in &cases {
                let got = op(s(from), s(to), *overwrite, &Progress::default());
                assert_eq!(got, Err(want.clone()), "{name} {} → {}", from.display(), to.display());
            }
        }
        assert_eq!(fs::read_to_string(a.join("child/f.txt")).unwrap(), "x");
        assert_eq!(names_in(dir.path()), ["a"]);
        assert_eq!(names_in(&a), ["child"]);
    }

    /// Replace stages the new item next to the old one, sends the old one to
    /// the Trash, then renames the new one into place: for files and folders,
    /// copied and moved.
    #[test]
    fn replace_stages_then_trashes_then_renames_into_place() {
        let ops: [(&str, OpWith); 2] = [("copy", copy_entry_with), ("move", move_entry_with)];
        for (name, op) in ops {
            for folder in [false, true] {
                let dir = tempdir().unwrap();
                let dest = dir.path().join("dest");
                let (src, dst, bin) = (dir.path().join("new"), dest.join("old"), dir.path().join("bin"));
                fs::create_dir_all(&dest).unwrap();
                fs::create_dir(&bin).unwrap();
                if folder {
                    fs::create_dir_all(src.join("sub")).unwrap();
                    fs::write(src.join("sub/x"), "new").unwrap();
                    fs::create_dir(&dst).unwrap();
                    fs::write(dst.join("y"), "old").unwrap();
                } else {
                    fs::write(&src, "new").unwrap();
                    fs::write(&dst, "old").unwrap();
                }
                let seen = std::cell::RefCell::new(Vec::new());
                let trash = |p: &Path| {
                    seen.borrow_mut().push((p.to_path_buf(), names_in(&dest)));
                    fs::rename(p, bin.join("old")).map_err(|e| e.to_string())?;
                    Ok(Some(bin.join("old")))
                };
                op(s(&src), s(&dst), true, &Progress::default(), &trash).unwrap();

                let seen = seen.into_inner();
                assert_eq!(seen.len(), 1, "{name}");
                assert_eq!(seen[0].0, dst, "{name}: the old item is what goes to the Trash");
                let staged: Vec<_> = seen[0].1.iter().map(|n| n.to_string_lossy().into_owned()).collect();
                assert_eq!(staged.len(), 2, "{name}: {staged:?}");
                assert_eq!(staged[0], format!(".old.filebrowser-{}-0", std::process::id()), "{name}");
                assert_eq!(staged[1], "old");
                assert_eq!(names_in(&dest), ["old"], "{name}");
                let (new, old) = if folder {
                    (dst.join("sub/x"), bin.join("old/y"))
                } else {
                    (dst.clone(), bin.join("old"))
                };
                assert_eq!(fs::read_to_string(new).unwrap(), "new", "{name}");
                assert_eq!(fs::read_to_string(old).unwrap(), "old", "{name}");
                assert_eq!(src.exists(), name == "copy", "{name}");
            }
        }
    }

    /// New with the placement module: Replace onto `dir/..` or `dir/.` would
    /// send a whole folder to the Trash, so copy and move refuse it, as Drive
    /// downloads already did.
    #[test]
    fn copy_and_move_refuse_a_target_that_is_not_a_name() {
        let (dir, elsewhere) = (tempdir().unwrap(), tempdir().unwrap());
        let src = elsewhere.path().join("a.txt");
        fs::write(&src, "mine").unwrap();
        let sub = dir.path().join("sub");
        fs::create_dir(&sub).unwrap();
        fs::write(sub.join("keep.txt"), "kept").unwrap();
        let never = |p: &Path| -> Result<Option<PathBuf>, String> { panic!("{} went to the Trash", p.display()) };
        let ops: [OpWith; 2] = [copy_entry_with, move_entry_with];
        for op in ops {
            for to in [sub.join(".."), sub.join(".")] {
                let want = format!("“{}” is not a valid name for a local item.", to.display());
                assert_eq!(op(s(&src), s(&to), true, &Progress::default(), &never), Err(want));
            }
        }
        assert_eq!(fs::read_to_string(&src).unwrap(), "mine");
        assert_eq!(fs::read_to_string(sub.join("keep.txt")).unwrap(), "kept");
    }

    /// The error a failed Replace reports when the Trash refuses.
    #[test]
    fn a_refused_trash_is_reported_word_for_word() {
        let dir = tempdir().unwrap();
        let (src, dst) = (dir.path().join("a.txt"), dir.path().join("b.txt"));
        fs::write(&src, "mine").unwrap();
        fs::write(&dst, "old").unwrap();
        let want = Err("Could not move the existing item to the Trash: no Trash on this volume".to_string());
        assert_eq!(copy_entry_with(s(&src), s(&dst), true, &Progress::default(), &failing_trash), want);
        assert_eq!(move_entry_with(s(&src), s(&dst), true, &Progress::default(), &failing_trash), want);
        assert_eq!(names_in(dir.path()), ["a.txt", "b.txt"]);
    }

    fn across_volumes(_: &Path, _: &Path) -> std::io::Result<()> {
        Err(std::io::Error::from_raw_os_error(18)) // EXDEV
    }

    #[test]
    fn a_move_across_volumes_copies_then_removes_the_source() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("a");
        fs::create_dir_all(src.join("sub")).unwrap();
        fs::write(src.join("sub/x"), "data").unwrap();
        let dst = dir.path().join("b");
        let progress = Progress::default();
        move_entry_by(s(&src), s(&dst), false, &progress, &failing_trash, &across_volumes).unwrap();
        assert!(!src.exists());
        assert_eq!(fs::read_to_string(dst.join("sub/x")).unwrap(), "data");
        assert_eq!(progress.snapshot().items, 3);
    }

    /// The source goes only once its copy has replaced the old item, and a
    /// cancel arriving after the copy can't stop the move halfway.
    #[test]
    fn a_replacing_move_across_volumes_removes_the_source_last() {
        let dir = tempdir().unwrap();
        let (src, dst, bin) = (dir.path().join("a.txt"), dir.path().join("b.txt"), dir.path().join("bin"));
        fs::write(&src, "mine").unwrap();
        fs::write(&dst, "old").unwrap();
        fs::create_dir(&bin).unwrap();
        let progress = Progress::default();
        let source_there = std::cell::Cell::new(false);
        let trash = |p: &Path| {
            source_there.set(src.exists());
            progress.cancel();
            fs::rename(p, bin.join("b.txt")).map_err(|e| e.to_string())?;
            Ok(Some(bin.join("b.txt")))
        };
        move_entry_by(s(&src), s(&dst), true, &progress, &trash, &across_volumes).unwrap();
        assert!(source_there.get(), "the source was still there during the Replace");
        assert!(!src.exists());
        assert_eq!(fs::read_to_string(&dst).unwrap(), "mine");
        assert_eq!(fs::read_to_string(bin.join("b.txt")).unwrap(), "old");
        assert_eq!(names_in(dir.path()), ["b.txt", "bin"]);
    }

    #[test]
    fn a_failed_replace_across_volumes_keeps_the_source_and_the_target() {
        let dir = tempdir().unwrap();
        let (src, dst) = (dir.path().join("a.txt"), dir.path().join("b.txt"));
        fs::write(&src, "mine").unwrap();
        fs::write(&dst, "old").unwrap();
        let err = move_entry_by(s(&src), s(&dst), true, &Progress::default(), &failing_trash, &across_volumes)
            .unwrap_err();
        assert_eq!(err, "Could not move the existing item to the Trash: no Trash on this volume");
        assert_eq!(fs::read_to_string(&src).unwrap(), "mine");
        assert_eq!(fs::read_to_string(&dst).unwrap(), "old");
        assert_eq!(names_in(dir.path()), ["a.txt", "b.txt"]);
    }

    #[test]
    fn a_cancelled_copy_across_volumes_leaves_nothing_and_keeps_the_source() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("a");
        fs::create_dir(&src).unwrap();
        fs::write(src.join("x"), "data").unwrap();
        for (to, overwrite) in [(dir.path().join("b"), false), (dir.path().join("c"), true)] {
            if overwrite {
                fs::write(&to, "old").unwrap();
            }
            let progress = Progress::default();
            progress.cancel();
            let got = move_entry_by(s(&src), s(&to), overwrite, &progress, &failing_trash, &across_volumes);
            assert_eq!(got, Err("Cancelled.".to_string()));
            assert_eq!(fs::read_to_string(src.join("x")).unwrap(), "data");
        }
        assert!(!dir.path().join("b").exists());
        assert_eq!(fs::read_to_string(dir.path().join("c")).unwrap(), "old");
        assert_eq!(names_in(dir.path()), ["a", "c"]);
    }

    /// A rename that fails for another reason touches nothing, staged or not.
    #[test]
    fn a_failed_rename_is_reported_and_touches_nothing() {
        let dir = tempdir().unwrap();
        let (src, dst) = (dir.path().join("a.txt"), dir.path().join("b.txt"));
        fs::write(&src, "mine").unwrap();
        let denied = |_: &Path, _: &Path| Err(std::io::Error::from_raw_os_error(13));
        let want = format!("Failed to move: {}", std::io::Error::from_raw_os_error(13));
        for overwrite in [false, true] {
            let got = move_entry_by(s(&src), s(&dst), overwrite, &Progress::default(), &failing_trash, &denied);
            assert_eq!(got, Err(want.clone()));
            fs::write(&dst, "old").unwrap();
        }
        assert_eq!(fs::read_to_string(&src).unwrap(), "mine");
        assert_eq!(names_in(dir.path()), ["a.txt", "b.txt"]);
    }

    #[test]
    fn a_failed_copy_reports_its_cause() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("a.txt");
        fs::write(&src, "mine").unwrap();
        let to = dir.path().join("missing/b.txt");
        let err = copy_entry(s(&src), s(&to), false, &Progress::default()).unwrap_err();
        assert!(err.starts_with("Failed to copy: "), "{err}");
        assert_eq!(names_in(dir.path()), ["a.txt"]);
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

    #[test]
    fn operations_on_vanished_path_return_err() {
        let dir = tempdir().unwrap();
        let gone = dir.path().join("gone");
        let dst = dir.path().join("dst");

        assert!(copy_entry(s(&gone), s(&dst), false, &Progress::default()).is_err());
        assert!(move_entry(s(&gone), s(&dst), false, &Progress::default()).is_err());
        assert!(!path_exists(s(&gone)));
    }

    #[test]
    fn copy_reports_bytes_and_items() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("a");
        fs::create_dir_all(src.join("sub")).unwrap();
        fs::write(src.join("x"), vec![1u8; 1000]).unwrap();
        fs::write(src.join("y"), vec![2u8; 24]).unwrap();
        fs::write(src.join("sub/z"), b"").unwrap();
        let progress = Progress::default();
        copy_entry(s(&src), s(&dir.path().join("b")), false, &progress).unwrap();
        let update = progress.snapshot();
        assert_eq!(update.bytes, 1024);
        assert_eq!(update.items, 5);
    }

    #[test]
    fn empty_files_and_folders_count_as_items() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("empty");
        fs::create_dir(&src).unwrap();
        let progress = Progress::default();
        copy_entry(s(&src), s(&dir.path().join("copy")), false, &progress).unwrap();
        assert_eq!(progress.snapshot().items, 1);
        assert_eq!(progress.snapshot().bytes, 0);
    }

    #[test]
    fn cancelled_copy_leaves_nothing() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("a.txt");
        fs::write(&src, b"hello").unwrap();
        let dst = dir.path().join("b.txt");
        let progress = Progress::default();
        progress.cancel();
        assert_eq!(
            copy_entry(s(&src), s(&dst), false, &progress),
            Err("Cancelled.".to_string())
        );
        assert!(!dst.exists());
        assert_eq!(fs::read(&src).unwrap(), b"hello");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn cancelled_copy_mid_file_leaves_nothing() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("big.bin");
        fs::write(&src, vec![9u8; 64 * 1024 * 1024]).unwrap();
        let dst = dir.path().join("copy.bin");
        let progress = Progress::default();
        let watcher = {
            let progress = progress.clone();
            std::thread::spawn(move || {
                while progress.snapshot().bytes < 1024 * 1024 {
                    std::thread::yield_now();
                }
                progress.cancel();
            })
        };
        let result = copy_entry(s(&src), s(&dst), false, &progress);
        watcher.join().unwrap();
        assert_eq!(result, Err("Cancelled.".to_string()));
        assert!(!dst.exists());
    }

    #[test]
    fn cancelled_replace_keeps_the_original() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("new.txt");
        fs::write(&src, b"new").unwrap();
        let dst = dir.path().join("target.txt");
        fs::write(&dst, b"old").unwrap();
        let progress = Progress::default();
        progress.cancel();
        assert_eq!(
            copy_entry(s(&src), s(&dst), true, &progress),
            Err("Cancelled.".to_string())
        );
        assert_eq!(fs::read(&dst).unwrap(), b"old");
        let names: Vec<_> = fs::read_dir(dir.path()).unwrap().flatten().map(|e| e.file_name()).collect();
        assert_eq!(names.len(), 2, "no staging file left: {names:?}");
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

    // ----- what took the name after the check -----

    /// A file, or a folder holding `x`, with `body` in it.
    fn make_item(path: &Path, folder: bool, body: &str) {
        if folder {
            fs::create_dir(path).unwrap();
            fs::write(path.join("x"), body).unwrap();
        } else {
            fs::write(path, body).unwrap();
        }
    }

    fn item_body(path: &Path) -> String {
        let file = if path.is_dir() { path.join("x") } else { path.to_path_buf() };
        fs::read_to_string(file).unwrap()
    }

    /// Every pair of (source is a folder, what took the name is a folder).
    const KINDS: [(bool, bool); 4] = [(false, false), (false, true), (true, false), (true, true)];

    /// The target passes the refusals, then another app takes its name
    /// before the copy or move begins.
    fn checked_then_taken(src: &Path, dst: &Path, theirs_folder: bool) -> Target {
        let target = placement::check_transfer(src, dst, false).unwrap();
        make_item(dst, theirs_folder, "theirs");
        target
    }

    /// A copy never writes into what took its name after the check (a file
    /// was truncated and overwritten) nor cleans it up as its own partial
    /// copy (a folder was removed). It fails as the check would have.
    #[test]
    fn a_copy_leaves_alone_what_took_its_name_after_the_check() {
        for (mine_folder, theirs_folder) in KINDS {
            let dir = tempdir().unwrap();
            let (src, dst) = (dir.path().join("mine"), dir.path().join("dest"));
            make_item(&src, mine_folder, "mine");
            let target = checked_then_taken(&src, &dst, theirs_folder);
            let got = copy_to(&target, &src, &Progress::default(), &failing_trash);
            let case = format!("folder → {mine_folder}, theirs a folder → {theirs_folder}");
            assert_eq!(got, Err(format!("Target already exists: {}", dst.display())), "{case}");
            assert_eq!(item_body(&dst), "theirs", "{case}");
            assert_eq!(item_body(&src), "mine", "{case}");
            assert_eq!(names_in(dir.path()), ["dest", "mine"], "{case}");
        }
    }

    /// The same for a move, renamed on one volume or copied across two: the
    /// source stays where it was.
    #[test]
    fn a_move_leaves_alone_what_took_its_name_after_the_check() {
        let renames: [(&str, RenameFn); 2] = [("rename", MOVE_RENAME), ("across volumes", &across_volumes)];
        for (how, rename) in renames {
            for (mine_folder, theirs_folder) in KINDS {
                let dir = tempdir().unwrap();
                let (src, dst) = (dir.path().join("mine"), dir.path().join("dest"));
                make_item(&src, mine_folder, "mine");
                let target = checked_then_taken(&src, &dst, theirs_folder);
                let got = move_to(&target, &src, &Progress::default(), &failing_trash, rename);
                let case = format!("{how}: folder → {mine_folder}, theirs a folder → {theirs_folder}");
                assert_eq!(got, Err(format!("Target already exists: {}", dst.display())), "{case}");
                assert_eq!(item_body(&dst), "theirs", "{case}");
                assert_eq!(item_body(&src), "mine", "{case}");
                assert_eq!(names_in(dir.path()), ["dest", "mine"], "{case}");
            }
        }
    }

    /// A file is copied under a staging name and appears whole; a folder
    /// fills in place, so it can be watched filling, as in GNOME Files.
    #[test]
    fn files_are_staged_and_folders_fill_in_place() {
        for folder in [false, true] {
            let dir = tempdir().unwrap();
            let (src, dst) = (dir.path().join("mine"), dir.path().join("dest"));
            make_item(&src, folder, "mine");
            let seen = std::cell::RefCell::new(Vec::new());
            let watch = |from: &Path, to: &Path| {
                seen.borrow_mut().push(to.to_path_buf());
                across_volumes(from, to)
            };
            let target = placement::check_transfer(&src, &dst, false).unwrap();
            move_to(&target, &src, &Progress::default(), &failing_trash, &watch).unwrap();
            let filled = seen.into_inner();
            if folder {
                assert_eq!(filled, [dst.clone()]);
            } else {
                let staged = dir.path().join(format!(".dest.filebrowser-{}-0", std::process::id()));
                assert_eq!(filled, [staged]);
            }
            assert_eq!(item_body(&dst), "mine");
            assert_eq!(names_in(dir.path()), ["dest"]);
        }
    }
}
