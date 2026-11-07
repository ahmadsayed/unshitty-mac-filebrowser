//! How a copy, a move or a download reaches its final local path without
//! losing anything: the refusals checked before anything is written, when the
//! new item is staged under a hidden name, Replace's order (stage, send the old
//! item to the Trash, rename into place) and what a failure undoes.
//!
//! Plain functions, no Tauri. Callers only say how to fill a path
//! (`Target::place`'s `fill`): a rename, a recursive copy, a download.

use std::fmt;
use std::path::{Path, PathBuf};

// ---- Refusals ----------------------------------------------------------------

/// Why a copy, move or download was refused before anything was written.
/// `Display` is the message the user sees.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// The source is gone; it holds the source as shown (a path, or a
    /// Google Drive name).
    Vanished(String),
    SamePath,
    IntoItself,
    /// Something is at the target and Replace wasn't asked for; it holds the
    /// target as shown.
    Exists(String),
    /// Replace would send the source's own folder to the Trash.
    ReplaceWithChild,
    /// The target's last component is empty, `.` or `..`, so Replace would
    /// act on its folder instead.
    NotAName(PathBuf),
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Refusal::Vanished(shown) => write!(f, "“{shown}” no longer exists"),
            Refusal::SamePath => f.write_str("The source and destination are the same"),
            Refusal::IntoItself => f.write_str("Cannot copy or move a folder into itself"),
            Refusal::Exists(shown) => write!(f, "Target already exists: {shown}"),
            Refusal::ReplaceWithChild => {
                f.write_str("Cannot replace a folder with an item inside it")
            }
            Refusal::NotAName(to) => {
                write!(f, "“{}” is not a valid name for a local item.", to.display())
            }
        }
    }
}

impl From<Refusal> for String {
    fn from(refusal: Refusal) -> String {
        refusal.to_string()
    }
}

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

// ---- Placing -----------------------------------------------------------------

/// Sends an item to the Trash, or discards one. Placing takes it as a
/// parameter so tests can make it fail or record it.
pub(crate) type TrashFn<'a> = &'a dyn Fn(&Path) -> Result<(), String>;

/// The system Trash, as Replace uses it.
pub(crate) fn to_trash(path: &Path) -> Result<(), String> {
    trash::delete(path).map_err(|e| e.to_string())
}

/// Removes a file, a link or a whole folder for good. Only for what the app
/// made itself (a partial or staged copy) and for a moved source whose copy
/// is in place.
pub(crate) fn remove_any(path: &Path) -> std::io::Result<()> {
    let meta = std::fs::symlink_metadata(path)?;
    if meta.is_dir() {
        std::fs::remove_dir_all(path)
    } else {
        std::fs::remove_file(path)
    }
}

/// `remove_any` as a discard: a partial copy filled in place is ours, so it
/// goes for good rather than to the Trash.
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
    /// filled in place goes to `discard`.
    WhenReplacing { discard: TrashFn<'a> },
    /// Always under a hidden staging name, so the item appears only whole.
    Always,
}

impl Target {
    /// Fill the target through `fill`, which writes at the path it is given
    /// and says whose item is there now. A staged item is renamed into place;
    /// when it replaces one, the old item goes to `trash` first.
    ///
    /// On failure nothing of the user's is lost: a partial or staged copy is
    /// removed (or discarded, when filled in place), an original is renamed
    /// back, and if that is impossible it stays at the staging name and the
    /// error says where.
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
    fn swap_in(&self, staged: &Path, kind: Staged, trash: TrashFn) -> Result<(), String> {
        if self.replacing {
            return replace_with_staged(staged, &self.to, kind, trash);
        }
        std::fs::rename(staged, &self.to)
            .or_else(|e| undo(staged, kind, format!("Failed to copy: {e}")))
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

/// A free hidden name next to `to`, used to stage an item before it takes
/// `to`'s place.
fn staging_path(to: &Path) -> PathBuf {
    let dir = to.parent().unwrap_or(Path::new("/"));
    let name = to
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let mut n = 0u32;
    loop {
        let candidate = dir.join(format!(
            ".{}.filebrowser-{}-{}",
            name,
            std::process::id(),
            n
        ));
        if std::fs::symlink_metadata(&candidate).is_err() {
            return candidate;
        }
        n += 1;
    }
}

/// Swap the staged item into place, sending whatever was at `to` to the Trash.
/// If either step fails, a staged copy is removed and a staged original goes
/// back where it came from.
fn replace_with_staged(staged: &Path, to: &Path, kind: Staged, trash: TrashFn) -> Result<(), String> {
    let failed = match trash(to) {
        Err(e) => format!("Could not move the existing item to the Trash: {}", e),
        Ok(()) => match std::fs::rename(staged, to) {
            Ok(()) => return Ok(()),
            Err(e) => format!("Failed to replace: {}", e),
        },
    };
    undo(staged, kind, failed)
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
        Staged::Original(from) => {
            let back = if std::fs::symlink_metadata(from).is_ok() {
                Err("another item is there now".to_string())
            } else {
                std::fs::rename(staged, from).map_err(|e| e.to_string())
            };
            back.map_err(|e| {
                format!(
                    "The item was kept at “{}” and could not be put back: {}",
                    staged.display(),
                    e
                )
            })
        }
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

    fn failing_trash(_: &Path) -> Result<(), String> {
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
    fn staging_names_are_hidden_free_and_next_to_the_target() {
        let dir = tempdir().unwrap();
        let to = dir.path().join("a.txt");
        let pid = std::process::id();
        let first = staging_path(&to);
        assert_eq!(first, dir.path().join(format!(".a.txt.filebrowser-{pid}-0")));
        fs::write(&first, "taken").unwrap();
        assert_eq!(staging_path(&to), dir.path().join(format!(".a.txt.filebrowser-{pid}-1")));
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

    #[test]
    fn a_staged_item_that_cannot_take_its_place_is_removed() {
        let dir = tempdir().unwrap();
        let to = dir.path().join("a.txt");
        let target = check_new(&to, false).unwrap();
        // Something else takes the name meanwhile: a folder with content.
        let fill = |dest: &Path| {
            fs::create_dir(&to).unwrap();
            fs::write(to.join("x"), "other").unwrap();
            write(dest, "new")
        };
        let err = target.place(Stage::Always, fill, &failing_trash).unwrap_err();
        assert!(err.starts_with("Failed to copy: "), "{err}");
        assert_eq!(names_in(dir.path()), ["a.txt"]);
        assert_eq!(fs::read_to_string(to.join("x")).unwrap(), "other");
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
            fs::rename(p, bin.join("a.txt")).map_err(|e| e.to_string())
        };
        let target = check_new(&to, true).unwrap();
        let stage = Stage::WhenReplacing { discard: &no_discard };
        assert_eq!(target.place(stage, |d: &Path| write(d, "new"), &trash), Ok(Staged::Copy));
        let staged = format!(".a.txt.filebrowser-{}-0", std::process::id());
        assert_eq!(*seen.borrow(), [vec![staged, "a.txt".into(), "bin".into()]]);
        assert_eq!(fs::read_to_string(&to).unwrap(), "new");
        assert_eq!(fs::read_to_string(bin.join("a.txt")).unwrap(), "old");
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
}
