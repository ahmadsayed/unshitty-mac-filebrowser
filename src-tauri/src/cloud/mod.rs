// The grammar and dispatcher are unit-tested now and wired into the commands
// in a later task; until then nothing in the binary calls them.
#![allow(dead_code, unused_imports)]

//! Google Drive support that does not depend on the operating system.
//!
//! A path is either local (`Location::Local`) or a `gdrive://` URI
//! (`Location::Drive`). Commands resolve the kind first and must never hand a
//! cloud path to `std::fs`.

pub mod accounts;
pub mod client;
pub mod drive;
pub mod location;
pub mod oauth;
pub mod service;
pub mod token;

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};

use serde::Serialize;

use accounts::Accounts;
use client::{ClientStore, OAuthClient};
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

/// A one-shot HTTP server for tests: serves one canned response per
/// connection, in order, on a random local port, and keeps every request
/// (head and body) so a test can check what was sent.
#[cfg(test)]
pub(crate) mod test_server {
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::{Arc, Mutex};
    use std::thread;

    pub struct Reply {
        pub status: u16,
        pub body: Vec<u8>,
        pub content_type: &'static str,
        /// `{base}` in a value is replaced with the server's base URL.
        pub headers: Vec<(&'static str, String)>,
    }

    impl Reply {
        pub fn json(status: u16, body: &str) -> Self {
            Self {
                status,
                body: body.as_bytes().to_vec(),
                content_type: "application/json",
                headers: Vec::new(),
            }
        }

        pub fn header(mut self, name: &'static str, value: &str) -> Self {
            self.headers.push((name, value.to_string()));
            self
        }
    }

    /// One request as received: the head percent-decoded (`+` as space) for
    /// easy matching, and the raw body.
    #[derive(Clone, Debug)]
    pub struct Request {
        pub head: String,
        pub body: Vec<u8>,
    }

    impl Request {
        pub fn body_text(&self) -> String {
            String::from_utf8_lossy(&self.body).into_owned()
        }
    }

    pub type Log = Arc<Mutex<Vec<Request>>>;

    pub fn serve_replies(replies: Vec<Reply>) -> (String, Log, thread::JoinHandle<()>) {
        let log: Log = Arc::new(Mutex::new(Vec::new()));
        let seen = log.clone();
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server_base = base.clone();
        let handle = thread::spawn(move || {
            for reply in replies {
                let Ok((mut stream, _)) = listener.accept() else {
                    return;
                };
                seen.lock().unwrap().push(read_request(&mut stream));
                let reason = if reply.status == 200 { "OK" } else { "Status" };
                let mut head = format!(
                    "HTTP/1.1 {} {reason}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n",
                    reply.status,
                    reply.content_type,
                    reply.body.len()
                );
                for (name, value) in &reply.headers {
                    head.push_str(&format!(
                        "{name}: {}\r\n",
                        value.replace("{base}", &server_base)
                    ));
                }
                head.push_str("\r\n");
                let _ = stream.write_all(head.as_bytes());
                let _ = stream.write_all(&reply.body);
            }
        });
        (base, log, handle)
    }

    /// Read the head up to the blank line, then `Content-Length` bytes.
    fn read_request(stream: &mut TcpStream) -> Request {
        let mut data = Vec::new();
        let mut buffer = [0u8; 65536];
        let head_end = loop {
            if let Some(end) = data.windows(4).position(|w| w == b"\r\n\r\n") {
                break end + 4;
            }
            match stream.read(&mut buffer) {
                Ok(0) | Err(_) => break data.len(),
                Ok(read) => data.extend_from_slice(&buffer[..read]),
            }
        };
        let raw_head = String::from_utf8_lossy(&data[..head_end]).into_owned();
        let length = raw_head
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().ok())?
            })
            .unwrap_or(0);
        while data.len() < head_end + length {
            match stream.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(read) => data.extend_from_slice(&buffer[..read]),
            }
        }
        let head = percent_encoding::percent_decode_str(&raw_head.replace('+', " "))
            .decode_utf8_lossy()
            .into_owned();
        Request {
            head,
            body: data[head_end..].to_vec(),
        }
    }

    fn to_replies(responses: Vec<(u16, Vec<u8>, &'static str)>) -> Vec<Reply> {
        responses
            .into_iter()
            .map(|(status, body, content_type)| Reply {
                status,
                body,
                content_type,
                headers: Vec::new(),
            })
            .collect()
    }

    pub fn serve(responses: Vec<(u16, Vec<u8>, &'static str)>) -> (String, thread::JoinHandle<()>) {
        let (base, _log, handle) = serve_replies(to_replies(responses));
        (base, handle)
    }

    /// Like `serve`, keeping each request's decoded head for assertions.
    pub fn serve_logged(
        responses: Vec<(u16, Vec<u8>, &'static str)>,
    ) -> (String, Arc<Mutex<Vec<String>>>, thread::JoinHandle<()>) {
        let (base, log, handle) = serve_replies(to_replies(responses));
        // A view of the heads only, filled in as requests arrive.
        let heads = Arc::new(Mutex::new(Vec::new()));
        let (log_view, heads_view) = (log.clone(), heads.clone());
        let joined = thread::spawn(move || {
            handle.join().unwrap();
            *heads_view.lock().unwrap() = log_view
                .lock()
                .unwrap()
                .iter()
                .map(|r| r.head.clone())
                .collect();
        });
        (base, heads, joined)
    }

    pub fn json(value: &str) -> Vec<u8> {
        value.as_bytes().to_vec()
    }
}
