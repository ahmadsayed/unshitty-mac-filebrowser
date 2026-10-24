//! One open version history: ownership, the per-folder lock, and its two
//! SQLite connections (D16). Plain Rust; `lib.rs` supplies the root paths.

use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use rusqlite::Connection;

use crate::versions::{
    APP_ID, Error, HistoryInfo, INDEX_FILE, LOCK_FILE, OBJECTS_DIR, Result, TMP_DIR, VERSIONS_DIR,
    index, meta_path, now_seconds, read_marker, write_marker,
};

/// One open history. The `lock` field must stay alive for as long as the
/// writer exists; `try_lock` is released when the file is closed.
pub struct History {
    /// Canonical root of the versioned folder (R9).
    pub root: PathBuf,
    /// `<root>/.versions`.
    pub dir: PathBuf,
    pub folder_id: String,
    /// False when another process holds the lock (D16).
    pub writable: bool,
    pub objects: PathBuf,
    pub tmp: PathBuf,
    pub index_path: PathBuf,
    /// Serializes captures, scans, restores that write, and transfers.
    pub capture_lock: Mutex<()>,
    #[allow(dead_code)]
    lock: File,
    writer: Option<Mutex<Connection>>,
    reader: Mutex<Connection>,
}

impl std::fmt::Debug for History {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("History")
            .field("root", &self.root)
            .field("writable", &self.writable)
            .finish_non_exhaustive()
    }
}

impl History {
    /// Create a brand-new owned history under `root` (D13, D14, D18).
    /// On failure the folder is left; a `.versions/` without our marker is
    /// inert and is never adopted.
    pub fn create(root: &Path) -> Result<History> {
        let root = canonical_root(root)?;
        let dir = root.join(VERSIONS_DIR);
        if dir.exists() {
            return Err(Error::NotOurs(
                "a .versions directory already exists".into(),
            ));
        }
        fs::create_dir(&dir)?;
        set_mode(&dir, 0o700)?;
        fs::create_dir(dir.join(OBJECTS_DIR))?;
        fs::create_dir(dir.join(TMP_DIR))?;

        let lock_path = dir.join(LOCK_FILE);
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&lock_path)?;
        set_mode(&lock_path, 0o600)?;
        lock.try_lock().map_err(|_| Error::ReadOnly)?;

        let index_path = dir.join(INDEX_FILE);
        let writer = index::create_index(&index_path)?;
        let folder_id = new_folder_id(&root);
        // The DB marker and meta.json are the last bits of ownership (R6).
        index::write_meta(&writer, &folder_id)?;
        if let Err(e) = write_marker(&root, &folder_id) {
            // Leave an inert directory: no marker means never adopted.
            let _ = writer.execute("DELETE FROM meta", []);
            return Err(e);
        }
        let reader = index::open_reader(&index_path)?;
        let objects = dir.join(OBJECTS_DIR);
        let tmp = dir.join(TMP_DIR);
        let history = History {
            root,
            dir,
            folder_id,
            writable: true,
            objects,
            tmp,
            index_path,
            capture_lock: Mutex::new(()),
            lock,
            writer: Some(Mutex::new(writer)),
            reader: Mutex::new(reader),
        };
        history.clear_tmp();
        Ok(history)
    }

    /// Open an existing history. A foreign or damaged `.versions/` is an
    /// error; a history locked by another instance opens read-only.
    pub fn open(root: &Path) -> Result<History> {
        let root = canonical_root(root)?;
        let dir = root.join(VERSIONS_DIR);
        if !dir.is_dir() {
            return Err(Error::NotOurs("no .versions directory".into()));
        }
        let marker = read_marker(&root)?;
        let index_path = dir.join(INDEX_FILE);

        let lock_path = dir.join(LOCK_FILE);
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)?;
        #[cfg(unix)]
        set_mode(&lock_path, 0o600)?;
        let writable = match lock.try_lock() {
            Ok(()) => true,
            Err(std::fs::TryLockError::WouldBlock) => false,
            Err(std::fs::TryLockError::Error(e)) => return Err(Error::Io(e.to_string())),
        };
        // The owner opens read-write first so SQLite can recover a WAL left by
        // a crash; a second instance opens read-only.
        let writer = if writable {
            Some(Mutex::new(index::open_writer(&index_path)?))
        } else {
            None
        };
        let reader = index::open_reader(&index_path)?;
        index::verify_owner(&reader)?;
        let db_meta = index::read_meta(&reader)?;
        if db_meta.get("folder_id") != Some(&marker.folder_id) {
            return Err(Error::NotOurs(
                "meta.json and the index disagree about the folder id".into(),
            ));
        }

        let objects = dir.join(OBJECTS_DIR);
        let tmp = dir.join(TMP_DIR);
        let history = History {
            root,
            dir,
            folder_id: marker.folder_id,
            writable,
            objects,
            tmp,
            index_path,
            capture_lock: Mutex::new(()),
            lock,
            writer,
            reader: Mutex::new(reader),
        };
        if writable {
            history.clear_tmp();
        }
        Ok(history)
    }

    /// Give a copied history a fresh identity (transfer). The caller must own
    /// the copy; the source keeps its own id.
    pub fn reassign_folder_id(&mut self) -> Result<()> {
        let writer = self.writer()?;
        let new_id = new_folder_id(&self.root);
        index::write_meta(&writer, &new_id)?;
        drop(writer);
        write_marker(&self.root, &new_id)?;
        self.folder_id = new_id;
        Ok(())
    }

    pub fn writer(&self) -> Result<MutexGuard<'_, Connection>> {
        self.writer
            .as_ref()
            .ok_or(Error::ReadOnly)?
            .lock()
            .map_err(|_| Error::Db("writer lock poisoned".into()))
    }

    pub fn reader(&self) -> Result<MutexGuard<'_, Connection>> {
        self.reader
            .lock()
            .map_err(|_| Error::Db("reader lock poisoned".into()))
    }

    pub fn checkpoint(&self) -> Result<()> {
        let writer = self.writer()?;
        index::checkpoint(&writer)
    }

    pub fn info(&self) -> Result<HistoryInfo> {
        let reader = self.reader()?;
        let mut info = index::history_info(&reader)?;
        info.bytes = info
            .bytes
            .saturating_add(file_len(&self.index_path))
            .saturating_add(file_len(&self.index_path.with_extension("sqlite-wal")))
            .saturating_add(file_len(&meta_path(&self.root)));
        Ok(info)
    }

    /// Remove staging files; only the owning process may call this (D16).
    pub fn clear_tmp(&self) {
        let Ok(entries) = fs::read_dir(&self.tmp) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let _ = if path.is_dir() {
                fs::remove_dir_all(&path)
            } else {
                fs::remove_file(&path)
            };
        }
    }
}

/// The nearest ancestor-or-self with a valid, owned `.versions/meta.json`.
/// Never opens SQLite, so it is cheap enough for every navigation (R1).
pub fn find_root(path: &Path) -> Option<PathBuf> {
    let start = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let mut current = Some(start.as_path());
    while let Some(dir) = current {
        if meta_path(dir).is_file() {
            if let Ok(marker) = read_marker(dir) {
                if marker.app == APP_ID {
                    return Some(dir.to_path_buf());
                }
            }
        }
        current = dir.parent();
    }
    None
}

/// True when `path` lies in a nested owned history below `root` (D2).
pub fn nested_owned_root(root: &Path, path: &Path) -> Option<PathBuf> {
    if !path.starts_with(root) {
        return None;
    }
    let mut current = path.parent();
    while let Some(dir) = current {
        if dir == root || !dir.starts_with(root) {
            break;
        }
        if meta_path(dir).is_file() {
            if let Ok(marker) = read_marker(dir) {
                if marker.app == APP_ID {
                    return Some(dir.to_path_buf());
                }
            }
        }
        current = dir.parent();
    }
    None
}

pub fn canonical_root(root: &Path) -> Result<PathBuf> {
    fs::canonicalize(root).map_err(|e| Error::Io(e.to_string()))
}

fn file_len(path: &Path) -> u64 {
    fs::metadata(path).map(|m| m.len()).unwrap_or(0)
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(mode))?;
    Ok(())
}

#[cfg(not(unix))]
fn set_mode(_path: &Path, _mode: u32) -> Result<()> {
    Ok(())
}

/// A random 128-bit folder id in hex. Both targets are Unix; `/dev/urandom`
/// avoids a dependency. A hash fallback keeps creation working if it is
/// unavailable.
fn new_folder_id(root: &Path) -> String {
    let mut bytes = [0u8; 16];
    if let Ok(mut file) = File::open("/dev/urandom") {
        use std::io::Read;
        if file.read_exact(&mut bytes).is_ok() {
            return bytes.iter().map(|b| format!("{b:02x}")).collect();
        }
    }
    let seed = format!(
        "{}\0{}\0{}",
        now_seconds(),
        std::process::id(),
        root.display()
    );
    blake3::hash(seed.as_bytes()).to_hex().to_string()[..32].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn new_history() -> (tempfile::TempDir, History) {
        let dir = tempdir().unwrap();
        let history = History::create(dir.path()).unwrap();
        (dir, history)
    }

    #[test]
    fn creates_and_reopens_an_owned_history() {
        let (_dir, history) = new_history();
        assert!(history.writable);
        assert!(meta_path(&history.root).is_file());
        assert!(history.index_path.is_file());
        assert!(history.dir.join(OBJECTS_DIR).is_dir());
        let id = history.folder_id.clone();
        drop(history);

        let reopened = History::open(&history_root(&_dir)).unwrap();
        assert_eq!(reopened.folder_id, id);
        assert!(reopened.writable);
    }

    // The root lives in the temp dir; `_dir` is a TempDir.
    fn history_root(dir: &tempfile::TempDir) -> PathBuf {
        dir.path().to_path_buf()
    }

    #[test]
    fn refuses_to_create_over_a_versions_directory() {
        let (dir, _history) = new_history();
        let err = History::create(dir.path()).unwrap_err();
        assert!(matches!(err, Error::NotOurs(_)));
    }

    #[test]
    fn a_second_open_is_read_only() {
        let (_dir, history) = new_history();
        let second = History::open(&history.root).unwrap();
        assert!(!second.writable);
        assert!(second.writer().is_err());
        assert!(second.reader().is_ok());
        // The first can still write.
        assert!(history.writer().is_ok());
    }

    #[test]
    fn a_foreign_marker_is_refused() {
        let (dir, history) = new_history();
        let root = history.root.clone();
        drop(history);
        // Replace meta.json with another app's marker.
        fs::write(
            meta_path(&root),
            r#"{"app":"other","folder_id":"0123456789abcdef0123456789abcdef","schema_version":1}"#,
        )
        .unwrap();
        let err = History::open(&root).unwrap_err();
        assert!(matches!(err, Error::NotOurs(_)));
        let _ = dir;
    }

    #[test]
    fn meta_json_and_index_must_agree() {
        let (dir, history) = new_history();
        let root = history.root.clone();
        drop(history);
        // Change only meta.json's folder id.
        let marker = format!(
            r#"{{"app":"{APP_ID}","folder_id":"{}","schema_version":1}}"#,
            "a".repeat(32)
        );
        fs::write(meta_path(&root), marker).unwrap();
        let err = History::open(&root).unwrap_err();
        assert!(matches!(err, Error::NotOurs(_)));
        let _ = dir;
    }

    #[test]
    fn find_root_and_nested_owned() {
        let (dir, history) = new_history();
        let root = history.root.clone();
        let nested = root.join("sub/deeper");
        fs::create_dir_all(&nested).unwrap();
        let nested_history = History::create(&nested).unwrap();
        assert_eq!(
            find_root(&nested).as_deref(),
            Some(nested_history.root.as_path())
        );
        assert_eq!(find_root(&root).as_deref(), Some(root.as_path()));
        assert_eq!(
            nested_owned_root(&root, &nested.join("file.txt")).as_deref(),
            Some(nested_history.root.as_path())
        );
        let _ = dir;
    }

    #[test]
    fn reassigning_a_copy_changes_both_markers() {
        let (_dir, mut history) = new_history();
        let old = history.folder_id.clone();
        history.reassign_folder_id().unwrap();
        assert_ne!(history.folder_id, old);
        let marker = read_marker(&history.root).unwrap();
        assert_eq!(marker.folder_id, history.folder_id);
        let db = index::read_meta(&history.reader().unwrap()).unwrap();
        assert_eq!(db.get("folder_id"), Some(&history.folder_id));
    }

    #[test]
    fn tmp_is_cleared_only_by_the_owner() {
        let (_dir, history) = new_history();
        let staging = history.tmp.join("leftover");
        fs::write(&staging, b"x").unwrap();
        history.clear_tmp();
        assert!(!staging.exists());

        let second = History::open(&history.root).unwrap();
        assert!(!second.writable);
        let staging = history.tmp.join("still-here");
        fs::write(&staging, b"x").unwrap();
        // A read-only open never clears tmp.
        assert!(staging.exists());
    }
}
