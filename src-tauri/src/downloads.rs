//! Copies of remote files (SFTP, Google Drive, shares) downloaded into the
//! app cache so an app can open them. Nothing uploads changes back, so a copy
//! someone edited may hold the only copy of that work: pruning deletes only
//! copies that are still exactly as downloaded and sends the rest to the Trash.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Where each source keeps its downloads, under the app cache folder.
pub const DIRS: [&str; 3] = ["sftp/sftp-open", "drive", "share-open"];

/// How long a download may go untouched before it is pruned.
pub const MAX_AGE: Duration = Duration::from_secs(7 * 86_400);

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
pub fn mark(copy: &Path) {
    let print = fs::metadata(copy).ok().as_ref().and_then(fingerprint);
    if let (Some(mark), Some(print)) = (mark_path(copy), print) {
        let _ = fs::write(mark, print);
    }
}

/// Whether `copy` is still exactly as downloaded. Copies without a mark
/// (downloaded before marks existed) count as changed.
fn unchanged(copy: &Path, meta: &fs::Metadata) -> bool {
    let saved = mark_path(copy).and_then(|mark| fs::read_to_string(mark).ok());
    saved.is_some() && saved == fingerprint(meta)
}

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

    #[test]
    fn a_missing_base_is_ignored() {
        let base = tempdir().unwrap();
        assert!(prune_after(&base.path().join("nope"), 8).is_empty());
    }
}
