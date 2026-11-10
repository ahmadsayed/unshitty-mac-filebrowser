//! The cache files this crate makes: a build's `.part` that is renamed into
//! place only once whole, and the `.downloaded` mark that says a copy is
//! still exactly what Arbesk sent. These are the app's own files, not the
//! user's, so a rename may replace one; a save still refuses to overwrite a
//! file that is already there.

use std::fs;
use std::io;
use std::path::Path;

/// The mark the app's `downloads.rs` prunes the cache by, shared through the
/// `places` crate so both write the same suffix and fingerprint.
pub(crate) use places::{is_unchanged, mark};

/// Rename a whole cache file into place, refusing a name that is taken: the
/// caller writes to a temporary `.part` first, and never over a file someone
/// put there meanwhile.
pub(crate) fn rename_new(from: &Path, to: &Path) -> io::Result<()> {
    if fs::symlink_metadata(to).is_ok() {
        return Err(io::ErrorKind::AlreadyExists.into());
    }
    fs::rename(from, to)
}
