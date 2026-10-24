//! Managed state for open histories: one `History` and one watcher per root.
//! Kept Tauri-free so `lib.rs` can own it and tests can use it directly.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::versions::{
    APP_ID, Error, Result, capture, index,
    history::{self, History},
    meta_path, read_marker, registry,
    watch::WatcherHandle,
};

pub struct VersionsState {
    roots: Mutex<HashMap<PathBuf, Arc<History>>>,
    watchers: Mutex<HashMap<PathBuf, WatcherHandle>>,
    registry_path: PathBuf,
    registry_lock: Mutex<()>,
}

impl VersionsState {
    pub fn new(registry_path: PathBuf) -> Self {
        Self {
            roots: Mutex::new(HashMap::new()),
            watchers: Mutex::new(HashMap::new()),
            registry_path,
            registry_lock: Mutex::new(()),
        }
    }

    pub fn registry_path(&self) -> &Path {
        &self.registry_path
    }

    /// Open (or return) the history at `root`. A writable history gets a
    /// watcher; a history locked by another instance stays read-only.
    pub fn open(&self, root: &Path) -> Result<Arc<History>> {
        let root = fs_canonical(root)?;
        if let Some(history) = self.roots.lock().unwrap().get(&root) {
            return Ok(history.clone());
        }
        let history = Arc::new(History::open(&root)?);
        if history.writable {
            self.start_watcher(history.clone());
        }
        self.roots
            .lock()
            .unwrap()
            .entry(root)
            .or_insert_with(|| history.clone());
        Ok(history)
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

    /// The nearest registered/open root containing `path`, opening it if a
    /// `.versions/meta.json` is found.
    pub fn root_for(&self, path: &Path) -> Result<Option<PathBuf>> {
        let path = fs_canonical(path)?;
        if let Some(root) = self.longest_root(&path) {
            return Ok(Some(root));
        }
        let Some(root) = history::find_root(&path) else {
            return Ok(None);
        };
        self.open_registered(&root)?;
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

    /// Registered roots at or below `path`, deepest first.
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

    /// Every owned history on disk at or below `path`, even ones that were
    /// never opened (copy/move/trash must suspend those too).
    pub fn discover_under(&self, path: &Path) -> Vec<PathBuf> {
        let path = fs_canonical(path).unwrap_or_else(|_| path.to_path_buf());
        let mut roots = self.roots_under(&path);
        discover_roots(&path, &mut roots);
        roots.sort();
        roots.dedup();
        roots
    }

    /// Stop and remove every history at or below `path`; the caller performs
    /// the filesystem operation while the locks are released.
    pub fn suspend_under(&self, path: &Path) -> Vec<PathBuf> {
        let roots = self.roots_under(path);
        for root in &roots {
            self.take(root);
        }
        roots
    }

    /// Stop and remove one history, returning it so the lock stays alive until
    /// the caller is done (Clear History holds it through the Trash move).
    pub fn take(&self, root: &Path) -> Option<Arc<History>> {
        // After a rename the old path no longer canonicalises; fall back to the
        // key we stored so the watcher is still stopped.
        let root = fs_canonical(root).unwrap_or_else(|_| root.to_path_buf());
        if let Some(watcher) = self.watchers.lock().unwrap().remove(&root) {
            watcher.stop();
        }
        let history = self.roots.lock().unwrap().remove(&root);
        if let Some(history) = &history {
            if history.writable {
                let _ = history.checkpoint();
            }
        }
        history
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
        if let Some(watcher) = self.watchers.lock().unwrap().remove(&key) {
            watcher.stop();
        }
        self.roots.lock().unwrap().remove(&key);
        self.unregister(&key);
    }

    pub fn unregister(&self, root: &Path) {
        let _guard = self.registry_lock.lock().unwrap();
        let _ = registry::unregister(&self.registry_path, root);
    }

    /// Reopen a moved root at its new path and update the registry.
    pub fn move_registered(&self, from: &Path, to: &Path) -> Result<Arc<History>> {
        self.take(from);
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

/// Walk `dir` for owned `.versions/meta.json` markers without opening SQLite.
fn discover_roots(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_symlink() || !file_type.is_dir() {
            continue;
        }
        if path
            .file_name()
            .is_some_and(|name| name.to_string_lossy() == crate::versions::VERSIONS_DIR)
        {
            continue;
        }
        if meta_path(&path).is_file()
            && read_marker(&path)
                .map(|marker| marker.app == APP_ID)
                .unwrap_or(false)
        {
            if let Ok(canonical) = std::fs::canonicalize(&path) {
                out.push(canonical);
            }
            continue;
        }
        discover_roots(&path, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::versions::registry::read;
    use tempfile::tempdir;

    #[test]
    fn finds_opens_and_registers_a_root() {
        let dir = tempdir().unwrap();
        let root = dir.path().join("Projects");
        std::fs::create_dir(&root).unwrap();
        let history = History::create(&root).unwrap();
        drop(history);
        let state = VersionsState::new(dir.path().join("versioned.json"));
        let found = state.root_for(&root).unwrap().unwrap();
        assert_eq!(found, root);
        assert_eq!(read(state.registry_path()), vec![root.clone()]);
        assert!(state.history_for(&root.join("a.txt")).unwrap().is_some());
        state.take(&root);
        assert!(
            read(state.registry_path()).is_empty() || read(state.registry_path()) == vec![root]
        );
    }

    #[test]
    fn suspend_under_stops_and_removes_roots() {
        let dir = tempdir().unwrap();
        let root = dir.path().join("Projects");
        std::fs::create_dir(&root).unwrap();
        let history = History::create(&root).unwrap();
        drop(history);
        let state = VersionsState::new(dir.path().join("versioned.json"));
        state.open(&root).unwrap();
        assert_eq!(state.roots_under(&root), vec![root.clone()]);
        let suspended = state.suspend_under(&root);
        assert_eq!(suspended, vec![root.clone()]);
        assert!(state.roots_under(&root).is_empty());
    }

    #[test]
    fn a_copied_history_opens_with_a_new_identity_and_its_versions() {
        let dir = tempdir().unwrap();
        let root = dir.path().join("Projects");
        std::fs::create_dir(&root).unwrap();
        let history = History::create(&root).unwrap();
        std::fs::write(root.join("a.txt"), b"hello").unwrap();
        crate::versions::capture::record(&history, Path::new("a.txt")).unwrap();
        let old_id = history.folder_id.clone();
        drop(history);

        let state = VersionsState::new(dir.path().join("versioned.json"));
        state.open(&root).unwrap();
        state.suspend_under(&root);
        let dest = dir.path().join("Copy");
        crate::fs_ops::copy_entry(
            root.to_string_lossy().to_string(),
            dest.to_string_lossy().to_string(),
            false,
        )
        .unwrap();
        let copy = state.register_copy(&dest).unwrap();
        assert_ne!(copy.folder_id, old_id);
        let rows = crate::versions::restore::list_infos(&copy, b"a.txt", None, None).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].size, 5);
        state.take(&dest);
    }

    #[test]
    fn a_moved_root_reopens_at_its_new_path() {
        let dir = tempdir().unwrap();
        let root = dir.path().join("Projects");
        std::fs::create_dir(&root).unwrap();
        let history = History::create(&root).unwrap();
        let old_id = history.folder_id.clone();
        drop(history);

        let state = VersionsState::new(dir.path().join("versioned.json"));
        state.open(&root).unwrap();
        state.suspend_under(&root);
        let dest = dir.path().join("Renamed");
        std::fs::rename(&root, &dest).unwrap();
        let moved = state.move_registered(&root, &dest).unwrap();
        assert_eq!(moved.root, dest);
        assert_eq!(moved.folder_id, old_id);
        assert_eq!(read(state.registry_path()), vec![dest.clone()]);
        state.take(&dest);
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
