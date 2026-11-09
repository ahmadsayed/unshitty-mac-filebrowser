//! Connected Arbesk accounts, in `arbesk_accounts.json` with the other app
//! settings. Nothing secret is kept: reading an account needs only its
//! wallet address, and the session the sign-in made is ended at once.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::chain::Site;
use crate::location::account_path;

/// One connected account: who signed in, their wallet, and where.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Account {
    pub id: String,
    pub email: String,
    /// The wallet address, lower case: what every listing asks about.
    pub address: String,
    pub origin: String,
    pub chain_id: u64,
    /// The `arbesk://` location Network opens.
    pub root_path: String,
}

impl Account {
    pub fn site(&self) -> Site {
        Site {
            origin: self.origin.clone(),
            chain_id: self.chain_id,
        }
    }
}

/// The accounts, kept in memory and written through on every change.
#[derive(Debug)]
pub struct Accounts {
    path: PathBuf,
    items: Vec<Account>,
}

impl Accounts {
    /// Read `path`; a missing or damaged file means no accounts.
    pub fn load(path: PathBuf) -> Self {
        let items = std::fs::read_to_string(&path)
            .ok()
            .and_then(|json| serde_json::from_str(&json).ok())
            .unwrap_or_default();
        Self { path, items }
    }

    pub fn items(&self) -> &[Account] {
        &self.items
    }

    pub fn get_by_address(&self, address: &str) -> Option<&Account> {
        self.items
            .iter()
            .find(|account| account.address.eq_ignore_ascii_case(address))
    }

    /// Add the account that signed in, or update the one with its address
    /// on that site, which keeps its id.
    pub fn connect(
        &mut self,
        email: &str,
        address: &str,
        site: &Site,
    ) -> Result<Account, String> {
        let address = address.to_ascii_lowercase();
        let known = self.items.iter().position(|a| {
            a.address == address && a.origin == site.origin && a.chain_id == site.chain_id
        });
        let account = Account {
            id: known.map_or_else(crate::ids::new_account_id, |i| {
                self.items[i].id.clone()
            }),
            email: email.to_string(),
            root_path: account_path(&address),
            address,
            origin: site.origin.clone(),
            chain_id: site.chain_id,
        };
        match known {
            Some(index) => self.items[index] = account.clone(),
            None => self.items.push(account.clone()),
        }
        self.save()?;
        Ok(account)
    }

    /// Remove an account, returning it if it was connected.
    pub fn remove(&mut self, id: &str) -> Result<Option<Account>, String> {
        let Some(index) = self.items.iter().position(|account| account.id == id) else {
            return Ok(None);
        };
        let removed = self.items.remove(index);
        self.save()?;
        Ok(Some(removed))
    }

    fn save(&self) -> Result<(), String> {
        crate::store::write_json(&self.path, &self.items)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ADDRESS: &str = "0x00000000000000000000000000000000000000ab";

    fn site() -> Site {
        Site {
            origin: "https://arbesk.test".into(),
            chain_id: 84532,
        }
    }

    #[test]
    fn a_connected_account_is_saved_and_loaded() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("arbesk_accounts.json");
        let mut accounts = Accounts::load(file.clone());
        let account = accounts
            .connect("me@example.com", ADDRESS, &site())
            .unwrap();
        assert_eq!(account.root_path, format!("arbesk://{ADDRESS}"));
        assert_eq!(account.site(), site());
        let again = Accounts::load(file);
        assert_eq!(again.items(), [account.clone()]);
        assert_eq!(
            again.get_by_address(&ADDRESS.to_uppercase().replace("0X", "0x")),
            Some(&account)
        );
    }

    #[test]
    fn connecting_again_keeps_the_id_and_takes_the_new_email() {
        let dir = tempfile::tempdir().unwrap();
        let mut accounts = Accounts::load(dir.path().join("a.json"));
        let first = accounts
            .connect("old@example.com", ADDRESS, &site())
            .unwrap();
        let second = accounts
            .connect("new@example.com", ADDRESS, &site())
            .unwrap();
        assert_eq!(second.id, first.id);
        assert_eq!(accounts.items().len(), 1);
        assert_eq!(accounts.items()[0].email, "new@example.com");
    }

    #[test]
    fn the_file_holds_no_session() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.json");
        Accounts::load(file.clone())
            .connect("me@example.com", ADDRESS, &site())
            .unwrap();
        let text = std::fs::read_to_string(file).unwrap();
        assert!(
            !text.contains("token") && !text.contains("session"),
            "{text}"
        );
    }

    #[test]
    fn a_removed_account_is_gone() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.json");
        let mut accounts = Accounts::load(file.clone());
        let account = accounts
            .connect("me@example.com", ADDRESS, &site())
            .unwrap();
        assert_eq!(accounts.remove(&account.id).unwrap(), Some(account));
        assert!(Accounts::load(file).items().is_empty());
        assert_eq!(accounts.remove("nope").unwrap(), None);
    }

    #[test]
    fn a_damaged_file_means_no_accounts() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.json");
        std::fs::write(&file, "{ not json").unwrap();
        assert!(Accounts::load(file).items().is_empty());
    }
}
