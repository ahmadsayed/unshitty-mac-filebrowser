//! Managed state for open histories: one `History` and one watcher per root.
//! Kept Tauri-free so `lib.rs` can own it and tests can use it directly.
//!
//! Copy, move, rename, trash, Clear History and Stop Keeping Versions are
//! orchestrated here as well, with the filesystem step passed in as a
//! closure, so every path through them is unit-tested.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::versions::{
    Error, HistoryInfo, Result, capture, index,
    history::{self, History},
    read_marker, registry,
    watch::WatcherHandle,
};

/// Whether a transfer leaves the source in place.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transfer {
    Copy,
    Move,
}

pub struct VersionsState {
    roots: Mutex<HashMap<PathBuf, Arc<History>>>,
    watchers: Mutex<HashMap<PathBuf, WatcherHandle>>,
    /// Held for a whole `open`, so two threads opening one root can't both
    /// run `History::open`; the second would hit this process's own lock and
    /// come back read-only.
    open_lock: Mutex<()>,
    registry_path: PathBuf,
    registry_lock: Mutex<()>,
}

impl VersionsState {
    pub fn new(registry_path: PathBuf) -> Self {
        Self {
            roots: Mutex::new(HashMap::new()),
            watchers: Mutex::new(HashMap::new()),
            open_lock: Mutex::new(()),
            registry_path,
            registry_lock: Mutex::new(()),
        }
    }

    #[cfg(test)]
    pub fn registry_path(&self) -> &Path {
        &self.registry_path
    }

    /// Open (or return) the history at `root`. A writable history gets a
    /// watcher; a history locked by another instance stays read-only.
    pub fn open(&self, root: &Path) -> Result<Arc<History>> {
        let root = fs_canonical(root)?;
        let _opening = self.open_lock.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(history) = self.roots.lock().unwrap().get(&root) {
            return Ok(history.clone());
        }
        let history = Arc::new(History::open(&root)?);
        self.roots.lock().unwrap().insert(root, history.clone());
        if history.writable {
            self.start_watcher(history.clone());
        }
        Ok(history)
    }

    pub fn is_open(&self, root: &Path) -> bool {
        let root = fs_canonical(root).unwrap_or_else(|_| root.to_path_buf());
        self.roots.lock().unwrap().contains_key(&root)
    }

    /// Open and add to the registry (a root discovered by navigation).
    pub fn open_registered(&self, root: &Path) -> Result<Arc<History>> {
        let history = self.open(root)?;
        {
            let _guard = self.registry_lock.lock().unwrap();
            let _ = registry::register(&self.registry_path, &history.root);
        }
        Ok(history)
    }

    /// Open a copy, give it a new identity, register it, and scan it.
    pub fn register_copy(&self, root: &Path) -> Result<Arc<History>> {
        let root = fs_canonical(root)?;
        let _opening = self.open_lock.lock().unwrap_or_else(|e| e.into_inner());
        let mut history = History::open(&root)?;
        if !history.writable {
            return Err(Error::ReadOnly);
        }
        {
            let reader = history.reader()?;
            if !index::integrity_check(&reader)? {
                return Err(Error::IndexDamaged("the copied index is inconsistent".into()));
            }
        }
        history.reassign_folder_id()?;
        let history = Arc::new(history);
        {
            let _guard = self.registry_lock.lock().unwrap();
            let _ = registry::register(&self.registry_path, &root);
        }
        self.roots
            .lock()
            .unwrap()
            .insert(root.clone(), history.clone());
        self.start_watcher(history.clone());
        let scan = history.clone();
        std::thread::spawn(move || {
            let _ = capture::scan(&scan);
        });
        Ok(history)
    }

    /// The nearest root that owns `path`, from its `meta.json` marker. Never
    /// opens SQLite, so navigation can call it on every folder change.
    pub fn root_quick(&self, path: &Path) -> Option<PathBuf> {
        let path = fs_canonical(path).ok()?;
        history::find_root(&path).or_else(|| self.longest_root(&path))
    }

    /// The nearest root containing `path`, opened (and registered) if needed.
    pub fn root_for(&self, path: &Path) -> Result<Option<PathBuf>> {
        let Some(root) = self.root_quick(path) else {
            return Ok(None);
        };
        if !self.is_open(&root) {
            self.open_registered(&root)?;
        }
        Ok(Some(root))
    }

    pub fn history_for(&self, path: &Path) -> Result<Option<(Arc<History>, PathBuf)>> {
        let Some(root) = self.root_for(path)? else {
            return Ok(None);
        };
        let path = fs_canonical(path)?;
        let rel = path
            .strip_prefix(&root)
            .map(Path::to_path_buf)
            .map_err(|_| Error::NotOurs("path is outside the history root".into()))?;
        let history = self.open(&root)?;
        Ok(Some((history, rel)))
    }

    /// Adopt a freshly created or reopened history, starting its watcher and
    /// adding it to the registry.
    pub fn adopt(&self, history: History) -> Arc<History> {
        let history = Arc::new(history);
        let root = history.root.clone();
        if let Some(watcher) = self.watchers.lock().unwrap().remove(&root) {
            watcher.stop();
        }
        self.roots
            .lock()
            .unwrap()
            .insert(root.clone(), history.clone());
        if history.writable {
            self.start_watcher(history.clone());
        }
        {
            let _guard = self.registry_lock.lock().unwrap();
            let _ = registry::register(&self.registry_path, &root);
        }
        history
    }

    /// Open roots at or below `path`, deepest first.
    pub fn roots_under(&self, path: &Path) -> Vec<PathBuf> {
        let path = fs_canonical(path).unwrap_or_else(|_| path.to_path_buf());
        let mut roots: Vec<PathBuf> = self
            .roots
            .lock()
            .unwrap()
            .keys()
            .filter(|root| root.starts_with(&path))
            .cloned()
            .collect();
        roots.sort_by_key(|root| std::cmp::Reverse(root.components().count()));
        roots
    }

    /// Stop and remove one history: running scans stop, its watcher is joined,
    /// the capture in progress finishes, and the WAL is checkpointed. The lock
    /// stays held by the returned history until `release_lock` or drop, so
    /// Clear History can keep it through the Trash move.
    pub fn take(&self, root: &Path) -> Option<Arc<History>> {
        // After a rename the old path no longer canonicalises; fall back to the
        // key we stored so the watcher is still stopped.
        let root = fs_canonical(root).unwrap_or_else(|_| root.to_path_buf());
        let history = self.roots.lock().unwrap().remove(&root);
        // Close first so a watcher in the middle of a scan stops at its next
        // file instead of finishing the walk before `stop` can join it.
        if let Some(history) = &history {
            history.begin_close();
        }
        if let Some(watcher) = self.watchers.lock().unwrap().remove(&root) {
            watcher.stop();
        }
        if let Some(history) = &history {
            history.quiesce();
        }
        history
    }

    /// Histories a copy, move or trash of `path` must close: the ones open
    /// here plus the registered ones under it. The registry is shared by every
    /// app instance, so this also finds a history another window has open,
    /// without walking the tree. A history in neither list has never been
    /// opened by this app and is safe to treat as plain files. (Walking the
    /// whole tree for markers made every rename of a big folder slow.)
    fn roots_affected(&self, path: &Path) -> Vec<PathBuf> {
        let path = fs_canonical(path).unwrap_or_else(|_| path.to_path_buf());
        let mut roots = self.roots_under(&path);
        let registered = {
            let _guard = self.registry_lock.lock().unwrap();
            registry::read(&self.registry_path)
        };
        for root in registered {
            if root.starts_with(&path)
                && root.join(crate::versions::VERSIONS_DIR).is_dir()
                && !roots.contains(&root)
            {
                roots.push(root);
            }
        }
        roots.sort_by_key(|root| std::cmp::Reverse(root.components().count()));
        roots
    }

    /// Take every root and release their locks so the folders can be copied,
    /// moved or trashed and then reopened writable. Refuses, touching nothing,
    /// if any of them is read-only (another instance has it open, D16).
    fn suspend_all(&self, roots: &[PathBuf]) -> std::result::Result<(), String> {
        // Registered roots that aren't open here are opened first, which is
        // how a lock held by another instance shows up (as read-only).
        for root in roots {
            if !self.is_open(root) {
                let _ = self.open(root);
            }
        }
        {
            let map = self.roots.lock().unwrap();
            if roots
                .iter()
                .any(|root| map.get(root).is_some_and(|history| !history.writable))
            {
                return Err(Error::ReadOnly.to_string());
            }
        }
        for root in roots {
            if let Some(history) = self.take(root) {
                history.release_lock();
            }
        }
        Ok(())
    }

    fn resume_all(&self, roots: &[PathBuf]) {
        for root in roots {
            if root.join(crate::versions::VERSIONS_DIR).is_dir() {
                let _ = self.resume(root);
            }
        }
    }

    /// Copy or move `from` to `to` with `op`, keeping the histories inside
    /// either one consistent: they are closed (checkpointed, unlocked) for the
    /// filesystem step and reopened afterwards. A copied history gets a new
    /// identity; the source of a copy is reopened where it was.
    pub fn transfer(
        &self,
        from: &Path,
        to: &Path,
        mode: Transfer,
        op: impl FnOnce() -> std::result::Result<(), String>,
    ) -> std::result::Result<(), String> {
        let from = fs_canonical(from).unwrap_or_else(|_| from.to_path_buf());
        let to = fs_canonical(to).unwrap_or_else(|_| to.to_path_buf());
        let source_roots = self.roots_affected(&from);
        let target_roots = self.roots_affected(&to);
        self.suspend_all(&source_roots)?;
        if let Err(error) = self.suspend_all(&target_roots) {
            self.resume_all(&source_roots);
            return Err(error);
        }

        let result = op();
        if result.is_err() {
            self.resume_all(&source_roots);
            self.resume_all(&target_roots);
            return result;
        }
        for root in &source_roots {
            let Some(dest) = transfer_dest(&from, &to, root) else {
                continue;
            };
            match mode {
                Transfer::Copy => {
                    let _ = self.resume(root);
                    if dest.join(crate::versions::VERSIONS_DIR).is_dir() {
                        if let Err(e) = self.register_copy(&dest) {
                            eprintln!("versions: could not register copy {}: {e}", dest.display());
                        }
                    }
                }
                Transfer::Move => {
                    if let Err(e) = self.move_registered(root, &dest) {
                        eprintln!("versions: could not reopen {}: {e}", dest.display());
                    }
                }
            }
        }
        for root in &target_roots {
            if root.join(crate::versions::VERSIONS_DIR).is_dir() {
                let _ = self.resume(root);
            } else {
                self.unregister(root);
            }
        }
        result
    }

    /// Trash `path` with `op`. Histories inside it are closed and unregistered
    /// first, and reopened if the Trash move fails.
    pub fn trash<T>(
        &self,
        path: &Path,
        op: impl FnOnce() -> std::result::Result<T, String>,
    ) -> std::result::Result<T, String> {
        let roots = self.roots_affected(path);
        self.suspend_all(&roots)?;
        for root in &roots {
            self.unregister(root);
        }
        match op() {
            Ok(value) => Ok(value),
            Err(error) => {
                for root in &roots {
                    let _ = self.open_registered(root);
                }
                Err(error)
            }
        }
    }

    /// Clear History…: move `.versions/` to the Trash with `trash_dir`, then
    /// start a fresh history recording the current files (D7).
    pub fn clear(
        &self,
        root: &Path,
        trash_dir: impl FnOnce(&Path) -> std::result::Result<(), String>,
    ) -> std::result::Result<(), String> {
        let history = self.close_for_removal(root)?;
        let root = history.root.clone();
        if let Err(error) = trash_dir(&history.dir) {
            history.release_lock();
            let _ = self.resume(&root);
            return Err(error);
        }
        history.release_lock();
        drop(history);
        let fresh = History::create(&root).map_err(|e| e.to_string())?;
        let history = self.adopt(fresh);
        let _ = capture::scan(&history);
        Ok(())
    }

    /// Stop Keeping Versions…: move `.versions/` to the Trash with `trash_dir`
    /// and forget the root (D7).
    pub fn stop_keeping(
        &self,
        root: &Path,
        trash_dir: impl FnOnce(&Path) -> std::result::Result<(), String>,
    ) -> std::result::Result<(), String> {
        let history = self.close_for_removal(root)?;
        let root = history.root.clone();
        if let Err(error) = trash_dir(&history.dir) {
            history.release_lock();
            let _ = self.resume(&root);
            return Err(error);
        }
        history.release_lock();
        drop(history);
        self.forget(&root);
        Ok(())
    }

    /// Close a root for Clear/Stop while keeping its lock, after checking it is
    /// writable and still ours in both the index and `meta.json` (D14). Every
    /// refusal reopens the history, so a failed Clear never stops recording.
    fn close_for_removal(&self, root: &Path) -> std::result::Result<Arc<History>, String> {
        let not_versioned = || "This folder does not keep versions.".to_string();
        self.open(root).map_err(|_| not_versioned())?;
        let history = self.take(root).ok_or_else(not_versioned)?;
        if !history.writable {
            let _ = self.resume(&history.root);
            return Err(Error::ReadOnly.to_string());
        }
        if let Err(error) = verify_ownership(&history) {
            history.release_lock();
            let _ = self.resume(&history.root);
            return Err(error.to_string());
        }
        Ok(history)
    }

    /// Bytes, versions and files of `root` and the open histories nested in
    /// it (D20).
    pub fn info_under(&self, root: &Path) -> Result<HistoryInfo> {
        self.open(root)?;
        let mut total = HistoryInfo::default();
        for nested in self.roots_under(root) {
            if let Ok(history) = self.open(&nested) {
                if let Ok(info) = history.info() {
                    total.bytes = total.bytes.saturating_add(info.bytes);
                    total.versions = total.versions.saturating_add(info.versions);
                    total.files = total.files.saturating_add(info.files);
                }
            }
        }
        Ok(total)
    }

    pub fn resume(&self, root: &Path) -> Result<Arc<History>> {
        let history = self.open(root)?;
        if history.writable {
            let scan = history.clone();
            std::thread::spawn(move || {
                let _ = capture::scan(&scan);
            });
        }
        Ok(history)
    }

    pub fn forget(&self, root: &Path) {
        let key = fs_canonical(root).unwrap_or_else(|_| root.to_path_buf());
        if let Some(history) = self.take(&key) {
            history.release_lock();
        }
        self.unregister(&key);
    }

    pub fn unregister(&self, root: &Path) {
        let _guard = self.registry_lock.lock().unwrap();
        let _ = registry::unregister(&self.registry_path, root);
    }

    /// Reopen a moved root at its new path and update the registry.
    pub fn move_registered(&self, from: &Path, to: &Path) -> Result<Arc<History>> {
        if let Some(history) = self.take(from) {
            history.release_lock();
        }
        {
            let _guard = self.registry_lock.lock().unwrap();
            let _ = registry::rewrite_prefix(&self.registry_path, from, to);
        }
        self.open_registered(to)
    }

    /// Read `versioned.json`, open and watch each existing root, and drop
    /// missing entries. Run from a background thread at startup.
    pub fn startup(&self) {
        let roots = registry::read(&self.registry_path);
        let mut kept = Vec::new();
        for root in roots {
            if !root.is_dir() || history::find_root(&root).as_deref() != Some(root.as_path()) {
                continue;
            }
            match self.open(&root) {
                Ok(history) => {
                    kept.push(history.root.clone());
                    if history.writable {
                        let _ = capture::scan(&history);
                    }
                }
                Err(error) => eprintln!(
                    "versions: could not open {} at startup: {error}",
                    root.display()
                ),
            }
        }
        let _ = {
            let _guard = self.registry_lock.lock().unwrap();
            registry::write(&self.registry_path, &kept)
        };
    }

    fn start_watcher(&self, history: Arc<History>) {
        match crate::versions::watch::start(history.clone()) {
            Ok(watcher) => {
                self.watchers
                    .lock()
                    .unwrap()
                    .insert(history.root.clone(), watcher);
            }
            Err(error) => {
                eprintln!(
                    "versions: could not watch {}: {error}; scanning every five minutes",
                    history.root.display()
                );
                let fallback = crate::versions::watch::start_fallback(history.clone());
                self.watchers
                    .lock()
                    .unwrap()
                    .insert(history.root.clone(), fallback);
            }
        }
    }

    fn longest_root(&self, path: &Path) -> Option<PathBuf> {
        self.roots
            .lock()
            .unwrap()
            .keys()
            .filter(|root| path.starts_with(root))
            .max_by_key(|root| root.components().count())
            .cloned()
    }
}

/// Where `root` (under `from`) ends up after a transfer to `to`.
fn transfer_dest(from: &Path, to: &Path, root: &Path) -> Option<PathBuf> {
    root.strip_prefix(from).ok().map(|relative| to.join(relative))
}

/// Ownership re-check right before Clear/Stop act: the index's `meta` rows and
/// `meta.json` must both still name this app and this folder id (D14).
fn verify_ownership(history: &History) -> Result<()> {
    {
        let reader = history.reader()?;
        index::verify_owner(&reader)?;
    }
    let marker = read_marker(&history.root)?;
    if marker.folder_id != history.folder_id {
        return Err(Error::NotOurs(
            "meta.json no longer matches this history".into(),
        ));
    }
    Ok(())
}

fn fs_canonical(path: &Path) -> Result<PathBuf> {
    if let Ok(canonical) = std::fs::canonicalize(path) {
        return Ok(canonical);
    }
    // The path may not exist (a deleted file's Versions…/restore, or a root
    // just renamed). Resolve the longest existing ancestor and re-append the
    // rest.
    let mut existing = path;
    let mut rest = Vec::new();
    loop {
        if let Ok(mut canonical) = std::fs::canonicalize(existing) {
            for part in rest.iter().rev() {
                canonical.push(part);
            }
            return Ok(canonical);
        }
        match (existing.parent(), existing.file_name()) {
            (Some(parent), Some(name)) => {
                rest.push(name.to_os_string());
                existing = parent;
            }
            _ => return Err(Error::Io(format!("could not resolve {}", path.display()))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::versions::registry::read;
    use tempfile::tempdir;

    /// A temp dir with an owned history at `<dir>/Projects` (closed again)
    /// and a state whose registry lives in the same temp dir.
    fn setup() -> (tempfile::TempDir, PathBuf, VersionsState) {
        let dir = tempdir().unwrap();
        let root = dir.path().join("Projects");
        std::fs::create_dir(&root).unwrap();
        drop(History::create(&root).unwrap());
        let root = std::fs::canonicalize(&root).unwrap();
        let state = VersionsState::new(dir.path().join("versioned.json"));
        (dir, root, state)
    }

    fn record(history: &History, rel: &str, bytes: &[u8]) {
        std::fs::write(history.root.join(rel), bytes).unwrap();
        capture::record(history, Path::new(rel)).unwrap();
    }

    fn rows(history: &History, rel: &str) -> usize {
        crate::versions::restore::list_infos(history, rel.as_bytes(), None, None)
            .unwrap()
            .len()
    }

    #[test]
    fn finds_opens_and_registers_a_root() {
        let (_dir, root, state) = setup();
        assert_eq!(state.root_quick(&root.join("a.txt")), Some(root.clone()));
        assert!(!state.is_open(&root), "root_quick never opens SQLite");
        let found = state.root_for(&root).unwrap().unwrap();
        assert_eq!(found, root);
        assert_eq!(read(state.registry_path()), vec![root.clone()]);
        assert!(state.history_for(&root.join("a.txt")).unwrap().is_some());
    }

    #[test]
    fn concurrent_opens_share_one_writable_history() {
        let (_dir, root, state) = setup();
        let state = Arc::new(state);
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let (state, root) = (state.clone(), root.clone());
                std::thread::spawn(move || state.open(&root).unwrap())
            })
            .collect();
        let opened: Vec<Arc<History>> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        assert!(opened.iter().all(|h| h.writable));
        assert!(opened.iter().all(|h| Arc::ptr_eq(h, &opened[0])));
    }

    #[test]
    fn copying_keeps_the_source_recording_and_gives_the_copy_a_new_identity() {
        let (dir, root, state) = setup();
        let source = state.open(&root).unwrap();
        record(&source, "a.txt", b"hello");
        let old_id = source.folder_id.clone();
        drop(source);

        let dest = dir.path().join("Copy");
        state
            .transfer(&root, &dest, Transfer::Copy, || {
                crate::fs_ops::copy_entry(
                    root.to_string_lossy().to_string(),
                    dest.to_string_lossy().to_string(),
                    false,
                )
            })
            .unwrap();

        // The source is open again, writable, and still records.
        assert!(state.is_open(&root));
        let source = state.open(&root).unwrap();
        assert!(source.writable && !source.is_closed());
        record(&source, "a.txt", b"hello again");
        assert_eq!(rows(&source, "a.txt"), 2);

        let dest = std::fs::canonicalize(&dest).unwrap();
        let copy = state.open(&dest).unwrap();
        assert_ne!(copy.folder_id, old_id);
        assert_eq!(rows(&copy, "a.txt"), 1);
        assert!(read(state.registry_path()).contains(&dest));
    }

    #[test]
    fn a_running_scan_does_not_lock_a_moved_history_out() {
        let (dir, root, state) = setup();
        // A scan thread's `Arc` that outlives the state's reference.
        let lingering = state.open(&root).unwrap();
        let dest = dir.path().join("Renamed");
        state
            .transfer(&root, &dest, Transfer::Move, || {
                std::fs::rename(&root, &dest).map_err(|e| e.to_string())
            })
            .unwrap();
        assert!(lingering.is_closed());
        let moved = state.open(&dest).unwrap();
        assert!(moved.writable, "the old Arc must not keep the lock");
        assert_eq!(moved.folder_id, lingering.folder_id);
        assert_eq!(read(state.registry_path()), vec![std::fs::canonicalize(&dest).unwrap()]);
    }

    #[test]
    fn a_failed_transfer_reopens_everything() {
        let (dir, root, state) = setup();
        state.open(&root).unwrap();
        let result = state.transfer(&root, &dir.path().join("x"), Transfer::Move, || {
            Err("disk full".to_string())
        });
        assert_eq!(result, Err("disk full".to_string()));
        assert!(state.open(&root).unwrap().writable);
    }

    #[test]
    fn a_history_held_by_another_instance_blocks_the_transfer() {
        let (dir, root, state) = setup();
        // Registered (shared versioned.json) but locked by "another instance".
        registry::register(state.registry_path(), &root).unwrap();
        let other = History::open(&root).unwrap();
        assert!(other.writable);
        let mut ran = false;
        let result = state.transfer(&root, &dir.path().join("x"), Transfer::Move, || {
            ran = true;
            Ok(())
        });
        assert_eq!(result, Err(Error::ReadOnly.to_string()));
        assert!(!ran, "the folder must not be moved under the other instance");
    }

    #[test]
    fn trashing_unregisters_first_and_a_failure_reopens() {
        let (_dir, root, state) = setup();
        state.open_registered(&root).unwrap();
        let result: std::result::Result<(), String> = state.trash(&root, || {
            assert!(read(state.registry_path()).is_empty(), "unregistered before the move");
            Err("no Trash".into())
        });
        assert!(result.is_err());
        assert!(state.is_open(&root));
        assert_eq!(read(state.registry_path()), vec![root.clone()]);

        state.trash(&root, || Ok(())).unwrap();
        assert!(!state.is_open(&root));
        assert!(read(state.registry_path()).is_empty());
    }

    #[test]
    fn clear_moves_the_history_away_and_starts_a_new_one() {
        let (dir, root, state) = setup();
        let history = state.open_registered(&root).unwrap();
        record(&history, "a.txt", b"one");
        record(&history, "a.txt", b"two");
        let old_id = history.folder_id.clone();
        drop(history);

        let trashed = dir.path().join("trashed-versions");
        state
            .clear(&root, |versions| {
                std::fs::rename(versions, &trashed).map_err(|e| e.to_string())
            })
            .unwrap();
        assert!(trashed.join("index.sqlite").exists());
        let fresh = state.open(&root).unwrap();
        assert_ne!(fresh.folder_id, old_id);
        assert_eq!(rows(&fresh, "a.txt"), 1, "the current file is the first version");
    }

    #[test]
    fn clear_refuses_a_history_whose_marker_changed_and_keeps_recording() {
        let (_dir, root, state) = setup();
        state.open_registered(&root).unwrap();
        crate::versions::write_marker(&root, &"ab".repeat(16)).unwrap();
        let mut trashed = false;
        let result = state.clear(&root, |_| {
            trashed = true;
            Ok(())
        });
        assert!(result.is_err());
        assert!(!trashed);
        assert!(root.join(crate::versions::VERSIONS_DIR).join("index.sqlite").exists());
        // Reopen attempts fail on the mismatched marker, but nothing was lost.
        assert!(History::open(&root).is_err());
    }

    #[test]
    fn a_failed_clear_keeps_the_history_open_and_writable() {
        let (_dir, root, state) = setup();
        state.open_registered(&root).unwrap();
        let result = state.clear(&root, |_| Err("no Trash".into()));
        assert_eq!(result, Err("no Trash".to_string()));
        let history = state.open(&root).unwrap();
        assert!(history.writable && !history.is_closed());
        record(&history, "a.txt", b"still recording");
    }

    #[test]
    fn stop_keeping_forgets_the_root() {
        let (dir, root, state) = setup();
        state.open_registered(&root).unwrap();
        let trashed = dir.path().join("trashed-versions");
        state
            .stop_keeping(&root, |versions| {
                std::fs::rename(versions, &trashed).map_err(|e| e.to_string())
            })
            .unwrap();
        assert!(!state.is_open(&root));
        assert!(read(state.registry_path()).is_empty());
        assert_eq!(state.root_quick(&root), None);
    }

    #[test]
    fn startup_opens_registered_roots_and_drops_missing_ones() {
        let (dir, root, state) = setup();
        let gone = dir.path().join("Gone");
        registry::write(state.registry_path(), &[root.clone(), gone]).unwrap();
        std::fs::write(root.join("made-while-closed.txt"), b"x").unwrap();
        state.startup();
        assert!(state.is_open(&root));
        assert_eq!(read(state.registry_path()), vec![root.clone()]);
        // The launch scan recorded the file changed while the app was closed.
        let history = state.open(&root).unwrap();
        assert_eq!(rows(&history, "made-while-closed.txt"), 1);
    }

    #[test]
    fn history_info_adds_up_open_nested_histories() {
        let (_dir, root, state) = setup();
        let outer = state.open(&root).unwrap();
        record(&outer, "a.txt", b"outer");
        let nested = root.join("Nested");
        std::fs::create_dir(&nested).unwrap();
        let inner = state.adopt(History::create(&nested).unwrap());
        record(&inner, "b.txt", b"inner");
        let total = state.info_under(&root).unwrap();
        assert_eq!(total.versions, 2);
        assert_eq!(total.files, 2);
        assert!(total.bytes >= outer.info().unwrap().bytes + inner.info().unwrap().bytes);
    }

    #[test]
    fn rewrite_prefix_updates_the_registry() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("versioned.json");
        crate::versions::registry::write(&file, &[PathBuf::from("/foo/child")]).unwrap();
        crate::versions::registry::rewrite_prefix(&file, Path::new("/foo"), Path::new("/new"))
            .unwrap();
        assert_eq!(read(&file), vec![PathBuf::from("/new/child")]);
    }
}
