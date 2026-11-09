//! The cache files this crate makes: a build's `.part` that is renamed into
//! place only once whole, and the `.downloaded` mark that says a copy is
//! still exactly what Arbesk sent. These are the app's own files, not the
//! user's, so a rename may replace one; a save still refuses to overwrite a
//! file that is already there.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

// The app's `downloads.rs` prunes the cache by these marks, so the suffix and
// the fingerprint must stay identical there.
const MARK_SUFFIX: &str = ".downloaded";

/// The hidden file beside `copy` that remembers it as downloaded.
fn mark_path(copy: &Path) -> Option<PathBuf> {
    let name = copy.file_name()?.to_string_lossy();
    Some(copy.with_file_name(format!(".{name}{MARK_SUFFIX}")))
}

/// Size and modification time: enough to notice an edit, in place or by an
/// editor that saves a new file over the old one.
fn fingerprint(meta: &fs::Metadata) -> Option<String> {
    let modified = meta.modified().ok()?.duration_since(UNIX_EPOCH).ok()?;
    Some(format!("{} {}", meta.len(), modified.as_nanos()))
}

/// Remember `copy` as it was just downloaded.
pub(crate) fn mark(copy: &Path) {
    let print = fs::metadata(copy).ok().as_ref().and_then(fingerprint);
    if let (Some(mark), Some(print)) = (mark_path(copy), print) {
        let _ = fs::write(mark, print);
    }
}

/// Whether `copy` is still exactly as [`mark`] left it. Copies without a mark
/// count as changed.
pub(crate) fn is_unchanged(copy: &Path) -> bool {
    let Ok(meta) = fs::metadata(copy) else {
        return false;
    };
    let saved = mark_path(copy).and_then(|mark| fs::read_to_string(mark).ok());
    saved.is_some() && saved == fingerprint(&meta)
}

/// Rename a whole cache file into place, refusing a name that is taken: the
/// caller writes to a temporary `.part` first, and never over a file someone
/// put there meanwhile.
pub(crate) fn rename_new(from: &Path, to: &Path) -> io::Result<()> {
    if fs::symlink_metadata(to).is_ok() {
        return Err(io::ErrorKind::AlreadyExists.into());
    }
    fs::rename(from, to)
}
