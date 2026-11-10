//! Renames on this computer that never replace an item: the exclusive rename
//! with its checked fallback (`rename_new`), the user's Rename in place, which
//! also changes only the case on a disk that ignores it (`rename_in_place`),
//! and the hidden staging names a copy or a case change waits under. The
//! app's transfer engine (`placement.rs`) places items with them, and the
//! Trash puts items back with them.

use std::io;
use std::path::{Path, PathBuf};

// ---- Renames that never replace -----------------------------------------------

/// Rename `from` to `to`, failing with `AlreadyExists` when something is at
/// `to`: `std::fs::rename` replaces a file there (and an empty folder) on
/// Unix, so an item that took the name meanwhile would be lost. The rename
/// that refuses by itself is the platform's (`platform::rename_exclusive`);
/// where the filesystem has none (some network ones), `to` is checked just
/// before the rename, which leaves only that moment open.
pub fn rename_new(from: &Path, to: &Path) -> io::Result<()> {
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
pub fn rename_in_place(from: &Path, to: &Path) -> io::Result<()> {
    rename_in_place_by(from, to, crate::platform::rename_exclusive, folded_onto_itself)
}

/// Whether `to` reaches `from`'s own entry only by folding its name: the
/// same entry, and no entry in the folder carries `to`'s exact name.
pub fn folded_onto_itself(from: &Path, to: &Path) -> bool {
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
pub fn same_entry(a: &Path, b: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    match (std::fs::symlink_metadata(a), std::fs::symlink_metadata(b)) {
        (Ok(ma), Ok(mb)) => ma.dev() == mb.dev() && ma.ino() == mb.ino(),
        _ => false,
    }
}

#[cfg(not(unix))]
pub fn same_entry(_a: &Path, _b: &Path) -> bool {
    false
}

/// The longest name, in bytes, that ext4, btrfs, tmpfs and APFS take.
const NAME_MAX: usize = 255;

/// A free hidden name next to `to`, used to stage an item before it takes
/// `to`'s place.
pub fn staging_path(to: &Path) -> PathBuf {
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

/// Rename `item` back to `origin`, unless another item is there now.
pub fn rename_back(item: &Path, origin: &Path) -> Result<(), String> {
    rename_new(item, origin).map_err(|e| match e.kind() {
        io::ErrorKind::AlreadyExists => "another item is there now".to_string(),
        _ => e.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
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
}
