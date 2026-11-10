//! How a copy, a move or a download reaches its final local path without
//! losing anything: the refusals checked before anything is written, when the
//! new item is staged under a hidden name, Replace's order (stage, send the old
//! item to the Trash, rename into place), renames that never replace an item
//! that took the name meanwhile, and what a failure undoes.
//!
//! Plain functions, no Tauri. Callers only say how to fill a path
//! (`Target::place`'s `fill`): the engine's local routes (a rename, or
//! this computer's read of the item, `engine::local`) and its downloads
//! (a Drive item's read).

use std::io;
use std::path::{Path, PathBuf};

// ---- Refusals ----------------------------------------------------------------

/// Why a copy, move or download was refused before anything was written,
/// in the user's words; shared with Google Drive through the `places` crate.
pub use places::Refusal;

/// A destination that passed the refusals: where the item goes, and whether
/// it replaces one that is there.
#[derive(Debug)]
pub struct Target {
    to: PathBuf,
    replacing: bool,
}

/// The refusals for copying or moving `from` to `to`, in the order the user
/// meets them.
pub fn check_transfer(from: &Path, to: &Path, overwrite: bool) -> Result<Target, Refusal> {
    use crate::fs_ops::{is_subpath, resolve};
    if std::fs::symlink_metadata(from).is_err() {
        return Err(Refusal::Vanished(from.display().to_string()));
    }
    if resolve(from) == resolve(to) {
        return Err(Refusal::SamePath);
    }
    if is_subpath(to, from) {
        return Err(Refusal::IntoItself);
    }
    let target = existing(to, overwrite)?;
    if target.replacing && is_subpath(from, to) {
        return Err(Refusal::ReplaceWithChild);
    }
    // Last, so every refusal above keeps the message it always had.
    not_a_name(to)?;
    Ok(target)
}

/// The refusals for a new item at `to` that has no local source (a download).
pub fn check_new(to: &Path, overwrite: bool) -> Result<Target, Refusal> {
    not_a_name(to)?;
    existing(to, overwrite)
}

fn existing(to: &Path, overwrite: bool) -> Result<Target, Refusal> {
    let replacing = std::fs::symlink_metadata(to).is_ok();
    if replacing && !overwrite {
        return Err(Refusal::Exists(to.display().to_string()));
    }
    Ok(Target {
        to: to.to_path_buf(),
        replacing,
    })
}

/// `/dest/..` or `/dest/.` would make Replace trash `dest` itself. The raw
/// text is checked: `Path::components` quietly drops a trailing `.`.
fn not_a_name(to: &Path) -> Result<(), Refusal> {
    let raw = to.to_string_lossy();
    let last = raw.rsplit(std::path::MAIN_SEPARATOR).next().unwrap_or("");
    match last {
        "" | "." | ".." => Err(Refusal::NotAName(to.to_path_buf())),
        _ => Ok(()),
    }
}

// ---- Renames that never replace -----------------------------------------------

/// The renames that never replace an item, and the staging names, are this
/// computer's own primitives (`places_local::rename`); placing uses them.
pub(crate) use places_local::rename::{rename_back, rename_new};
use places_local::rename::staging_path;

// ---- Placing -----------------------------------------------------------------

/// Where a Replace sends the item it replaces, and how it takes it back out
/// when the Replace fails a moment later: this computer's
/// (`places_local::transfer`), where its transfer session's Trash and the
/// app's logged Trash implement it. A closure that sends an item away is
/// one too, taking an item back with `trash_ops::untrash`; tests use it to
/// make the Trash fail or record it.
pub(crate) use places_local::transfer::ReplaceTrash;

/// Replace's Trash, as placing takes it (`ReplaceTrash`).
pub(crate) type TrashFn<'a> = &'a dyn ReplaceTrash;

/// Discards a partial copy filled in place: for good, or to the Trash.
pub(crate) type DiscardFn<'a> = &'a dyn Fn(&Path) -> Result<(), String>;

/// Removes a file, a link or a whole folder for good, only what the app
/// made itself: this computer's discard (`places_local::transfer`).
pub(crate) use places_local::transfer::remove_any;

/// `remove_any` as a discard: a partial copy filled in place is ours, so it
/// goes for good rather than to the Trash. The engine's local route
/// discards through this computer's transfer session (the same removal);
/// tests discard with this.
#[cfg(test)]
pub(crate) fn remove(path: &Path) -> Result<(), String> {
    remove_any(path).map_err(|e| e.to_string())
}

/// Whose item a fill left at its path, which decides what a failure may do
/// with it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Staged<'a> {
    /// A copy the app made while the source still exists: it may be removed.
    Copy,
    /// The user's own item, renamed here from this path: it is only ever
    /// renamed back, never removed.
    Original(&'a Path),
}

/// A fill that failed, and what it left at its path.
#[derive(Debug, PartialEq, Eq)]
pub enum Unfilled {
    /// Nothing of ours is there (a rename that failed, a folder that could
    /// not be created): nothing is cleaned up.
    Nothing(String),
    /// A partial copy is there, and is cleaned up.
    Partial(String),
}

/// Where a fill writes.
pub enum Stage<'a> {
    /// At the target itself, unless it replaces an item; a partial copy
    /// filled in place goes to `discard`. Something may take the name after
    /// the check, so a fill in place reports `Unfilled::Partial` only once it
    /// has created the item itself (`create_dir`, never a file that would
    /// truncate what is there).
    WhenReplacing { discard: DiscardFn<'a> },
    /// Always under a hidden staging name, so the item appears only whole.
    Always,
}

impl<'a> Stage<'a> {
    /// A new folder fills in place, so it can be watched filling, as in
    /// GNOME Files; anything else is staged and appears only whole.
    pub fn by_kind(folder: bool, discard: DiscardFn<'a>) -> Self {
        if folder {
            Stage::WhenReplacing { discard }
        } else {
            Stage::Always
        }
    }
}

impl Target {
    /// The target itself when nothing is there to replace. A move on one
    /// volume renames straight onto it rather than through `place`: one
    /// `rename_new` is atomic and refuses a name taken after the check, so
    /// the user's item never waits under a staging name, where a crash
    /// would leave it hidden.
    pub fn free_path(&self) -> Option<&Path> {
        (!self.replacing).then_some(self.to.as_path())
    }

    /// Fill the target through `fill`, which writes at the path it is given
    /// and says whose item is there now. A staged item is renamed into place;
    /// when it replaces one, the old item goes to `trash` first.
    ///
    /// On failure nothing of the user's is lost: a partial or staged copy is
    /// removed (or discarded, when filled in place), an original is renamed
    /// back, and if that is impossible it stays at the staging name and the
    /// error says where. An old item already in the Trash comes back out
    /// unless something took its place; then the error says where it is.
    pub fn place<'a>(
        &self,
        stage: Stage,
        fill: impl FnOnce(&Path) -> Result<Staged<'a>, Unfilled>,
        trash: TrashFn,
    ) -> Result<Staged<'a>, String> {
        let staged = self.replacing || matches!(stage, Stage::Always);
        let dest = if staged {
            staging_path(&self.to)
        } else {
            self.to.clone()
        };
        let kind = fill(&dest).map_err(|unfilled| clear(unfilled, &dest, staged, &stage))?;
        if staged {
            self.swap_in(&dest, kind, trash)?;
        }
        Ok(kind)
    }

    /// Rename the staged item onto the target, through Replace if needed.
    /// An item that took the target's name meanwhile is never replaced: the
    /// placing fails as if it had been there at the start.
    fn swap_in(&self, staged: &Path, kind: Staged, trash: TrashFn) -> Result<(), String> {
        if self.replacing {
            return replace_with_staged(staged, &self.to, kind, trash);
        }
        rename_new(staged, &self.to).or_else(|e| {
            let failed = match e.kind() {
                io::ErrorKind::AlreadyExists => Refusal::Exists(self.to.display().to_string()).into(),
                _ => format!("Failed to copy: {e}"),
            };
            undo(staged, kind, failed)
        })
    }
}

/// The error of a failed fill, once a partial copy is out of the way.
fn clear(unfilled: Unfilled, dest: &Path, staged: bool, stage: &Stage) -> String {
    match unfilled {
        Unfilled::Nothing(error) => error,
        Unfilled::Partial(error) => {
            if staged {
                let _ = remove_any(dest);
            } else if let Stage::WhenReplacing { discard } = stage {
                let _ = discard(dest);
            }
            error
        }
    }
}

/// Swap the staged item into place, sending whatever was at `to` to the Trash.
/// If either step fails, a staged copy is removed and a staged original goes
/// back where it came from; if the old item is in the Trash by then, it comes
/// back out too.
fn replace_with_staged(staged: &Path, to: &Path, kind: Staged, trash: TrashFn) -> Result<(), String> {
    let failed = match trash.send(to) {
        Err(e) => format!("Could not move the existing item to the Trash: {}", e),
        Ok(trashed) => match rename_new(staged, to) {
            Ok(()) => return Ok(()),
            Err(e) => take_back(trash, trashed.as_deref(), to, format!("Failed to replace: {}", e)),
        },
    };
    undo(staged, kind, failed)
}

/// After a Replace failed with `failed` once the old item was in the Trash,
/// put that item back at `to` through `trash`, as Drive untrashes it. If it
/// can't go back (another item took its place, or where the Trash put it is
/// unknown), it stays in the Trash and the error says where.
fn take_back(trash: TrashFn, trashed: Option<&Path>, to: &Path, failed: String) -> String {
    let Some(trashed) = trashed else {
        return format!("{failed}. The replaced item is in the Trash");
    };
    match trash.take_back(trashed, to) {
        Ok(()) => failed,
        Err(e) => format!(
            "{failed}. The replaced item is in the Trash at “{}” and could not be put back: {e}",
            trashed.display()
        ),
    }
}

/// Fail with `failed` after undoing the staging, naming where the user's item
/// was kept if it could not go back.
fn undo(staged: &Path, kind: Staged, failed: String) -> Result<(), String> {
    Err(match give_back(staged, kind) {
        Ok(()) => failed,
        Err(kept) => format!("{failed}. {kept}"),
    })
}

/// Undo staging after a failure. The user's own item is never removed: if it
/// can't be renamed back, it stays at the staging name and the error says
/// where.
fn give_back(staged: &Path, kind: Staged) -> Result<(), String> {
    match kind {
        Staged::Copy => {
            let _ = remove_any(staged);
            Ok(())
        }
        Staged::Original(from) => rename_back(staged, from).map_err(|e| {
            format!(
                "The item was kept at “{}” and could not be put back: {}",
                staged.display(),
                e
            )
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::fs;
    use tempfile::tempdir;

    fn names_in(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    fn failing_trash(_: &Path) -> Result<Option<PathBuf>, String> {
        Err("no Trash on this volume".to_string())
    }

    fn no_discard(_: &Path) -> Result<(), String> {
        panic!("nothing to discard")
    }

    fn write(dest: &Path, body: &str) -> Result<Staged<'static>, Unfilled> {
        fs::write(dest, body).unwrap();
        Ok(Staged::Copy)
    }

    #[test]
    fn refusals_read_as_they_always_have() {
        let cases = [
            (Refusal::Vanished("/a/b".into()), "“/a/b” no longer exists"),
            (Refusal::SamePath, "The source and destination are the same"),
            (Refusal::IntoItself, "Cannot copy or move a folder into itself"),
            (Refusal::Exists("a.txt".into()), "Target already exists: a.txt"),
            (Refusal::ReplaceWithChild, "Cannot replace a folder with an item inside it"),
            (Refusal::NotAName("/a/..".into()), "“/a/..” is not a valid name for a local item."),
        ];
        for (refusal, want) in cases {
            assert_eq!(String::from(refusal), want);
        }
    }

    /// The name check comes after every other refusal, so a target that
    /// was refused before keeps its message.
    #[test]
    fn a_transfer_to_a_dot_or_empty_name_is_refused_last() {
        let (dir, elsewhere) = (tempdir().unwrap(), tempdir().unwrap());
        // Outside `sub/..`, which would be "an item inside it" otherwise.
        let src = elsewhere.path().join("a.txt");
        fs::write(&src, "x").unwrap();
        fs::create_dir(dir.path().join("sub")).unwrap();
        let dotdot = dir.path().join("sub/..");
        let dot = dir.path().join("sub/.");
        let slash = PathBuf::from(format!("{}/new/", dir.path().display()));
        assert_eq!(
            check_transfer(&src, &dotdot, false).unwrap_err(),
            Refusal::Exists(dotdot.display().to_string())
        );
        for to in [&dotdot, &dot, &slash] {
            assert_eq!(
                check_transfer(&src, to, true).unwrap_err(),
                Refusal::NotAName(to.clone()),
                "{}",
                to.display()
            );
        }
        // The name check doesn't hide "into itself" or "same path".
        let sub = dir.path().join("sub");
        assert_eq!(check_transfer(&sub, &dot, true).unwrap_err(), Refusal::SamePath);
        assert_eq!(
            check_transfer(&sub, &sub.join("x/.."), true).unwrap_err(),
            Refusal::IntoItself
        );
    }

    #[test]
    fn a_new_item_checks_its_name_before_what_is_there() {
        let dir = tempdir().unwrap();
        let there = dir.path().join("a.txt");
        fs::write(&there, "x").unwrap();
        let dotdot = dir.path().join("sub/..");
        assert_eq!(check_new(&dotdot, false).unwrap_err(), Refusal::NotAName(dotdot.clone()));
        assert_eq!(
            check_new(&there, false).unwrap_err(),
            Refusal::Exists(there.display().to_string())
        );
        assert!(check_new(&there, true).unwrap().replacing);
        assert!(!check_new(&dir.path().join("b.txt"), false).unwrap().replacing);
    }

    #[test]
    fn a_new_item_fills_in_place_unless_always_staged() {
        let dir = tempdir().unwrap();
        let to = dir.path().join("a.txt");
        let target = check_new(&to, false).unwrap();
        let given = RefCell::new(PathBuf::new());
        let fill = |dest: &Path| {
            given.replace(dest.to_path_buf());
            write(dest, "new")
        };
        let stage = Stage::WhenReplacing { discard: &no_discard };
        assert_eq!(target.place(stage, fill, &failing_trash), Ok(Staged::Copy));
        assert_eq!(*given.borrow(), to);

        let to = dir.path().join("b.txt");
        let target = check_new(&to, false).unwrap();
        let fill = |dest: &Path| {
            given.replace(dest.to_path_buf());
            write(dest, "new")
        };
        assert_eq!(target.place(Stage::Always, fill, &failing_trash), Ok(Staged::Copy));
        assert_eq!(*given.borrow(), dir.path().join(format!(".b.txt.filebrowser-{}-0", std::process::id())));
        assert_eq!(fs::read_to_string(&to).unwrap(), "new");
        assert_eq!(names_in(dir.path()), ["a.txt", "b.txt"]);
    }

    /// Whatever took the name while the item was staged stays (`rename`
    /// would replace a file on Unix): the staged item is removed and the
    /// error is the refusal it would have met at the start.
    #[test]
    fn a_staged_item_never_replaces_what_took_its_name() {
        for folder in [true, false] {
            let dir = tempdir().unwrap();
            let to = dir.path().join("a.txt");
            let target = check_new(&to, false).unwrap();
            let theirs = if folder { to.join("x") } else { to.clone() };
            let fill = |dest: &Path| {
                if folder {
                    fs::create_dir(&to).unwrap();
                }
                fs::write(&theirs, "someone else's").unwrap();
                write(dest, "new")
            };
            let err = target.place(Stage::Always, fill, &failing_trash).unwrap_err();
            assert_eq!(err, format!("Target already exists: {}", to.display()));
            assert_eq!(fs::read_to_string(&theirs).unwrap(), "someone else's");
            assert_eq!(names_in(dir.path()), ["a.txt"]);
        }
    }

    /// `std::fs::rename` replaces a file at its target on Unix, so the
    /// user's items are placed with `rename_new` / `rename_in_place` (#192).
    /// The modules left rename only what the app itself made, over its own
    /// files: saves written to a temporary name first, caches, and
    /// content-addressed version objects. Test code is not scanned. The
    /// app's sources and every crate under `crates/` are.
    #[test]
    fn only_listed_modules_call_std_fs_rename() {
        const ALLOWED: [&str; 10] = [
            "crates/places-local/src/rename.rs", // rename_new's fallback, after its check
            "config_store.rs",    // pins.json, starred.json, flags.json saves
            "crates/places-gdrive/src/token.rs",   // Drive settings save
            "crates/places-gdrive/src/service.rs", // Drive's open cache (#204)
            "thumbnails.rs",      // the thumbnail cache
            "versions/mod.rs",    // .versions/meta.json save
            "versions/store.rs",  // an object is its bytes' hash
            "crates/arbesk/src/cache.rs", // an asset's `.part`, renamed whole
            "crates/arbesk/src/store.rs", // the session's 0600 save
            "crates/places/src/files.rs", // write_json: settings and account lists
        ];
        let roots = places_local::test_support::source_roots();
        assert!(roots.iter().any(|(_, prefix)| prefix == "crates/places/src/"));
        let mut callers = Vec::new();
        for (root, prefix) in roots {
            let mut folders = vec![root.clone()];
            while let Some(folder) = folders.pop() {
                for path in fs::read_dir(&folder).unwrap().flatten().map(|e| e.path()) {
                    if path.is_dir() {
                        folders.push(path);
                        continue;
                    }
                    let text = fs::read_to_string(&path).unwrap_or_default();
                    // The code up to the top-level test module, without comments.
                    let code = text.split("\n#[cfg(test)]\nmod ").next().unwrap_or_default();
                    let calls = code
                        .lines()
                        .filter(|line| !line.trim_start().starts_with("//"))
                        .any(|line| line.contains("fs::rename"));
                    let name =
                        format!("{prefix}{}", path.strip_prefix(&root).unwrap().to_string_lossy());
                    if calls && !ALLOWED.contains(&name.as_str()) {
                        callers.push(name);
                    }
                }
            }
        }
        callers.sort();
        assert_eq!(callers, Vec::<String>::new(), "place the user's items with placement::rename_new");
    }

    #[test]
    fn a_partial_copy_is_removed_when_staged_and_discarded_in_place() {
        let dir = tempdir().unwrap();
        let discarded = RefCell::new(Vec::new());
        let discard = |p: &Path| {
            discarded.borrow_mut().push(p.to_path_buf());
            Ok(())
        };
        let partial = |dest: &Path| -> Result<Staged<'static>, Unfilled> {
            fs::write(dest, "half").unwrap();
            Err(Unfilled::Partial("broke".into()))
        };
        let in_place = dir.path().join("a.txt");
        let target = check_new(&in_place, false).unwrap();
        let got = target.place(Stage::WhenReplacing { discard: &discard }, partial, &failing_trash);
        assert_eq!(got, Err("broke".to_string()));
        assert_eq!(*discarded.borrow(), [in_place.clone()]);

        let staged = dir.path().join("b.txt");
        let target = check_new(&staged, false).unwrap();
        assert_eq!(target.place(Stage::Always, partial, &failing_trash), Err("broke".to_string()));
        assert_eq!(discarded.borrow().len(), 1);
        // Only the in-place one is left, for `discard` to deal with.
        assert_eq!(names_in(dir.path()), ["a.txt"]);
    }

    /// A fill that wrote nothing of ours leaves whatever is at its path.
    #[test]
    fn a_fill_that_wrote_nothing_cleans_nothing_up() {
        let dir = tempdir().unwrap();
        let to = dir.path().join("a.txt");
        let target = check_new(&to, false).unwrap();
        let nothing = |dest: &Path| -> Result<Staged<'static>, Unfilled> {
            fs::write(dest, "someone else's").unwrap();
            Err(Unfilled::Nothing("refused".into()))
        };
        let got = target.place(Stage::WhenReplacing { discard: &no_discard }, nothing, &failing_trash);
        assert_eq!(got, Err("refused".to_string()));
        assert_eq!(fs::read_to_string(&to).unwrap(), "someone else's");
    }

    #[test]
    fn replace_trashes_the_old_item_only_once_the_new_one_is_whole() {
        let dir = tempdir().unwrap();
        let to = dir.path().join("a.txt");
        fs::write(&to, "old").unwrap();
        let bin = dir.path().join("bin");
        fs::create_dir(&bin).unwrap();
        let seen = RefCell::new(Vec::new());
        let trash = |p: &Path| {
            seen.borrow_mut().push(names_in(dir.path()));
            fs::rename(p, bin.join("a.txt")).map_err(|e| e.to_string())?;
            Ok(Some(bin.join("a.txt")))
        };
        let target = check_new(&to, true).unwrap();
        let stage = Stage::WhenReplacing { discard: &no_discard };
        assert_eq!(target.place(stage, |d: &Path| write(d, "new"), &trash), Ok(Staged::Copy));
        let staged = format!(".a.txt.filebrowser-{}-0", std::process::id());
        assert_eq!(*seen.borrow(), [vec![staged, "a.txt".into(), "bin".into()]]);
        assert_eq!(fs::read_to_string(&to).unwrap(), "new");
        assert_eq!(fs::read_to_string(bin.join("a.txt")).unwrap(), "old");
    }

    /// When the last rename of a Replace fails, the old item comes back out
    /// of the Trash, as Drive untrashes it. Here the new item vanished
    /// before it could be renamed in.
    #[test]
    fn a_replace_whose_last_rename_fails_puts_the_old_item_back() {
        let dir = tempdir().unwrap();
        let to = dir.path().join("a.txt");
        fs::write(&to, "old").unwrap();
        let bin = dir.path().join("bin");
        fs::create_dir(&bin).unwrap();
        let staged = RefCell::new(PathBuf::new());
        let fill = |dest: &Path| {
            staged.replace(dest.to_path_buf());
            write(dest, "new")
        };
        let trash = |p: &Path| {
            fs::rename(p, bin.join("a.txt")).unwrap();
            fs::remove_file(&*staged.borrow()).unwrap();
            Ok(Some(bin.join("a.txt")))
        };
        let target = check_new(&to, true).unwrap();
        let err = target.place(Stage::Always, fill, &trash).unwrap_err();
        assert!(err.starts_with("Failed to replace: "), "{err}");
        assert!(!err.contains("Trash"), "{err}");
        assert_eq!(fs::read_to_string(&to).unwrap(), "old");
        assert_eq!(names_in(&bin), Vec::<String>::new());
        assert_eq!(names_in(dir.path()), ["a.txt", "bin"]);
    }

    /// Where the Trash put the old item isn't known: the error says it is
    /// in the Trash.
    #[test]
    fn a_failed_replace_says_the_old_item_is_in_the_trash_when_it_cannot_tell_where() {
        let dir = tempdir().unwrap();
        let to = dir.path().join("a.txt");
        fs::write(&to, "old").unwrap();
        let bin = dir.path().join("bin");
        fs::create_dir(&bin).unwrap();
        let staged = RefCell::new(PathBuf::new());
        let fill = |dest: &Path| {
            staged.replace(dest.to_path_buf());
            write(dest, "new")
        };
        let trash = |p: &Path| {
            fs::rename(p, bin.join("a 2.txt")).unwrap();
            fs::remove_file(&*staged.borrow()).unwrap();
            Ok(None)
        };
        let target = check_new(&to, true).unwrap();
        let err = target.place(Stage::Always, fill, &trash).unwrap_err();
        assert!(err.starts_with("Failed to replace: "), "{err}");
        assert!(err.ends_with(". The replaced item is in the Trash"), "{err}");
        assert_eq!(fs::read_to_string(bin.join("a 2.txt")).unwrap(), "old");
    }

    /// Something that takes the name between the Trash and the last rename
    /// isn't replaced either: the staged copy is removed, the old item stays
    /// in the Trash, and the error says where.
    #[test]
    fn replace_never_renames_over_what_took_the_name_after_the_trash() {
        let dir = tempdir().unwrap();
        let to = dir.path().join("a.txt");
        fs::write(&to, "old").unwrap();
        let bin = dir.path().join("bin");
        fs::create_dir(&bin).unwrap();
        let trash = |p: &Path| {
            fs::rename(p, bin.join("a.txt")).unwrap();
            fs::write(p, "newer").unwrap();
            Ok(Some(bin.join("a.txt")))
        };
        let target = check_new(&to, true).unwrap();
        let err = target.place(Stage::Always, |d: &Path| write(d, "new"), &trash).unwrap_err();
        assert!(err.starts_with("Failed to replace: "), "{err}");
        let kept = format!(
            ". The replaced item is in the Trash at “{}” and could not be put back: another item is there now",
            bin.join("a.txt").display()
        );
        assert!(err.ends_with(&kept), "{err}");
        assert_eq!(fs::read_to_string(&to).unwrap(), "newer");
        assert_eq!(fs::read_to_string(bin.join("a.txt")).unwrap(), "old");
        assert_eq!(names_in(dir.path()), ["a.txt", "bin"]);
    }

    #[test]
    fn a_staged_copy_is_removed_when_the_trash_fails() {
        let dir = tempdir().unwrap();
        let to = dir.path().join("a.txt");
        fs::write(&to, "old").unwrap();
        let target = check_new(&to, true).unwrap();
        let err = target.place(Stage::Always, |d: &Path| write(d, "new"), &failing_trash).unwrap_err();
        assert_eq!(err, "Could not move the existing item to the Trash: no Trash on this volume");
        assert_eq!(names_in(dir.path()), ["a.txt"]);
        assert_eq!(fs::read_to_string(&to).unwrap(), "old");
    }

    #[test]
    fn a_staged_original_is_put_back_where_it_came_from() {
        let dir = tempdir().unwrap();
        let from = dir.path().join("a.txt");
        let to = dir.path().join("b.txt");
        fs::write(&from, "mine").unwrap();
        fs::write(&to, "old").unwrap();
        let target = check_transfer(&from, &to, true).unwrap();
        let fill = |dest: &Path| {
            fs::rename(&from, dest).unwrap();
            Ok(Staged::Original(&from))
        };
        let stage = Stage::WhenReplacing { discard: &no_discard };
        assert!(target.place(stage, fill, &failing_trash).is_err());
        assert_eq!(fs::read_to_string(&from).unwrap(), "mine");
        assert_eq!(names_in(dir.path()), ["a.txt", "b.txt"]);
    }

    #[test]
    fn a_staged_original_that_cannot_go_back_stays_and_is_named() {
        let dir = tempdir().unwrap();
        let to = dir.path().join("b.txt");
        fs::write(&to, "old").unwrap();
        let staged = staging_path(&to);
        fs::write(&staged, "mine").unwrap();
        // Its old folder is gone, so renaming it back fails.
        let from = dir.path().join("gone/a.txt");
        let err = replace_with_staged(&staged, &to, Staged::Original(&from), &failing_trash).unwrap_err();
        assert_eq!(fs::read_to_string(&staged).unwrap(), "mine");
        assert!(err.starts_with("Could not move the existing item to the Trash: no Trash on this volume. "), "{err}");
        assert!(err.contains(&staged.display().to_string()), "{err}");
    }

    /// Replace through the app's logged Trash (`places_local::trash_ops`),
    /// in both layouts, from home and from a stick: moved here from
    /// `trash_ops`' tests when it became its own crate, since placing is the
    /// app's.
    mod logged_trash {
        use places_local::trash_ops::testing::{files, into_its_trash, logged, send_with, text, with_usb};
        use places_local::trash_ops::{Layout, Trash};
        use std::collections::HashSet;
        use std::ffi::OsString;
        use std::fs;
        use std::path::{Path, PathBuf};
        use tempfile::tempdir;

        fn names_in(dir: &Path) -> HashSet<OsString> {
            fs::read_dir(dir)
                .map(|rd| rd.flatten().map(|e| e.file_name()).collect())
                .unwrap_or_default()
        }

        /// The app's Trash as a Replace uses it (`ReplaceTrash`, as `lib.rs`
        /// passes it): `Trash::send`, here with the trashing stood in for, and
        /// `Trash::untrash`.
        struct LoggedReplaceTrash<'a> {
            trash: &'a Trash,
            delete: &'a dyn Fn(&Path) -> Result<(), String>,
        }

        impl crate::placement::ReplaceTrash for LoggedReplaceTrash<'_> {
            fn send(&self, path: &Path) -> Result<Option<PathBuf>, String> {
                send_with(self.trash, path, self.delete)
            }

            fn take_back(&self, trashed: &Path, origin: &Path) -> Result<(), String> {
                self.trash.untrash(trashed, origin)
            }
        }

        /// Replace `old` through the logged Trash with a new file whose last
        /// rename fails: once the old item is in the Trash, the staged copy
        /// vanishes and `meanwhile` runs. Returns the error.
        fn failed_replace(trash: &Trash, old: &Path, meanwhile: impl Fn(&Path)) -> String {
            use crate::placement::{Stage, Staged, check_new};
            let staged = std::cell::RefCell::new(PathBuf::new());
            let fill = |dest: &Path| {
                fs::write(dest, "new").unwrap();
                staged.replace(dest.to_path_buf());
                Ok(Staged::Copy)
            };
            let into_trash = into_its_trash(trash);
            let delete = |path: &Path| {
                into_trash(path)?;
                fs::remove_file(&*staged.borrow()).unwrap();
                meanwhile(path);
                Ok(())
            };
            let replace_trash = LoggedReplaceTrash { trash, delete: &delete };
            let target = check_new(old, true).unwrap();
            target.place(Stage::Always, fill, &replace_trash).unwrap_err()
        }

        /// A Replace whose last rename fails takes the old item back out of the
        /// logged Trash and forgets its entry: the item is back, nothing is left
        /// in the Trash, and the log names nothing that left it. When something
        /// took its place, it stays in the Trash and stays logged, so Restore
        /// still knows where it came from. Both layouts, from home and a stick.
        #[test]
        fn a_failed_replace_takes_the_old_item_back_out_of_the_logged_trash() {
            for layout in [Layout::Finder, Layout::Freedesktop] {
                let dir = tempdir().unwrap();
                let v = with_usb(dir.path(), layout);
                let at_home = files(&v.home.join("Documents"), &["a.txt"]).remove(0);
                let on_usb = files(&v.usb.join("Projects"), &["b.txt"]).remove(0);
                let cases = [(&at_home, &v.items, "a.txt"), (&on_usb, &v.usb_items, "b.txt")];

                for (old, items, name) in cases {
                    let err = failed_replace(&v.trash, old, |_| ());
                    assert!(err.starts_with("Failed to replace: "), "{layout:?}: {err}");
                    assert!(!err.contains("Trash"), "{layout:?}: {err}");
                    assert_eq!(fs::read_to_string(old).unwrap(), name, "{layout:?}");
                    assert!(names_in(items).is_empty(), "{layout:?}: {name} left in the Trash");
                    assert!(logged(&v.trash).items.is_empty(), "{layout:?}: {name} still logged");
                    assert_eq!(names_in(old.parent().unwrap()), HashSet::from([OsString::from(name)]));
                }

                for (old, items, name) in cases {
                    let err = failed_replace(&v.trash, old, |old| fs::write(old, "newer").unwrap());
                    let trashed = items.join(name);
                    let kept = format!(
                        "in the Trash at “{}” and could not be put back: another item is there now",
                        trashed.display()
                    );
                    assert!(err.ends_with(&kept), "{layout:?}: {err}");
                    assert_eq!(fs::read_to_string(old).unwrap(), "newer", "{layout:?}");
                    assert_eq!(fs::read_to_string(&trashed).unwrap(), name, "{layout:?}");
                    assert_eq!(logged(&v.trash).items[&text(&trashed)].origin, text(old), "{layout:?}");
                }
            }
        }
    }
}
