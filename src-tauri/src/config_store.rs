//! Small JSON files in the app's config directory: sidebar bookmarks (pins)
//! and UI flags such as "Full Disk Access banner dismissed". Kept free of
//! Tauri types so it can be unit-tested; lib.rs supplies the directory.

use serde::Serialize;
use std::collections::HashMap;
use std::path::Path;

/// Pinned folder paths stored in `path`. A missing or corrupt file means no pins.
pub fn read_pins(path: &Path) -> Vec<String> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Named boolean flags stored in `path`. A missing or corrupt file means none set.
pub fn read_flags(path: &Path) -> HashMap<String, bool> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Write `value` as JSON via a temp file + rename, so a crash mid-write can't
/// leave a half-written file behind.
pub fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("Failed to create config directory: {}", e))?;
    }
    let json = serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json).map_err(|e| format!("Failed to save settings: {}", e))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("Failed to save settings: {}", e))
}

/// Whether the app can read files that macOS protects behind Full Disk Access.
/// The per-user TCC database is only readable with that grant. If the check is
/// inconclusive (file missing, other OS) this says yes, so the user isn't nagged.
pub fn has_full_disk_access(home: &Path) -> bool {
    if !cfg!(target_os = "macos") {
        return true;
    }
    let tcc = home.join("Library/Application Support/com.apple.TCC/TCC.db");
    match std::fs::File::open(&tcc) {
        Ok(_) => true,
        Err(e) => e.kind() != std::io::ErrorKind::PermissionDenied && e.raw_os_error() != Some(1), // EPERM
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn missing_files_mean_empty_settings() {
        let dir = tempdir().unwrap();
        assert!(read_pins(&dir.path().join("pins.json")).is_empty());
        assert!(read_flags(&dir.path().join("flags.json")).is_empty());
    }

    #[test]
    fn corrupt_files_mean_empty_settings() {
        let dir = tempdir().unwrap();
        let pins = dir.path().join("pins.json");
        std::fs::write(&pins, "{ not json").unwrap();
        assert!(read_pins(&pins).is_empty());
    }

    #[test]
    fn pins_round_trip_and_create_the_directory() {
        let dir = tempdir().unwrap();
        let pins = dir.path().join("nested/config/pins.json");
        let paths = vec!["/Users/me/Projects".to_string(), "/Volumes/Backup".to_string()];

        write_json(&pins, &paths).unwrap();
        assert_eq!(read_pins(&pins), paths);
        assert!(!pins.with_extension("json.tmp").exists());
    }

    #[test]
    fn flags_round_trip() {
        let dir = tempdir().unwrap();
        let flags = dir.path().join("flags.json");
        let mut map = read_flags(&flags);
        map.insert("fda-banner-dismissed".to_string(), true);

        write_json(&flags, &map).unwrap();
        assert_eq!(read_flags(&flags).get("fda-banner-dismissed"), Some(&true));
    }

    #[test]
    fn full_disk_access_check_never_panics() {
        let dir = tempdir().unwrap();
        // No TCC database under a fresh home: inconclusive, so treated as granted.
        assert!(has_full_disk_access(dir.path()));
    }
}
