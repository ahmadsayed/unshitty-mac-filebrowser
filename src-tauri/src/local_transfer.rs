//! Copy and move on this computer, as the transfer engine runs them (D6):
//! the refusals and Replace from `placement.rs`, the files themselves from
//! this computer's primitives (`places_local::fs_ops`: the recursive copy,
//! a new folder that refuses a taken name, the errors as the user reads
//! them). Nothing here ever deletes data outright except the source of a
//! finished cross-volume move. A replaced target goes to the Trash, and the
//! new item is staged under a temporary name first so a failed copy leaves
//! the target alone.
//!
//! Plain functions, no Tauri.

use std::path::Path;

use crate::fs_ops::{copy_children, copy_error, copy_recursive, fill_error, is_cross_device, is_folder, make_folder};
use crate::placement::{self, Stage, Staged, Target, TrashFn, Unfilled, remove, remove_any};
use crate::progress::Progress;

/// Fill `dest` with a copy of `from`, for a copy or a move across volumes.
/// A folder fills in place (`Stage::by_kind`), so it is made first: until
/// it is, a failure is no partial copy of ours, and whatever took the name
/// after the check is left alone. Anything else goes to a staging name.
fn copy_fill<'a>(
    from: &Path,
    dest: &Path,
    folder: bool,
    progress: &Progress,
    verb: &str,
) -> Result<Staged<'a>, Unfilled> {
    let partial = |e| Unfilled::Partial(copy_error(verb, e));
    if !folder {
        return copy_recursive(from, dest, progress).map(|()| Staged::Copy).map_err(partial);
    }
    let meta = make_folder(from, dest, progress).map_err(|e| Unfilled::Nothing(fill_error(verb, dest, e)))?;
    copy_children(from, dest, &meta, progress).map_err(partial)?;
    progress.add_item();
    Ok(Staged::Copy)
}

/// Copy `from` to `to` (recursively for folders). An item it replaces goes
/// to `trash`: the app's logged Trash, so Restore knows where it was.
pub fn copy_entry_with(
    from: String,
    to: String,
    overwrite: bool,
    progress: &Progress,
    trash: TrashFn,
) -> Result<(), String> {
    let (from, to) = (Path::new(&from), Path::new(&to));
    let target = placement::check_transfer(from, to, overwrite)?;
    copy_to(&target, from, progress, trash)
}

/// Copy `from` to a target that passed the refusals.
fn copy_to(target: &Target, from: &Path, progress: &Progress, trash: TrashFn) -> Result<(), String> {
    let folder = is_folder(from);
    let fill = |dest: &Path| copy_fill(from, dest, folder, progress, "copy");
    target.place(Stage::by_kind(folder, &remove), fill, trash)?;
    Ok(())
}

/// Move `from` to `to`: a rename on the same volume, copy-then-delete across
/// volumes. An item it replaces goes to `trash`, as for `copy_entry_with`.
pub fn move_entry_with(
    from: String,
    to: String,
    overwrite: bool,
    progress: &Progress,
    trash: TrashFn,
) -> Result<(), String> {
    move_entry_by(from, to, overwrite, progress, trash, MOVE_RENAME)
}

/// Renames an item. A move takes it as a parameter so tests can make it fail
/// as it does across volumes.
type RenameFn<'a> = &'a dyn Fn(&Path, &Path) -> std::io::Result<()>;

/// How a move renames its item: never over one that took the name.
const MOVE_RENAME: RenameFn<'static> = &placement::rename_new;

fn move_entry_by(
    from: String,
    to: String,
    overwrite: bool,
    progress: &Progress,
    trash: TrashFn,
    rename: RenameFn,
) -> Result<(), String> {
    let (from, to) = (Path::new(&from), Path::new(&to));
    let target = placement::check_transfer(from, to, overwrite)?;
    move_to(&target, from, progress, trash, rename)
}

/// Move `from` to a target that passed the refusals. With nothing to
/// replace, one rename puts the user's item in place (`Target::free_path`),
/// and only a copy across volumes is staged; a Replace stages the item,
/// renamed or copied, before the old one goes to the Trash.
fn move_to(
    target: &Target,
    from: &Path,
    progress: &Progress,
    trash: TrashFn,
    rename: RenameFn,
) -> Result<(), String> {
    let folder = is_folder(from);
    let stage = Stage::by_kind(folder, &remove);
    let placed = match target.free_path() {
        Some(to) => match rename_item(from, to, progress, rename)? {
            Some(renamed) => renamed,
            None => target.place(stage, |dest: &Path| copy_fill(from, dest, folder, progress, "move"), trash)?,
        },
        None => target.place(stage, |dest: &Path| move_or_copy(from, dest, folder, progress, rename), trash)?,
    };
    if placed == Staged::Copy {
        // Across volumes the source goes only now, with its copy in place.
        // No progress is checked from the end of the copy on, so a cancel
        // arriving now can't leave both items or neither.
        remove_any(from).map_err(|e| format!("Moved, but could not remove the original: {}", e))?;
    }
    Ok(())
}

/// Fill `dest` with `from`: renamed (the user's original is there now) or,
/// across volumes, copied (the original stays at `from` until the end).
fn move_or_copy<'a>(
    from: &'a Path,
    dest: &Path,
    folder: bool,
    progress: &Progress,
    rename: RenameFn,
) -> Result<Staged<'a>, Unfilled> {
    match rename_item(from, dest, progress, rename).map_err(Unfilled::Nothing)? {
        Some(renamed) => Ok(renamed),
        None => copy_fill(from, dest, folder, progress, "move"),
    }
}

/// Rename the user's item `from` to `dest`; `None` when the two are on
/// different volumes, so it has to be copied.
fn rename_item<'a>(
    from: &'a Path,
    dest: &Path,
    progress: &Progress,
    rename: RenameFn,
) -> Result<Option<Staged<'a>>, String> {
    match rename(from, dest) {
        Ok(()) => {
            progress.add_item();
            Ok(Some(Staged::Original(from)))
        }
        Err(e) if is_cross_device(&e) => Ok(None),
        Err(e) => Err(fill_error("move", dest, e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fs_ops::path_exists;
    use crate::progress::Progress;
    use std::fs;
    use std::path::PathBuf;
    use tempfile::tempdir;

    // ----- copy / move / trash tests -----

    fn s(p: &Path) -> String {
        p.to_string_lossy().to_string()
    }

    /// `copy_entry_with` for tests that don't look at where a replaced item
    /// goes: it is discarded, never sent to the user's own Trash.
    fn copy_entry(from: String, to: String, overwrite: bool, progress: &Progress) -> Result<(), String> {
        copy_entry_with(from, to, overwrite, progress, &discard_replaced)
    }

    /// `move_entry_with` as `copy_entry` above.
    fn move_entry(from: String, to: String, overwrite: bool, progress: &Progress) -> Result<(), String> {
        move_entry_with(from, to, overwrite, progress, &discard_replaced)
    }

    /// The Trash of `copy_entry` and `move_entry`: removes what they replace
    /// and says it is nowhere.
    fn discard_replaced(old: &Path) -> Result<Option<PathBuf>, String> {
        remove(old).map(|()| None)
    }

    #[test]
    fn copy_entry_copies_file() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("a.txt");
        let dst = dir.path().join("b.txt");
        fs::write(&src, "hello").unwrap();

        copy_entry(s(&src), s(&dst), false, &Progress::default()).unwrap();
        assert_eq!(fs::read_to_string(&dst).unwrap(), "hello");
        assert!(src.exists());
    }

    #[test]
    fn copy_entry_copies_dir_tree() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("tree");
        fs::create_dir_all(src.join("sub/deeper")).unwrap();
        fs::write(src.join("top.txt"), "1").unwrap();
        fs::write(src.join("sub/deeper/leaf.txt"), "2").unwrap();
        let dst = dir.path().join("copy");

        copy_entry(s(&src), s(&dst), false, &Progress::default()).unwrap();
        assert_eq!(fs::read_to_string(dst.join("top.txt")).unwrap(), "1");
        assert_eq!(
            fs::read_to_string(dst.join("sub/deeper/leaf.txt")).unwrap(),
            "2"
        );
    }

    #[cfg(unix)]
    #[test]
    fn copy_entry_recreates_symlinks_instead_of_following() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("tree");
        fs::create_dir(&src).unwrap();
        // A link back to its own parent would recurse forever if followed.
        std::os::unix::fs::symlink(&src, src.join("loop")).unwrap();
        let dst = dir.path().join("copy");

        copy_entry(s(&src), s(&dst), false, &Progress::default()).unwrap();
        let meta = fs::symlink_metadata(dst.join("loop")).unwrap();
        assert!(meta.file_type().is_symlink());
    }

    #[test]
    fn copy_entry_refuses_existing_target_without_overwrite() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("a.txt");
        let dst = dir.path().join("b.txt");
        fs::write(&src, "new").unwrap();
        fs::write(&dst, "old").unwrap();

        let err = copy_entry(s(&src), s(&dst), false, &Progress::default()).unwrap_err();
        assert!(err.contains("already exists"), "unexpected error: {}", err);
        assert_eq!(fs::read_to_string(&dst).unwrap(), "old");
    }

    #[test]
    fn copy_entry_overwrites_when_asked() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("a.txt");
        let dst = dir.path().join("b.txt");
        fs::write(&src, "new").unwrap();
        fs::write(&dst, "old").unwrap();

        copy_entry(s(&src), s(&dst), true, &Progress::default()).unwrap();
        assert_eq!(fs::read_to_string(&dst).unwrap(), "new");
        // No staging leftovers in the folder.
        let names: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names.len(), 2, "unexpected entries: {:?}", names);
    }

    #[test]
    fn copy_and_move_refuse_into_itself() {
        let dir = tempdir().unwrap();
        let a = dir.path().join("a");
        fs::create_dir(&a).unwrap();
        let inside = a.join("a");

        assert!(copy_entry(s(&a), s(&inside), false, &Progress::default()).is_err());
        assert!(move_entry(s(&a), s(&inside), false, &Progress::default()).is_err());
        assert!(!inside.exists());
    }

    #[test]
    fn copy_and_move_refuse_same_path() {
        let dir = tempdir().unwrap();
        let f = dir.path().join("f.txt");
        fs::write(&f, "x").unwrap();

        assert!(copy_entry(s(&f), s(&f), true, &Progress::default()).is_err());
        assert!(move_entry(s(&f), s(&f), true, &Progress::default()).is_err());
        assert_eq!(fs::read_to_string(&f).unwrap(), "x");
    }

    #[test]
    fn replacing_a_folder_with_its_own_child_is_refused() {
        let dir = tempdir().unwrap();
        let parent = dir.path().join("p");
        fs::create_dir_all(parent.join("child")).unwrap();

        assert!(copy_entry(s(&parent.join("child")), s(&parent), true, &Progress::default()).is_err());
        assert!(move_entry(s(&parent.join("child")), s(&parent), true, &Progress::default()).is_err());
        assert!(parent.join("child").exists());
    }

    #[test]
    fn move_entry_moves_file_and_removes_source() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("a.txt");
        fs::create_dir(dir.path().join("dest")).unwrap();
        let dst = dir.path().join("dest/a.txt");
        fs::write(&src, "hello").unwrap();

        move_entry(s(&src), s(&dst), false, &Progress::default()).unwrap();
        assert!(!src.exists());
        assert_eq!(fs::read_to_string(&dst).unwrap(), "hello");
    }

    #[test]
    fn move_entry_refuses_existing_target_without_overwrite() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("a.txt");
        let dst = dir.path().join("b.txt");
        fs::write(&src, "new").unwrap();
        fs::write(&dst, "old").unwrap();

        assert!(move_entry(s(&src), s(&dst), false, &Progress::default()).is_err());
        assert!(src.exists());
        assert_eq!(fs::read_to_string(&dst).unwrap(), "old");
    }

    #[test]
    fn move_entry_overwrites_when_asked() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("a.txt");
        let dst = dir.path().join("b.txt");
        fs::write(&src, "new").unwrap();
        fs::write(&dst, "old").unwrap();

        move_entry(s(&src), s(&dst), true, &Progress::default()).unwrap();
        assert!(!src.exists());
        assert_eq!(fs::read_to_string(&dst).unwrap(), "new");
    }

    fn failing_trash(_: &Path) -> Result<Option<PathBuf>, String> {
        Err("no Trash on this volume".to_string())
    }

    fn names_in(dir: &Path) -> Vec<std::ffi::OsString> {
        let mut names: Vec<_> = fs::read_dir(dir).unwrap().flatten().map(|e| e.file_name()).collect();
        names.sort();
        names
    }

    #[test]
    fn replace_move_keeps_the_original_when_the_trash_fails() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("a.txt");
        let dst = dir.path().join("b.txt");
        fs::write(&src, "mine").unwrap();
        fs::write(&dst, "old").unwrap();

        let err = move_entry_with(s(&src), s(&dst), true, &Progress::default(), &failing_trash)
            .unwrap_err();
        assert!(err.contains("Could not move the existing item to the Trash"), "{err}");
        assert_eq!(fs::read_to_string(&src).unwrap(), "mine");
        assert_eq!(fs::read_to_string(&dst).unwrap(), "old");
        assert_eq!(names_in(dir.path()), ["a.txt", "b.txt"]);
    }

    #[test]
    fn replace_folder_move_keeps_the_original_when_the_trash_fails() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("mine");
        fs::create_dir_all(src.join("sub")).unwrap();
        fs::write(src.join("sub/x"), "data").unwrap();
        let dst = dir.path().join("dest/mine");
        fs::create_dir_all(&dst).unwrap();

        assert!(move_entry_with(s(&src), s(&dst), true, &Progress::default(), &failing_trash).is_err());
        assert_eq!(fs::read_to_string(src.join("sub/x")).unwrap(), "data");
        assert!(dst.is_dir());
        assert_eq!(names_in(&dir.path().join("dest")), ["mine"]);
    }

    #[test]
    fn replace_copy_removes_only_its_copy_when_the_trash_fails() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("a.txt");
        let dst = dir.path().join("b.txt");
        fs::write(&src, "mine").unwrap();
        fs::write(&dst, "old").unwrap();

        assert!(copy_entry_with(s(&src), s(&dst), true, &Progress::default(), &failing_trash).is_err());
        assert_eq!(fs::read_to_string(&src).unwrap(), "mine");
        assert_eq!(fs::read_to_string(&dst).unwrap(), "old");
        assert_eq!(names_in(dir.path()), ["a.txt", "b.txt"]);
    }

    #[test]
    fn a_staged_original_never_overwrites_what_took_its_old_place() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("a.txt");
        let dst = dir.path().join("b.txt");
        fs::write(&src, "mine").unwrap();
        fs::write(&dst, "old").unwrap();
        // Something new appears at the original's old path, then the Trash fails.
        let trash = |_: &Path| {
            fs::write(&src, "newer").unwrap();
            Err("no Trash on this volume".to_string())
        };

        let err = move_entry_with(s(&src), s(&dst), true, &Progress::default(), &trash).unwrap_err();
        assert_eq!(fs::read_to_string(&src).unwrap(), "newer");
        assert_eq!(fs::read_to_string(&dst).unwrap(), "old");
        let kept: Vec<_> = names_in(dir.path())
            .into_iter()
            .filter(|n| n.to_string_lossy().starts_with(".b.txt.filebrowser-"))
            .collect();
        assert_eq!(kept.len(), 1, "the original stays at its staging name");
        let staged = dir.path().join(&kept[0]);
        assert_eq!(fs::read_to_string(&staged).unwrap(), "mine");
        assert!(err.contains(&staged.display().to_string()), "{err}");
    }

    #[test]
    fn replace_move_puts_the_original_back_when_the_last_rename_fails() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("a.txt");
        let dst = dir.path().join("b.txt");
        fs::write(&src, "mine").unwrap();
        fs::write(&dst, "old").unwrap();
        let bin = dir.path().join("bin");
        fs::create_dir(&bin).unwrap();
        // Trashes the target, then something else takes its name: a folder
        // with content, which a file can't be renamed onto.
        let trash = |p: &Path| {
            fs::rename(p, bin.join("b.txt")).unwrap();
            fs::create_dir(p).unwrap();
            fs::write(p.join("x"), "other").unwrap();
            Ok(Some(bin.join("b.txt")))
        };

        let err = move_entry_with(s(&src), s(&dst), true, &Progress::default(), &trash).unwrap_err();
        assert!(err.contains("Failed to replace"), "{err}");
        // The old item can't come back over the folder: the error says where it is.
        assert!(err.contains(&format!("in the Trash at “{}”", bin.join("b.txt").display())), "{err}");
        assert_eq!(fs::read_to_string(&src).unwrap(), "mine");
        assert_eq!(fs::read_to_string(bin.join("b.txt")).unwrap(), "old");
        assert_eq!(names_in(dir.path()), ["a.txt", "b.txt", "bin"]);
    }

    #[test]
    fn replace_trashes_through_the_given_function() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("a.txt");
        let dst = dir.path().join("b.txt");
        fs::write(&src, "mine").unwrap();
        fs::write(&dst, "old").unwrap();
        let bin = dir.path().join("bin");
        fs::create_dir(&bin).unwrap();
        let trash = |p: &Path| {
            fs::rename(p, bin.join("b.txt")).map_err(|e| e.to_string())?;
            Ok(Some(bin.join("b.txt")))
        };

        move_entry_with(s(&src), s(&dst), true, &Progress::default(), &trash).unwrap();
        assert!(!src.exists());
        assert_eq!(fs::read_to_string(&dst).unwrap(), "mine");
        assert_eq!(fs::read_to_string(bin.join("b.txt")).unwrap(), "old");
        assert_eq!(names_in(dir.path()), ["b.txt", "bin"]);
    }

    // ----- what the placement refactor must keep -----

    type Op = fn(String, String, bool, &Progress) -> Result<(), String>;
    type OpWith = fn(String, String, bool, &Progress, TrashFn) -> Result<(), String>;

    /// Each refusal's exact words, and which one wins when several apply.
    #[test]
    fn refusals_keep_their_wording_and_order() {
        let dir = tempdir().unwrap();
        let a = dir.path().join("a");
        fs::create_dir_all(a.join("child")).unwrap();
        fs::write(a.join("child/f.txt"), "x").unwrap();
        let gone = dir.path().join("gone");
        let cases = [
            // A vanished source wins over the same path.
            (gone.clone(), gone.clone(), true, format!("“{}” no longer exists", gone.display())),
            // The same path wins over "into itself" (a path is inside itself).
            (a.clone(), a.clone(), true, "The source and destination are the same".to_string()),
            // "Into itself" wins over an existing target, with or without Replace.
            (a.clone(), a.join("child"), false, "Cannot copy or move a folder into itself".to_string()),
            (a.clone(), a.join("child"), true, "Cannot copy or move a folder into itself".to_string()),
            // Without Replace, an existing target wins over "an item inside it".
            (a.join("child"), a.clone(), false, format!("Target already exists: {}", a.display())),
            (a.join("child"), a.clone(), true, "Cannot replace a folder with an item inside it".to_string()),
        ];
        let ops: [(&str, Op); 2] = [("copy", copy_entry), ("move", move_entry)];
        for (name, op) in ops {
            for (from, to, overwrite, want) in &cases {
                let got = op(s(from), s(to), *overwrite, &Progress::default());
                assert_eq!(got, Err(want.clone()), "{name} {} → {}", from.display(), to.display());
            }
        }
        assert_eq!(fs::read_to_string(a.join("child/f.txt")).unwrap(), "x");
        assert_eq!(names_in(dir.path()), ["a"]);
        assert_eq!(names_in(&a), ["child"]);
    }

    /// Replace stages the new item next to the old one, sends the old one to
    /// the Trash, then renames the new one into place: for files and folders,
    /// copied and moved.
    #[test]
    fn replace_stages_then_trashes_then_renames_into_place() {
        let ops: [(&str, OpWith); 2] = [("copy", copy_entry_with), ("move", move_entry_with)];
        for (name, op) in ops {
            for folder in [false, true] {
                let dir = tempdir().unwrap();
                let dest = dir.path().join("dest");
                let (src, dst, bin) = (dir.path().join("new"), dest.join("old"), dir.path().join("bin"));
                fs::create_dir_all(&dest).unwrap();
                fs::create_dir(&bin).unwrap();
                if folder {
                    fs::create_dir_all(src.join("sub")).unwrap();
                    fs::write(src.join("sub/x"), "new").unwrap();
                    fs::create_dir(&dst).unwrap();
                    fs::write(dst.join("y"), "old").unwrap();
                } else {
                    fs::write(&src, "new").unwrap();
                    fs::write(&dst, "old").unwrap();
                }
                let seen = std::cell::RefCell::new(Vec::new());
                let trash = |p: &Path| {
                    seen.borrow_mut().push((p.to_path_buf(), names_in(&dest)));
                    fs::rename(p, bin.join("old")).map_err(|e| e.to_string())?;
                    Ok(Some(bin.join("old")))
                };
                op(s(&src), s(&dst), true, &Progress::default(), &trash).unwrap();

                let seen = seen.into_inner();
                assert_eq!(seen.len(), 1, "{name}");
                assert_eq!(seen[0].0, dst, "{name}: the old item is what goes to the Trash");
                let staged: Vec<_> = seen[0].1.iter().map(|n| n.to_string_lossy().into_owned()).collect();
                assert_eq!(staged.len(), 2, "{name}: {staged:?}");
                assert_eq!(staged[0], format!(".old.filebrowser-{}-0", std::process::id()), "{name}");
                assert_eq!(staged[1], "old");
                assert_eq!(names_in(&dest), ["old"], "{name}");
                let (new, old) = if folder {
                    (dst.join("sub/x"), bin.join("old/y"))
                } else {
                    (dst.clone(), bin.join("old"))
                };
                assert_eq!(fs::read_to_string(new).unwrap(), "new", "{name}");
                assert_eq!(fs::read_to_string(old).unwrap(), "old", "{name}");
                assert_eq!(src.exists(), name == "copy", "{name}");
            }
        }
    }

    /// New with the placement module: Replace onto `dir/..` or `dir/.` would
    /// send a whole folder to the Trash, so copy and move refuse it, as Drive
    /// downloads already did.
    #[test]
    fn copy_and_move_refuse_a_target_that_is_not_a_name() {
        let (dir, elsewhere) = (tempdir().unwrap(), tempdir().unwrap());
        let src = elsewhere.path().join("a.txt");
        fs::write(&src, "mine").unwrap();
        let sub = dir.path().join("sub");
        fs::create_dir(&sub).unwrap();
        fs::write(sub.join("keep.txt"), "kept").unwrap();
        let never = |p: &Path| -> Result<Option<PathBuf>, String> { panic!("{} went to the Trash", p.display()) };
        let ops: [OpWith; 2] = [copy_entry_with, move_entry_with];
        for op in ops {
            for to in [sub.join(".."), sub.join(".")] {
                let want = format!("“{}” is not a valid name for a local item.", to.display());
                assert_eq!(op(s(&src), s(&to), true, &Progress::default(), &never), Err(want));
            }
        }
        assert_eq!(fs::read_to_string(&src).unwrap(), "mine");
        assert_eq!(fs::read_to_string(sub.join("keep.txt")).unwrap(), "kept");
    }

    /// The error a failed Replace reports when the Trash refuses.
    #[test]
    fn a_refused_trash_is_reported_word_for_word() {
        let dir = tempdir().unwrap();
        let (src, dst) = (dir.path().join("a.txt"), dir.path().join("b.txt"));
        fs::write(&src, "mine").unwrap();
        fs::write(&dst, "old").unwrap();
        let want = Err("Could not move the existing item to the Trash: no Trash on this volume".to_string());
        assert_eq!(copy_entry_with(s(&src), s(&dst), true, &Progress::default(), &failing_trash), want);
        assert_eq!(move_entry_with(s(&src), s(&dst), true, &Progress::default(), &failing_trash), want);
        assert_eq!(names_in(dir.path()), ["a.txt", "b.txt"]);
    }

    fn across_volumes(_: &Path, _: &Path) -> std::io::Result<()> {
        Err(std::io::Error::from_raw_os_error(18)) // EXDEV
    }

    #[test]
    fn a_move_across_volumes_copies_then_removes_the_source() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("a");
        fs::create_dir_all(src.join("sub")).unwrap();
        fs::write(src.join("sub/x"), "data").unwrap();
        let dst = dir.path().join("b");
        let progress = Progress::default();
        move_entry_by(s(&src), s(&dst), false, &progress, &failing_trash, &across_volumes).unwrap();
        assert!(!src.exists());
        assert_eq!(fs::read_to_string(dst.join("sub/x")).unwrap(), "data");
        assert_eq!(progress.snapshot().items, 3);
    }

    /// The source goes only once its copy has replaced the old item, and a
    /// cancel arriving after the copy can't stop the move halfway.
    #[test]
    fn a_replacing_move_across_volumes_removes_the_source_last() {
        let dir = tempdir().unwrap();
        let (src, dst, bin) = (dir.path().join("a.txt"), dir.path().join("b.txt"), dir.path().join("bin"));
        fs::write(&src, "mine").unwrap();
        fs::write(&dst, "old").unwrap();
        fs::create_dir(&bin).unwrap();
        let progress = Progress::default();
        let source_there = std::cell::Cell::new(false);
        let trash = |p: &Path| {
            source_there.set(src.exists());
            progress.cancel();
            fs::rename(p, bin.join("b.txt")).map_err(|e| e.to_string())?;
            Ok(Some(bin.join("b.txt")))
        };
        move_entry_by(s(&src), s(&dst), true, &progress, &trash, &across_volumes).unwrap();
        assert!(source_there.get(), "the source was still there during the Replace");
        assert!(!src.exists());
        assert_eq!(fs::read_to_string(&dst).unwrap(), "mine");
        assert_eq!(fs::read_to_string(bin.join("b.txt")).unwrap(), "old");
        assert_eq!(names_in(dir.path()), ["b.txt", "bin"]);
    }

    #[test]
    fn a_failed_replace_across_volumes_keeps_the_source_and_the_target() {
        let dir = tempdir().unwrap();
        let (src, dst) = (dir.path().join("a.txt"), dir.path().join("b.txt"));
        fs::write(&src, "mine").unwrap();
        fs::write(&dst, "old").unwrap();
        let err = move_entry_by(s(&src), s(&dst), true, &Progress::default(), &failing_trash, &across_volumes)
            .unwrap_err();
        assert_eq!(err, "Could not move the existing item to the Trash: no Trash on this volume");
        assert_eq!(fs::read_to_string(&src).unwrap(), "mine");
        assert_eq!(fs::read_to_string(&dst).unwrap(), "old");
        assert_eq!(names_in(dir.path()), ["a.txt", "b.txt"]);
    }

    #[test]
    fn a_cancelled_copy_across_volumes_leaves_nothing_and_keeps_the_source() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("a");
        fs::create_dir(&src).unwrap();
        fs::write(src.join("x"), "data").unwrap();
        for (to, overwrite) in [(dir.path().join("b"), false), (dir.path().join("c"), true)] {
            if overwrite {
                fs::write(&to, "old").unwrap();
            }
            let progress = Progress::default();
            progress.cancel();
            let got = move_entry_by(s(&src), s(&to), overwrite, &progress, &failing_trash, &across_volumes);
            assert_eq!(got, Err("Cancelled.".to_string()));
            assert_eq!(fs::read_to_string(src.join("x")).unwrap(), "data");
        }
        assert!(!dir.path().join("b").exists());
        assert_eq!(fs::read_to_string(dir.path().join("c")).unwrap(), "old");
        assert_eq!(names_in(dir.path()), ["a", "c"]);
    }

    /// A rename that fails for another reason touches nothing, staged or not.
    #[test]
    fn a_failed_rename_is_reported_and_touches_nothing() {
        let dir = tempdir().unwrap();
        let (src, dst) = (dir.path().join("a.txt"), dir.path().join("b.txt"));
        fs::write(&src, "mine").unwrap();
        let denied = |_: &Path, _: &Path| Err(std::io::Error::from_raw_os_error(13));
        let want = format!("Failed to move: {}", std::io::Error::from_raw_os_error(13));
        for overwrite in [false, true] {
            let got = move_entry_by(s(&src), s(&dst), overwrite, &Progress::default(), &failing_trash, &denied);
            assert_eq!(got, Err(want.clone()));
            fs::write(&dst, "old").unwrap();
        }
        assert_eq!(fs::read_to_string(&src).unwrap(), "mine");
        assert_eq!(names_in(dir.path()), ["a.txt", "b.txt"]);
    }

    #[test]
    fn a_failed_copy_reports_its_cause() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("a.txt");
        fs::write(&src, "mine").unwrap();
        let to = dir.path().join("missing/b.txt");
        let err = copy_entry(s(&src), s(&to), false, &Progress::default()).unwrap_err();
        assert!(err.starts_with("Failed to copy: "), "{err}");
        assert_eq!(names_in(dir.path()), ["a.txt"]);
    }

    #[test]
    fn operations_on_vanished_path_return_err() {
        let dir = tempdir().unwrap();
        let gone = dir.path().join("gone");
        let dst = dir.path().join("dst");

        assert!(copy_entry(s(&gone), s(&dst), false, &Progress::default()).is_err());
        assert!(move_entry(s(&gone), s(&dst), false, &Progress::default()).is_err());
        assert!(!path_exists(s(&gone)));
    }

    #[test]
    fn copy_reports_bytes_and_items() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("a");
        fs::create_dir_all(src.join("sub")).unwrap();
        fs::write(src.join("x"), vec![1u8; 1000]).unwrap();
        fs::write(src.join("y"), vec![2u8; 24]).unwrap();
        fs::write(src.join("sub/z"), b"").unwrap();
        let progress = Progress::default();
        copy_entry(s(&src), s(&dir.path().join("b")), false, &progress).unwrap();
        let update = progress.snapshot();
        assert_eq!(update.bytes, 1024);
        assert_eq!(update.items, 5);
    }

    #[test]
    fn empty_files_and_folders_count_as_items() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("empty");
        fs::create_dir(&src).unwrap();
        let progress = Progress::default();
        copy_entry(s(&src), s(&dir.path().join("copy")), false, &progress).unwrap();
        assert_eq!(progress.snapshot().items, 1);
        assert_eq!(progress.snapshot().bytes, 0);
    }

    #[test]
    fn cancelled_copy_leaves_nothing() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("a.txt");
        fs::write(&src, b"hello").unwrap();
        let dst = dir.path().join("b.txt");
        let progress = Progress::default();
        progress.cancel();
        assert_eq!(
            copy_entry(s(&src), s(&dst), false, &progress),
            Err("Cancelled.".to_string())
        );
        assert!(!dst.exists());
        assert_eq!(fs::read(&src).unwrap(), b"hello");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn cancelled_copy_mid_file_leaves_nothing() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("big.bin");
        fs::write(&src, vec![9u8; 64 * 1024 * 1024]).unwrap();
        let dst = dir.path().join("copy.bin");
        let progress = Progress::default();
        let watcher = {
            let progress = progress.clone();
            std::thread::spawn(move || {
                while progress.snapshot().bytes < 1024 * 1024 {
                    std::thread::yield_now();
                }
                progress.cancel();
            })
        };
        let result = copy_entry(s(&src), s(&dst), false, &progress);
        watcher.join().unwrap();
        assert_eq!(result, Err("Cancelled.".to_string()));
        assert!(!dst.exists());
    }

    #[test]
    fn cancelled_replace_keeps_the_original() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("new.txt");
        fs::write(&src, b"new").unwrap();
        let dst = dir.path().join("target.txt");
        fs::write(&dst, b"old").unwrap();
        let progress = Progress::default();
        progress.cancel();
        assert_eq!(
            copy_entry(s(&src), s(&dst), true, &progress),
            Err("Cancelled.".to_string())
        );
        assert_eq!(fs::read(&dst).unwrap(), b"old");
        let names: Vec<_> = fs::read_dir(dir.path()).unwrap().flatten().map(|e| e.file_name()).collect();
        assert_eq!(names.len(), 2, "no staging file left: {names:?}");
    }

    // ----- what took the name after the check -----

    /// A file, or a folder holding `x`, with `body` in it.
    fn make_item(path: &Path, folder: bool, body: &str) {
        if folder {
            fs::create_dir(path).unwrap();
            fs::write(path.join("x"), body).unwrap();
        } else {
            fs::write(path, body).unwrap();
        }
    }

    fn item_body(path: &Path) -> String {
        let file = if path.is_dir() { path.join("x") } else { path.to_path_buf() };
        fs::read_to_string(file).unwrap()
    }

    /// Every pair of (source is a folder, what took the name is a folder).
    const KINDS: [(bool, bool); 4] = [(false, false), (false, true), (true, false), (true, true)];

    /// The target passes the refusals, then another app takes its name
    /// before the copy or move begins.
    fn checked_then_taken(src: &Path, dst: &Path, theirs_folder: bool) -> Target {
        let target = placement::check_transfer(src, dst, false).unwrap();
        make_item(dst, theirs_folder, "theirs");
        target
    }

    /// A copy never writes into what took its name after the check (a file
    /// was truncated and overwritten) nor cleans it up as its own partial
    /// copy (a folder was removed). It fails as the check would have.
    #[test]
    fn a_copy_leaves_alone_what_took_its_name_after_the_check() {
        for (mine_folder, theirs_folder) in KINDS {
            let dir = tempdir().unwrap();
            let (src, dst) = (dir.path().join("mine"), dir.path().join("dest"));
            make_item(&src, mine_folder, "mine");
            let target = checked_then_taken(&src, &dst, theirs_folder);
            let got = copy_to(&target, &src, &Progress::default(), &failing_trash);
            let case = format!("folder → {mine_folder}, theirs a folder → {theirs_folder}");
            assert_eq!(got, Err(format!("Target already exists: {}", dst.display())), "{case}");
            assert_eq!(item_body(&dst), "theirs", "{case}");
            assert_eq!(item_body(&src), "mine", "{case}");
            assert_eq!(names_in(dir.path()), ["dest", "mine"], "{case}");
        }
    }

    /// The same for a move, renamed on one volume or copied across two: the
    /// source stays where it was.
    #[test]
    fn a_move_leaves_alone_what_took_its_name_after_the_check() {
        let renames: [(&str, RenameFn); 2] = [("rename", MOVE_RENAME), ("across volumes", &across_volumes)];
        for (how, rename) in renames {
            for (mine_folder, theirs_folder) in KINDS {
                let dir = tempdir().unwrap();
                let (src, dst) = (dir.path().join("mine"), dir.path().join("dest"));
                make_item(&src, mine_folder, "mine");
                let target = checked_then_taken(&src, &dst, theirs_folder);
                let got = move_to(&target, &src, &Progress::default(), &failing_trash, rename);
                let case = format!("{how}: folder → {mine_folder}, theirs a folder → {theirs_folder}");
                assert_eq!(got, Err(format!("Target already exists: {}", dst.display())), "{case}");
                assert_eq!(item_body(&dst), "theirs", "{case}");
                assert_eq!(item_body(&src), "mine", "{case}");
                assert_eq!(names_in(dir.path()), ["dest", "mine"], "{case}");
            }
        }
    }

    /// A move that replaces nothing renames straight onto the target, a file
    /// as much as a folder: one rename is atomic and refuses a name taken
    /// after the check, so the user's item never waits under a hidden
    /// staging name. Across volumes it is the only rename of the user's item
    /// tried: the copy that follows is placed like any copy (a file staged,
    /// a folder in place).
    #[test]
    fn a_move_that_replaces_nothing_renames_straight_onto_the_target() {
        let renames: [(&str, RenameFn); 2] = [("rename", MOVE_RENAME), ("across volumes", &across_volumes)];
        for (how, rename) in renames {
            for folder in [false, true] {
                let dir = tempdir().unwrap();
                let (src, dst) = (dir.path().join("mine"), dir.path().join("dest"));
                make_item(&src, folder, "mine");
                let seen = std::cell::RefCell::new(Vec::new());
                let watch = |from: &Path, to: &Path| {
                    seen.borrow_mut().push(to.to_path_buf());
                    rename(from, to)
                };
                let target = placement::check_transfer(&src, &dst, false).unwrap();
                move_to(&target, &src, &Progress::default(), &failing_trash, &watch).unwrap();
                let case = format!("{how}: folder → {folder}");
                assert_eq!(seen.into_inner(), [dst.clone()], "{case}");
                assert_eq!(item_body(&dst), "mine", "{case}");
                assert_eq!(names_in(dir.path()), ["dest"], "{case}");
            }
        }
    }

    fn move_across_volumes(
        from: String,
        to: String,
        overwrite: bool,
        progress: &Progress,
        trash: TrashFn,
    ) -> Result<(), String> {
        move_entry_by(from, to, overwrite, progress, trash, &across_volumes)
    }

    /// A name near the 255-byte limit most filesystems set still copies and
    /// moves, with and without Replace: the staging name is cut to fit
    /// (whole, it was 23 bytes longer, so the copy failed with "File name
    /// too long").
    #[test]
    fn a_name_near_the_length_limit_still_copies_and_moves() {
        let name = "é".repeat(120); // 240 bytes
        let ops: [(&str, OpWith); 3] =
            [("copy", copy_entry_with), ("move", move_entry_with), ("move across volumes", move_across_volumes)];
        for (how, op) in ops {
            for overwrite in [false, true] {
                let dir = tempdir().unwrap();
                let (dest, bin) = (dir.path().join("dest"), dir.path().join("bin"));
                fs::create_dir(&dest).unwrap();
                fs::create_dir(&bin).unwrap();
                let (src, dst) = (dir.path().join(&name), dest.join(&name));
                fs::write(&src, "mine").unwrap();
                if overwrite {
                    fs::write(&dst, "old").unwrap();
                }
                let trash = |p: &Path| {
                    fs::rename(p, bin.join("old")).map_err(|e| e.to_string())?;
                    Ok(Some(bin.join("old")))
                };
                let case = format!("{how}, Replace → {overwrite}");
                assert_eq!(op(s(&src), s(&dst), overwrite, &Progress::default(), &trash), Ok(()), "{case}");
                assert_eq!(fs::read_to_string(&dst).unwrap(), "mine", "{case}");
                assert_eq!(names_in(&dest), [name.as_str()], "{case}");
                assert_eq!(src.exists(), how == "copy", "{case}");
            }
        }
    }
}
