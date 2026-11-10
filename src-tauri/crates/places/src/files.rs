//! Small helpers on this computer's disk that providers share: the clock in
//! unix seconds, the atomic JSON save of the app's settings and account
//! lists, and the `.downloaded` mark beside a copy in the app cache.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;

// ---- Clock -------------------------------------------------------------------

/// Now, in unix seconds (0 if the clock is before 1970).
pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

// ---- JSON saves --------------------------------------------------------------

/// Write `value` as JSON via a temp file + rename, so a crash mid-write can't
/// leave a half-written file behind.
pub fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| format!("Failed to create config directory: {}", e))?;
    }
    let json = serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, json).map_err(|e| format!("Failed to save settings: {}", e))?;
    fs::rename(&tmp, path).map_err(|e| format!("Failed to save settings: {}", e))
}

// ---- Downloaded copies -------------------------------------------------------

/// The app prunes its cache by these marks (`downloads.rs`): a copy whose
/// mark still matches is deleted after a week, any other goes to the Trash.
pub const MARK_SUFFIX: &str = ".downloaded";

/// The hidden file beside `copy` that remembers it as downloaded.
pub fn mark_path(copy: &Path) -> Option<PathBuf> {
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

/// Whether `copy`, whose metadata is `meta`, is still exactly as [`mark`]
/// left it. Copies without a mark (downloaded before marks existed) count
/// as changed.
pub fn unchanged(copy: &Path, meta: &fs::Metadata) -> bool {
    let saved = mark_path(copy).and_then(|mark| fs::read_to_string(mark).ok());
    saved.is_some() && saved == fingerprint(meta)
}

/// [`unchanged`], reading `copy`'s metadata; a missing copy is changed.
pub fn is_unchanged(copy: &Path) -> bool {
    fs::metadata(copy).is_ok_and(|meta| unchanged(copy, &meta))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn now_is_after_2024() {
        assert!(now() > 1_700_000_000);
    }

    #[test]
    fn write_json_saves_pretty_json_and_makes_its_folder() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config/pins.json");
        write_json(&path, &vec!["/a"]).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "[\n  \"/a\"\n]");
        assert!(!path.with_extension("json.tmp").exists());
    }

    #[test]
    fn write_json_reports_a_folder_it_cannot_make() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("file");
        fs::write(&file, "x").unwrap();
        let error = write_json(&file.join("pins.json"), &1).unwrap_err();
        assert!(error.starts_with("Failed to create config directory: "), "{error}");
    }

    #[test]
    fn a_marked_copy_is_unchanged_until_edited() {
        let dir = tempdir().unwrap();
        let copy = dir.path().join("report.pdf");
        fs::write(&copy, "pdf").unwrap();
        assert!(!is_unchanged(&copy), "no mark yet");
        mark(&copy);
        assert_eq!(mark_path(&copy).unwrap(), dir.path().join(".report.pdf.downloaded"));
        assert!(is_unchanged(&copy));
        fs::write(&copy, "edited").unwrap();
        assert!(!is_unchanged(&copy));
        assert!(!is_unchanged(&dir.path().join("missing")));
    }
}
