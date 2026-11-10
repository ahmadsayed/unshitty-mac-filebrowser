//! Keep Versions as a decorator around this computer's provider (design D7,
//! migration step 6b): `Versioned` is what the registry holds for paths on
//! this computer. It answers everything as `places_local::Local` does, and
//! keeps the histories inside the items it changes consistent around the
//! writes, exactly as the commands did before:
//!
//! - Rename: the histories inside the item are closed for the rename and
//!   reopened where it went (`VersionsState::transfer`, as a move);
//! - Move to Trash: closed and unregistered before the items go, reopened
//!   for those that stayed (`VersionsState::trash_many`);
//! - a copy or move on this computer: closed for the whole transfer and
//!   reopened after, a copied history with a new identity (`within` of its
//!   transfer session, around the engine's route), its Replace sending the
//!   old item through the plain logged Trash, as the transfer already
//!   closed what it holds;
//! - Replace's Trash of a transfer that only lands here (a Drive download)
//!   or a moved local source (an upload): `VersionedTrash`, which closes the
//!   histories before the item goes and reopens them if a failed Replace
//!   takes it back.
//!
//! New folders and files and Put Back touch no history and pass through.
//! Batch Rename (the `LocalOnly` extension, `rename_many`) passes through
//! untouched too, as the command did before the registry: a history
//! inside a folder it renames is not moved with it, unlike Rename's.

use std::cell::Cell;
use std::path::{Path, PathBuf};

use places::{
    Capabilities, Cx, Entry, Error, Extensions, Fetch, Fetched, Landing, ListOptions, NewKind, PathInfo,
    Provider, Put, Reading, Result, Transfer, Trashed,
};
use places_local::Local;
use places_local::transfer::{ReplaceTrash, TrashOf};

use crate::providers::Reach;
use crate::versions::state::{Transfer as Mode, VersionedTrash, VersionsState};

/// This computer's provider with Keep Versions around its writes.
pub struct Versioned {
    inner: Local,
    versions: Box<dyn Reach<VersionsState>>,
}

impl Versioned {
    /// `inner`, keeping the histories `versions` reaches.
    pub fn new(inner: Local, versions: Box<dyn Reach<VersionsState>>) -> Self {
        Versioned { inner, versions }
    }

    fn versions(&self) -> &VersionsState {
        self.versions.reach()
    }
}

/// Text of a path the app made from its own strings.
fn text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

impl Provider for Versioned {
    fn scheme(&self) -> &'static str {
        self.inner.scheme()
    }
    fn capabilities(&self, path: &str) -> Capabilities {
        self.inner.capabilities(path)
    }
    fn list(&self, dir: &str, opts: &ListOptions, cx: &Cx) -> Result<Vec<Entry>> {
        self.inner.list(dir, opts, cx)
    }
    fn stat(&self, path: &str, cx: &Cx) -> Result<Option<Entry>> {
        self.inner.stat(path, cx)
    }
    fn lookup(&self, pending: &str, cx: &Cx) -> Result<Vec<Entry>> {
        self.inner.lookup(pending, cx)
    }
    fn exists(&self, path: &str, cx: &Cx) -> bool {
        self.inner.exists(path, cx)
    }
    fn path_info(&self, paths: &[String], cx: &Cx) -> Vec<Option<PathInfo>> {
        self.inner.path_info(paths, cx)
    }
    fn fetch(&self, path: &str, purpose: Fetch, cx: &Cx) -> Result<Fetched> {
        self.inner.fetch(path, purpose, cx)
    }
    fn fetch_reports_progress(&self) -> bool {
        self.inner.fetch_reports_progress()
    }
    fn create(&self, parent: &str, name: &str, kind: NewKind, cx: &Cx) -> Result<String> {
        self.inner.create(parent, name, kind, cx)
    }
    /// The histories inside the item move with it.
    fn rename(&self, path: &str, new_name: &str, cx: &Cx) -> Result<String> {
        let to = places_local::provider::renamed_path(path, new_name).map_err(Error::Other)?;
        let mut renamed = None;
        self.versions()
            .transfer(Path::new(path), Path::new(&to), Mode::Move, || {
                renamed = Some(self.inner.rename(path, new_name, cx)?);
                Ok(())
            })
            .map_err(Error::Other)?;
        Ok(renamed.unwrap_or(to))
    }
    /// The histories inside each item are closed before it goes, and
    /// reopened if it stays. Nothing is closed when this computer's Trash
    /// can't be reached at all.
    fn trash(&self, paths: &[&str], cx: &Cx) -> Vec<Trashed> {
        if self.inner.trash_home().is_err() {
            return self.inner.trash(paths, cx);
        }
        let origins: Vec<PathBuf> = paths.iter().map(PathBuf::from).collect();
        let results = self.versions().trash_many(&origins, |ready| {
            let ready: Vec<String> = ready.iter().map(|path| text(path)).collect();
            let ready: Vec<&str> = ready.iter().map(String::as_str).collect();
            self.inner.trash(&ready, cx).into_iter().map(|t| t.result.map_err(String::from)).collect()
        });
        paths
            .iter()
            .zip(results)
            .map(|(path, result)| Trashed { path: path.to_string(), result: result.map_err(Error::Other) })
            .collect()
    }
    fn restore(&self, trashed: &str, cx: &Cx) -> Result<String> {
        self.inner.restore(trashed, cx)
    }
    /// This computer's transfer session, with the histories kept around it
    /// (`Session`).
    fn transfer(&self, cx: &Cx, work: &mut dyn FnMut(&dyn Transfer)) {
        let versions = self.versions();
        self.inner.transfer(cx, &mut |inner| work(&Session::new(inner, versions, cx)));
    }
    fn extensions(&self) -> Extensions<'_> {
        self.inner.extensions()
    }
}

/// This computer's transfer session as Keep Versions keeps it: a transfer
/// inside this computer runs inside `VersionsState::transfer` (`within`),
/// and Replace's Trash of any other route goes through `VersionedTrash`.
struct Session<'a> {
    inner: &'a dyn Transfer,
    versions: &'a VersionsState,
    /// Replace's Trash outside `within`: the histories inside the old item
    /// closed before it goes, reopened if it comes back.
    versioned: VersionedTrash<'a, TrashOf<'a>>,
    /// Inside `within`, whose transfer closed the histories already.
    inside: Cell<bool>,
}

impl<'a> Session<'a> {
    fn new(inner: &'a dyn Transfer, versions: &'a VersionsState, cx: &'a Cx) -> Self {
        let versioned = VersionedTrash::new(versions, TrashOf::new(inner, cx));
        Session { inner, versions, versioned, inside: Cell::new(false) }
    }
}

impl Transfer for Session<'_> {
    fn check(&self, from: Option<&str>, to: Option<&str>, cx: &Cx) -> Result<()> {
        self.inner.check(from, to, cx)
    }
    fn landing(&self, from: Option<&str>, to: &str, cx: &Cx) -> Result<Landing> {
        self.inner.landing(from, to, cx)
    }
    fn pending(&self, parent: &str, name: &str) -> Result<String> {
        self.inner.pending(parent, name)
    }
    fn read(&self, path: &str, place: &mut dyn FnMut(&dyn Reading) -> Result<()>, cx: &Cx) -> Result<()> {
        self.inner.read(path, place, cx)
    }
    fn write(&self, source: &Path, target: &str, replace: bool, cx: &Cx) -> Result<Put> {
        self.inner.write(source, target, replace, cx)
    }
    fn read_into(&self, path: &str, target: &Path, cx: &Cx) -> Result<Put> {
        self.inner.read_into(path, target, cx)
    }
    fn export(&self, path: &str, cx: &Cx) -> Result<PathBuf> {
        self.inner.export(path, cx)
    }
    fn copy_within(&self, from: &str, to: &str, cx: &Cx) -> Result<Put> {
        self.inner.copy_within(from, to, cx)
    }
    fn rename_new(&self, from: &str, to: &str, cx: &Cx) -> Result<String> {
        self.inner.rename_new(from, to, cx)
    }
    fn move_to(&self, from: &str, to: &str, cx: &Cx) -> Result<String> {
        self.inner.move_to(from, to, cx)
    }
    fn trash_for_replace(&self, path: &str, cx: &Cx) -> Result<String> {
        if self.inside.get() {
            return self.inner.trash_for_replace(path, cx);
        }
        let trashed = self.versioned.send(Path::new(path)).map_err(Error::Other)?;
        Ok(trashed.as_deref().map(text).unwrap_or_default())
    }
    fn take_back(&self, trashed: &str, original: &str, cx: &Cx) -> Result<()> {
        if self.inside.get() {
            return self.inner.take_back(trashed, original, cx);
        }
        self.versioned.take_back(Path::new(trashed), Path::new(original)).map_err(Error::Other)
    }
    fn discard(&self, staged: &str, cx: &Cx) -> Result<()> {
        self.inner.discard(staged, cx)
    }
    /// The transfer inside `VersionsState::transfer`: the histories in the
    /// source and the target closed for it and reopened after, a copied
    /// one with a new identity, a moved one where it went.
    fn within(&self, from: &str, to: &str, moving: bool, run: &mut dyn FnMut() -> Result<String>) -> Result<String> {
        let mode = if moving { Mode::Move } else { Mode::Copy };
        let mut done = None;
        self.inside.set(true);
        let result = self.versions.transfer(Path::new(from), Path::new(to), mode, || {
            done = Some(self.inner.within(from, to, moving, &mut *run)?);
            Ok(())
        });
        self.inside.set(false);
        result.map_err(Error::Other)?;
        Ok(done.unwrap_or_else(|| to.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use places_local::transfer::LocalTransfer;

    use crate::providers::testing::NoTrash;
    use crate::versions::history::History;
    use crate::versions::registry::read;

    /// Keep Versions as the decorator reaches it, shared with the test.
    struct Shared(Arc<VersionsState>);

    impl Reach<VersionsState> for Shared {
        fn reach(&self) -> &VersionsState {
            &self.0
        }
    }

    struct NoThumbnails;

    impl places::ext::Thumbnails for NoThumbnails {
        fn thumbnail(&self, _: &str, _: u32, _: &dyn Fn() -> bool) -> Vec<u8> {
            Vec::new()
        }
    }

    /// A temp dir with a history at `<dir>/Projects`, open and registered,
    /// and the decorator over this computer's provider (no Trash).
    fn setup() -> (tempfile::TempDir, PathBuf, Arc<VersionsState>, Versioned) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("Projects");
        std::fs::create_dir(&root).unwrap();
        drop(History::create(&root).unwrap());
        let root = std::fs::canonicalize(&root).unwrap();
        let state = Arc::new(VersionsState::new(dir.path().join("versioned.json")));
        state.open_registered(&root).unwrap();
        let local = Local::new(Box::new(NoThumbnails), Box::new(NoTrash));
        let provider = Versioned::new(local, Box::new(Shared(state.clone())));
        (dir, root, state, provider)
    }

    /// A rename moves the histories inside the item with it.
    #[test]
    fn a_rename_takes_the_histories_inside_with_it() {
        let (dir, root, state, provider) = setup();
        let renamed = provider.rename(&text(&root), "Renamed", &Cx::default()).unwrap();
        let moved = std::fs::canonicalize(dir.path().join("Renamed")).unwrap();
        assert_eq!(renamed, text(&root.parent().unwrap().join("Renamed")));
        assert!(state.is_open(&moved) && !state.is_open(&root));
        assert_eq!(read(state.registry_path()), vec![moved]);
        let taken = provider.rename(&renamed, "Renamed", &Cx::default());
        assert!(taken.is_err());
    }

    /// A refused rename reopens the history where it was; a name with a
    /// slash is refused before anything is closed.
    #[test]
    fn a_refused_rename_keeps_the_history_where_it_was() {
        let (dir, root, state, provider) = setup();
        std::fs::create_dir(dir.path().join("Taken")).unwrap();
        let taken = provider.rename(&text(&root), "Taken", &Cx::default());
        let words = format!("Target already exists: {}", text(&root.parent().unwrap().join("Taken")));
        assert_eq!(taken, Err(Error::Other(words)));
        assert!(state.is_open(&root) && root.is_dir());
        assert_eq!(read(state.registry_path()), vec![root.clone()]);
        let elsewhere = provider.rename(&text(&root), &text(&dir.path().join("Taken/Projects")), &Cx::default());
        assert_eq!(elsewhere, Err(Error::Other(places_local::provider::RENAME_STAYS.into())));
        assert!(state.is_open(&root) && root.is_dir());
    }

    /// A move on this computer runs inside the decorator's `within` as a
    /// move: the history goes with the folder, keeping its identity, and
    /// is registered where it went only.
    #[test]
    fn a_move_on_this_computer_takes_the_history_with_it() {
        let (dir, root, state, provider) = setup();
        let folder_id = state.open(&root).unwrap().folder_id.clone();
        let into = dir.path().join("Into");
        std::fs::create_dir(&into).unwrap();
        let dest = into.join("Projects");
        let cx = Cx::default();
        let moved = places::with_transfer(&provider, &cx, |t| {
            crate::engine::local(t, &cx, &text(&root), &text(&dest), false, true)
        });
        assert_eq!(moved, Some(Ok(text(&dest))));
        let dest = std::fs::canonicalize(&dest).unwrap();
        assert!(state.is_open(&dest) && !state.is_open(&root) && !root.exists());
        assert_eq!(read(state.registry_path()), vec![dest.clone()]);
        assert_eq!(state.open(&dest).unwrap().folder_id, folder_id, "a move, not a copy");
    }

    /// The Trash at a temp home with its log, as `lib.rs` hands it over.
    struct HomeTrash(places_local::trash_ops::Trash, PathBuf);

    impl places_local::TrashReach for HomeTrash {
        fn trash(&self) -> std::result::Result<&places_local::trash_ops::Trash, String> {
            Ok(&self.0)
        }
        fn home(&self) -> std::result::Result<PathBuf, String> {
            Ok(self.1.clone())
        }
    }

    /// Move to Trash closes and unregisters the history inside the item
    /// before it goes (`VersionsState::trash_many`); Put Back returns it.
    /// The real Trash of the home folder, as `places-local`'s own Trash
    /// tests use; macOS without Full Disk Access may not say where it went.
    #[test]
    fn a_trashed_folder_takes_its_history_out_of_the_registry() {
        let (dir, root, state, _) = setup();
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| dir.path().to_path_buf());
        let trash = places_local::trash_ops::Trash::new(home.clone(), Some(dir.path().join("trash_log.json")));
        let local = Local::new(Box::new(NoThumbnails), Box::new(HomeTrash(trash, home)));
        let provider = Versioned::new(local, Box::new(Shared(state.clone())));
        let cx = Cx::default();
        let trashed = provider.trash(&[&text(&root)], &cx);
        assert!(trashed[0].result.is_ok(), "{:?}", trashed[0].result);
        assert!(!root.exists() && !state.is_open(&root));
        assert!(read(state.registry_path()).is_empty());
        match trashed[0].result.clone().unwrap() {
            Some(at) => assert_eq!(provider.restore(&at, &cx), Ok(text(&root))),
            None if cfg!(target_os = "macos") => {}
            None => panic!("only macOS may not say where it went"),
        }
    }

    /// A copy on this computer runs inside the decorator's `within`: the
    /// source records again where it was, the copy is registered with a
    /// new identity.
    #[test]
    fn a_copy_on_this_computer_keeps_both_histories() {
        let (dir, root, state, provider) = setup();
        let dest = dir.path().join("Copy");
        let cx = Cx::default();
        let copied = places::with_transfer(&provider, &cx, |t| {
            crate::engine::local(t, &cx, &text(&root), &text(&dest), false, false)
        });
        assert_eq!(copied, Some(Ok(text(&dest))));
        let dest = std::fs::canonicalize(&dest).unwrap();
        assert!(state.is_open(&root) && state.is_open(&dest));
        assert_ne!(state.open(&root).unwrap().folder_id, state.open(&dest).unwrap().folder_id);
        // A refused transfer reopens what it closed.
        let refused = places::with_transfer(&provider, &cx, |t| {
            crate::engine::local(t, &cx, &text(&root), &text(&dest), false, true)
        });
        assert_eq!(refused, Some(Err(format!("Target already exists: {}", dest.display()))));
        assert!(state.is_open(&root) && state.is_open(&dest));
    }

    /// Move to Trash with no home folder trashes nothing and closes nothing.
    #[test]
    fn a_trash_that_cannot_start_closes_nothing() {
        let (_dir, root, state, provider) = setup();
        let trashed = provider.trash(&[&text(&root)], &Cx::default());
        assert_eq!(trashed[0].result, Err(Error::Other("No home folder in tests.".into())));
        assert!(state.is_open(&root));
        assert!(root.is_dir());
    }

    /// Outside a transfer on this computer (a download's Replace), the old
    /// item's histories are closed before it goes and reopened when a
    /// failed Replace takes it back (`VersionedTrash`); inside one, the
    /// transfer closed them already and the plain Trash is used.
    #[test]
    fn replace_outside_a_local_transfer_closes_and_reopens_histories() {
        let (dir, root, state, _) = setup();
        let bin = dir.path().join("bin");
        std::fs::create_dir(&bin).unwrap();
        let seen = std::cell::RefCell::new(Vec::new());
        let into_bin = |old: &Path| {
            seen.borrow_mut().push(state.is_open(old));
            std::fs::rename(old, bin.join("Projects")).map_err(|e| e.to_string())?;
            Ok(Some(bin.join("Projects")))
        };
        let (inner, cx) = (LocalTransfer::new(&into_bin), Cx::default());
        let session = Session::new(&inner, &state, &cx);
        let trashed = session.trash_for_replace(&text(&root), &cx).unwrap();
        assert_eq!(trashed, text(&bin.join("Projects")));
        assert!(!state.is_open(&root));
        session.take_back(&trashed, &text(&root), &cx).unwrap();
        assert!(state.is_open(&root) && root.is_dir());
        assert!(read(state.registry_path()).contains(&root));

        let other = dir.path().join("Other");
        std::fs::create_dir(&other).unwrap();
        let mut run = || {
            let trashed = session.trash_for_replace(&text(&root), &cx)?;
            session.take_back(&trashed, &text(&root), &cx)?;
            Ok(String::from("ran"))
        };
        assert_eq!(session.within(&text(&other), &text(&dir.path().join("Other 2")), false, &mut run), Ok("ran".into()));
        assert_eq!(*seen.borrow(), [false, true], "closed first only outside a local transfer");
    }
}
