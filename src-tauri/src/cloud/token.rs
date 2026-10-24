//! Refresh tokens. The OS keychain is the primary store (Keychain on macOS,
//! Secret Service on Linux); a `0600` JSON file is the fallback for systems
//! where no keyring is available. A token never goes into `accounts.json`, a
//! log line or a `Debug` output.

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Token {
    pub access_token: String,
    pub refresh_token: Option<String>,
    /// Unix seconds. A minute of slack is subtracted when this is built.
    pub expires_at: i64,
    pub scopes: Vec<String>,
}

impl Token {
    pub fn new(
        access_token: String,
        refresh_token: Option<String>,
        expires_at: i64,
        scopes: Vec<String>,
    ) -> Self {
        Self {
            access_token,
            refresh_token,
            expires_at,
            scopes,
        }
    }

    pub fn is_expired(&self, now: i64) -> bool {
        now >= self.expires_at
    }

    pub fn refresh_token(&self) -> Option<&str> {
        self.refresh_token.as_deref()
    }
}

// Never let a token end up in a log or an error by accident.
impl fmt::Debug for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Token")
            .field("access_token", &"<redacted>")
            .field(
                "refresh_token",
                &self.refresh_token.as_ref().map(|_| "<redacted>"),
            )
            .field("expires_at", &self.expires_at)
            .field("scopes", &self.scopes)
            .finish()
    }
}

pub trait TokenStore: Send + Sync {
    fn load(&self, account: &str) -> Result<Option<Token>, String>;
    fn save(&self, account: &str, token: &Token) -> Result<(), String>;
    fn delete(&self, account: &str) -> Result<(), String>;
    /// "keyring" or "file"; the UI warns when a token had to use the file.
    fn storage(&self) -> &'static str;
}

/// A `0600` JSON file per account. Used directly when no keyring exists and as
/// the fallback behind [`KeyringTokenStore`].
pub struct FileTokenStore {
    dir: PathBuf,
}

impl FileTokenStore {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    fn path(&self, account: &str) -> Result<PathBuf, String> {
        if account.is_empty()
            || !account
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        {
            return Err("Invalid Google Drive account id".into());
        }
        Ok(self.dir.join(format!("{account}.json")))
    }
}

impl TokenStore for FileTokenStore {
    fn load(&self, account: &str) -> Result<Option<Token>, String> {
        let path = self.path(account)?;
        match fs::read_to_string(&path) {
            Ok(json) => Ok(serde_json::from_str(&json).ok()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(format!("Failed to read the Google Drive token: {error}")),
        }
    }

    fn save(&self, account: &str, token: &Token) -> Result<(), String> {
        let path = self.path(account)?;
        fs::create_dir_all(&self.dir)
            .map_err(|error| format!("Failed to create the token directory: {error}"))?;
        let json = serde_json::to_vec_pretty(token).map_err(|error| error.to_string())?;
        write_private(&path, &json)
    }

    fn delete(&self, account: &str) -> Result<(), String> {
        let path = self.path(account)?;
        match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(format!("Failed to delete the Google Drive token: {error}")),
        }
    }

    fn storage(&self) -> &'static str {
        "file"
    }
}

/// The keychain store, falling back to a private file when Keychain Services
/// or the Secret Service is unavailable (headless/minimal Linux desktops).
pub struct KeyringTokenStore {
    service: String,
    fallback: FileTokenStore,
    used_fallback: AtomicBool,
}

impl KeyringTokenStore {
    pub fn new(service: impl Into<String>, fallback_dir: PathBuf) -> Self {
        Self {
            service: service.into(),
            fallback: FileTokenStore::new(fallback_dir),
            used_fallback: AtomicBool::new(false),
        }
    }

    fn entry(&self, account: &str) -> Result<keyring::Entry, String> {
        keyring::Entry::new(&self.service, &format!("drive:{account}"))
            .map_err(|error| format!("The system keyring is unavailable: {error}"))
    }
}

impl TokenStore for KeyringTokenStore {
    fn load(&self, account: &str) -> Result<Option<Token>, String> {
        if let Ok(entry) = self.entry(account) {
            if let Ok(json) = entry.get_password() {
                if let Ok(token) = serde_json::from_str(&json) {
                    return Ok(Some(token));
                }
            }
        }
        self.fallback.load(account)
    }

    fn save(&self, account: &str, token: &Token) -> Result<(), String> {
        let json = serde_json::to_string(token).map_err(|error| error.to_string())?;
        if let Ok(entry) = self.entry(account) {
            if entry.set_password(&json).is_ok() {
                self.used_fallback.store(false, Ordering::Relaxed);
                let _ = self.fallback.delete(account);
                return Ok(());
            }
        }
        self.used_fallback.store(true, Ordering::Relaxed);
        self.fallback.save(account, token)
    }

    fn delete(&self, account: &str) -> Result<(), String> {
        if let Ok(entry) = self.entry(account) {
            // NoEntry is the normal case for a token that was already removed.
            let _ = entry.delete_credential();
        }
        self.fallback.delete(account)
    }

    fn storage(&self) -> &'static str {
        if self.used_fallback.load(Ordering::Relaxed) {
            "file"
        } else {
            "keyring"
        }
    }
}

fn write_private(path: &Path, bytes: &[u8]) -> Result<(), String> {
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
            .map_err(|error| format!("Failed to write the Google Drive token: {error}"))?;
        file.write_all(bytes)
            .map_err(|error| format!("Failed to write the Google Drive token: {error}"))?;
    }
    #[cfg(not(unix))]
    {
        fs::write(&tmp, bytes)
            .map_err(|error| format!("Failed to write the Google Drive token: {error}"))?;
    }
    fs::rename(&tmp, path)
        .map_err(|error| format!("Failed to save the Google Drive token: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn token() -> Token {
        Token::new(
            "access-secret".into(),
            Some("refresh-secret".into()),
            1_792_830_000,
            vec!["drive.readonly".into()],
        )
    }

    #[test]
    fn a_file_token_round_trips_and_is_not_world_readable() {
        let dir = tempdir().unwrap();
        let store = FileTokenStore::new(dir.path().to_path_buf());
        store.save("abc", &token()).unwrap();
        assert_eq!(store.load("abc").unwrap(), Some(token()));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(dir.path().join("abc.json"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[test]
    fn a_missing_or_corrupt_file_is_no_token() {
        let dir = tempdir().unwrap();
        let store = FileTokenStore::new(dir.path().to_path_buf());
        assert_eq!(store.load("abc").unwrap(), None);
        fs::write(dir.path().join("abc.json"), "{ nope").unwrap();
        assert_eq!(store.load("abc").unwrap(), None);
    }

    #[test]
    fn delete_is_idempotent_and_rejects_unsafe_account_ids() {
        let dir = tempdir().unwrap();
        let store = FileTokenStore::new(dir.path().to_path_buf());
        store.save("abc", &token()).unwrap();
        store.delete("abc").unwrap();
        store.delete("abc").unwrap();
        assert!(store.save("../escape", &token()).is_err());
        assert!(store.load("../escape").is_err());
    }

    #[test]
    fn debug_output_redacts_both_tokens() {
        let text = format!("{:?}", token());
        assert!(!text.contains("access-secret"));
        assert!(!text.contains("refresh-secret"));
        assert!(text.contains("<redacted>"));
    }

    #[test]
    fn expiry_is_checked_without_slack() {
        assert!(token().is_expired(1_792_830_000));
        assert!(token().is_expired(1_792_830_001));
        assert!(!token().is_expired(1_792_829_999));
        assert!(!token().is_expired(1_792_829_998));
    }

    #[test]
    fn the_file_store_reports_itself() {
        let dir = tempdir().unwrap();
        assert_eq!(
            FileTokenStore::new(dir.path().to_path_buf()).storage(),
            "file"
        );
    }
}
