// The grammar and dispatcher are unit-tested now and wired into the commands
// in a later task; until then nothing in the binary calls them.
#![allow(dead_code, unused_imports)]

//! Google Drive support that does not depend on the operating system.
//!
//! A path is either local (`Location::Local`) or a `gdrive://` URI
//! (`Location::Drive`). Commands resolve the kind first and must never hand a
//! cloud path to `std::fs`.

pub mod accounts;
pub mod drive;
pub mod location;
pub mod oauth;
pub mod service;
pub mod token;

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use serde::Serialize;

use accounts::Accounts;
use oauth::PendingFlow;
use token::{KeyringTokenStore, TokenStore};

/// Process-wide Google Drive state: connected accounts, token storage and
/// sign-in flows that have not finished yet.
pub struct CloudState {
    pub accounts: Mutex<Accounts>,
    pub tokens: Box<dyn TokenStore>,
    pub flows: Mutex<HashMap<String, Arc<PendingFlow>>>,
}

impl CloudState {
    pub fn new(config_dir: &Path) -> Self {
        Self {
            accounts: Mutex::new(Accounts::load(config_dir.join("accounts.json"))),
            // Keychain/Secret Service first; a 0600 file is the fallback.
            tokens: Box::new(KeyringTokenStore::new(
                "filebrowser",
                config_dir.join("tokens"),
            )),
            flows: Mutex::new(HashMap::new()),
        }
    }
}

pub use location::{DriveLocation, Location, Segment, is_cloud, parse};

/// Fields that only exist on an entry from a cloud provider. Omitted from the
/// serialized entry for local files, so existing frontend code is unchanged.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CloudEntry {
    pub account: String,
    pub id: String,
    pub mime_type: String,
    pub is_google_doc: bool,
    pub is_shortcut: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub web_url: Option<String>,
    pub can_download: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_paths_stay_local() {
        assert_eq!(
            parse("/Users/me/Documents").unwrap(),
            Location::Local("/Users/me/Documents".into())
        );
        assert!(!is_cloud("/Users/me/Documents"));
    }

    #[test]
    fn drive_paths_are_drive() {
        let loc = parse("gdrive://acct/root~My%20Drive").unwrap();
        match loc {
            Location::Drive(drive) => {
                assert_eq!(drive.account, "acct");
                assert_eq!(drive.file_id(), Some("root"));
            }
            Location::Local(_) => panic!("expected a Drive location"),
        }
        assert!(is_cloud("gdrive://acct/root~My%20Drive"));
    }

    #[test]
    fn a_malformed_drive_uri_is_an_error_not_a_local_path() {
        assert!(parse("gdrive://").is_err());
        assert!(parse("gdrive://acct").is_err());
        assert!(parse("gdrive://acct/").is_err());
    }

    #[test]
    fn cloud_entries_omit_empty_web_urls() {
        let entry = CloudEntry {
            account: "acct".into(),
            id: "id".into(),
            mime_type: "application/pdf".into(),
            is_google_doc: false,
            is_shortcut: false,
            web_url: None,
            can_download: true,
        };
        let json = serde_json::to_value(entry).unwrap();
        assert!(json.get("web_url").is_none());
    }
}
