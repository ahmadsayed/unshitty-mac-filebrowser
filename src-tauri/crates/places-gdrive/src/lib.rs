//! Google Drive for File Browser, a place of its own (design
//! `docs/superpowers/specs/2026-11-10-provider-crates-design.md`, step 4):
//! accounts and their sign-in, the REST client, `gdrive://` paths, listing,
//! opening, thumbnails, Drive's own writes (New Folder, Rename, Move to
//! Trash, Undo), the transfer primitives the app's engine copies and moves
//! with (`places::Transfer` on a [`Ctx`]), and Drive's `places::Provider`.
//! No Tauri: the app keeps the sign-in window and its commands.
//!
//! A path is either local (`Location::Local`) or a `gdrive://` URI
//! (`Location::Drive`). Commands resolve the kind first and must never hand a
//! cloud path to `std::fs`.

pub mod accounts;
pub mod client;
pub mod drive;
pub mod location;
pub mod loopback;
pub mod oauth;
pub mod provider;
pub mod service;
pub mod token;
pub mod transfer;
pub mod write;

/// A fake Drive server, for tests here and in the app (`testing`).
#[cfg(any(test, feature = "testing"))]
pub mod test_server;
/// Fixtures over the fake server, for tests here and in the app.
#[cfg(any(test, feature = "testing"))]
pub mod testing;

pub use provider::{Calls, Drive, Live, Reach};
pub use write::{Ctx, live_with};

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};


use accounts::{Accounts, DriveAccount};
use client::{ClientStore, OAuthClient};
use drive::AboutInfo;
use oauth::PendingFlow;
use token::{KeyringTokenStore, Token, TokenStore};

/// Forget remembered thumbnail links past this many; a listing refills them.
const THUMBNAIL_LINKS_MAX: usize = 50_000;

/// Process-wide Google Drive state: connected accounts, token storage and
/// sign-in flows that have not finished yet.
pub struct CloudState {
    pub accounts: Mutex<Accounts>,
    pub tokens: Box<dyn TokenStore>,
    pub flows: Mutex<HashMap<String, Arc<PendingFlow>>>,
    /// The user's own OAuth client; `None` until they add one.
    client: Mutex<Option<OAuthClient>>,
    client_store: ClientStore,
    /// Tokens in use, by account. The keychain is read once per account and
    /// written only when a token changes.
    live_tokens: Mutex<HashMap<String, Token>>,
    /// Held while a token is refreshed, so parallel requests refresh once.
    pub(crate) refreshing: Mutex<()>,
    /// Built on first use, on a blocking thread (a blocking client must not be
    /// created inside the async runtime).
    http: OnceLock<reqwest::blocking::Client>,
    /// `thumbnailLink` by `account/file-id`, remembered from listings so a
    /// thumbnail needs one request instead of two.
    thumbnail_links: Mutex<HashMap<String, String>>,
}

impl CloudState {
    pub fn new(config_dir: &Path) -> Self {
        Self::with_store(
            Accounts::load(config_dir.join("accounts.json")),
            // Keychain/Secret Service first; a 0600 file is the fallback.
            Box::new(KeyringTokenStore::new(
                "filebrowser",
                config_dir.join("tokens"),
            )),
            ClientStore::new(config_dir.join("google-client.json")),
        )
    }

    pub fn with_store(
        accounts: Accounts,
        tokens: Box<dyn TokenStore>,
        client_store: ClientStore,
    ) -> Self {
        Self {
            accounts: Mutex::new(accounts),
            tokens,
            flows: Mutex::new(HashMap::new()),
            client: Mutex::new(client_store.load()),
            client_store,
            live_tokens: Mutex::new(HashMap::new()),
            refreshing: Mutex::new(()),
            http: OnceLock::new(),
            thumbnail_links: Mutex::new(HashMap::new()),
        }
    }

    pub fn oauth_client(&self) -> Option<OAuthClient> {
        self.client
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    /// The user's client, or the error the Connect dialog shows without one.
    pub fn require_oauth_client(&self) -> Result<OAuthClient, String> {
        self.oauth_client().ok_or_else(|| {
            "Google Drive needs your own OAuth client. Add one in the Connect dialog.".into()
        })
    }

    /// Store a client from the JSON file Google lets users download.
    pub fn set_oauth_client(&self, json: &str) -> Result<OAuthClient, String> {
        self.refuse_while_connected()?;
        let client = client::parse(json)?;
        self.client_store.save(&client)?;
        *self.client.lock().unwrap_or_else(|p| p.into_inner()) = Some(client.clone());
        Ok(client)
    }

    pub fn clear_oauth_client(&self) -> Result<(), String> {
        self.refuse_while_connected()?;
        self.client_store.delete()?;
        *self.client.lock().unwrap_or_else(|p| p.into_inner()) = None;
        Ok(())
    }

    /// Keep the account a finished sign-in produced. `reconnect` is the
    /// account whose sign-in expired: its id stays, so paths, stars, tabs and
    /// history that point at `gdrive://<id>/…` keep working, and only its
    /// token and details are replaced.
    pub fn record_sign_in(
        &self,
        about: AboutInfo,
        token: &Token,
        reconnect: Option<&str>,
    ) -> Result<DriveAccount, String> {
        if token.refresh_token.is_none() {
            return Err("Google did not return a refresh token. Sign in again.".into());
        }
        let scopes = if token.scopes.is_empty() {
            vec![oauth::SCOPE_FULL.to_string()]
        } else {
            token.scopes.clone()
        };
        // Signing in for an account keeps its id (so its paths); so does a
        // fresh sign-in to a Google account that is already connected, which
        // replaces its token instead of adding the same account twice.
        let id = {
            let accounts = self
                .accounts
                .lock()
                .map_err(|_| "Google Drive state is unavailable.".to_string())?;
            let known = match reconnect {
                Some(id) => accounts.get(id),
                None => accounts
                    .items()
                    .iter()
                    .find(|account| account.email.eq_ignore_ascii_case(&about.email)),
            };
            known
                .map(|account| account.id.clone())
                .unwrap_or_else(accounts::new_account_id)
        };
        let account = DriveAccount::new(
            id,
            about.email,
            about.name,
            about.root_id,
            scopes,
            places::now(),
        );
        self.save_token(&account.id, token)?;
        if let Err(error) = self
            .accounts
            .lock()
            .map_err(|_| "Google Drive state is unavailable.".to_string())?
            .upsert(account.clone())
        {
            let _ = self.delete_token(&account.id);
            return Err(error);
        }
        Ok(account)
    }

    /// Tokens belong to the client that issued them and cannot be refreshed
    /// with another one, so the client only changes with no account connected.
    fn refuse_while_connected(&self) -> Result<(), String> {
        let connected = !self
            .accounts
            .lock()
            .map_err(|_| "Google Drive state is unavailable.".to_string())?
            .items()
            .is_empty();
        if connected {
            Err("Disconnect Google Drive first.".into())
        } else {
            Ok(())
        }
    }

    /// The shared HTTP client for Drive requests.
    pub fn http(&self) -> Result<reqwest::blocking::Client, String> {
        if let Some(client) = self.http.get() {
            return Ok(client.clone());
        }
        let client = drive::http_client()?;
        Ok(self.http.get_or_init(|| client).clone())
    }

    /// The account's current token: from memory, else from the keychain.
    pub fn token(&self, account: &str) -> Result<Option<Token>, String> {
        if let Some(token) = self.live_tokens().get(account) {
            return Ok(Some(token.clone()));
        }
        let token = self.tokens.load(account)?;
        if let Some(token) = &token {
            self.live_tokens().insert(account.into(), token.clone());
        }
        Ok(token)
    }

    /// Persist a new token and use it from now on.
    pub fn save_token(&self, account: &str, token: &Token) -> Result<(), String> {
        self.tokens.save(account, token)?;
        self.live_tokens().insert(account.into(), token.clone());
        Ok(())
    }

    /// Remove the account's token from memory and storage.
    pub fn delete_token(&self, account: &str) -> Result<(), String> {
        self.live_tokens().remove(account);
        self.thumbnail_links()
            .retain(|key, _| !key.starts_with(&format!("{account}/")));
        self.tokens.delete(account)
    }

    pub fn thumbnail_link(&self, account: &str, id: &str) -> Option<String> {
        self.thumbnail_links()
            .get(&format!("{account}/{id}"))
            .cloned()
    }

    pub fn remember_thumbnail_link(&self, account: &str, id: &str, link: &str) {
        let mut links = self.thumbnail_links();
        if links.len() >= THUMBNAIL_LINKS_MAX {
            links.clear();
        }
        links.insert(format!("{account}/{id}"), link.to_string());
    }

    // A poisoned lock only means another thread panicked mid-update; these
    // maps are caches, so their contents stay usable.
    fn live_tokens(&self) -> std::sync::MutexGuard<'_, HashMap<String, Token>> {
        self.live_tokens
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn thumbnail_links(&self) -> std::sync::MutexGuard<'_, HashMap<String, String>> {
        self.thumbnail_links
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

pub use location::{DriveLocation, Location, Segment, is_cloud, parse};

/// Fields that only exist on an entry from Google Drive, in the `places`
/// crate with the entry that carries them.
pub use places::CloudEntry;

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

    fn state_in(dir: &Path) -> CloudState {
        CloudState::with_store(
            Accounts::load(dir.join("accounts.json")),
            Box::new(token::FileTokenStore::new(dir.join("tokens"))),
            ClientStore::new(dir.join("google-client.json")),
        )
    }

    const DESKTOP: &str = r#"{"installed":{"client_id":"123-abc.apps.googleusercontent.com","client_secret":"GOCSPX-test"}}"#;

    #[test]
    fn the_oauth_client_is_saved_and_survives_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let state = state_in(dir.path());
        assert!(state.oauth_client().is_none());
        assert!(
            state
                .require_oauth_client()
                .unwrap_err()
                .contains("own OAuth client")
        );
        state.set_oauth_client(DESKTOP).unwrap();
        assert_eq!(
            state_in(dir.path()).oauth_client(),
            Some(client::test_client())
        );

        state.clear_oauth_client().unwrap();
        assert!(state.oauth_client().is_none());
        assert!(state_in(dir.path()).oauth_client().is_none());
    }

    #[test]
    fn an_invalid_client_leaves_the_current_one() {
        let dir = tempfile::tempdir().unwrap();
        let state = state_in(dir.path());
        state.set_oauth_client(DESKTOP).unwrap();
        assert!(state.set_oauth_client(r#"{"web":{}}"#).is_err());
        assert_eq!(state.oauth_client(), Some(client::test_client()));
    }

    #[test]
    fn the_client_cannot_change_while_an_account_is_connected() {
        let dir = tempfile::tempdir().unwrap();
        let state = state_in(dir.path());
        state.set_oauth_client(DESKTOP).unwrap();
        state
            .accounts
            .lock()
            .unwrap()
            .upsert(accounts::DriveAccount::new(
                "acct".into(),
                "me@example.com".into(),
                "Me".into(),
                "root".into(),
                vec![],
                0,
            ))
            .unwrap();
        assert_eq!(
            state.set_oauth_client(DESKTOP).unwrap_err(),
            "Disconnect Google Drive first."
        );
        assert_eq!(
            state.clear_oauth_client().unwrap_err(),
            "Disconnect Google Drive first."
        );
        assert!(state.oauth_client().is_some());
    }

    fn sign_in_about() -> AboutInfo {
        AboutInfo {
            email: "me@example.com".into(),
            name: "Me".into(),
            root_id: "root".into(),
        }
    }

    fn token(access: &str, refresh: &str, scopes: Vec<String>) -> Token {
        Token::new(
            access.into(),
            Some(refresh.into()),
            places::now() + 3600,
            scopes,
        )
    }

    #[test]
    fn a_reconnect_keeps_the_account_id_and_replaces_its_token() {
        let dir = tempfile::tempdir().unwrap();
        let state = state_in(dir.path());
        let first = state
            .record_sign_in(
                sign_in_about(),
                &token("access-1", "refresh-1", vec![]),
                None,
            )
            .unwrap();
        assert_eq!(first.scopes, vec![oauth::SCOPE_FULL.to_string()]);

        let mut changed = sign_in_about();
        changed.email = "new@example.com".into();
        let again = state
            .record_sign_in(
                changed,
                &token("access-2", "refresh-2", vec![oauth::SCOPE_READONLY.into()]),
                Some(&first.id),
            )
            .unwrap();

        assert_eq!(again.id, first.id);
        assert_eq!(again.email, "new@example.com");
        assert_eq!(
            again.scopes,
            vec![oauth::SCOPE_READONLY.to_string()],
            "a sign-in's own scopes are kept instead of the full scope"
        );
        let accounts = state.accounts.lock().unwrap();
        assert_eq!(accounts.items(), &[again.clone()]);
        drop(accounts);
        assert_eq!(
            state.token(&first.id).unwrap().unwrap().access_token,
            "access-2"
        );
    }

    #[test]
    fn signing_in_to_a_connected_email_keeps_its_account_and_another_email_adds_one() {
        let dir = tempfile::tempdir().unwrap();
        let state = state_in(dir.path());
        let first = state
            .record_sign_in(sign_in_about(), &token("access-1", "refresh-1", vec![]), None)
            .unwrap();

        // Online Accounts' "Connect an Account" signs in with no account in
        // mind; the same Google account must not become a second row.
        let mut same = sign_in_about();
        same.email = "ME@example.com".into();
        let again = state
            .record_sign_in(same, &token("access-2", "refresh-2", vec![]), None)
            .unwrap();
        assert_eq!(again.id, first.id);
        assert_eq!(state.accounts.lock().unwrap().items().len(), 1);
        assert_eq!(
            state.token(&first.id).unwrap().unwrap().access_token,
            "access-2"
        );

        let mut other = sign_in_about();
        other.email = "work@example.com".into();
        let second = state
            .record_sign_in(other, &token("access-3", "refresh-3", vec![]), None)
            .unwrap();
        assert_ne!(second.id, first.id);
        let accounts = state.accounts.lock().unwrap();
        assert_eq!(accounts.items().len(), 2);
        assert_eq!(accounts.items()[0].id, first.id);
        drop(accounts);
        assert_eq!(
            state.token(&first.id).unwrap().unwrap().access_token,
            "access-2",
            "the first account's token is untouched"
        );
    }

    #[test]
    fn a_reconnect_for_an_account_that_is_gone_adds_one() {
        let dir = tempfile::tempdir().unwrap();
        let state = state_in(dir.path());
        let account = state
            .record_sign_in(
                sign_in_about(),
                &token("access", "refresh", vec![]),
                Some("no-such-account"),
            )
            .unwrap();
        assert_ne!(account.id, "no-such-account");
        assert_eq!(state.accounts.lock().unwrap().items(), &[account]);
    }

    #[test]
    fn signing_in_without_a_refresh_token_is_refused_and_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let state = state_in(dir.path());
        let no_refresh = Token::new(
            "access".into(),
            None,
            places::now() + 3600,
            vec![],
        );
        assert_eq!(
            state
                .record_sign_in(sign_in_about(), &no_refresh, None)
                .unwrap_err(),
            "Google did not return a refresh token. Sign in again."
        );
        assert!(state.accounts.lock().unwrap().items().is_empty());
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
