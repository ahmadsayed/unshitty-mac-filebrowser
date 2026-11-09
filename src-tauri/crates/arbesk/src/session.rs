//! The Arbesk session a confirmed sign-in keeps, so copying into Arbesk can
//! act as the account. It is a secret: it lives in the OS keychain (a
//! `0600` file where none exists), never in `arbesk_accounts.json`, a log
//! line, a `Debug` output or an error message.

use std::fmt;
use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::store::write_private;

/// An Arbesk session: its token and when Arbesk stops accepting it.
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Session {
    pub token: String,
    pub expires_at_ms: i64,
}

impl fmt::Debug for Session {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Session")
            .field("token", &"<redacted>")
            .field("expires_at_ms", &self.expires_at_ms)
            .finish()
    }
}

/// Where sessions are kept, one per account id.
pub trait SessionStore: Send + Sync {
    fn load(&self, account: &str) -> Option<Session>;
    fn save(&self, account: &str, session: &Session) -> Result<(), String>;
    fn delete(&self, account: &str);
}

// ---- The keychain, with a private file behind it -----------------------

const SERVICE: &str = "filebrowser";

/// The keychain (service `filebrowser`, user `arbesk:{id}`), falling back to
/// `{dir}/arbesk-{id}.json` (`0600`) where no keychain is available.
pub struct KeyringSessions {
    dir: PathBuf,
}

impl KeyringSessions {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    fn entry(account: &str) -> Option<keyring::Entry> {
        keyring::Entry::new(SERVICE, &format!("arbesk:{account}")).ok()
    }

    /// The fallback file, for an id made of the characters ids are made of.
    fn file(&self, account: &str) -> Result<PathBuf, String> {
        let plain = !account.is_empty()
            && account
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
        if !plain {
            return Err("Invalid Arbesk account id".into());
        }
        Ok(self.dir.join(format!("arbesk-{account}.json")))
    }
}

impl SessionStore for KeyringSessions {
    fn load(&self, account: &str) -> Option<Session> {
        let kept = Self::entry(account)
            .and_then(|entry| entry.get_password().ok())
            .and_then(|json| serde_json::from_str(&json).ok());
        kept.or_else(|| {
            let json = fs::read_to_string(self.file(account).ok()?).ok()?;
            serde_json::from_str(&json).ok()
        })
    }

    fn save(&self, account: &str, session: &Session) -> Result<(), String> {
        let path = self.file(account)?;
        let json = serde_json::to_string(session).map_err(|error| error.to_string())?;
        let in_keychain = Self::entry(account).is_some_and(|e| e.set_password(&json).is_ok());
        if in_keychain {
            let _ = fs::remove_file(&path);
            return Ok(());
        }
        fs::create_dir_all(&self.dir)
            .map_err(|error| format!("Failed to create the token directory: {error}"))?;
        write_private(&path, json.as_bytes())
    }

    fn delete(&self, account: &str) {
        if let Some(entry) = Self::entry(account) {
            let _ = entry.delete_credential();
        }
        if let Ok(path) = self.file(account) {
            let _ = fs::remove_file(path);
        }
    }
}

// ---- For tests ---------------------------------------------------------

/// Sessions in memory; clones share them, so a test can look inside.
#[cfg(test)]
#[derive(Clone, Default)]
pub struct MemorySessions(
    std::sync::Arc<std::sync::Mutex<std::collections::HashMap<String, Session>>>,
);

#[cfg(test)]
impl SessionStore for MemorySessions {
    fn load(&self, account: &str) -> Option<Session> {
        self.0.lock().unwrap().get(account).cloned()
    }

    fn save(&self, account: &str, session: &Session) -> Result<(), String> {
        self.0
            .lock()
            .unwrap()
            .insert(account.to_string(), session.clone());
        Ok(())
    }

    fn delete(&self, account: &str) {
        self.0.lock().unwrap().remove(account);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session() -> Session {
        Session {
            token: "secret-token".into(),
            expires_at_ms: 1_762_000_000_000,
        }
    }

    #[test]
    fn a_session_never_shows_its_token_in_debug() {
        assert!(!format!("{:?}", session()).contains("secret-token"));
    }

    #[test]
    fn the_fallback_file_is_private_and_goes_when_the_session_does() {
        let dir = tempfile::tempdir().unwrap();
        let store = KeyringSessions::new(dir.path().to_path_buf());
        let file = dir.path().join("arbesk-abc.json");
        fs::write(&file, serde_json::to_string(&session()).unwrap()).unwrap();
        assert_eq!(store.load("abc"), Some(session()));
        store.delete("abc");
        assert!(!file.exists());
        assert_eq!(store.load("abc"), None);
    }

    #[test]
    fn an_unsafe_account_id_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let store = KeyringSessions::new(dir.path().to_path_buf());
        assert!(store.save("../x", &session()).is_err());
        assert_eq!(store.load("../x"), None);
    }

    #[test]
    fn memory_sessions_share_between_clones() {
        let store = MemorySessions::default();
        store.clone().save("a", &session()).unwrap();
        assert_eq!(store.load("a"), Some(session()));
        store.delete("a");
        assert_eq!(store.load("a"), None);
    }
}
