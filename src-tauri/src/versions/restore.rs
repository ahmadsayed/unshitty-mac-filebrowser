//! Version history for users: list, diff, Restore Copy, Replace, and deleted
//! files. Rebuilds happen through `store`; placement never overwrites.

use std::fs::{self, File, FileTimes, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

use similar::{ChangeTag, TextDiff};

use crate::versions::{
    DeletedFile, DiffLine, Error, Result, VersionDiff, VersionInfo, VersionRow, capture,
    history::History, index, join_rel, rel_path, store::Store, validate_rel_path,
};

const MAX_DIFF_INPUT: u64 = 1 << 20;
const MAX_DIFF_LINES: usize = 2000;
static NEXT_STAGING: AtomicU64 = AtomicU64::new(0);
static NEXT_OPEN: AtomicU64 = AtomicU64::new(0);

/// Rows for the Versions… dialog, newest first. `current` is stat-based: the
/// latest row plus a matching size/mtime, never a hash (D9/R6).
pub fn list_infos(
    history: &History,
    rel: &[u8],
    limit: Option<u32>,
    offset: Option<u32>,
) -> Result<Vec<VersionInfo>> {
    let reader = history.reader()?;
    let rows = index::list_versions(&reader, rel, limit, offset)?;
    let latest = index::latest_version(&reader, rel)?;
    drop(reader);

    let current_id = current_row_id(history, rel, latest.as_ref());
    Ok(rows
        .into_iter()
        .map(|row| VersionInfo {
            id: row.id,
            size: row.size,
            mtime: row.mtime / 1_000_000_000,
            recorded_at: row.recorded_at,
            deleted: row.is_tombstone(),
            current: current_id == Some(row.id),
        })
        .collect())
}

pub fn version_count(history: &History, rel: &[u8]) -> Result<u64> {
    let reader = history.reader()?;
    index::count_versions(&reader, rel)
}

fn current_row_id(history: &History, rel: &[u8], latest: Option<&VersionRow>) -> Option<i64> {
    let latest = latest.filter(|row| row.hash.is_some())?;
    let path = join_rel(&history.root, rel);
    let meta = fs::symlink_metadata(path).ok()?;
    if !meta.is_file() || meta.len() != latest.size {
        return None;
    }
    if crate::versions::mtime_nanos(&meta) != latest.mtime {
        return None;
    }
    Some(latest.id)
}

/// D9: the selected version against the file as it is now.
pub fn diff(history: &History, rel: &[u8], version: i64) -> Result<VersionDiff> {
    let row = {
        let reader = history.reader()?;
        index::get_version(&reader, version)?
    }
    .ok_or_else(|| Error::IndexDamaged("unknown version id".into()))?;
    if row.path != rel {
        return Err(Error::Damaged);
    }
    if row.is_tombstone() {
        return Ok(VersionDiff::Deleted);
    }
    let path = join_rel(&history.root, rel);
    let Ok(meta) = fs::symlink_metadata(&path) else {
        return Ok(VersionDiff::NotOnDisk);
    };
    if !meta.is_file() {
        return Ok(VersionDiff::NotOnDisk);
    }
    if row.size > MAX_DIFF_INPUT || meta.len() > MAX_DIFF_INPUT {
        return Ok(VersionDiff::TooLarge);
    }
    let old = rebuild(history, &row)?;
    let new = fs::read(&path)?;
    if old == new {
        return Ok(VersionDiff::Same);
    }
    if !is_text(&old) || !is_text(&new) {
        return Ok(VersionDiff::Binary);
    }
    let (old, new) = (String::from_utf8_lossy(&old), String::from_utf8_lossy(&new));
    let diff = TextDiff::from_lines(old.as_ref(), new.as_ref());
    let mut lines = Vec::new();
    let mut truncated = false;
    for change in diff.iter_all_changes() {
        if lines.len() >= MAX_DIFF_LINES {
            truncated = true;
            break;
        }
        let tag = match change.tag() {
            ChangeTag::Equal => ' ',
            ChangeTag::Delete => '-',
            ChangeTag::Insert => '+',
        };
        let text = change
            .value()
            .trim_end_matches('\n')
            .trim_end_matches('\r')
            .to_string();
        lines.push(DiffLine { tag, text });
    }
    Ok(VersionDiff::Text { lines, truncated })
}

fn is_text(bytes: &[u8]) -> bool {
    !bytes.iter().take(8 * 1024).any(|byte| *byte == 0) && std::str::from_utf8(bytes).is_ok()
}

/// Restore Copy: rebuild `version` next to the file as `name`, never
/// overwriting. Returns the new path.
pub fn restore_copy(history: &History, rel: &[u8], version: i64, name: &str) -> Result<PathBuf> {
    validate_name(name)?;
    let row = version_row(history, version)?;
    if row.path != rel || row.is_tombstone() {
        return Err(Error::Damaged);
    }
    let original = join_rel(&history.root, rel);
    let parent = original.parent().ok_or(Error::Damaged)?;
    fs::create_dir_all(parent)?;
    let staging = write_staging(history, &row)?;
    let result = place_copy(&staging, parent, name);
    let _ = fs::remove_file(&staging);
    result
}

/// Replace the current file with `version`. The current state is recorded
/// first (D8), then moved to the caller's Trash. The staged file is copied in
/// with `create_new`, so a race cannot overwrite anything.
pub fn replace_with_version(history: &History, rel: &[u8], version: i64) -> Result<()> {
    replace_with(history, rel, version, |path| {
        trash::delete(path).map_err(|e| Error::Io(format!("could not move to the Trash: {e}")))
    })
}

fn replace_with<F>(history: &History, rel: &[u8], version: i64, trash: F) -> Result<()>
where
    F: FnOnce(&Path) -> Result<()>,
{
    if !history.writable {
        return Err(Error::ReadOnly);
    }
    let row = version_row(history, version)?;
    if row.path != rel || row.is_tombstone() {
        return Err(Error::Damaged);
    }
    let _guard = history
        .capture_lock
        .lock()
        .map_err(|_| Error::Io("capture lock poisoned".into()))?;
    let relative = rel_path(rel);
    // D8: record the current state synchronously, even if the watcher is
    // behind. If that fails the dialog's promise ("It stays in the history
    // too") would be false, so refuse instead of replacing.
    capture::record_locked(history, &relative).map_err(|e| {
        Error::Io(format!(
            "the current file could not be saved to the history first, so it was not replaced ({e})"
        ))
    })?;

    let original = join_rel(&history.root, rel);
    let staging = write_staging(history, &row)?;
    let current = fs::symlink_metadata(&original).ok();
    // The replacement keeps the current file's permissions (an executable
    // script stays executable); a fresh `create_new` file would get 0666 & umask.
    let permissions = current
        .as_ref()
        .filter(|meta| meta.is_file())
        .map(|meta| meta.permissions());
    if current.is_some() {
        if let Err(e) = trash(&original) {
            let _ = fs::remove_file(&staging);
            return Err(e);
        }
    }
    if let Some(parent) = original.parent() {
        fs::create_dir_all(parent)?;
    }
    let placed = copy_new(&staging, &original);
    if placed.is_ok() {
        let _ = fs::remove_file(&staging);
        if let Some(permissions) = permissions {
            let _ = fs::set_permissions(&original, permissions);
        }
        Ok(())
    } else {
        // Keep the staged copy so the restored bytes are not lost; the old
        // file is in the Trash and the error names both.
        Err(Error::Io(format!(
            "The old file is in the Trash and the restored copy is at {}. {}",
            staging.display(),
            placed.unwrap_err()
        )))
    }
}

/// Deleted Files…: latest tombstones under `dir` (or the root), newest first.
pub fn deleted_files(history: &History, dir: Option<&[u8]>) -> Result<Vec<DeletedFile>> {
    let reader = history.reader()?;
    let mut rows = index::deleted_files(&reader, dir)?;
    rows.sort_by(|a, b| b.deleted_at.cmp(&a.deleted_at));
    Ok(rows
        .into_iter()
        .map(|row| DeletedFile {
            path: join_rel(&history.root, &row.path)
                .to_string_lossy()
                .into_owned(),
            deleted_at: row.deleted_at,
            size: row.size,
        })
        .collect())
}

/// Restore the last real version of a deleted path. If the original path is
/// occupied, place it under `fallback_name` in the same folder instead.
pub fn restore_deleted(history: &History, rel: &[u8], fallback_name: &str) -> Result<PathBuf> {
    validate_rel_path(rel)?;
    let row = {
        let reader = history.reader()?;
        index::last_live_version(&reader, rel)?
    }
    .ok_or_else(|| Error::Io("there is no version of that file to restore".into()))?;
    let original = join_rel(&history.root, rel);
    if let Some(parent) = original.parent() {
        fs::create_dir_all(parent)?;
    }
    let staging = write_staging(history, &row)?;
    let result = if fs::symlink_metadata(&original).is_err() {
        match copy_new(&staging, &original) {
            Ok(()) => Ok(original.clone()),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists && !fallback_name.is_empty() => {
                validate_name(fallback_name)?;
                let parent = original.parent().ok_or(Error::Damaged)?;
                place_copy(&staging, parent, fallback_name)
            }
            Err(e) => Err(Error::Io(e.to_string())),
        }
    } else {
        if fallback_name.is_empty() {
            let _ = fs::remove_file(&staging);
            return Err(Error::Io(
                "something is already at that path and no fallback name was given".into(),
            ));
        }
        validate_name(fallback_name)?;
        let parent = original.parent().ok_or(Error::Damaged)?;
        place_copy(&staging, parent, fallback_name)
    };
    let _ = fs::remove_file(&staging);
    result
}

/// Materialize `version` as a read-only file under `base` (the app cache),
/// named `name`, for opening in the default app. The versioned folder and its
/// history are never touched.
pub fn open_copy(
    history: &History,
    rel: &[u8],
    version: i64,
    name: &str,
    base: &Path,
) -> Result<PathBuf> {
    let row = version_row(history, version)?;
    if row.path != rel || row.is_tombstone() {
        return Err(Error::Damaged);
    }
    open_row(history, &row, name, base)
}

/// Materialize the last real version of a deleted path as a read-only copy.
pub fn open_deleted(history: &History, rel: &[u8], name: &str, base: &Path) -> Result<PathBuf> {
    validate_rel_path(rel)?;
    let row = {
        let reader = history.reader()?;
        index::last_live_version(&reader, rel)?
    }
    .ok_or_else(|| Error::Io("there is no version of that file to open".into()))?;
    open_row(history, &row, name, base)
}

fn open_row(history: &History, row: &VersionRow, name: &str, base: &Path) -> Result<PathBuf> {
    validate_name(name)?;
    if row.is_tombstone() {
        return Err(Error::Damaged);
    }
    let dir = new_open_dir(base)?;
    let path = dir.join(name);
    if let Err(error) = write_read_only(history, row, &path) {
        let _ = fs::remove_dir_all(&dir);
        return Err(error);
    }
    Ok(path)
}

fn write_read_only(history: &History, row: &VersionRow, path: &Path) -> Result<()> {
    let hash = row.hash.as_deref().ok_or(Error::Damaged)?;
    let chain = {
        let reader = history.reader()?;
        capture::chain_for(&reader, hash)?
    };
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    let store = Store::new(history.objects.clone(), history.tmp.clone());
    if let Err(error) = store.read_to(&chain, hash, &mut file) {
        drop(file);
        let _ = fs::remove_file(path);
        return Err(error);
    }
    file.sync_all()?;
    // Best effort: some filesystems keep coarser timestamps.
    if let Ok(nanos) = u64::try_from(row.mtime) {
        let when = SystemTime::UNIX_EPOCH + Duration::from_nanos(nanos);
        let _ = file.set_times(FileTimes::new().set_modified(when));
    }
    drop(file);
    make_read_only(path)
}

/// A private folder for one opened copy. The creation second is part of the
/// name so `prune_opened` does not have to trust directory mtimes.
fn new_open_dir(base: &Path) -> Result<PathBuf> {
    fs::create_dir_all(base)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(base, fs::Permissions::from_mode(0o700));
    }
    for _ in 0..1000 {
        let n = NEXT_OPEN.fetch_add(1, Ordering::Relaxed);
        let dir = base.join(format!(
            "{}-{}-{n}",
            crate::versions::now_seconds(),
            std::process::id()
        ));
        match fs::create_dir(&dir) {
            Ok(()) => {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let _ = fs::set_permissions(&dir, fs::Permissions::from_mode(0o700));
                }
                return Ok(dir);
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(Error::Io(e.to_string())),
        }
    }
    Err(Error::Io(
        "could not create a folder for the opened copy".into(),
    ))
}

/// Delete opened copies older than `max_age_seconds`, using the creation stamp
/// in the folder name. Unknown entries are left alone.
pub fn prune_opened(base: &Path, now: i64, max_age_seconds: i64) {
    let Ok(entries) = fs::read_dir(base) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let created = name.split('-').next().unwrap_or("");
        let Ok(created) = created.parse::<i64>() else {
            continue;
        };
        if now.saturating_sub(created) < max_age_seconds {
            continue;
        }
        let path = entry.path();
        let _ = if path.is_dir() {
            fs::remove_dir_all(&path)
        } else {
            fs::remove_file(&path)
        };
    }
}

#[cfg(unix)]
fn make_read_only(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o444))?;
    Ok(())
}

#[cfg(not(unix))]
fn make_read_only(path: &Path) -> Result<()> {
    let mut permissions = fs::metadata(path)?.permissions();
    permissions.set_readonly(true);
    fs::set_permissions(path, permissions)?;
    Ok(())
}

fn version_row(history: &History, version: i64) -> Result<VersionRow> {
    let reader = history.reader()?;
    index::get_version(&reader, version)?
        .ok_or_else(|| Error::IndexDamaged("unknown version id".into()))
}

fn rebuild(history: &History, row: &VersionRow) -> Result<Vec<u8>> {
    let hash = row.hash.as_deref().ok_or(Error::Damaged)?;
    let chain = {
        let reader = history.reader()?;
        capture::chain_for(&reader, hash)?
    };
    let store = Store::new(history.objects.clone(), history.tmp.clone());
    store.read(&chain, hash)
}

fn write_staging(history: &History, row: &VersionRow) -> Result<PathBuf> {
    let hash = row.hash.as_deref().ok_or(Error::Damaged)?;
    let chain = {
        let reader = history.reader()?;
        capture::chain_for(&reader, hash)?
    };
    let n = NEXT_STAGING.fetch_add(1, Ordering::Relaxed);
    let path = history
        .tmp
        .join(format!("restore-{}-{n}", std::process::id()));
    let mut file = File::create(&path)?;
    let store = Store::new(history.objects.clone(), history.tmp.clone());
    store.read_to(&chain, hash, &mut file)?;
    file.sync_all()?;
    Ok(path)
}

fn place_copy(staging: &Path, parent: &Path, name: &str) -> Result<PathBuf> {
    for n in 0..10_000u32 {
        let candidate = parent.join(if n == 0 {
            name.to_string()
        } else {
            suffixed_name(name, n + 1)
        });
        match copy_new(staging, &candidate) {
            Ok(()) => return Ok(candidate),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(Error::Io(e.to_string())),
        }
    }
    Err(Error::Io("could not find a free name".into()))
}

/// `notes.txt`, 2 → `notes 2.txt`; `archive.tar.gz` → `archive.tar 2.gz`.
fn suffixed_name(name: &str, n: u32) -> String {
    let path = Path::new(name);
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| name.to_string());
    match path.extension() {
        Some(ext) => format!("{stem} {n}.{}", ext.to_string_lossy()),
        None => format!("{stem} {n}"),
    }
}

fn copy_new(from: &Path, to: &Path) -> io::Result<()> {
    let mut input = File::open(from)?;
    let mut output = OpenOptions::new().write(true).create_new(true).open(to)?;
    io::copy(&mut input, &mut output)?;
    output.sync_all()?;
    Ok(())
}

pub fn validate_name(name: &str) -> Result<()> {
    if name.is_empty() || name == "." || name == ".." || name == crate::versions::VERSIONS_DIR {
        return Err(Error::Io("invalid restore name".into()));
    }
    if name.contains('/') || name.contains('\0') {
        return Err(Error::Io("invalid restore name".into()));
    }
    Ok(())
}

/// The last non-tombstone version size for a tombstone path (tests only).
#[cfg(test)]
pub fn last_size(history: &History, rel: &[u8]) -> Result<u64> {
    let reader = history.reader()?;
    Ok(index::last_live_version(&reader, rel)?
        .map(|row| row.size)
        .unwrap_or(0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::versions::{capture, history::History};
    use std::io::Write;
    use tempfile::tempdir;

    fn history_with_file(bytes: &[u8]) -> (tempfile::TempDir, History) {
        let dir = tempdir().unwrap();
        let history = History::create(dir.path()).unwrap();
        let path = history.root.join("notes.txt");
        let mut file = File::create(&path).unwrap();
        file.write_all(bytes).unwrap();
        file.sync_all().unwrap();
        capture::record(&history, Path::new("notes.txt")).unwrap();
        (dir, history)
    }

    fn edit(history: &History, bytes: &[u8]) {
        let path = history.root.join("notes.txt");
        fs::write(&path, bytes).unwrap();
        capture::record(history, Path::new("notes.txt")).unwrap();
    }

    fn ids(history: &History) -> Vec<i64> {
        list_infos(history, b"notes.txt", None, None)
            .unwrap()
            .into_iter()
            .map(|v| v.id)
            .collect()
    }

    #[test]
    fn lists_newest_first_and_marks_only_the_latest_current() {
        let (_dir, history) = history_with_file(b"one\n");
        edit(&history, b"two\n");
        let infos = list_infos(&history, b"notes.txt", None, None).unwrap();
        assert_eq!(infos.len(), 2);
        assert!(infos[0].current);
        assert!(!infos[1].current);
        assert_eq!(version_count(&history, b"notes.txt").unwrap(), 2);
        // A touched older row must not become current.
        assert_eq!(ids(&history), [2, 1]);
    }

    #[test]
    fn diffs_text_and_reports_same_binary_and_large() {
        let (_dir, history) = history_with_file(b"old line\n");
        edit(&history, b"old line\nnew line\n");
        let old = ids(&history)[1];
        let current = ids(&history)[0];
        match diff(&history, b"notes.txt", old).unwrap() {
            VersionDiff::Text { lines, truncated } => {
                assert!(!truncated);
                assert!(lines.iter().any(|l| l.tag == '+'));
                assert!(lines.iter().any(|l| l.tag == ' '));
            }
            other => panic!("expected text, got {other:?}"),
        }
        assert_eq!(
            diff(&history, b"notes.txt", current).unwrap(),
            VersionDiff::Same
        );

        edit(&history, &[0, 1, 2, 3]);
        let binary = ids(&history)[1];
        assert_eq!(
            diff(&history, b"notes.txt", binary).unwrap(),
            VersionDiff::Binary
        );
    }

    #[test]
    fn too_large_is_decided_before_rebuilding() {
        let (_dir, history) = history_with_file(&vec![b'a'; MAX_DIFF_INPUT as usize + 1]);
        edit(&history, b"small\n");
        let old = ids(&history)[1];
        assert_eq!(
            diff(&history, b"notes.txt", old).unwrap(),
            VersionDiff::TooLarge
        );
    }

    #[test]
    fn restore_copy_never_overwrites() {
        let (_dir, history) = history_with_file(b"one\n");
        edit(&history, b"two\n");
        let old = ids(&history)[1];
        let first = restore_copy(&history, b"notes.txt", old, "notes (old).txt").unwrap();
        assert_eq!(fs::read(&first).unwrap(), b"one\n");
        let second = restore_copy(&history, b"notes.txt", old, "notes (old).txt").unwrap();
        assert_ne!(first, second);
        assert_eq!(fs::read(&second).unwrap(), b"one\n");
        assert_eq!(fs::read(history.root.join("notes.txt")).unwrap(), b"two\n");
    }

    #[test]
    fn replace_records_the_old_state_and_swaps_in_the_version() {
        let (_dir, history) = history_with_file(b"one\n");
        edit(&history, b"two\n");
        let old = ids(&history)[1];
        let trashed = history.root.join("trashed-current");
        replace_with(&history, b"notes.txt", old, |path| {
            fs::rename(path, &trashed).map_err(|e| Error::Io(e.to_string()))
        })
        .unwrap();
        assert_eq!(fs::read(history.root.join("notes.txt")).unwrap(), b"one\n");
        assert_eq!(fs::read(&trashed).unwrap(), b"two\n");
        // The replaced state is still in history.
        let rows = list_infos(&history, b"notes.txt", None, None).unwrap();
        assert!(rows.iter().any(|row| row.size == 4));
        assert!(rows.len() >= 2);
    }

    #[test]
    fn deleted_files_and_restore() {
        let (_dir, history) = history_with_file(b"gone\n");
        fs::remove_file(history.root.join("notes.txt")).unwrap();
        capture::record(&history, Path::new("notes.txt")).unwrap();
        let deleted = deleted_files(&history, None).unwrap();
        assert_eq!(deleted.len(), 1);
        assert_eq!(
            deleted[0].path,
            history.root.join("notes.txt").to_string_lossy()
        );
        assert_eq!(deleted[0].size, 5);
        assert_eq!(last_size(&history, b"notes.txt").unwrap(), 5);

        let restored = restore_deleted(&history, b"notes.txt", "").unwrap();
        assert_eq!(fs::read(restored).unwrap(), b"gone\n");
    }

    #[test]
    fn open_copy_materializes_a_private_read_only_file() {
        let (_root, history) = history_with_file(b"one\n");
        edit(&history, b"two\n");
        let old = ids(&history)[1];
        let cache = tempdir().unwrap();
        let base = cache.path().join("opened");
        let opened = open_copy(&history, b"notes.txt", old, "notes (old).txt", &base).unwrap();
        assert_eq!(fs::read(&opened).unwrap(), b"one\n");
        assert_eq!(opened.file_name().unwrap(), "notes (old).txt");
        assert_eq!(fs::read(history.root.join("notes.txt")).unwrap(), b"two\n");
        assert!(opened.starts_with(&base));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&opened).unwrap().permissions().mode() & 0o777,
                0o444
            );
            assert_eq!(
                fs::metadata(opened.parent().unwrap())
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o700
            );
        }
        // A second open gets its own folder instead of replacing the first.
        let again = open_copy(&history, b"notes.txt", old, "notes (old).txt", &base).unwrap();
        assert_ne!(opened, again);
        assert_eq!(fs::read(&again).unwrap(), b"one\n");
    }

    #[test]
    fn open_copy_refuses_a_tombstone_or_another_files_version() {
        let (_root, history) = history_with_file(b"gone\n");
        fs::write(history.root.join("other.txt"), b"other\n").unwrap();
        capture::record(&history, Path::new("other.txt")).unwrap();
        let other = list_infos(&history, b"other.txt", None, None).unwrap()[0].id;
        let base = tempdir().unwrap();
        assert!(open_copy(&history, b"notes.txt", other, "wrong.txt", base.path()).is_err());

        fs::remove_file(history.root.join("notes.txt")).unwrap();
        capture::record(&history, Path::new("notes.txt")).unwrap();
        let tombstone = list_infos(&history, b"notes.txt", None, None).unwrap()[0].id;
        assert!(open_copy(&history, b"notes.txt", tombstone, "gone.txt", base.path()).is_err());
    }

    #[test]
    fn a_read_only_history_can_open_a_copy() {
        let (_root, history) = history_with_file(b"one\n");
        let version = ids(&history)[0];
        let second = History::open(&history.root).unwrap();
        assert!(!second.writable);
        let base = tempdir().unwrap();
        let opened = open_copy(&second, b"notes.txt", version, "notes.txt", base.path()).unwrap();
        assert_eq!(fs::read(opened).unwrap(), b"one\n");
        // The owner still works and the lock was not disturbed.
        assert!(history.writer().is_ok());
    }

    #[test]
    fn open_deleted_materializes_the_last_live_version() {
        let (_root, history) = history_with_file(b"gone\n");
        fs::remove_file(history.root.join("notes.txt")).unwrap();
        capture::record(&history, Path::new("notes.txt")).unwrap();
        let base = tempdir().unwrap();
        let opened =
            open_deleted(&history, b"notes.txt", "notes (deleted).txt", base.path()).unwrap();
        assert_eq!(fs::read(opened).unwrap(), b"gone\n");
        assert!(
            !history.root.join("notes.txt").exists(),
            "opening is not restoring"
        );
        assert!(open_deleted(&history, b"never.txt", "x.txt", base.path()).is_err());
    }

    #[test]
    fn old_opened_copies_are_pruned_by_their_creation_stamp() {
        let dir = tempdir().unwrap();
        let base = dir.path().join("opened");
        fs::create_dir_all(base.join("1000-1-0")).unwrap();
        fs::write(base.join("1000-1-0/file.txt"), b"x").unwrap();
        fs::create_dir_all(base.join("1800-1-0")).unwrap();
        fs::create_dir_all(base.join("keep")).unwrap();
        prune_opened(&base, 2000, 500);
        assert!(!base.join("1000-1-0").exists());
        assert!(base.join("1800-1-0").exists());
        assert!(base.join("keep").exists());
        // A missing cache directory is not an error.
        prune_opened(&dir.path().join("missing"), 2000, 500);
    }

    #[cfg(unix)]
    #[test]
    fn replace_refuses_when_the_current_file_cannot_be_recorded() {
        use std::os::unix::fs::PermissionsExt;
        let (_dir, history) = history_with_file(b"one\n");
        edit(&history, b"two\n");
        let old = ids(&history)[1];
        let path = history.root.join("notes.txt");
        fs::write(&path, b"three, not yet recorded\n").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).unwrap();
        if File::open(&path).is_ok() {
            return; // running as root: permissions can't make it unreadable
        }
        let mut trashed = false;
        let result = replace_with(&history, b"notes.txt", old, |_| {
            trashed = true;
            Ok(())
        });
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(result.is_err());
        assert!(!trashed, "an unrecorded file must not be replaced");
        assert_eq!(fs::read(&path).unwrap(), b"three, not yet recorded\n");
    }

    #[cfg(unix)]
    #[test]
    fn replace_keeps_the_current_files_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let (_dir, history) = history_with_file(b"#!/bin/sh\necho one\n");
        let path = history.root.join("notes.txt");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o750)).unwrap();
        edit(&history, b"#!/bin/sh\necho two\n");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o750)).unwrap();
        let old = ids(&history)[1];
        let trashed = history.root.join("trashed-current");
        replace_with(&history, b"notes.txt", old, |p| {
            fs::rename(p, &trashed).map_err(|e| Error::Io(e.to_string()))
        })
        .unwrap();
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o750);
    }

    #[test]
    fn a_failed_placement_after_the_trash_keeps_the_staged_copy() {
        let (_dir, history) = history_with_file(b"one\n");
        edit(&history, b"two\n");
        let old = ids(&history)[1];
        let trashed = history.root.join("trashed-current");
        let original = history.root.join("notes.txt");
        // Something appears at the path between the Trash move and placement.
        let err = replace_with(&history, b"notes.txt", old, |p| {
            fs::rename(p, &trashed).map_err(|e| Error::Io(e.to_string()))?;
            fs::create_dir(&original).map_err(|e| Error::Io(e.to_string()))
        })
        .unwrap_err();
        assert!(err.to_string().contains("restored copy is at"), "{err}");
        assert_eq!(fs::read(&trashed).unwrap(), b"two\n");
        let staged: Vec<_> = fs::read_dir(&history.tmp).unwrap().flatten().collect();
        assert_eq!(staged.len(), 1);
        assert_eq!(fs::read(staged[0].path()).unwrap(), b"one\n");
    }

    #[test]
    fn restoring_a_deleted_file_never_overwrites() {
        let (_dir, history) = history_with_file(b"gone\n");
        let path = history.root.join("notes.txt");
        fs::remove_file(&path).unwrap();
        capture::record(&history, Path::new("notes.txt")).unwrap();

        // Something new took the name: without a fallback name, refuse.
        fs::write(&path, b"newcomer").unwrap();
        assert!(restore_deleted(&history, b"notes.txt", "").is_err());
        assert_eq!(fs::read(&path).unwrap(), b"newcomer");

        // With one, restore next to it.
        let restored = restore_deleted(&history, b"notes.txt", "notes (old).txt").unwrap();
        assert_eq!(restored, history.root.join("notes (old).txt"));
        assert_eq!(fs::read(&restored).unwrap(), b"gone\n");
        assert_eq!(fs::read(&path).unwrap(), b"newcomer");
        assert!(
            fs::read_dir(&history.tmp).unwrap().next().is_none(),
            "staging cleaned up"
        );
    }

    #[test]
    fn restoring_a_deleted_file_recreates_its_folder() {
        let dir = tempdir().unwrap();
        let history = History::create(dir.path()).unwrap();
        let sub = history.root.join("a/b");
        fs::create_dir_all(&sub).unwrap();
        fs::write(sub.join("deep.txt"), b"deep").unwrap();
        capture::record(&history, Path::new("a/b/deep.txt")).unwrap();
        fs::remove_dir_all(history.root.join("a")).unwrap();
        capture::tombstone_prefix(&history, Path::new("a")).unwrap();
        let restored = restore_deleted(&history, b"a/b/deep.txt", "").unwrap();
        assert_eq!(fs::read(restored).unwrap(), b"deep");
    }

    #[test]
    fn invalid_names_are_refused() {
        for name in ["", ".", "..", ".versions", "a/b", "a\0b"] {
            assert!(validate_name(name).is_err(), "{name:?}");
        }
        assert!(validate_name("notes (2026-10-24 14-05).txt").is_ok());
    }
}
