//! How a copy, a move or a download reaches its final local path without
//! losing anything: the refusals checked before anything is written, when the
//! new item is staged under a hidden name, Replace's order (stage, send the old
//! item to the Trash, rename into place), renames that never replace an item
//! that took the name meanwhile, and what a failure undoes.
//!
//! Plain functions, no Tauri. Callers only say how to fill a path
//! (`Target::place`'s `fill`): a rename, a recursive copy, a download.

use std::fmt;
use std::io;
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

// ---- Renames that never replace -----------------------------------------------

/// Rename `from` to `to`, failing with `AlreadyExists` when something is at
/// `to`: `std::fs::rename` replaces a file there (and an empty folder) on
/// Unix, so an item that took the name meanwhile would be lost. The rename
/// that refuses by itself is the platform's (`platform::rename_exclusive`);
/// where the filesystem has none (some network ones), `to` is checked just
/// before the rename, which leaves only that moment open.
pub(crate) fn rename_new(from: &Path, to: &Path) -> io::Result<()> {
    rename_new_by(from, to, crate::platform::rename_exclusive)
}

/// `rename_new` with the exclusive rename given, so tests can see the
/// fallback.
fn rename_new_by(
    from: &Path,
    to: &Path,
    exclusive: fn(&Path, &Path) -> io::Result<()>,
) -> io::Result<()> {
    match exclusive(from, to) {
        Err(e) if crate::platform::cannot_refuse(&e) => {
            if std::fs::symlink_metadata(to).is_ok() {
                return Err(io::ErrorKind::AlreadyExists.into());
            }
            std::fs::rename(from, to)
        }
        done => done,
    }
}

/// Rename an item within its folder (the user's Rename and Batch Rename),
/// never over another item. On a disk that ignores case or Unicode
/// normalization (APFS by default, FAT), `Foo` names the same entry as
/// `foo`, so the exclusive rename refuses a rename between the two; that
/// one goes through a hidden staging name instead, and back if the second
/// step is refused. Only that one: a hard link to the same file is another
/// entry under the exact name, and is refused without staging anything.
pub(crate) fn rename_in_place(from: &Path, to: &Path) -> io::Result<()> {
    rename_in_place_by(from, to, crate::platform::rename_exclusive, folded_onto_itself)
}

/// Whether `to` reaches `from`'s own entry only by folding its name: the
/// same entry, and no entry in the folder carries `to`'s exact name.
pub(crate) fn folded_onto_itself(from: &Path, to: &Path) -> bool {
    let (Some(dir), Some(name)) = (to.parent(), to.file_name()) else {
        return false;
    };
    same_entry(from, to)
        && std::fs::read_dir(dir).is_ok_and(|mut entries| {
            entries.all(|e| e.is_ok_and(|e| e.file_name() != name))
        })
}

/// `rename_in_place` with the exclusive rename and the same-entry test
/// given, so tests can play a disk that ignores case.
fn rename_in_place_by(
    from: &Path,
    to: &Path,
    exclusive: fn(&Path, &Path) -> io::Result<()>,
    same: fn(&Path, &Path) -> bool,
) -> io::Result<()> {
    match rename_new_by(from, to, exclusive) {
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists && same(from, to) => {
            let staged = staging_path(to);
            rename_new_by(from, &staged, exclusive)?;
            rename_new_by(&staged, to, exclusive).map_err(|e| {
                match rename_new_by(&staged, from, exclusive) {
                    Ok(()) => e,
                    Err(back) => io::Error::other(format!(
                        "The item was kept at “{}” and could not be put back: {}",
                        staged.display(),
                        back
                    )),
                }
            })
        }
        done => done,
    }
}

/// Whether `a` and `b` name the same directory entry (device and inode,
/// not following links): a link to `b` is not `b`.
#[cfg(unix)]
pub(crate) fn same_entry(a: &Path, b: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    match (std::fs::symlink_metadata(a), std::fs::symlink_metadata(b)) {
        (Ok(ma), Ok(mb)) => ma.dev() == mb.dev() && ma.ino() == mb.ino(),
        _ => false,
    }
}

#[cfg(not(unix))]
pub(crate) fn same_entry(_a: &Path, _b: &Path) -> bool {
    false
}

// ---- Placing -----------------------------------------------------------------

/// Where a Replace sends the item it replaces, and how it takes it back out
/// when the Replace fails a moment later. The app passes its logged Trash
/// (`trash_ops::Trash`, through `lib.rs`): Restore knows where the old item
/// came from, and an item taken back is forgotten. A closure that sends an
/// item away is one too, taking an item back with `trash_ops::untrash`;
/// tests use it to make the Trash fail or record it.
pub(crate) trait ReplaceTrash {
    /// Send `path` to the Trash and say where it went there, when that can
    /// be told, so a Replace that fails afterwards can put it back.
    fn send(&self, path: &Path) -> Result<Option<PathBuf>, String>;

    /// Put `trashed`, just sent to the Trash from `origin`, straight back,
    /// never over an item that took its place.
    fn take_back(&self, trashed: &Path, origin: &Path) -> Result<(), String> {
        crate::trash_ops::untrash(trashed, origin)
    }
}

impl<F: Fn(&Path) -> Result<Option<PathBuf>, String>> ReplaceTrash for F {
    fn send(&self, path: &Path) -> Result<Option<PathBuf>, String> {
        self(path)
    }
}

/// Replace's Trash, as placing takes it (`ReplaceTrash`).
pub(crate) type TrashFn<'a> = &'a dyn ReplaceTrash;

/// Discards a partial copy filled in place: for good, or to the Trash.
pub(crate) type DiscardFn<'a> = &'a dyn Fn(&Path) -> Result<(), String>;

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

/// The longest name, in bytes, that ext4, btrfs, tmpfs and APFS take.
const NAME_MAX: usize = 255;

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
        let candidate = dir.join(staging_name(&name, n));
        if std::fs::symlink_metadata(&candidate).is_err() {
            return candidate;
        }
        n += 1;
    }
}

/// `.{name}.filebrowser-{pid}-{n}`, with `name` cut at a character where
/// the whole would pass `NAME_MAX`, so a long name still copies.
fn staging_name(name: &str, n: u32) -> String {
    let tag = format!(".filebrowser-{}-{}", std::process::id(), n);
    let mut end = name.len().min(NAME_MAX - 1 - tag.len());
    while !name.is_char_boundary(end) {
        end -= 1;
    }
    format!(".{}{}", &name[..end], tag)
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

/// Rename `item` back to `origin`, unless another item is there now.
pub(crate) fn rename_back(item: &Path, origin: &Path) -> Result<(), String> {
    rename_new(item, origin).map_err(|e| match e.kind() {
        io::ErrorKind::AlreadyExists => "another item is there now".to_string(),
        _ => e.to_string(),
    })
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
    fn staging_names_are_hidden_free_and_next_to_the_target() {
        let dir = tempdir().unwrap();
        let to = dir.path().join("a.txt");
        let pid = std::process::id();
        let first = staging_path(&to);
        assert_eq!(first, dir.path().join(format!(".a.txt.filebrowser-{pid}-0")));
        fs::write(&first, "taken").unwrap();
        assert_eq!(staging_path(&to), dir.path().join(format!(".a.txt.filebrowser-{pid}-1")));
    }

    /// A staging name stays within the 255 bytes most filesystems take: a
    /// long name is cut, at a character, to leave room for the tag.
    #[test]
    fn a_staging_name_fits_however_long_the_name() {
        let dir = tempdir().unwrap();
        let tag = format!(".filebrowser-{}-0", std::process::id());
        let room = 255 - 1 - tag.len();
        for long in ["é".repeat(127), "e".repeat(255)] {
            let staged = staging_path(&dir.path().join(&long));
            let kept = &long[..room - room % long.chars().next().unwrap().len_utf8()];
            assert_eq!(staged, dir.path().join(format!(".{kept}{tag}")));
            // The filesystem takes it.
            fs::write(&staged, "x").unwrap();
        }
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

    /// Where the filesystem has no rename that refuses a taken name, the
    /// name is checked just before renaming.
    #[test]
    fn a_rename_that_cannot_refuse_checks_the_name_first() {
        let dir = tempdir().unwrap();
        let (from, to) = (dir.path().join("a"), dir.path().join("b"));
        fs::write(&from, "mine").unwrap();
        // Which errors mean "no such rename here" is the platform's to say
        // (`platform::cannot_refuse`); `Unsupported` is one everywhere.
        let unsupported: fn(&Path, &Path) -> io::Result<()> =
            |_, _| Err(io::ErrorKind::Unsupported.into());
        fs::write(&to, "theirs").unwrap();
        let err = rename_new_by(&from, &to, unsupported).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read_to_string(&to).unwrap(), "theirs");
        fs::remove_file(&to).unwrap();
        rename_new_by(&from, &to, unsupported).unwrap();
        assert_eq!(fs::read_to_string(&to).unwrap(), "mine");
        // Any other failure is the rename's own.
        let denied: fn(&Path, &Path) -> io::Result<()> =
            |_, _| Err(io::ErrorKind::PermissionDenied.into());
        let err = rename_new_by(&to, &from, denied).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(names_in(dir.path()), ["b"]);
    }

    /// `std::fs::rename` replaces a file at its target on Unix, so the
    /// user's items are placed with `rename_new` / `rename_in_place` (#192).
    /// The modules left rename only what the app itself made, over its own
    /// files: saves written to a temporary name first, caches, and
    /// content-addressed version objects. Test code is not scanned.
    #[test]
    fn only_listed_modules_call_std_fs_rename() {
        const ALLOWED: [&str; 7] = [
            "placement.rs",       // rename_new's fallback, after its check
            "config_store.rs",    // pins.json, starred.json, flags.json saves
            "cloud/token.rs",     // Drive settings save
            "cloud/service.rs",   // Drive's open cache (#204)
            "thumbnails.rs",      // the thumbnail cache
            "versions/mod.rs",    // .versions/meta.json save
            "versions/store.rs",  // an object is its bytes' hash
        ];
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut callers = Vec::new();
        let mut folders = vec![src.clone()];
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
                let name = path.strip_prefix(&src).unwrap().to_string_lossy().to_string();
                if calls && !ALLOWED.contains(&name.as_str()) {
                    callers.push(name);
                }
            }
        }
        callers.sort();
        assert_eq!(callers, Vec::<String>::new(), "place the user's items with placement::rename_new");
    }

    /// The exclusive rename of a disk that ignores case: a name is taken
    /// when any entry in the folder matches it ignoring case.
    fn rename_ignoring_case(from: &Path, to: &Path) -> io::Result<()> {
        let name = to.file_name().unwrap().to_string_lossy().to_lowercase();
        let taken = fs::read_dir(to.parent().unwrap())?
            .flatten()
            .any(|e| e.file_name().to_string_lossy().to_lowercase() == name);
        if taken {
            return Err(io::ErrorKind::AlreadyExists.into());
        }
        fs::rename(from, to)
    }

    /// On that disk, two names that differ only in case are one entry.
    fn same_ignoring_case(a: &Path, b: &Path) -> bool {
        let lower = |p: &Path| p.file_name().unwrap().to_string_lossy().to_lowercase();
        a.parent() == b.parent() && lower(a) == lower(b) && fs::symlink_metadata(a).is_ok()
    }

    #[test]
    fn a_rename_in_place_never_replaces_another_item() {
        let dir = tempdir().unwrap();
        let (from, file, link) = (dir.path().join("a"), dir.path().join("b"), dir.path().join("c"));
        fs::write(&from, "mine").unwrap();
        fs::write(&file, "theirs").unwrap();
        std::os::unix::fs::symlink(dir.path().join("gone"), &link).unwrap();
        for taken in [&file, &link] {
            let err = rename_in_place(&from, taken).unwrap_err();
            assert_eq!(err.kind(), io::ErrorKind::AlreadyExists, "{}", taken.display());
        }
        assert_eq!(fs::read_to_string(&file).unwrap(), "theirs");
        assert!(fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
        assert_eq!(names_in(dir.path()), ["a", "b", "c"]);
        rename_in_place(&from, &dir.path().join("d")).unwrap();
        assert_eq!(names_in(dir.path()), ["b", "c", "d"]);
    }

    thread_local! {
        static RENAMES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    }

    /// The platform's exclusive rename, counting its calls on this thread.
    fn counted_exclusive(from: &Path, to: &Path) -> io::Result<()> {
        RENAMES.with(|n| n.set(n.get() + 1));
        crate::platform::rename_exclusive(from, to)
    }

    /// A hard link is the same file under another entry: renaming one onto
    /// the other is refused at once, the item never leaving its name for a
    /// staging one.
    #[test]
    fn a_rename_in_place_refuses_another_link_to_the_same_file() {
        let dir = tempdir().unwrap();
        let (from, to) = (dir.path().join("a"), dir.path().join("b"));
        fs::write(&from, "mine").unwrap();
        fs::hard_link(&from, &to).unwrap();
        RENAMES.with(|n| n.set(0));
        let err = rename_in_place_by(&from, &to, counted_exclusive, folded_onto_itself).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(RENAMES.with(|n| n.get()), 1, "only the refused rename, no staging");
        assert_eq!(names_in(dir.path()), ["a", "b"]);
        assert_eq!(rename_in_place(&from, &to).unwrap_err().kind(), io::ErrorKind::AlreadyExists);
    }

    /// The same entry by device and inode, never following a link; the
    /// staging step only when no entry has the exact name. (That `true`
    /// case needs a disk that ignores case, which a test can't make here.)
    #[test]
    fn the_same_entry_is_told_apart_from_links_and_exact_names() {
        let dir = tempdir().unwrap();
        let p = |name: &str| dir.path().join(name);
        fs::write(p("a"), "mine").unwrap();
        fs::hard_link(p("a"), p("hard")).unwrap();
        std::os::unix::fs::symlink(p("a"), p("soft")).unwrap();

        assert!(same_entry(&p("a"), &p("a")));
        assert!(same_entry(&p("a"), &p("hard")));
        assert!(!same_entry(&p("soft"), &p("a")));
        assert!(!same_entry(&p("a"), &p("missing")));
        assert!(!same_entry(&p("missing"), &p("a")));

        assert!(!folded_onto_itself(&p("a"), &p("a")), "the exact name is there");
        assert!(!folded_onto_itself(&p("a"), &p("hard")), "the exact name is there");
        assert!(!folded_onto_itself(&p("soft"), &p("a")));
        assert!(!folded_onto_itself(&p("a"), &p("missing")));
    }

    /// `foo` to `Foo` on a disk that ignores case: the exclusive rename
    /// refuses (it finds `foo` itself), so the item goes through a staging
    /// name and arrives under the new case.
    #[test]
    fn a_rename_in_place_changes_only_the_case() {
        let dir = tempdir().unwrap();
        let (from, to) = (dir.path().join("foo.txt"), dir.path().join("Foo.txt"));
        fs::write(&from, "mine").unwrap();
        rename_in_place_by(&from, &to, rename_ignoring_case, same_ignoring_case).unwrap();
        assert_eq!(names_in(dir.path()), ["Foo.txt"]);
        assert_eq!(fs::read_to_string(&to).unwrap(), "mine");
    }

    /// If the name is taken between the two steps, the item goes back to
    /// its old name and the refusal is the one a rename would have met.
    #[test]
    fn a_case_change_refused_halfway_goes_back() {
        let dir = tempdir().unwrap();
        let (from, to) = (dir.path().join("foo.txt"), dir.path().join("Foo.txt"));
        fs::write(&from, "mine").unwrap();
        let refuse_foo: fn(&Path, &Path) -> io::Result<()> = |from, to| {
            if to.file_name().unwrap() == "Foo.txt" {
                return Err(io::ErrorKind::AlreadyExists.into());
            }
            fs::rename(from, to)
        };
        let err = rename_in_place_by(&from, &to, refuse_foo, |_, _| true).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(names_in(dir.path()), ["foo.txt"]);
        assert_eq!(fs::read_to_string(&from).unwrap(), "mine");
    }

    /// If it can't go back either, the error says where the item is.
    #[test]
    fn a_case_change_that_cannot_go_back_says_where_the_item_is() {
        let dir = tempdir().unwrap();
        let (from, to) = (dir.path().join("foo.txt"), dir.path().join("Foo.txt"));
        fs::write(&from, "mine").unwrap();
        let only_staging: fn(&Path, &Path) -> io::Result<()> = |from, to| {
            if !to.file_name().unwrap().to_string_lossy().contains(".filebrowser-") {
                return Err(io::ErrorKind::AlreadyExists.into());
            }
            fs::rename(from, to)
        };
        let err = rename_in_place_by(&from, &to, only_staging, |_, _| true).unwrap_err();
        let staged = format!(".Foo.txt.filebrowser-{}-0", std::process::id());
        assert!(err.to_string().contains(&staged), "{err}");
        assert_eq!(names_in(dir.path()), [staged]);
    }

    /// The real one, on this filesystem: a free name, then a taken file and
    /// an empty folder, which `rename` would both replace.
    #[test]
    fn rename_new_refuses_a_taken_name() {
        let dir = tempdir().unwrap();
        let (from, to) = (dir.path().join("a"), dir.path().join("b"));
        fs::write(&from, "mine").unwrap();
        rename_new(&from, &to).unwrap();
        assert_eq!(fs::read_to_string(&to).unwrap(), "mine");
        let empty = dir.path().join("empty");
        fs::create_dir(&empty).unwrap();
        fs::write(&from, "other").unwrap();
        for taken in [&to, &empty] {
            assert_eq!(rename_new(&from, taken).unwrap_err().kind(), io::ErrorKind::AlreadyExists);
        }
        assert_eq!(fs::read_to_string(&to).unwrap(), "mine");
        assert_eq!(fs::read_to_string(&from).unwrap(), "other");
        assert!(empty.is_dir());
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
}
