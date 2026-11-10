//! This computer's `places::Transfer` (design D6, migration step 6b): the
//! primitives the app's transfer engine (`engine.rs`) builds a copy or a
//! move on this computer from, and its local Trash for Replace.
//!
//! - the move's rename that never replaces (`move_to`, also `rename_new`):
//!   `rename::rename_new`, an item on another volume refused as
//!   `Unsupported` so the engine copies it instead;
//! - reading an item into a target the engine picks (`read`): the file or
//!   the tree itself, a file copied whole at the path it is given and a
//!   folder made there and filled in place, a failure saying whether it
//!   left anything (`Unread`), worded as a copy's or a move's (`within`);
//! - Replace's Trash (`trash_for_replace`, `take_back`): `Trash::send` and
//!   `Trash::untrash` through the app's logged Trash ([`ReplaceTrash`]);
//! - discarding a partial or staged copy the engine made (`discard`):
//!   removed for good ([`remove_any`]).
//!
//! What the engine decides itself stays out: the refusals, staging names
//! and Replace's order are `placement.rs`' in the app. The rest of the
//! primitives (`check`, `landing`, `pending`, `write`, `read_into`,
//! `export`, `copy_within`) are other places': this one answers
//! `Unsupported`.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};

use places::{Cx, Error, Landing, Progress, Put, Reading, Result, Transfer, Unread};

use crate::fs_ops::{copy_children, copy_error, copy_recursive, fill_error, is_cross_device, is_folder, make_folder};
use crate::rename;

// ---- Replace's Trash --------------------------------------------------------------

/// Where a Replace sends the item it replaces, and how it takes it back out
/// when the Replace fails a moment later. The app passes its logged Trash
/// (`trash_ops::Trash`, as [`crate::provider::LoggedTrash`]): Restore knows
/// where the old item came from, and an item taken back is forgotten. A
/// closure that sends an item away is one too, taking an item back with
/// `trash_ops::untrash`; tests use it to make the Trash fail or record it.
pub trait ReplaceTrash {
    /// Send `path` to the Trash and say where it went there, when that can
    /// be told, so a Replace that fails afterwards can put it back.
    fn send(&self, path: &Path) -> std::result::Result<Option<PathBuf>, String>;

    /// Put `trashed`, just sent to the Trash from `origin`, straight back,
    /// never over an item that took its place.
    fn take_back(&self, trashed: &Path, origin: &Path) -> std::result::Result<(), String> {
        crate::trash_ops::untrash(trashed, origin)
    }
}

impl<F: Fn(&Path) -> std::result::Result<Option<PathBuf>, String>> ReplaceTrash for F {
    fn send(&self, path: &Path) -> std::result::Result<Option<PathBuf>, String> {
        self(path)
    }
}

/// A place's Replace-trash primitives (`trash_for_replace`, `take_back`) as
/// a [`ReplaceTrash`], for placing on this computer (`placement`): what the
/// engine hands a Replace, a partial folder and a moved source of a route
/// that lands here.
pub struct TrashOf<'a> {
    transfer: &'a dyn Transfer,
    cx: &'a Cx,
}

impl<'a> TrashOf<'a> {
    pub fn new(transfer: &'a dyn Transfer, cx: &'a Cx) -> Self {
        TrashOf { transfer, cx }
    }
}

impl ReplaceTrash for TrashOf<'_> {
    fn send(&self, path: &Path) -> std::result::Result<Option<PathBuf>, String> {
        let trashed = self.transfer.trash_for_replace(&text(path), self.cx)?;
        Ok((!trashed.is_empty()).then(|| PathBuf::from(trashed)))
    }

    fn take_back(&self, trashed: &Path, origin: &Path) -> std::result::Result<(), String> {
        Ok(self.transfer.take_back(&text(trashed), &text(origin), self.cx)?)
    }
}

fn text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// Removes a file, a link or a whole folder for good. Only for what the app
/// made itself (a partial or staged copy) and for a moved source whose copy
/// is in place.
pub fn remove_any(path: &Path) -> io::Result<()> {
    let meta = std::fs::symlink_metadata(path)?;
    if meta.is_dir() {
        std::fs::remove_dir_all(path)
    } else {
        std::fs::remove_file(path)
    }
}

// ---- The transfer session ----------------------------------------------------------

/// Renames an item. The move takes it as a parameter so tests can make it
/// fail as it does across volumes.
pub type RenameFn<'a> = &'a dyn Fn(&Path, &Path) -> io::Result<()>;

/// How a move renames its item: never over one that took the name.
pub const MOVE_RENAME: RenameFn<'static> = &rename::rename_new;

/// One transfer on this computer: its Replace Trash, the move's rename, the
/// wording of its failures (a copy's or a move's, set by `within`) and
/// where each item it trashed went, as the Trash said it.
pub struct LocalTransfer<'a> {
    trash: &'a dyn ReplaceTrash,
    rename: RenameFn<'a>,
    verb: Cell<&'static str>,
    /// The exact place of each item `trash_for_replace` sent, by the text
    /// it answered, so `take_back` finds a name that isn't UTF-8 too.
    trashed: RefCell<HashMap<String, PathBuf>>,
}

impl<'a> LocalTransfer<'a> {
    /// A transfer whose Replace sends the old item to `trash`.
    pub fn new(trash: &'a dyn ReplaceTrash) -> Self {
        Self::with_rename(trash, MOVE_RENAME)
    }

    /// `new`, renaming a moved item with `rename`.
    pub fn with_rename(trash: &'a dyn ReplaceTrash, rename: RenameFn<'a>) -> Self {
        LocalTransfer { trash, rename, verb: Cell::new("copy"), trashed: RefCell::default() }
    }
}

/// What this computer's transfers leave to other places' primitives.
fn not_here<T>(what: &str) -> Result<T> {
    Err(Error::Unsupported(format!("{what} isn't a primitive of this computer's transfers.")))
}

/// The item a local read fills a target with: the file or the tree at
/// `from`.
struct Item<'a> {
    from: &'a Path,
    folder: bool,
    verb: &'static str,
}

impl Reading for Item<'_> {
    fn is_dir(&self) -> bool {
        self.folder
    }

    fn fill(&self, dest: &Path, cx: &Cx) -> std::result::Result<Vec<String>, Unread> {
        copy_fill(self.from, dest, self.folder, &cx.progress, self.verb).map(|()| Vec::new())
    }
}

/// Fill `dest` with a copy of `from`, for a copy or a move across volumes.
/// A folder fills in place (`Stage::by_kind`), so it is made first: until
/// it is, a failure is no partial copy of ours, and whatever took the name
/// after the check is left alone. Anything else goes to a staging name.
fn copy_fill(from: &Path, dest: &Path, folder: bool, progress: &Progress, verb: &str) -> std::result::Result<(), Unread> {
    let partial = |e| Unread::Partial(Error::Other(copy_error(verb, e)));
    if !folder {
        return copy_recursive(from, dest, progress).map_err(partial);
    }
    let meta = make_folder(from, dest, progress).map_err(|e| Unread::Nothing(Error::Other(fill_error(verb, dest, e))))?;
    copy_children(from, dest, &meta, progress).map_err(partial)?;
    progress.add_item();
    Ok(())
}

impl Transfer for LocalTransfer<'_> {
    fn check(&self, _: Option<&str>, _: Option<&str>, _: &Cx) -> Result<()> {
        not_here("Checking a transfer")
    }
    fn landing(&self, _: Option<&str>, _: &str, _: &Cx) -> Result<Landing> {
        not_here("Looking at a target")
    }
    fn pending(&self, _: &str, _: &str) -> Result<String> {
        not_here("A pending name")
    }
    /// The file or tree at `path`, handed to `place` as it is.
    fn read(&self, path: &str, place: &mut dyn FnMut(&dyn Reading) -> Result<()>, _: &Cx) -> Result<()> {
        let from = Path::new(path);
        place(&Item { from, folder: is_folder(from), verb: self.verb.get() })
    }
    fn write(&self, _: &Path, _: &str, _: bool, _: &Cx) -> Result<Put> {
        not_here("Writing a copy in")
    }
    fn read_into(&self, _: &str, _: &Path, _: &Cx) -> Result<Put> {
        not_here("Reading into a target")
    }
    fn export(&self, _: &str, _: &Cx) -> Result<PathBuf> {
        not_here("Exporting")
    }
    fn copy_within(&self, _: &str, _: &str, _: &Cx) -> Result<Put> {
        not_here("Copying without the engine")
    }
    fn rename_new(&self, from: &str, to: &str, cx: &Cx) -> Result<String> {
        self.move_to(from, to, cx)
    }
    /// One rename that never replaces; an item on another volume can't be
    /// renamed there, and is `Unsupported`, for the engine to copy.
    fn move_to(&self, from: &str, to: &str, _: &Cx) -> Result<String> {
        match (self.rename)(Path::new(from), Path::new(to)) {
            Ok(()) => Ok(to.to_string()),
            Err(e) if is_cross_device(&e) => Err(Error::Unsupported(format!("“{to}” is on another volume than “{from}”."))),
            Err(e) => Err(Error::Other(fill_error("move", Path::new(to), e))),
        }
    }
    /// `Trash::send`, through the app's logged Trash: where the item went,
    /// or "" when the Trash can't tell.
    fn trash_for_replace(&self, path: &str, _: &Cx) -> Result<String> {
        let Some(trashed) = self.trash.send(Path::new(path)).map_err(Error::Other)? else {
            return Ok(String::new());
        };
        let key = text(&trashed);
        self.trashed.borrow_mut().insert(key.clone(), trashed);
        Ok(key)
    }
    /// `Trash::untrash`: back to `original`, no folder made, never over an
    /// item that took its place.
    fn take_back(&self, trashed: &str, original: &str, _: &Cx) -> Result<()> {
        let exact = self.trashed.borrow_mut().remove(trashed).unwrap_or_else(|| PathBuf::from(trashed));
        self.trash.take_back(&exact, Path::new(original)).map_err(Error::Other)
    }
    /// Removed for good: only ever a copy the engine made.
    fn discard(&self, staged: &str, _: &Cx) -> Result<()> {
        remove_any(Path::new(staged)).map_err(|e| Error::Other(e.to_string()))
    }
    /// `run` with its failures worded as a move's or a copy's.
    fn within(&self, _: &str, _: &str, moving: bool, run: &mut dyn FnMut() -> Result<String>) -> Result<String> {
        let before = self.verb.replace(if moving { "move" } else { "copy" });
        let result = run();
        self.verb.set(before);
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn text_of(path: &Path) -> String {
        path.to_string_lossy().into_owned()
    }

    fn never(_: &Path) -> std::result::Result<Option<PathBuf>, String> {
        panic!("nothing goes to the Trash here")
    }

    fn across_volumes(_: &Path, _: &Path) -> io::Result<()> {
        Err(io::Error::from_raw_os_error(18)) // EXDEV
    }

    /// The primitives other places have and this computer's transfers
    /// leave to the engine say so.
    #[test]
    fn what_the_engine_does_itself_is_unsupported_here() {
        let t = LocalTransfer::new(&never);
        let cx = Cx::default();
        let unsupported = |error: Error| matches!(error, Error::Unsupported(words) if words.ends_with("this computer's transfers."));
        assert!(unsupported(t.check(Some("/a"), Some("/b"), &cx).unwrap_err()));
        assert!(unsupported(t.landing(None, "/b", &cx).unwrap_err()));
        assert!(unsupported(t.pending("/a", "b").unwrap_err()));
        assert!(unsupported(t.write(Path::new("/a"), "/b", false, &cx).unwrap_err()));
        assert!(unsupported(t.read_into("/a", Path::new("/b"), &cx).unwrap_err()));
        assert!(unsupported(t.export("/a", &cx).unwrap_err()));
        assert!(unsupported(t.copy_within("/a", "/b", &cx).unwrap_err()));
    }

    /// A move renames, never over an item, and leaves an item on another
    /// volume to be copied; a discard removes the engine's copy for good.
    #[test]
    fn a_move_renames_never_over_an_item() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b, c) = (dir.path().join("a"), dir.path().join("b"), dir.path().join("c"));
        fs::write(&a, "a").unwrap();
        fs::write(&b, "b").unwrap();
        let (t, cx) = (LocalTransfer::new(&never), Cx::default());
        let taken = t.move_to(&text_of(&a), &text_of(&b), &cx).unwrap_err();
        assert_eq!(taken, Error::Other(format!("Target already exists: {}", b.display())));
        assert_eq!(t.rename_new(&text_of(&a), &text_of(&c), &cx), Ok(text_of(&c)));
        assert_eq!(fs::read_to_string(&c).unwrap(), "a");
        let elsewhere = LocalTransfer::with_rename(&never, &across_volumes);
        assert!(matches!(elsewhere.move_to(&text_of(&c), &text_of(&a), &cx), Err(Error::Unsupported(_))));
        fs::create_dir_all(dir.path().join("d/e")).unwrap();
        assert_eq!(t.discard(&text_of(&dir.path().join("d")), &cx), Ok(()));
        assert!(!dir.path().join("d").exists());
        assert!(t.discard(&text_of(&dir.path().join("d")), &cx).is_err());
    }

    /// A read hands the item as it is; its fill words a failure as the
    /// transfer's, and says whether it left anything.
    #[test]
    fn a_read_fills_its_target_worded_as_the_transfer() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src");
        fs::create_dir(&src).unwrap();
        fs::write(src.join("x"), "x").unwrap();
        let (t, cx) = (LocalTransfer::new(&never), Cx::default());
        let taken = dir.path().join("taken");
        fs::create_dir(&taken).unwrap();
        let mut seen = Vec::new();
        let mut place = |item: &dyn Reading| {
            seen.push(item.is_dir());
            item.fill(&taken, &cx).map(drop).map_err(|unread| match unread {
                Unread::Nothing(e) => Error::NotFound(e.to_string()),
                Unread::Partial(e) => e,
            })
        };
        let refused = Err(Error::NotFound(format!("Target already exists: {}", taken.display())));
        assert_eq!(t.read(&text_of(&src), &mut place, &cx), refused);
        let mut moved = || t.read(&text_of(&src), &mut place, &cx).map(|()| String::new());
        assert_eq!(t.within("", "", true, &mut moved), refused.map(|()| String::new()));
        assert_eq!(seen, [true, true]);
        let missing = dir.path().join("missing");
        let mut fill_file = |item: &dyn Reading| {
            item.fill(&dir.path().join("nowhere/copy"), &cx).map(drop).map_err(|unread| match unread {
                Unread::Partial(e) => e,
                Unread::Nothing(_) => panic!("a file copy is partial"),
            })
        };
        let mut moved = || t.read(&text_of(&missing), &mut fill_file, &cx).map(|()| String::new());
        let failed = t.within("", "", true, &mut moved).unwrap_err().to_string();
        assert!(failed.starts_with("Failed to move: "), "{failed}");
        let copied = t.read(&text_of(&missing), &mut fill_file, &cx).unwrap_err().to_string();
        assert!(copied.starts_with("Failed to copy: "), "{copied}");
    }

    /// Replace's Trash goes through the given Trash, and an item comes back
    /// from exactly where the Trash put it, or stays unknown.
    #[test]
    fn replace_trashes_and_takes_back_through_the_given_trash() {
        let dir = tempfile::tempdir().unwrap();
        let (old, bin) = (dir.path().join("old.txt"), dir.path().join("bin"));
        fs::write(&old, "old").unwrap();
        fs::create_dir(&bin).unwrap();
        let into_bin = |path: &Path| {
            fs::rename(path, bin.join("old.txt")).map_err(|e| e.to_string())?;
            Ok(Some(bin.join("old.txt")))
        };
        let (t, cx) = (LocalTransfer::new(&into_bin), Cx::default());
        let trashed = t.trash_for_replace(&text_of(&old), &cx).unwrap();
        assert_eq!(trashed, text_of(&bin.join("old.txt")));
        assert_eq!(t.take_back(&trashed, &text_of(&old), &cx), Ok(()));
        assert_eq!(fs::read_to_string(&old).unwrap(), "old");
        let unknown = |_: &Path| Ok(None);
        assert_eq!(LocalTransfer::new(&unknown).trash_for_replace(&text_of(&old), &cx), Ok(String::new()));
        let refusing = |_: &Path| Err("no Trash on this volume".to_string());
        let refused = LocalTransfer::new(&refusing).trash_for_replace(&text_of(&old), &cx);
        assert_eq!(refused, Err(Error::Other("no Trash on this volume".into())));
        // As a `ReplaceTrash` for placing: the same steps, in paths.
        let as_trash = TrashOf::new(&t, &cx);
        assert_eq!(as_trash.send(&old), Ok(Some(bin.join("old.txt"))));
        assert_eq!(as_trash.take_back(&bin.join("old.txt"), &old), Ok(()));
        let unknown = LocalTransfer::new(&unknown);
        assert_eq!(TrashOf::new(&unknown, &cx).send(&old), Ok(None));
    }
}
