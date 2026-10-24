//! Connected Google accounts, stored in `accounts.json` next to the other
//! app settings. Tokens are never part of this file; they live in `token.rs`.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::cloud::location::DriveLocation;

/// One connected account. `root_path` is the `gdrive://` location the sidebar
/// opens and is derived from the account and root ids.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DriveAccount {
    pub id: String,
    pub email: String,
    pub name: String,
    pub root_id: String,
    pub root_name: String,
    pub root_path: String,
    pub scopes: Vec<String>,
    pub connected_at: i64,
}

impl DriveAccount {
    pub fn new(
        id: String,
        email: String,
        name: String,
        root_id: String,
        scopes: Vec<String>,
        connected_at: i64,
    ) -> Self {
        let root_name = "My Drive".to_string();
        let root_path = DriveLocation::root(&id, &root_id, &root_name).to_uri();
        Self {
            id,
            email,
            name,
            root_id,
            root_name,
            root_path,
            scopes,
            connected_at,
        }
    }
}

/// The list of accounts, kept in memory and written through on every change.
#[derive(Debug)]
pub struct Accounts {
    path: PathBuf,
    items: Vec<DriveAccount>,
}

impl Accounts {
    /// Read `path`. A missing or corrupt file means no accounts, which is also
    /// what the rest of the app should do rather than refusing to start.
    pub fn load(path: PathBuf) -> Self {
        let items = std::fs::read_to_string(&path)
            .ok()
            .and_then(|json| serde_json::from_str(&json).ok())
            .unwrap_or_default();
        Self { path, items }
    }

    pub fn items(&self) -> &[DriveAccount] {
        &self.items
    }

    pub fn get(&self, id: &str) -> Option<&DriveAccount> {
        self.items.iter().find(|account| account.id == id)
    }

    /// Add an account or replace the one with the same id.
    pub fn upsert(&mut self, account: DriveAccount) -> Result<(), String> {
        match self.items.iter_mut().find(|item| item.id == account.id) {
            Some(existing) => *existing = account,
            None => self.items.push(account),
        }
        self.save()
    }

    /// Remove an account, returning it if it was connected.
    pub fn remove(&mut self, id: &str) -> Result<Option<DriveAccount>, String> {
        let Some(index) = self.items.iter().position(|account| account.id == id) else {
            return Ok(None);
        };
        let removed = self.items.remove(index);
        self.save()?;
        Ok(Some(removed))
    }

    fn save(&self) -> Result<(), String> {
        config_write(&self.path, &self.items)
    }
}

fn config_write(path: &Path, accounts: &[DriveAccount]) -> Result<(), String> {
    crate::config_store::write_json(path, &accounts)
}

/// A short random account id. It only has to be unique on this machine; the
/// Drive file ids are the security-relevant identifiers.
pub fn new_account_id() -> String {
    let mut bytes = [0u8; 8];
    if getrandom::fill(&mut bytes).is_err() {
        // Extremely unlikely; the clock still gives a usable number.
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        return format!("{nanos:x}");
    }
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn account(id: &str) -> DriveAccount {
        DriveAccount::new(
            id.into(),
            "me@example.com".into(),
            "Me".into(),
            "root".into(),
            vec!["drive.readonly".into()],
            1_792_830_000,
        )
    }

    #[test]
    fn accounts_round_trip_through_json() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("accounts.json");
        let mut accounts = Accounts::load(path.clone());
        assert!(accounts.items().is_empty());

        accounts.upsert(account("abc")).unwrap();
        let reloaded = Accounts::load(path);
        assert_eq!(reloaded.items(), &[account("abc")]);
    }

    #[test]
    fn upsert_replaces_by_id_and_remove_is_idempotent() {
        let dir = tempdir().unwrap();
        let mut accounts = Accounts::load(dir.path().join("accounts.json"));
        accounts.upsert(account("abc")).unwrap();
        let mut changed = account("abc");
        changed.email = "new@example.com".into();
        accounts.upsert(changed.clone()).unwrap();
        assert_eq!(accounts.items(), &[changed]);
        assert!(accounts.remove("abc").unwrap().is_some());
        assert!(accounts.remove("abc").unwrap().is_none());
        assert!(accounts.items().is_empty());
    }

    #[test]
    fn a_corrupt_file_means_no_accounts() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("accounts.json");
        std::fs::write(&path, "{ not json").unwrap();
        assert!(Accounts::load(path).items().is_empty());
    }

    #[test]
    fn the_root_path_is_the_drive_uri() {
        let account = account("abc");
        assert!(account.root_path.starts_with("gdrive://abc/"));
        assert!(account.root_path.ends_with("root~My%20Drive"));
    }

    #[test]
    fn ids_are_random_and_distinct() {
        assert_ne!(new_account_id(), new_account_id());
        assert_eq!(new_account_id().len(), 16);
    }
}
