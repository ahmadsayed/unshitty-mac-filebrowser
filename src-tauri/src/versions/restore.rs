//! Version history for users: list, diff, Restore Copy, Replace, and deleted
//! files. Rebuilds happen through `store`; placement never overwrites.

use std::fs::{self, File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use similar::{ChangeTag, TextDiff};

use crate::versions::{
    DeletedFile, DiffLine, Error, Result, VersionDiff, VersionInfo, VersionRow, capture,
    history::History, index, join_rel, rel_path, store::Store, validate_rel_path,
};

const MAX_DIFF_INPUT: u64 = 1 << 20;
const MAX_DIFF_LINES: usize = 2000;
static NEXT_STAGING: AtomicU64 = AtomicU64::new(0);

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
            path: row.display,
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

/// The last non-tombstone version size for a tombstone path (for tests/UI).
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
        assert_eq!(deleted[0].path, "notes.txt");
        assert_eq!(deleted[0].size, 5);
        assert_eq!(last_size(&history, b"notes.txt").unwrap(), 5);

        let restored = restore_deleted(&history, b"notes.txt", "").unwrap();
        assert_eq!(fs::read(restored).unwrap(), b"gone\n");
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
        assert!(fs::read_dir(&history.tmp).unwrap().next().is_none(), "staging cleaned up");
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
