//! Capture: turn a file's current bytes into a version, tombstone deletions,
//! and scan a whole versioned folder at launch.
//!
//! The order is deliberate: the `objects` row decides whether a content hash
//! is stored as a full frame or a delta. A hash can only have one encoding, so
//! capture asks the index before installing anything (`write_*_unless`).

use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

use crate::versions::{
    Error, MAX_DELTA_DEPTH, MAX_DELTA_SIZE, MAX_FILE_SIZE, ObjectKind, ObjectMeta, Result,
    VersionRow,
    history::{History, nested_owned_root},
    index, mtime_nanos, now_seconds, path_bytes,
    store::Store,
    validate_rel_path,
};

/// Record the current state of `rel` (relative to the history root).
pub fn record(history: &History, rel: &Path) -> Result<()> {
    let _guard = history
        .capture_lock
        .lock()
        .map_err(|_| Error::Io("capture lock poisoned".into()))?;
    record_locked(history, rel)
}

/// Scan the whole root: capture changed/new files, then tombstone indexed
/// paths that are gone or no longer regular files.
pub fn scan(history: &History) -> Result<()> {
    let _guard = history
        .capture_lock
        .lock()
        .map_err(|_| Error::Io("capture lock poisoned".into()))?;
    scan_locked(history, &history.root)?;
    tombstone_missing(history, None)?;
    history.checkpoint()?;
    Ok(())
}

/// Scan one folder inside the root (a watcher folder event): capture what
/// changed below it and tombstone what disappeared from it. Cheaper than a
/// whole-root scan for a folder created, renamed or deleted deep in a tree.
pub fn scan_dir(history: &History, rel_dir: &Path) -> Result<()> {
    let rel = path_bytes(rel_dir);
    if rel.is_empty() {
        return scan(history);
    }
    validate_rel_path(&rel)?;
    let _guard = history
        .capture_lock
        .lock()
        .map_err(|_| Error::Io("capture lock poisoned".into()))?;
    let dir = history.root.join(rel_dir);
    if dir.is_dir() && same_device(&history.root, &dir) {
        scan_locked(history, &dir)?;
    }
    tombstone_missing(history, Some(&rel))
}

fn scan_locked(history: &History, dir: &Path) -> Result<()> {
    let mut files = Vec::new();
    walk(history, dir, &mut files);
    for rel in files {
        // Stop at once when the history is closed for a copy, move or trash;
        // the scan after it reopens picks up the rest.
        if history.is_closed() {
            return Err(Error::Closed);
        }
        // A file that changes again while we read it is picked up by the next
        // event or scan; one failure must not stop the whole walk.
        let _ = record_locked(history, &rel);
    }
    Ok(())
}

/// Tombstone every path under `rel_dir` that is gone or no longer a regular
/// file (directory remove/rename, D4/D17).
pub fn tombstone_prefix(history: &History, rel_dir: &Path) -> Result<()> {
    let _guard = history
        .capture_lock
        .lock()
        .map_err(|_| Error::Io("capture lock poisoned".into()))?;
    let rel = path_bytes(rel_dir);
    if rel.is_empty() {
        tombstone_missing(history, None)
    } else {
        validate_rel_path(&rel)?;
        tombstone_missing(history, Some(&rel))
    }
}

fn tombstone_missing(history: &History, dir: Option<&[u8]>) -> Result<()> {
    let rows = {
        let reader = history.reader()?;
        index::latest_versions_under(&reader, dir)?
    };
    for row in rows {
        if row.is_tombstone() {
            continue;
        }
        if is_regular_file(&history.root, &row.path) {
            continue;
        }
        insert_tombstone(history, &row.path, now_seconds())?;
    }
    Ok(())
}

fn insert_tombstone(history: &History, rel: &[u8], now: i64) -> Result<()> {
    validate_rel_path(rel)?;
    let writer = history.writer()?;
    index::insert_tombstone(&writer, rel, now)?;
    Ok(())
}

fn is_regular_file(root: &Path, rel: &[u8]) -> bool {
    let path = crate::versions::join_rel(root, rel);
    fs::symlink_metadata(path).is_ok_and(|meta| meta.is_file())
}

pub(crate) fn record_locked(history: &History, rel: &Path) -> Result<()> {
    let rel_bytes = path_bytes(rel);
    if validate_rel_path(&rel_bytes).is_err() {
        return Ok(()); // internal or malformed event; nothing to record
    }
    if is_ignored(rel) {
        return Ok(());
    }
    let abs = history.root.join(rel);
    if !abs.starts_with(&history.root) {
        return Ok(());
    }
    if nested_owned_root(&history.root, &abs).is_some() {
        return Ok(());
    }

    let latest = {
        let reader = history.reader()?;
        index::latest_version(&reader, &rel_bytes)?
    };
    let Some(meta) = fs::symlink_metadata(&abs).ok() else {
        return tombstone_if_live(history, &rel_bytes, &latest);
    };
    if !same_device(&history.root, &abs) {
        return Ok(());
    }
    if !meta.is_file() {
        // D17: a path that becomes a folder/symlink ends the file's history.
        return tombstone_if_live(history, &rel_bytes, &latest);
    }
    if meta.len() > MAX_FILE_SIZE {
        return Ok(());
    }

    // Open once and use the handle's metadata (F4); a path swapped after the
    // stat cannot make us hash a different file than the one we cap-checked.
    let mut file = File::open(&abs)?;
    let handle_meta = file.metadata()?;
    if !handle_meta.is_file() {
        return tombstone_if_live(history, &rel_bytes, &latest);
    }
    let size = handle_meta.len();
    if size > MAX_FILE_SIZE {
        return Ok(());
    }
    let mtime = mtime_nanos(&handle_meta);
    if let Some(latest) = &latest {
        if latest.hash.is_some() && latest.size == size && latest.mtime == mtime {
            return Ok(());
        }
    }

    let store = Store::new(history.objects.clone(), history.tmp.clone());
    let now = now_seconds();

    if size > MAX_DELTA_SIZE {
        return capture_full_stream(
            history, &store, &mut file, &rel_bytes, size, mtime, latest, now,
        );
    }

    let mut bytes = Vec::with_capacity(size as usize);
    file.read_to_end(&mut bytes)?;
    if bytes.len() as u64 != size {
        return Err(Error::Changed);
    }
    let after = file.metadata()?;
    if after.len() != size || mtime_nanos(&after) != mtime {
        return Err(Error::Changed);
    }
    let hash = blake3::hash(&bytes).to_hex().to_string();
    if let Some(latest) = &latest {
        if latest.hash.as_deref() == Some(hash.as_str()) {
            // Content unchanged since the last capture; only the timestamp moved.
            let writer = history.writer()?;
            index::update_mtime(&writer, latest.id, mtime)?;
            return Ok(());
        }
    }

    // A usable base must exist, be within the memory budget, and not already
    // be at the depth cap.
    let base = latest.as_ref().and_then(|v| v.hash.clone());
    let base_object = match base.as_deref() {
        Some(hash) => {
            let reader = history.reader()?;
            index::get_object(&reader, hash)?
        }
        None => None,
    };
    if let (Some(base_hash), Some(base_object)) = (base.as_deref(), base_object.as_ref()) {
        if base_object.depth < MAX_DELTA_DEPTH
            && base_object.size <= MAX_DELTA_SIZE
            && size <= MAX_DELTA_SIZE
        {
            let chain = {
                let reader = history.reader()?;
                chain_for(&reader, base_hash)?
            };
            let base_bytes = store.read(&chain, base_hash)?;
            let delta = store.write_delta_unless(&bytes, &base_bytes, |hash| {
                Ok(object_is_new(history, hash)?)
            })?;
            let Some(delta) = delta else {
                return insert_version_only(history, &rel_bytes, &hash, size, mtime, now);
            };
            if crate::versions::delta_is_worthwhile(delta.stored_size, size) {
                let object = ObjectMeta {
                    hash: delta.hash.clone(),
                    kind: ObjectKind::Delta,
                    base: Some(base_hash.to_string()),
                    depth: base_object.depth + 1,
                    size,
                    stored_size: delta.stored_size,
                };
                return insert_capture(history, &rel_bytes, &hash, size, mtime, now, Some(object));
            }
            // Not worth a delta: fall through to a full copy. The unused delta
            // file is an orphan object and is harmless.
        }
    }

    let mut seen = None;
    let full = store.write_full_unless(std::io::Cursor::new(&bytes), size, |hash| {
        seen = Some(hash.to_string());
        Ok(object_is_new(history, hash)?)
    })?;
    let hash = seen.unwrap_or(hash);
    let Some(full) = full else {
        return insert_version_only(history, &rel_bytes, &hash, size, mtime, now);
    };
    let object = ObjectMeta {
        hash: full.hash.clone(),
        kind: ObjectKind::Full,
        base: None,
        depth: 0,
        size,
        stored_size: full.stored_size,
    };
    insert_capture(history, &rel_bytes, &hash, size, mtime, now, Some(object))
}

#[allow(clippy::too_many_arguments)]
fn capture_full_stream(
    history: &History,
    store: &Store,
    file: &mut File,
    rel: &[u8],
    size: u64,
    mtime: i64,
    latest: Option<VersionRow>,
    now: i64,
) -> Result<()> {
    let check = file.try_clone()?;
    let mut seen = None;
    let full = store.write_full_unless(file, size, |hash| {
        seen = Some(hash.to_string());
        Ok(object_is_new(history, hash)?)
    })?;
    let after = check.metadata()?;
    if after.len() != size || mtime_nanos(&after) != mtime {
        return Err(Error::Changed);
    }
    let hash = seen.ok_or(Error::Changed)?;
    if latest.as_ref().and_then(|v| v.hash.as_deref()) == Some(hash.as_str()) {
        if let Some(latest) = latest {
            let writer = history.writer()?;
            index::update_mtime(&writer, latest.id, mtime)?;
        }
        return Ok(());
    }
    let Some(full) = full else {
        return insert_version_only(history, rel, &hash, size, mtime, now);
    };
    let object = ObjectMeta {
        hash: full.hash.clone(),
        kind: ObjectKind::Full,
        base: None,
        depth: 0,
        size,
        stored_size: full.stored_size,
    };
    insert_capture(history, rel, &hash, size, mtime, now, Some(object))
}

fn object_is_new(history: &History, hash: &str) -> Result<bool> {
    let reader = history.reader()?;
    Ok(index::get_object(&reader, hash)?.is_none())
}

#[allow(clippy::too_many_arguments)]
fn insert_capture(
    history: &History,
    rel: &[u8],
    hash: &str,
    size: u64,
    mtime: i64,
    now: i64,
    object: Option<ObjectMeta>,
) -> Result<()> {
    let writer = history.writer()?;
    let tx = writer.unchecked_transaction()?;
    if let Some(object) = object {
        index::insert_object(&tx, &object)?;
    }
    index::insert_version(&tx, rel, Some(hash), size, mtime, now)?;
    tx.commit()?;
    Ok(())
}

fn insert_version_only(
    history: &History,
    rel: &[u8],
    hash: &str,
    size: u64,
    mtime: i64,
    now: i64,
) -> Result<()> {
    insert_capture(history, rel, hash, size, mtime, now, None)
}

fn tombstone_if_live(history: &History, rel: &[u8], latest: &Option<VersionRow>) -> Result<()> {
    if latest.as_ref().is_some_and(|row| !row.is_tombstone()) {
        insert_tombstone(history, rel, now_seconds())
    } else {
        Ok(())
    }
}

/// The full → delta chain that ends at `hash`, oldest first.
pub fn chain_for(conn: &rusqlite::Connection, hash: &str) -> Result<Vec<ObjectMeta>> {
    let mut chain = Vec::new();
    let mut current = Some(hash.to_string());
    while let Some(hash) = current {
        let object = index::get_object(conn, &hash)?.ok_or(Error::Damaged)?;
        current = object.base.clone();
        chain.push(object);
    }
    chain.reverse();
    Ok(chain)
}

fn is_ignored(rel: &Path) -> bool {
    let Some(name) = rel.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    is_ignored_name(name)
}

fn is_ignored_name(name: &str) -> bool {
    matches!(name, ".DS_Store" | "Thumbs.db" | "desktop.ini" | "4913")
        || name.starts_with("~$")
        || (name.starts_with(".~lock.") && name.ends_with('#'))
        || name.starts_with(".#")
        || name.ends_with(".swp")
        || name.ends_with(".swo")
        || name.ends_with(".swx")
        || name.ends_with('~')
        || name.ends_with(".tmp")
        || name.ends_with(".crdownload")
        || name.ends_with(".part")
        || name.ends_with(".download")
        || (name.starts_with('.') && name.contains(".filebrowser-"))
}

/// `read_marker` accepts only a `meta.json` that names this app.
fn is_owned_root(dir: &Path) -> bool {
    crate::versions::read_marker(dir).is_ok()
}

fn same_device(root: &Path, path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        match (fs::metadata(root), fs::metadata(path)) {
            (Ok(a), Ok(b)) => a.dev() == b.dev(),
            _ => false,
        }
    }
    #[cfg(not(unix))]
    {
        let _ = (root, path);
        true
    }
}

fn walk(history: &History, dir: &Path, out: &mut Vec<PathBuf>) {
    if history.is_closed() {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            if path
                .file_name()
                .is_some_and(|n| n.to_string_lossy() == crate::versions::VERSIONS_DIR)
            {
                continue;
            }
            // Only an owned marker makes a nested history (D2); a foreign
            // `meta.json` must not hide a subtree from this one.
            if path != history.root
                && (is_owned_root(&path) || nested_owned_root(&history.root, &path).is_some())
            {
                continue;
            }
            if !same_device(&history.root, &path) {
                continue;
            }
            walk(history, &path, out);
        } else if file_type.is_file() {
            if let Ok(rel) = path.strip_prefix(&history.root) {
                out.push(rel.to_path_buf());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::versions::{VERSIONS_DIR, history::History, index};
    use std::io::Write;
    use tempfile::tempdir;

    fn new_history() -> (tempfile::TempDir, History) {
        let dir = tempdir().unwrap();
        let history = History::create(dir.path()).unwrap();
        (dir, history)
    }

    fn write(root: &Path, rel: &str, bytes: &[u8]) -> PathBuf {
        let path = root.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        let mut file = File::create(&path).unwrap();
        file.write_all(bytes).unwrap();
        file.sync_all().unwrap();
        path
    }

    fn versions(history: &History, rel: &str) -> Vec<VersionRow> {
        let reader = history.reader().unwrap();
        index::list_versions(&reader, rel.as_bytes(), None, None).unwrap()
    }

    #[test]
    fn captures_a_new_file_and_detects_unchanged_content() {
        let (_dir, history) = new_history();
        write(&history.root, "notes.txt", b"hello");
        record(&history, Path::new("notes.txt")).unwrap();
        assert_eq!(versions(&history, "notes.txt").len(), 1);

        // Same size/mtime: no read, no new row.
        record(&history, Path::new("notes.txt")).unwrap();
        assert_eq!(versions(&history, "notes.txt").len(), 1);

        // Touch with the same content: mtime-only update, no new row.
        let path = history.root.join("notes.txt");
        let file = fs::OpenOptions::new().append(true).open(&path).unwrap();
        file.set_times(
            std::fs::FileTimes::new()
                .set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(2)),
        )
        .unwrap();
        drop(file);
        record(&history, Path::new("notes.txt")).unwrap();
        assert_eq!(versions(&history, "notes.txt").len(), 1);
    }

    #[test]
    fn captures_changes_as_versions_and_stores_a_delta() {
        let (_dir, history) = new_history();
        let base = b"line one\nline two\nline three\n".repeat(500);
        write(&history.root, "notes.txt", &base);
        record(&history, Path::new("notes.txt")).unwrap();

        let mut next = base.clone();
        next.extend_from_slice(b"a new line\n");
        let path = history.root.join("notes.txt");
        fs::write(&path, &next).unwrap();
        record(&history, Path::new("notes.txt")).unwrap();

        let rows = versions(&history, "notes.txt");
        assert_eq!(rows.len(), 2);
        let reader = history.reader().unwrap();
        let chain = chain_for(&reader, rows[0].hash.as_deref().unwrap()).unwrap();
        assert!(chain.len() >= 1);
        let store = Store::new(history.objects.clone(), history.tmp.clone());
        let rebuilt = store
            .read(&chain, rows[0].hash.as_deref().unwrap())
            .unwrap();
        assert_eq!(rebuilt, next);
    }

    #[test]
    fn deletion_is_one_tombstone() {
        let (_dir, history) = new_history();
        write(&history.root, "gone.txt", b"x");
        record(&history, Path::new("gone.txt")).unwrap();
        fs::remove_file(history.root.join("gone.txt")).unwrap();
        record(&history, Path::new("gone.txt")).unwrap();
        record(&history, Path::new("gone.txt")).unwrap();
        let rows = versions(&history, "gone.txt");
        assert_eq!(rows.len(), 2);
        assert!(rows[0].is_tombstone());
        assert!(!rows[1].is_tombstone());
    }

    #[test]
    fn a_folder_replaced_by_a_file_tombstones_the_old_file() {
        let (_dir, history) = new_history();
        write(&history.root, "draft.md", b"x");
        record(&history, Path::new("draft.md")).unwrap();
        fs::remove_file(history.root.join("draft.md")).unwrap();
        fs::create_dir(history.root.join("draft.md")).unwrap();
        record(&history, Path::new("draft.md")).unwrap();
        assert!(versions(&history, "draft.md")[0].is_tombstone());
    }

    #[test]
    fn internal_ignored_and_nested_paths_are_skipped() {
        let (_dir, history) = new_history();
        write(&history.root, ".DS_Store", b"x");
        record(&history, Path::new(".DS_Store")).unwrap();
        assert!(versions(&history, ".DS_Store").is_empty());

        write(&history.root, ".versions/secret", b"x");
        record(&history, Path::new(".versions/secret")).unwrap();
        assert!(versions(&history, ".versions/secret").is_empty());

        let nested = history.root.join("nested");
        fs::create_dir(&nested).unwrap();
        let nested_history = History::create(&nested).unwrap();
        write(&nested, "file.txt", b"x");
        record(&history, Path::new("nested/file.txt")).unwrap();
        assert!(versions(&history, "nested/file.txt").is_empty());
        drop(nested_history);
    }

    #[test]
    fn a_symlink_is_not_captured() {
        let (_dir, history) = new_history();
        write(&history.root, "target.txt", b"x");
        #[cfg(unix)]
        std::os::unix::fs::symlink(
            history.root.join("target.txt"),
            history.root.join("link.txt"),
        )
        .unwrap();
        record(&history, Path::new("link.txt")).unwrap();
        #[cfg(unix)]
        assert!(versions(&history, "link.txt").is_empty());
    }

    #[test]
    fn scan_finds_new_changed_and_deleted_files() {
        let (_dir, history) = new_history();
        write(&history.root, "a.txt", b"a");
        write(&history.root, "b.txt", b"b");
        record(&history, Path::new("a.txt")).unwrap();
        record(&history, Path::new("b.txt")).unwrap();

        write(&history.root, "a.txt", b"a changed");
        write(&history.root, "c.txt", b"c");
        fs::remove_file(history.root.join("b.txt")).unwrap();
        scan(&history).unwrap();

        assert_eq!(versions(&history, "a.txt").len(), 2);
        assert_eq!(versions(&history, "c.txt").len(), 1);
        assert!(versions(&history, "b.txt")[0].is_tombstone());
    }

    #[test]
    fn tombstone_prefix_handles_a_deleted_directory() {
        let (_dir, history) = new_history();
        fs::create_dir(history.root.join("sub")).unwrap();
        write(&history.root, "sub/a.txt", b"a");
        write(&history.root, "sub/b.txt", b"b");
        write(&history.root, "sibling.txt", b"s");
        record(&history, Path::new("sub/a.txt")).unwrap();
        record(&history, Path::new("sub/b.txt")).unwrap();
        record(&history, Path::new("sibling.txt")).unwrap();

        fs::remove_dir_all(history.root.join("sub")).unwrap();
        tombstone_prefix(&history, Path::new("sub")).unwrap();

        assert!(versions(&history, "sub/a.txt")[0].is_tombstone());
        assert!(versions(&history, "sub/b.txt")[0].is_tombstone());
        assert!(!versions(&history, "sibling.txt")[0].is_tombstone());
    }

    #[test]
    fn a_sparse_file_over_the_limit_is_not_read() {
        let (_dir, history) = new_history();
        let path = history.root.join("huge.bin");
        let file = File::create(&path).unwrap();
        file.set_len(MAX_FILE_SIZE + 1).unwrap();
        drop(file);
        record(&history, Path::new("huge.bin")).unwrap();
        assert!(versions(&history, "huge.bin").is_empty());
    }

    #[test]
    fn a_file_over_the_delta_limit_is_streamed_as_a_full_copy() {
        // Sparse, so the test doesn't need 128 MiB of real disk.
        let (_dir, history) = new_history();
        let path = history.root.join("big.bin");
        let file = File::create(&path).unwrap();
        file.set_len(MAX_DELTA_SIZE + 1).unwrap();
        drop(file);
        record(&history, Path::new("big.bin")).unwrap();

        let rows = versions(&history, "big.bin");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].size, MAX_DELTA_SIZE + 1);
        let hash = rows[0].hash.clone().unwrap();
        let reader = history.reader().unwrap();
        let object = index::get_object(&reader, &hash).unwrap().unwrap();
        assert_eq!(object.kind, ObjectKind::Full);
        assert!(object.stored_size < 1 << 20, "zeros compress well");
        drop(reader);

        // The stored object rebuilds to the same bytes (hash-checked).
        let copy = crate::versions::restore::restore_copy(&history, b"big.bin", rows[0].id, "copy.bin")
            .unwrap();
        assert_eq!(fs::metadata(&copy).unwrap().len(), MAX_DELTA_SIZE + 1);
        fs::remove_file(copy).unwrap();

        // Unchanged size and mtime: a second record stores nothing new.
        record(&history, Path::new("big.bin")).unwrap();
        assert_eq!(versions(&history, "big.bin").len(), 1);
    }

    #[test]
    fn a_foreign_marker_does_not_hide_a_subtree_from_the_scan() {
        let (_dir, history) = new_history();
        let other = history.root.join("other");
        fs::create_dir_all(other.join(VERSIONS_DIR)).unwrap();
        fs::write(
            other.join(VERSIONS_DIR).join("meta.json"),
            r#"{"app":"someone-else","folder_id":"00","schema_version":1}"#,
        )
        .unwrap();
        write(&history.root, "other/notes.txt", b"mine");
        scan(&history).unwrap();
        assert_eq!(versions(&history, "other/notes.txt").len(), 1);
    }

    #[test]
    fn a_closed_history_records_nothing_and_its_scan_stops() {
        let (_dir, history) = new_history();
        write(&history.root, "a.txt", b"a");
        history.quiesce();
        assert_eq!(record(&history, Path::new("a.txt")), Err(Error::Closed));
        assert_eq!(scan(&history), Err(Error::Closed));
        assert!(versions(&history, "a.txt").is_empty());
    }

    #[test]
    fn scan_dir_records_a_new_folder_and_tombstones_a_vanished_one() {
        let (_dir, history) = new_history();
        write(&history.root, "old/a.txt", b"a");
        write(&history.root, "old/b.txt", b"b");
        scan(&history).unwrap();
        fs::rename(history.root.join("old"), history.root.join("new")).unwrap();
        // What the watcher does for a renamed folder: tombstone the old path,
        // scan the new one.
        tombstone_prefix(&history, Path::new("old")).unwrap();
        scan_dir(&history, Path::new("new")).unwrap();
        for rel in ["old/a.txt", "old/b.txt"] {
            assert!(versions(&history, rel)[0].is_tombstone(), "{rel}");
        }
        for rel in ["new/a.txt", "new/b.txt"] {
            assert!(!versions(&history, rel)[0].is_tombstone(), "{rel}");
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn names_keep_their_exact_bytes() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;
        let (_dir, history) = new_history();
        // "é" precomposed (NFC) and decomposed (NFD) look the same but are
        // different names on Linux; each keeps its own history (D15).
        let nfc = "caf\u{e9}.txt";
        let nfd = "cafe\u{301}.txt";
        write(&history.root, nfc, b"nfc");
        write(&history.root, nfd, b"nfd!");
        record(&history, Path::new(nfc)).unwrap();
        record(&history, Path::new(nfd)).unwrap();
        assert_eq!(versions(&history, nfc)[0].size, 3);
        assert_eq!(versions(&history, nfd)[0].size, 4);

        // A name that isn't UTF-8 is stored as its bytes, not a lossy string.
        let raw = OsStr::from_bytes(b"bad\xffname.txt");
        fs::write(history.root.join(raw), b"x").unwrap();
        record(&history, Path::new(raw)).unwrap();
        let reader = history.reader().unwrap();
        let row = index::latest_version(&reader, b"bad\xffname.txt").unwrap().unwrap();
        assert_eq!(row.path, b"bad\xffname.txt");
        assert!(index::latest_version(&reader, "bad\u{fffd}name.txt".as_bytes()).unwrap().is_none());
    }

    #[test]
    fn identical_content_is_one_object() {
        let (_dir, history) = new_history();
        write(&history.root, "a.txt", b"same content");
        write(&history.root, "b.txt", b"same content");
        record(&history, Path::new("a.txt")).unwrap();
        record(&history, Path::new("b.txt")).unwrap();
        let reader = history.reader().unwrap();
        let a = index::latest_version(&reader, b"a.txt").unwrap().unwrap();
        let b = index::latest_version(&reader, b"b.txt").unwrap().unwrap();
        assert_eq!(a.hash, b.hash);
        let objects = walk_objects(&history.objects);
        assert_eq!(objects.len(), 1, "{objects:?}");
    }

    fn walk_objects(dir: &Path) -> Vec<PathBuf> {
        let mut out = Vec::new();
        let Ok(entries) = fs::read_dir(dir) else {
            return out;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                out.extend(walk_objects(&path));
            } else {
                out.push(path);
            }
        }
        out
    }

    #[test]
    fn the_versions_directory_is_never_captured() {
        let (_dir, history) = new_history();
        let internal = history.root.join(VERSIONS_DIR).join("index.sqlite");
        assert!(fs::metadata(&internal).is_ok());
        record(&history, Path::new(".versions/index.sqlite")).unwrap();
        let reader = history.reader().unwrap();
        assert!(
            index::latest_version(&reader, b".versions/index.sqlite")
                .unwrap()
                .is_none()
        );
    }
}
