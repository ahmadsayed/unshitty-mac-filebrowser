//! Structure manifests for shared folders (spec D2, D3, D8, D13).
//!
//! A manifest lists one folder's entries (name, kind, size, mtime; folders
//! also carry their child manifest's hash). Files carry no content hash:
//! `ContentCache` hashes content lazily, when a visitor asks for it.

use crate::versions::VERSIONS_DIR;
use iroh_blobs::Hash;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

pub const MAX_MANIFEST: usize = 64 * 1024 * 1024;
const VERSION: u8 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kind {
    File,
    Folder,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ManifestEntry {
    pub name: String,
    pub kind: Kind,
    pub size: u64,
    pub mtime: i64,
    /// The child manifest's hash; `Some` exactly for folders.
    pub folder: Option<Hash>,
}

#[derive(Serialize, Deserialize)]
struct WireEntry {
    name: String,
    kind: Kind,
    size: u64,
    mtime: i64,
    folder: Option<[u8; 32]>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Manifest {
    pub entries: Vec<ManifestEntry>,
}

impl Manifest {
    /// Canonical bytes: version byte, then postcard entries sorted by name.
    pub fn encode(&self) -> Vec<u8> {
        let mut wire: Vec<WireEntry> = self
            .entries
            .iter()
            .map(|e| WireEntry {
                name: e.name.clone(),
                kind: e.kind,
                size: e.size,
                mtime: e.mtime,
                folder: e.folder.map(|h| *h.as_bytes()),
            })
            .collect();
        wire.sort_by(|a, b| a.name.as_bytes().cmp(b.name.as_bytes()));
        let mut out = vec![VERSION];
        out.extend(postcard::to_stdvec(&wire).expect("manifest encodes"));
        out
    }

    pub fn decode(bytes: &[u8]) -> Result<Manifest, String> {
        match bytes.split_first() {
            Some((&VERSION, rest)) => {
                let wire: Vec<WireEntry> =
                    postcard::from_bytes(rest).map_err(|e| format!("Bad manifest: {e}"))?;
                wire.into_iter()
                    .map(from_wire)
                    .collect::<Result<_, _>>()
                    .map(|entries| Manifest { entries })
            }
            _ => Err("Unsupported manifest version.".to_string()),
        }
    }

    pub fn hash(&self) -> Hash {
        Hash::new(self.encode())
    }
}

fn from_wire(w: WireEntry) -> Result<ManifestEntry, String> {
    if (w.kind == Kind::Folder) != w.folder.is_some() {
        return Err("Bad manifest: folder hash mismatch.".to_string());
    }
    Ok(ManifestEntry {
        name: w.name,
        kind: w.kind,
        size: w.size,
        mtime: w.mtime,
        folder: w.folder.map(Hash::from_bytes),
    })
}

// ---- Tree -------------------------------------------------------------

struct Stored {
    bytes: Vec<u8>,
    manifest: Manifest,
}

pub struct Tree {
    pub root: Hash,
    previous: Option<Hash>,
    root_dir: PathBuf,
    canon_root: PathBuf,
    cap: usize,
    /// True once a symlinked folder was followed: subtrees can then change
    /// without their own path being reported, so nothing is reused.
    has_links: bool,
    /// Manifest hash of each real folder, for reusing unchanged siblings.
    by_dir: HashMap<PathBuf, Hash>,
    stored: HashMap<Hash, Stored>,
}

/// Nanoseconds since the epoch (negative before it).
fn nanos(t: std::io::Result<SystemTime>) -> i64 {
    match t.map(|t| t.duration_since(SystemTime::UNIX_EPOCH)) {
        Ok(Ok(d)) => d.as_nanos() as i64,
        Ok(Err(e)) => -(e.duration().as_nanos() as i64),
        Err(_) => 0,
    }
}

fn too_many(folder: &Path) -> String {
    let name = folder.file_name().unwrap_or(folder.as_os_str());
    format!("“{}” has too many items to share.", name.to_string_lossy())
}

fn valid_name(name: &str) -> bool {
    !(name.is_empty() || name == "." || name == ".." || name.contains(['/', '\0']))
}

/// How a walk treats folders it already knows.
#[derive(Clone, Copy, PartialEq)]
enum Reuse {
    No,
    /// Reuse real sibling folders, except this one (just rebuilt).
    Siblings,
}

impl Tree {
    pub fn build(folder: &Path) -> Result<Tree, String> {
        Self::build_with_cap(folder, MAX_MANIFEST)
    }

    fn build_with_cap(folder: &Path, cap: usize) -> Result<Tree, String> {
        let canon_root = fs::canonicalize(folder).map_err(|e| e.to_string())?;
        let mut tree = Tree {
            root: Hash::new(b""),
            previous: None,
            root_dir: folder.to_path_buf(),
            canon_root,
            cap,
            has_links: false,
            by_dir: HashMap::new(),
            stored: HashMap::new(),
        };
        tree.root = tree.walk_dir(folder, None)?;
        Ok(tree)
    }

    /// Rebuilds the folder containing `changed` (or `changed` itself when it
    /// is a folder) from scratch, then each ancestor up to the shared folder,
    /// reusing only unchanged real sibling folders.
    pub fn rebuild(&mut self, folder: &Path, changed: &Path) -> Result<(), String> {
        let old = self.root;
        let mut dir = nearest_folder(folder, changed);
        let mut hash = if self.has_links {
            self.walk_dir(folder, None)?
        } else {
            self.by_dir.retain(|k, _| !k.starts_with(&dir));
            let h = self.walk_dir(&dir, None)?;
            self.climb(folder, &mut dir, h)?
        };
        std::mem::swap(&mut self.root, &mut hash);
        if self.root != old {
            self.previous = Some(old);
        }
        self.prune();
        Ok(())
    }

    fn climb(&mut self, folder: &Path, dir: &mut PathBuf, mut hash: Hash) -> Result<Hash, String> {
        while dir.as_path() != folder {
            let child = dir.clone();
            dir.pop();
            hash = self.walk_dir(dir, Some((&child, hash)))?;
        }
        Ok(hash)
    }

    /// Walks one folder. `done` is a child folder already rebuilt (path and
    /// manifest); other real folders known from before are reused when set.
    fn walk_dir(&mut self, dir: &Path, done: Option<(&PathBuf, Hash)>) -> Result<Hash, String> {
        let mut stack = self.ancestors_of(dir);
        let reuse = if done.is_some() {
            Reuse::Siblings
        } else {
            Reuse::No
        };
        self.walk(dir, &mut stack, reuse, done)?
            .ok_or_else(|| "Couldn’t read the folder.".to_string())
    }

    /// Canonical paths of the shared folder's ancestors of `dir`, so loop
    /// detection agrees between a full build and a partial rebuild.
    fn ancestors_of(&self, dir: &Path) -> Vec<PathBuf> {
        dir.ancestors()
            .skip(1)
            .take_while(|a| a.starts_with(&self.root_dir))
            .filter_map(|a| fs::canonicalize(a).ok())
            .collect()
    }

    /// Builds the manifest for `dir`. `None` means the folder can't be read
    /// (or is a loop).
    fn walk(
        &mut self,
        dir: &Path,
        stack: &mut Vec<PathBuf>,
        reuse: Reuse,
        done: Option<(&PathBuf, Hash)>,
    ) -> Result<Option<Hash>, String> {
        let Ok(canon) = fs::canonicalize(dir) else {
            return Ok(None);
        };
        if stack.contains(&canon) {
            return Ok(None);
        }
        let Ok(rd) = fs::read_dir(dir) else {
            return Ok(None);
        };
        stack.push(canon);
        let mut entries = Vec::new();
        for item in rd.flatten() {
            if let Some(e) = self.entry_for(&item, stack, reuse, done)? {
                entries.push(e);
            }
        }
        stack.pop();
        Ok(Some(self.store(dir, Manifest { entries })?))
    }

    fn entry_for(
        &mut self,
        item: &fs::DirEntry,
        stack: &mut Vec<PathBuf>,
        reuse: Reuse,
        done: Option<(&PathBuf, Hash)>,
    ) -> Result<Option<ManifestEntry>, String> {
        let Ok(name) = item.file_name().into_string() else {
            return Ok(None);
        };
        let path = item.path();
        if name == VERSIONS_DIR {
            return Ok(None);
        }
        let Some((meta, linked)) = self.visible_meta(&path) else {
            return Ok(None);
        };
        self.has_links |= linked;
        let mtime = nanos(meta.modified());
        if meta.is_dir() {
            let known = self.known_folder(&path, linked, reuse, done);
            let hash = match known {
                Some(h) => Some(h),
                None => self.walk(&path, stack, reuse, None)?,
            };
            return Ok(hash.map(|h| ManifestEntry {
                name,
                kind: Kind::Folder,
                size: 0,
                mtime,
                folder: Some(h),
            }));
        }
        // Files are never opened here: unreadable ones are still listed.
        Ok(meta.is_file().then(|| ManifestEntry {
            name,
            kind: Kind::File,
            size: meta.len(),
            mtime,
            folder: None,
        }))
    }

    fn known_folder(
        &self,
        path: &Path,
        linked: bool,
        reuse: Reuse,
        done: Option<(&PathBuf, Hash)>,
    ) -> Option<Hash> {
        if let Some((d, h)) = done {
            if d == path {
                return Some(h);
            }
        }
        if reuse == Reuse::Siblings && !linked {
            return self.by_dir.get(path).copied();
        }
        None
    }

    /// Metadata of what the entry points to (and whether it was reached
    /// through a symlink); symlinks only when they resolve inside the shared
    /// folder.
    fn visible_meta(&self, path: &Path) -> Option<(fs::Metadata, bool)> {
        let link = fs::symlink_metadata(path).ok()?;
        if !link.file_type().is_symlink() {
            return Some((link, false));
        }
        let target = fs::canonicalize(path).ok()?;
        if !target.starts_with(&self.canon_root) {
            return None;
        }
        Some((fs::metadata(path).ok()?, true))
    }

    fn store(&mut self, dir: &Path, manifest: Manifest) -> Result<Hash, String> {
        let bytes = manifest.encode();
        if bytes.len() > self.cap {
            return Err(too_many(dir));
        }
        let hash = Hash::new(&bytes);
        self.stored
            .entry(hash)
            .or_insert(Stored { bytes, manifest });
        self.by_dir.insert(dir.to_path_buf(), hash);
        Ok(hash)
    }

    /// Drops manifests unreachable from the current and previous roots.
    fn prune(&mut self) {
        let mut keep = std::collections::HashSet::new();
        let mut todo: Vec<Hash> = std::iter::once(self.root).chain(self.previous).collect();
        while let Some(h) = todo.pop() {
            if let Some(s) = self.stored.get(&h).filter(|_| keep.insert(h)) {
                todo.extend(s.manifest.entries.iter().filter_map(|e| e.folder));
            }
        }
        self.stored.retain(|h, _| keep.contains(h));
        let stored = &self.stored;
        self.by_dir.retain(|_, h| stored.contains_key(h));
    }

    pub fn manifest(&self, hash: &Hash) -> Option<&[u8]> {
        self.stored.get(hash).map(|s| s.bytes.as_slice())
    }

    /// Resolves a chain of entry names, starting at the current or previous
    /// root, to the file's absolute path.
    pub fn entry_path(&self, root: &Hash, names: &[String]) -> Option<(PathBuf, &ManifestEntry)> {
        if *root != self.root && Some(*root) != self.previous {
            return None;
        }
        let mut path = self.root_dir.clone();
        let mut cur = Some(*root);
        let mut found = None;
        for name in names {
            if !valid_name(name) {
                return None;
            }
            // A file in the middle of the chain leaves `cur` empty.
            let manifest = &self.stored.get(&cur?)?.manifest;
            let entry = manifest.entries.iter().find(|e| e.name == *name)?;
            path.push(name);
            cur = entry.folder;
            found = Some(entry);
        }
        found.map(|e| (path, e))
    }

    pub fn manifests(&self) -> impl Iterator<Item = Hash> + '_ {
        self.stored.keys().copied()
    }
}

/// The folder to rebuild for a change at `changed`: itself if it is a folder
/// inside `folder`, otherwise its nearest existing folder ancestor.
fn nearest_folder(folder: &Path, changed: &Path) -> PathBuf {
    changed
        .ancestors()
        .take_while(|a| a.starts_with(folder))
        .find(|a| a.is_dir())
        .unwrap_or(folder)
        .to_path_buf()
}

// ---- Content hashes ---------------------------------------------------

/// Lazily computed content hashes, cached by (path, size, mtime).
#[derive(Default)]
pub struct ContentCache {
    known: HashMap<PathBuf, (u64, i64, Hash)>,
    #[cfg(test)]
    between_reads: Option<Box<dyn FnMut(&Path)>>,
}

fn stat(path: &Path) -> Result<(u64, i64), String> {
    let m = fs::metadata(path).map_err(|e| e.to_string())?;
    Ok((m.len(), nanos(m.modified())))
}

impl ContentCache {
    /// The file's hash and size, or `None` when it changed while being read.
    /// A `versions_hint` must already be checked against the file's current
    /// size and mtime by the caller.
    pub fn hash(
        &mut self,
        path: &Path,
        versions_hint: Option<Hash>,
    ) -> Result<Option<(Hash, u64)>, String> {
        let (size, mtime) = stat(path)?;
        if let Some(&(s, m, h)) = self.known.get(path) {
            if (s, m) == (size, mtime) {
                return Ok(Some((h, size)));
            }
        }
        let hash = match versions_hint {
            Some(h) => h,
            None => self.read_hash(path)?,
        };
        if stat(path)? != (size, mtime) {
            return Ok(None);
        }
        self.known.insert(path.to_path_buf(), (size, mtime, hash));
        Ok(Some((hash, size)))
    }

    fn read_hash(&mut self, path: &Path) -> Result<Hash, String> {
        let mut file = fs::File::open(path).map_err(|e| e.to_string())?;
        let mut hasher = blake3::Hasher::new();
        std::io::copy(&mut file, &mut hasher).map_err(|e| e.to_string())?;
        #[cfg(test)]
        if let Some(hook) = self.between_reads.as_mut() {
            hook(path);
        }
        Ok(Hash::from_bytes(*hasher.finalize().as_bytes()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn write(p: &Path, body: &str) {
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, body).unwrap();
    }
    fn set_mtime(p: &Path, secs: u64) {
        let t = SystemTime::UNIX_EPOCH + Duration::from_secs(secs);
        fs::File::open(p).unwrap().set_modified(t).unwrap();
    }

    fn fixture() -> tempfile::TempDir {
        let t = tempfile::tempdir().unwrap();
        for f in ["a/b/x", "a/c/z", "top"] {
            write(&t.path().join(f), "hi");
            set_mtime(&t.path().join(f), 1_000);
        }
        t
    }

    fn dir_hash(tree: &Tree, p: &Path) -> Hash {
        tree.by_dir[p]
    }

    #[test]
    fn the_same_folder_gives_the_same_root() {
        let t = fixture();
        let one = Tree::build(t.path()).unwrap().root;
        assert_eq!(one, Tree::build(t.path()).unwrap().root);
        let u = tempfile::tempdir().unwrap();
        for f in ["top", "a/c/z", "a/b/x"] {
            write(&u.path().join(f), "hi");
            set_mtime(&u.path().join(f), 1_000);
        }
        for d in ["a", "a/b", "a/c"] {
            set_mtime(&u.path().join(d), 5);
            set_mtime(&t.path().join(d), 5);
        }
        set_mtime(u.path(), 5);
        set_mtime(t.path(), 5);
        assert_eq!(
            Tree::build(t.path()).unwrap().root,
            Tree::build(u.path()).unwrap().root
        );
    }

    #[test]
    fn a_rename_changes_that_folder_and_its_parents_only() {
        let t = fixture();
        let p = t.path();
        let before = Tree::build(p).unwrap();
        fs::rename(p.join("a/b/x"), p.join("a/b/y")).unwrap();
        let after = Tree::build(p).unwrap();
        assert_ne!(before.root, after.root);
        assert_ne!(
            dir_hash(&before, &p.join("a")),
            dir_hash(&after, &p.join("a"))
        );
        assert_ne!(
            dir_hash(&before, &p.join("a/b")),
            dir_hash(&after, &p.join("a/b"))
        );
        assert_eq!(
            dir_hash(&before, &p.join("a/c")),
            dir_hash(&after, &p.join("a/c"))
        );
    }

    #[test]
    fn a_size_or_mtime_change_changes_the_root() {
        let t = fixture();
        let p = t.path();
        let r0 = Tree::build(p).unwrap().root;
        set_mtime(&p.join("top"), 2_000);
        let r1 = Tree::build(p).unwrap().root;
        assert_ne!(r0, r1);
        write(&p.join("top"), "longer");
        set_mtime(&p.join("top"), 2_000);
        assert_ne!(r1, Tree::build(p).unwrap().root);
    }

    #[test]
    fn rebuild_matches_a_fresh_build() {
        let t = fixture();
        let p = t.path();
        let mut tree = Tree::build(p).unwrap();
        fs::rename(p.join("a/b/x"), p.join("a/b/y")).unwrap();
        write(&p.join("new/deep/f"), "n");
        tree.rebuild(p, &p.join("a/b/y")).unwrap();
        tree.rebuild(p, &p.join("new")).unwrap();
        assert_eq!(tree.root, Tree::build(p).unwrap().root);
        fs::remove_file(p.join("a/c/z")).unwrap();
        tree.rebuild(p, &p.join("a/c/z")).unwrap();
        assert_eq!(tree.root, Tree::build(p).unwrap().root);
    }

    fn names(tree: &Tree, h: &Hash) -> Vec<String> {
        Manifest::decode(tree.manifest(h).unwrap())
            .unwrap()
            .entries
            .into_iter()
            .map(|e| e.name)
            .collect()
    }

    #[cfg(unix)]
    #[test]
    fn excluded_items_never_appear() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let t = tempfile::tempdir().unwrap();
        let p = t.path();
        write(&p.join(".versions/obj"), "v");
        write(&p.join("keep"), "k");
        write(&p.join("sub/inside"), "i");
        symlink("/etc", p.join("evil")).unwrap();
        symlink(p.join("sub"), p.join("good")).unwrap();
        symlink(p, p.join("sub/loop")).unwrap();
        let fifo = std::process::Command::new("mkfifo")
            .arg(p.join("pipe"))
            .status()
            .unwrap();
        assert!(fifo.success());
        write(&p.join(".hidden"), "h");
        write(&p.join("secret"), "s");
        fs::set_permissions(p.join("secret"), fs::Permissions::from_mode(0o000)).unwrap();
        write(&p.join("locked/in"), "l");
        fs::set_permissions(p.join("locked"), fs::Permissions::from_mode(0o000)).unwrap();
        let privileged = fs::read(p.join("secret")).is_ok();
        let tree = Tree::build(p).unwrap();
        let n = names(&tree, &tree.root);
        for gone in [".versions", "evil", "pipe"] {
            assert!(!n.contains(&gone.to_string()), "{gone} present");
        }
        // Unreadable files are listed (the walk opens nothing); unreadable folders are not.
        assert!(n.contains(&"secret".to_string()));
        assert!(n.contains(&".hidden".to_string()));
        assert_eq!(privileged || !n.contains(&"locked".to_string()), true);
        let m = Manifest::decode(tree.manifest(&tree.root).unwrap()).unwrap();
        let good = m.entries.iter().find(|e| e.name == "good").unwrap();
        assert_eq!(good.kind, Kind::Folder);
        assert!(n.contains(&"keep".to_string()));
        fs::set_permissions(p.join("secret"), fs::Permissions::from_mode(0o600)).unwrap();
        fs::set_permissions(p.join("locked"), fs::Permissions::from_mode(0o700)).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_names_are_left_out() {
        use std::os::unix::ffi::OsStrExt;
        let t = tempfile::tempdir().unwrap();
        fs::write(t.path().join(std::ffi::OsStr::from_bytes(b"bad\xff")), "x").unwrap();
        write(&t.path().join("ok"), "x");
        let tree = Tree::build(t.path()).unwrap();
        assert_eq!(names(&tree, &tree.root), vec!["ok"]);
    }

    #[test]
    fn a_ten_thousand_entry_folder_fits_one_manifest() {
        let t = tempfile::tempdir().unwrap();
        for i in 0..10_000 {
            fs::write(t.path().join(format!("file-{i}")), "").unwrap();
        }
        let tree = Tree::build(t.path()).unwrap();
        assert_eq!(tree.manifests().count(), 1);
        let bytes = tree.manifest(&tree.root).unwrap();
        assert!(bytes.len() < MAX_MANIFEST);
        assert_eq!(Manifest::decode(bytes).unwrap().entries.len(), 10_000);
    }

    #[test]
    fn an_oversized_manifest_is_refused() {
        let t = tempfile::tempdir().unwrap();
        let dir = t.path().join("big");
        for i in 0..50 {
            write(&dir.join(format!("file-{i}")), "");
        }
        let err = Tree::build_with_cap(&dir, 100).err().unwrap();
        assert_eq!(err, "“big” has too many items to share.");
    }

    fn v(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn entry_path_resolves_only_names_in_that_manifest() {
        let t = fixture();
        let tree = Tree::build(t.path()).unwrap();
        let (path, e) = tree.entry_path(&tree.root, &v(&["top"])).unwrap();
        assert_eq!((path, e.kind), (t.path().join("top"), Kind::File));
        let (path, e) = tree.entry_path(&tree.root, &v(&["a"])).unwrap();
        assert_eq!((path, e.kind), (t.path().join("a"), Kind::Folder));
        let (path, _) = tree.entry_path(&tree.root, &v(&["a", "b", "x"])).unwrap();
        assert_eq!(path, t.path().join("a/b/x"));
        for bad in [
            v(&["nope"]),
            v(&["../x"]),
            v(&["a/b"]),
            v(&[""]),
            v(&[".."]),
            v(&["."]),
            v(&["a", "nope"]),
            v(&["top", "x"]),
            v(&[]),
        ] {
            assert!(tree.entry_path(&tree.root, &bad).is_none(), "{bad:?}");
        }
        assert!(tree
            .entry_path(&Hash::new(b"unknown"), &v(&["top"]))
            .is_none());
    }

    #[test]
    fn identical_folders_resolve_each_to_its_own_path() {
        let t = tempfile::tempdir().unwrap();
        let p = t.path();
        for d in ["one", "two"] {
            write(&p.join(d).join("f"), "same");
            set_mtime(&p.join(d).join("f"), 7);
        }
        let tree = Tree::build(p).unwrap();
        for d in ["one", "two"] {
            let (path, _) = tree.entry_path(&tree.root, &v(&[d, "f"])).unwrap();
            assert_eq!(path, p.join(d).join("f"));
        }
    }

    #[test]
    fn only_the_current_and_previous_roots_resolve() {
        let t = fixture();
        let p = t.path();
        let mut tree = Tree::build(p).unwrap();
        let r0 = tree.root;
        fs::rename(p.join("a/b"), p.join("a/d")).unwrap();
        tree.rebuild(p, &p.join("a/d")).unwrap();
        let (path, _) = tree.entry_path(&tree.root, &v(&["a", "d", "x"])).unwrap();
        assert_eq!(path, p.join("a/d/x"));
        assert!(tree.entry_path(&r0, &v(&["top"])).is_some());
        write(&p.join("more"), "m");
        tree.rebuild(p, &p.join("more")).unwrap();
        assert!(tree.entry_path(&r0, &v(&["top"])).is_none());
        assert!(tree.manifest(&r0).is_none());
        let reachable: Vec<Hash> = tree.manifests().collect();
        assert!(!reachable.contains(&r0));
        assert!(reachable.contains(&tree.root));
    }

    #[test]
    fn manifest_decoding_rejects_garbage() {
        assert!(Manifest::decode(&[]).is_err());
        assert!(Manifest::decode(&[9]).is_err());
        assert!(Manifest::decode(&[1, 0xff, 0xff]).is_err());
        let bad = Manifest {
            entries: vec![ManifestEntry {
                name: "d".into(),
                kind: Kind::Folder,
                size: 0,
                mtime: 0,
                folder: None,
            }],
        };
        assert!(Manifest::decode(&bad.encode()).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn the_content_cache_skips_unchanged_files() {
        use std::os::unix::fs::PermissionsExt;
        let t = tempfile::tempdir().unwrap();
        let f = t.path().join("f");
        write(&f, "hello");
        let mut cache = ContentCache::default();
        let (h, size) = cache.hash(&f, None).unwrap().unwrap();
        assert_eq!((h, size), (Hash::new(b"hello"), 5));
        fs::set_permissions(&f, fs::Permissions::from_mode(0o000)).unwrap();
        if fs::read(&f).is_ok() {
            return; // privileged: removing permission proves nothing
        }
        let again = cache.hash(&f, None);
        fs::set_permissions(&f, fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(again.unwrap(), Some((h, 5)));
    }

    #[test]
    fn a_hint_is_used_without_reading() {
        let t = tempfile::tempdir().unwrap();
        let f = t.path().join("f");
        write(&f, "hello");
        let hint = Hash::new(b"from the index");
        let got = ContentCache::default().hash(&f, Some(hint)).unwrap();
        assert_eq!(got, Some((hint, 5)));
    }

    #[test]
    fn a_file_that_changes_while_hashing_is_not_hashed() {
        let t = tempfile::tempdir().unwrap();
        let f = t.path().join("f");
        write(&f, "hello");
        let mut cache = ContentCache::default();
        cache.between_reads = Some(Box::new(|p| fs::write(p, "hello, changed").unwrap()));
        assert_eq!(cache.hash(&f, None).unwrap(), None);
    }

    #[test]
    fn a_file_whose_mtime_changes_while_hashing_is_not_hashed() {
        let t = tempfile::tempdir().unwrap();
        let f = t.path().join("f");
        write(&f, "hello");
        let mut cache = ContentCache::default();
        cache.between_reads = Some(Box::new(|p| set_mtime(p, 42)));
        assert_eq!(cache.hash(&f, None).unwrap(), None);
    }

    fn set_mtime_ns(p: &Path, secs: u64, nanos: u32) {
        let t = SystemTime::UNIX_EPOCH + Duration::new(secs, nanos);
        fs::File::open(p).unwrap().set_modified(t).unwrap();
    }

    #[test]
    fn a_same_second_rewrite_changes_the_root_and_misses_the_cache() {
        let t = tempfile::tempdir().unwrap();
        let f = t.path().join("f");
        write(&f, "aaaa");
        set_mtime_ns(&f, 1_000, 100);
        let r0 = Tree::build(t.path()).unwrap().root;
        let mut cache = ContentCache::default();
        let (h0, _) = cache.hash(&f, None).unwrap().unwrap();
        write(&f, "bbbb");
        set_mtime_ns(&f, 1_000, 200);
        assert_ne!(r0, Tree::build(t.path()).unwrap().root);
        let (h1, _) = cache.hash(&f, None).unwrap().unwrap();
        assert_ne!(h0, h1);
    }

    #[test]
    fn rebuild_does_not_reuse_stale_subtrees_of_the_changed_folder() {
        let t = tempfile::tempdir().unwrap();
        let p = t.path();
        write(&p.join("a/sub/f"), "one");
        let mut tree = Tree::build(p).unwrap();
        fs::remove_dir_all(p.join("a")).unwrap();
        write(&p.join("a/sub/h"), "different");
        tree.rebuild(p, &p.join("a")).unwrap();
        assert_eq!(tree.root, Tree::build(p).unwrap().root);
    }

    #[cfg(unix)]
    #[test]
    fn rebuild_refreshes_symlinked_folders() {
        let t = tempfile::tempdir().unwrap();
        let p = t.path();
        write(&p.join("sub/f"), "one");
        std::os::unix::fs::symlink(p.join("sub"), p.join("good")).unwrap();
        let mut tree = Tree::build(p).unwrap();
        write(&p.join("sub/new"), "n");
        tree.rebuild(p, &p.join("sub/new")).unwrap();
        assert_eq!(tree.root, Tree::build(p).unwrap().root);
    }

    #[cfg(unix)]
    #[test]
    fn rebuild_excludes_links_back_to_the_shared_folder() {
        let t = tempfile::tempdir().unwrap();
        let p = t.path();
        write(&p.join("keep"), "k");
        let mut tree = Tree::build(p).unwrap();
        fs::create_dir(p.join("n")).unwrap();
        std::os::unix::fs::symlink(p, p.join("n/link")).unwrap();
        tree.rebuild(p, &p.join("n")).unwrap();
        let fresh = Tree::build(p).unwrap();
        assert_eq!(tree.root, fresh.root);
        let n = Manifest::decode(tree.manifest(&tree.root).unwrap()).unwrap();
        let n_hash = n
            .entries
            .iter()
            .find(|e| e.name == "n")
            .unwrap()
            .folder
            .unwrap();
        assert!(names(&tree, &n_hash).is_empty());
    }

    #[test]
    fn keep_versions_and_iroh_hash_alike() {
        let data = b"the same bytes";
        assert_eq!(blake3::hash(data).as_bytes(), Hash::new(data).as_bytes());
    }
}
