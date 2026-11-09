//! The app's own files: the account list (`arbesk_accounts.json`) and the
//! session files' `0600` save. Small on purpose; paths and errors are the
//! app's, not Arbesk's.

use std::fs;
use std::path::Path;

use serde::Serialize;

/// Write `value` as JSON to `path`, through a temporary file so a failed save
/// leaves the old one whole.
pub(crate) fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| format!("Failed to create config directory: {}", e))?;
    }
    let json = serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, json).map_err(|e| format!("Failed to save settings: {}", e))?;
    fs::rename(&tmp, path).map_err(|e| format!("Failed to save settings: {}", e))
}

/// Write a file only the user can read (`0600`), atomically. Used for the
/// session files that stand in for the OS keychain.
pub(crate) fn write_private(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let tmp = path.with_extension("json.tmp");
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp)
            .map_err(|error| format!("Failed to write the session: {error}"))?;
        file.write_all(bytes)
            .map_err(|error| format!("Failed to write the session: {error}"))?;
    }
    #[cfg(not(unix))]
    {
        fs::write(&tmp, bytes)
            .map_err(|error| format!("Failed to write the session: {error}"))?;
    }
    fs::rename(&tmp, path).map_err(|error| format!("Failed to save the session: {error}"))
}
