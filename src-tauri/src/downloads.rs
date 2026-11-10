//! Copies of remote files (SFTP, Google Drive, shares) downloaded into the
//! app cache so an app can open them. Nothing uploads changes back, so a copy
//! someone edited may hold the only copy of that work: pruning deletes only
//! copies that are still exactly as downloaded and sends the rest to the Trash.

use std::fs;
use std::path::Path;
use std::time::{Duration, SystemTime};

/// Where each source keeps its downloads, under the app cache folder.
pub const DIRS: [&str; 4] = ["sftp/sftp-open", "drive", "share-open", "arbesk"];

/// How long a download may go untouched before it is pruned.
pub const MAX_AGE: Duration = Duration::from_secs(7 * 86_400);

/// The `.downloaded` mark beside a copy, in the `places` crate: every
/// provider that leaves a copy here (SFTP, Drive, shares, Arbesk builds)
/// writes the same suffix and fingerprint.
pub use places::mark;
use places::{MARK_SUFFIX, mark_path, unchanged};

/// Prune the downloads under `base` that nobody has touched for `max_age`:
/// unchanged copies and unfinished `.part` files are deleted, other copies go
/// to `trash`. Folders left empty are removed; `base` itself stays.
pub fn prune(
    base: &Path,
    now: SystemTime,
    max_age: Duration,
    trash: &dyn Fn(&Path) -> Result<(), String>,
) {
    let Ok(entries) = fs::read_dir(base) else {
        return;
    };
    for path in entries.flatten().map(|entry| entry.path()) {
        let Ok(meta) = fs::symlink_metadata(&path) else {
            continue;
        };
        if meta.is_dir() {
            prune(&path, now, max_age, trash);
            let _ = fs::remove_dir(&path); // only succeeds once empty
        } else if is_stale(&meta, now, max_age) {
            prune_file(&path, &meta, trash);
        }
    }
}

fn is_stale(meta: &fs::Metadata, now: SystemTime, max_age: Duration) -> bool {
    let age = meta
        .modified()
        .ok()
        .and_then(|modified| now.duration_since(modified).ok());
    age.is_some_and(|age| age >= max_age)
}

fn prune_file(path: &Path, meta: &fs::Metadata, trash: &dyn Fn(&Path) -> Result<(), String>) {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    if let Some(copy) = name
        .strip_prefix('.')
        .and_then(|n| n.strip_suffix(MARK_SUFFIX))
    {
        // A mark goes with its copy; one left without a copy goes on its own.
        if !path.with_file_name(copy).exists() {
            let _ = fs::remove_file(path);
        }
        return;
    }
    let partial = name.ends_with(".part") && !mark_path(path).is_some_and(|m| m.exists());
    let gone = if partial || unchanged(path, meta) {
        fs::remove_file(path).is_ok()
    } else {
        trash(path).is_ok()
    };
    if let Some(mark) = mark_path(path).filter(|_| gone) {
        let _ = fs::remove_file(mark);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::path::PathBuf;
    use tempfile::tempdir;

    const WEEK: Duration = MAX_AGE;

    fn write(path: &Path, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    /// Prune as of `days` from now, recording what went to the Trash.
    fn prune_after(base: &Path, days: u64) -> Vec<PathBuf> {
        let trashed = RefCell::new(Vec::new());
        let trash = |path: &Path| {
            trashed.borrow_mut().push(path.to_path_buf());
            fs::remove_file(path).map_err(|e| e.to_string())
        };
        let now = SystemTime::now() + Duration::from_secs(days * 86_400);
        prune(base, now, WEEK, &trash);
        trashed.into_inner()
    }

    #[test]
    fn recent_downloads_are_kept() {
        let base = tempdir().unwrap();
        let copy = base.path().join("1a-0/report.pdf");
        write(&copy, "pdf");
        mark(&copy);
        assert!(prune_after(base.path(), 6).is_empty());
        assert!(copy.exists());
        assert!(mark_path(&copy).unwrap().exists());
    }

    #[test]
    fn an_unchanged_download_is_deleted_with_its_mark_and_folder() {
        let base = tempdir().unwrap();
        let copy = base.path().join("acct/file-id/stamp-photo.png");
        write(&copy, "png");
        mark(&copy);
        assert!(prune_after(base.path(), 8).is_empty(), "nothing trashed");
        assert!(!copy.exists());
        assert!(!mark_path(&copy).unwrap().exists());
        assert!(!base.path().join("acct").exists(), "empty folders removed");
        assert!(base.path().exists(), "the base folder stays");
    }

    #[test]
    fn an_edited_or_unmarked_download_goes_to_the_trash() {
        let base = tempdir().unwrap();
        let edited = base.path().join("1a-0/notes.txt");
        write(&edited, "as downloaded");
        mark(&edited);
        write(&edited, "with my changes");
        let unmarked = base.path().join("1b-0/old.txt");
        write(&unmarked, "from before marks");
        let mut trashed = prune_after(base.path(), 8);
        trashed.sort();
        assert_eq!(trashed, [edited.clone(), unmarked.clone()]);
        assert!(!mark_path(&edited).unwrap().exists());
        assert!(!base.path().join("1a-0").exists());
    }

    #[test]
    fn a_copy_that_cannot_be_trashed_is_kept_with_its_mark() {
        let base = tempdir().unwrap();
        let copy = base.path().join("1a-0/notes.txt");
        write(&copy, "x");
        mark(&copy);
        write(&copy, "edited");
        let refuse = |_: &Path| Err("no Trash here".to_string());
        let now = SystemTime::now() + Duration::from_secs(8 * 86_400);
        prune(base.path(), now, WEEK, &refuse);
        assert!(copy.exists());
        assert!(mark_path(&copy).unwrap().exists());
    }

    #[test]
    fn unfinished_downloads_and_orphan_marks_are_deleted() {
        let base = tempdir().unwrap();
        let part = base.path().join("acct/id/stamp-big.part");
        write(&part, "half");
        let orphan = base.path().join("1c-0/.gone.txt.downloaded");
        write(&orphan, "3 0");
        // A remote file really named *.part is a finished, marked copy.
        let named_part = base.path().join("1d-0/backup.part");
        write(&named_part, "whole");
        mark(&named_part);
        assert!(prune_after(base.path(), 8).is_empty());
        assert!(!part.exists());
        assert!(!orphan.exists());
        assert!(
            !named_part.exists(),
            "unchanged, so deleted rather than trashed"
        );
    }

    /// Whether `copy` is still exactly as [`mark`] left it.
    fn is_unchanged(copy: &Path) -> bool {
        fs::metadata(copy).is_ok_and(|meta| unchanged(copy, &meta))
    }

    #[test]
    fn a_copy_is_unchanged_until_someone_edits_it() {
        let dir = tempdir().unwrap();
        let copy = dir.path().join("Box.gltf");
        write(&copy, "{}");
        assert!(!is_unchanged(&copy), "unmarked");
        mark(&copy);
        assert!(is_unchanged(&copy));
        write(&copy, "{\"edited\":true}");
        assert!(!is_unchanged(&copy));
        assert!(!is_unchanged(&dir.path().join("missing.gltf")));
    }

    #[test]
    fn arbesk_assets_are_pruned_like_other_downloads() {
        assert!(DIRS.contains(&"arbesk"));
    }

    /// The Arbesk manifest and thumbnail caches are regenerable and are never
    /// sent to the Trash, so they stay out of the downloaded-files folders.
    #[test]
    fn arbesk_caches_are_not_downloads() {
        assert!(!DIRS.contains(&"arbesk-manifests"));
        assert!(!DIRS.contains(&"arbesk-thumbnails"));
    }

    #[test]
    fn a_missing_base_is_ignored() {
        let base = tempdir().unwrap();
        assert!(prune_after(&base.path().join("nope"), 8).is_empty());
    }
}
