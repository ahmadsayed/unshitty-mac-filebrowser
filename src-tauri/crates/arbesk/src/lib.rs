//! Arbesk (promptscad.com) collections, for the File Browser. Sign in with an
//! email and the code Arbesk emails, browse the account's collections as
//! folders and their assets as glTF and 3MF files, build an asset to open or
//! preview it, copy a local model into a collection, and create collections.
//!
//! Arbesk's read path is rebuilt from public sources (the deployment's config
//! and token index, the chain's `tokenURI`, the IPFS gateway); the one write
//! uploads to IPFS through Arbesk's upload URLs and saves through its wallet
//! relay, with the session kept from sign-in. Every module below is an
//! implementation detail: the app uses only the types and the [`Client`]
//! re-exported here. `Client` is also Arbesk's `places::Provider` (with its
//! `Transfer` and extensions), which the app registers for `arbesk://`
//! paths.

mod accounts;
mod cache;
mod callback;
mod chain;
mod compose;
mod create;
mod decompose;
mod http;
mod ids;
mod image;
mod ipfs;
mod location;
mod merkle;
mod progress;
mod provider;
mod representations;
mod scene;
mod service;
mod session;
mod share;
mod signin;
mod store;
#[cfg(test)]
pub(crate) mod testing;
mod write;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use serde::Serialize;

use crate::accounts::Accounts;
use crate::chain::Site;
use crate::http::{Http, ReqwestHttp};
use crate::session::{Session, SessionStore};
use crate::signin::{CALLBACK_TIMEOUT, Flow, now_ms};

pub use accounts::Account;
pub use location::{ArbeskPath as Location, is_arbesk, is_folder, parse, parse_target};
pub use progress::{CANCELLED, Phase, Progress, Snapshot};
pub use provider::SCHEME;
pub use scene::Scene;
pub use service::Version;
pub use share::Shared;

/// A row in an account's or a collection's listing, with the same fields the
/// app's own entries have, so it can be handed to the view unchanged.
#[derive(Clone, Debug, Serialize)]
pub struct Entry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub size: u64,
    pub modified: i64,
    pub is_hidden: bool,
    pub is_symlink: bool,
    /// An asset's other files, summed up from the manifest the row is read
    /// from (spec F2); absent for collections and assets with none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub representations: Option<places::Representations>,
}

/// Where new accounts sign in: Arbesk's public deployment on Base Sepolia.
/// `FILEBROWSER_ARBESK_ORIGIN` points the app at another one (a local
/// Arbesk on `http://localhost:9090`, say).
fn sign_in_site() -> Site {
    let origin = std::env::var("FILEBROWSER_ARBESK_ORIGIN")
        .unwrap_or_else(|_| "https://promptscad.com".into());
    Site {
        origin: origin.trim_end_matches('/').to_string(),
        chain_id: 84532,
    }
}

/// What a [`Client`] keeps: one HTTP client, the cache folder, the connected
/// accounts and the sign-ins waiting for the browser.
pub struct Client {
    http: Result<Arc<dyn Http>, String>,
    cache_dir: PathBuf,
    accounts: Mutex<Accounts>,
    sessions: Box<dyn SessionStore>,
    flows: Mutex<HashMap<String, Arc<Flow>>>,
    builds: service::Builds,
    scenes: scene::Scenes,
    thumbnails: service::Thumbnails,
    /// One for the app's whole run: it remembers writes that may still land.
    locks: write::CollectionLocks,
}

/// The key a scene is kept under: its asset and version.
fn kept(path: &str, version: Option<&str>) -> String {
    format!("{path}\n{}", version.unwrap_or(""))
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl Client {
    pub fn new(cache_dir: PathBuf, accounts_file: PathBuf, sessions_dir: PathBuf) -> Self {
        let http = ReqwestHttp::new().map(|http| Arc::new(http) as Arc<dyn Http>);
        let sessions = Box::new(session::KeyringSessions::new(sessions_dir));
        Self::with(http, cache_dir, accounts_file, sessions)
    }

    #[cfg(test)]
    pub fn with_http(http: Arc<dyn Http>, cache_dir: PathBuf, accounts_file: PathBuf) -> Self {
        let sessions = Box::new(session::MemorySessions::default());
        Self::with(Ok(http), cache_dir, accounts_file, sessions)
    }

    #[cfg(test)]
    pub fn with_sessions(
        http: Arc<dyn Http>,
        cache_dir: PathBuf,
        accounts_file: PathBuf,
        sessions: Box<dyn SessionStore>,
    ) -> Self {
        Self::with(Ok(http), cache_dir, accounts_file, sessions)
    }

    fn with(
        http: Result<Arc<dyn Http>, String>,
        cache_dir: PathBuf,
        accounts_file: PathBuf,
        sessions: Box<dyn SessionStore>,
    ) -> Self {
        let accounts = Mutex::new(Accounts::load(accounts_file));
        let thumbnails = service::Thumbnails::new(cache_dir.clone());
        Self {
            http,
            cache_dir,
            accounts,
            sessions,
            flows: Mutex::new(HashMap::new()),
            builds: service::Builds::default(),
            scenes: scene::Scenes::default(),
            thumbnails,
            locks: write::CollectionLocks::default(),
        }
    }

    fn http(&self) -> Result<&dyn Http, String> {
        self.http.as_deref().map_err(Clone::clone)
    }

    pub fn accounts(&self) -> Vec<Account> {
        lock(&self.accounts).items().to_vec()
    }

    /// The deployment and chain of the account `address`.
    fn site_for(&self, address: &str) -> Result<Site, String> {
        let accounts = lock(&self.accounts);
        let account = accounts
            .get_by_address(address)
            .ok_or("Connect this Arbesk account first.")?;
        Ok(account.site())
    }

    /// How to reach the account `path` belongs to.
    fn reach(&self, path: &str) -> Result<(&dyn Http, Site), String> {
        let site = self.site_for(parse(path)?.address())?;
        Ok((self.http()?, site))
    }

    /// `service::list` for the account `path` belongs to.
    pub fn list(&self, path: &str) -> Result<Vec<Entry>, String> {
        let (http, site) = self.reach(path)?;
        service::list(http, &site, path, &self.cache_dir, &self.thumbnails)
    }

    /// `service::thumbnail` for the account `path` belongs to, as PNG bytes:
    /// empty when the asset has none, as the `thumbnail` command answers.
    pub fn thumbnail(&self, path: &str, size: u32, stop: &dyn Fn() -> bool) -> Vec<u8> {
        let site = parse(path).and_then(|path| self.site_for(path.address()));
        let (Ok(site), Ok(http)) = (site, self.http()) else {
            return Vec::new();
        };
        service::thumbnail(
            http,
            &site,
            path,
            size,
            &self.cache_dir,
            &self.thumbnails,
            stop,
        )
        .unwrap_or_default()
    }

    /// The Studio link of the asset at `path`, on its account's deployment.
    pub fn studio_url(&self, path: &str) -> Result<String, String> {
        service::studio_url(&self.site_for(parse(path)?.address())?, path)
    }

    /// `service::build` for the account `path` belongs to.
    pub fn build(&self, path: &str, progress: &Progress) -> Result<PathBuf, String> {
        let site = self.site_for(parse(path)?.address())?;
        service::build(
            self.http()?,
            &site,
            path,
            &self.cache_dir,
            progress,
            &self.builds,
        )
    }

    /// `service::export` for the account `path` belongs to.
    pub fn export(&self, path: &str, progress: &Progress) -> Result<PathBuf, String> {
        let site = self.site_for(parse(path)?.address())?;
        service::export(
            self.http()?,
            &site,
            path,
            &self.cache_dir,
            progress,
            &self.builds,
        )
    }

    /// `service::versions` for the account `path` belongs to.
    pub fn versions(&self, path: &str) -> Result<Vec<service::Version>, String> {
        let (http, site) = self.reach(path)?;
        service::versions(http, &site, path, &self.cache_dir)
    }

    /// `service::scene` for the account `path` belongs to, kept for `part`.
    pub fn scene(&self, path: &str, version: Option<&str>) -> Result<scene::Scene, String> {
        let (http, site) = self.reach(path)?;
        let resolved = service::scene(http, &site, path, version, &self.cache_dir)?;
        Ok(self
            .scenes
            .keep(&kept(path, version), resolved)
            .as_ref()
            .clone())
    }

    /// `service::part` for the account `path` belongs to, from the scene of
    /// `version` (the asset's newest without one).
    pub fn part(
        &self,
        path: &str,
        version: Option<&str>,
        key: &str,
        progress: &Progress,
    ) -> Result<PathBuf, String> {
        let (http, site) = self.reach(path)?;
        let scene = self.scenes.get_or_keep(&kept(path, version), || {
            service::scene(http, &site, path, version, &self.cache_dir)
        })?;
        service::part(
            http,
            &site,
            &scene,
            key,
            &self.cache_dir,
            progress,
            &self.builds,
        )
    }

    /// Start a sign-in for `email`: its id and the page to open in the browser.
    pub fn connect_start(&self, email: &str) -> Result<(String, String), String> {
        let flow = signin::start(&sign_in_site(), email)?;
        let started = (flow.id.clone(), flow.sign_in_url.clone());
        let mut flows = lock(&self.flows);
        // A frontend that vanished mid-dialog must not leave listeners behind.
        flows.retain(|_, flow| flow.created.elapsed() < CALLBACK_TIMEOUT && !flow.is_cancelled());
        flows.insert(flow.id.clone(), Arc::new(flow));
        Ok(started)
    }

    /// Wait for the browser, confirm the account and connect it.
    pub fn connect_finish(&self, flow_id: &str) -> Result<Account, String> {
        let flow = lock(&self.flows)
            .get(flow_id)
            .cloned()
            .ok_or("This sign-in attempt is no longer active.")?;
        flow.claim()?;
        let site = sign_in_site();
        let signed_in = signin::finish(&flow, self.http()?, &site, CALLBACK_TIMEOUT);
        lock(&self.flows).remove(flow_id);
        let signed_in = signed_in?;
        // Cancel may have come while Arbesk was confirming: it still cancels.
        if flow.is_cancelled() {
            self.end_session(&site, &signed_in.session);
            return Err("Sign-in was cancelled.".into());
        }
        let account = lock(&self.accounts).connect(&signed_in.email, &signed_in.address, &site)?;
        if let Err(error) = self.sessions.save(&account.id, &signed_in.session) {
            // Without a kept session the account still reads; copying in asks to sign in again.
            self.sessions.delete(&account.id);
            self.end_session(&site, &signed_in.session);
            eprintln!("Arbesk: the session was not kept: {error}");
        }
        Ok(account)
    }

    /// Ask Arbesk to end `session`; a failure changes nothing here.
    fn end_session(&self, site: &Site, session: &Session) {
        if let Ok(http) = self.http() {
            let _ = http.delete(
                &format!("{}/api/v1/sessions", site.origin),
                Some(&session.token),
            );
        }
    }

    /// The id of the account `path` belongs to.
    fn account_id(&self, path: &str) -> Option<String> {
        let address = parse(path).ok()?.address().to_string();
        lock(&self.accounts)
            .get_by_address(&address)
            .map(|account| account.id.clone())
    }

    /// The token to act as the account `path` belongs to, while Arbesk still
    /// accepts it.
    pub fn session_for(&self, path: &str) -> Result<String, String> {
        let kept = self
            .account_id(path)
            .and_then(|id| self.sessions.load(&id))
            .filter(|session| session.expires_at_ms > now_ms() + 60_000);
        kept.map(|session| session.token)
            .ok_or_else(|| "Sign in to Arbesk again to copy files into it.".to_string())
    }

    /// Whether copying into the account `path` belongs to can start.
    pub fn signed_in(&self, path: &str) -> bool {
        self.session_for(path).is_ok()
    }

    /// Copy the local model `file` to the pending name `to` in a collection
    /// (`write::add`): the new asset's path. Arbesk's 401 ends the session.
    pub fn add(
        &self,
        to: &str,
        file: &Path,
        overwrite: bool,
        progress: &Progress,
    ) -> Result<String, String> {
        let (address, token_id, name) = parse_target(to)?;
        let site = self.site_for(&address)?;
        let account = location::account_path(&address);
        let session = self.session_for(&account)?;
        let target = write::Target { address, token_id, name };
        let added = write::add(
            self.http()?,
            &site,
            &session,
            &target,
            file,
            overwrite,
            progress,
            &self.cache_dir,
            &self.locks,
        );
        if added.as_ref().err().is_some_and(|error| error == write::SIGN_IN) {
            self.forget_session(&account);
        }
        added
    }

    /// Create (mint) the named collection of the account `path` belongs to,
    /// as the Studio's New Collection and `besk create` do: the new
    /// collection's `arbesk://` path. The same name in one account is one
    /// collection, so an existing one is refused. Arbesk's 401 ends the session.
    pub fn create_collection(&self, path: &str, name: &str) -> Result<String, String> {
        let address = match parse(path)? {
            location::ArbeskPath::Account { address } => address,
            _ => return Err("Create collections in the account's folder.".into()),
        };
        let site = self.site_for(&address)?;
        let account = location::account_path(&address);
        let session = self
            .session_for(&account)
            .map_err(|_| create::SIGN_IN.to_string())?;
        let created = create::create(
            self.http()?,
            &site,
            &session,
            &address,
            name,
            &self.cache_dir,
        )
        .map_err(|error| match error.as_str() {
            write::SIGN_IN => create::SIGN_IN.to_string(),
            _ => error,
        });
        if created
            .as_ref()
            .err()
            .is_some_and(|error| error == create::SIGN_IN)
        {
            self.forget_session(&account);
        }
        created
    }

    /// Add the account behind `email` to the collection `path` names as a
    /// viewer (`1`) or an editor (`2`), or change its role. The session's
    /// account must be an editor. Arbesk's 401 ends the session.
    pub fn share_collection(&self, path: &str, email: &str, role: u8) -> Result<Shared, String> {
        let (address, token_id) = match parse(path)? {
            location::ArbeskPath::Collection { address, token_id } => (address, token_id),
            _ => return Err("Share a collection.".into()),
        };
        let site = self.site_for(&address)?;
        let account = location::account_path(&address);
        let session = self
            .session_for(&account)
            .map_err(|_| share::SIGN_IN.to_string())?;
        let shared = share::add(
            self.http()?,
            &site,
            &session,
            &address,
            &token_id,
            email,
            role,
            &self.cache_dir,
            &self.locks,
        )
        .map_err(|error| match error.as_str() {
            write::SIGN_IN => share::SIGN_IN.to_string(),
            _ => error,
        });
        if shared.as_ref().err().is_some_and(|error| error == share::SIGN_IN) {
            self.forget_session(&account);
        }
        shared
    }

    /// Whether the collection of the pending name `path` lists that name.
    pub fn exists(&self, path: &str) -> bool {
        let Ok((address, token_id, name)) = parse_target(path) else {
            return false;
        };
        let collection = location::collection_path(&address, &token_id, "");
        self.list(&collection)
            .is_ok_and(|entries| entries.iter().any(|entry| entry.name == name))
    }

    /// Drop the session of `path`'s account, after Arbesk refused it.
    pub fn forget_session(&self, path: &str) {
        if let Some(id) = self.account_id(path) {
            self.sessions.delete(&id);
        }
    }

    pub fn connect_cancel(&self, flow_id: &str) {
        if let Some(flow) = lock(&self.flows).remove(flow_id) {
            flow.cancel();
        }
    }

    /// Forget the account and end its session; nothing else on Arbesk changes.
    pub fn disconnect(&self, id: &str) -> Result<(), String> {
        let removed = lock(&self.accounts).remove(id)?;
        if let Some(account) = removed {
            if let Some(session) = self.sessions.load(id) {
                self.end_session(&account.site(), &session);
            }
        }
        self.sessions.delete(id);
        // Where its assets' thumbnails are is no longer needed.
        self.thumbnails.clear();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::fake::FakeHttp;
    use crate::session::MemorySessions;

    const ORIGIN: &str = "https://promptscad.com";

    const ADDRESS: &str = "0x00000000000000000000000000000000000000ab";

    /// Arbesk confirming every session as `ADDRESS`.
    fn confirming() -> Arc<FakeHttp> {
        Arc::new(FakeHttp::new(|request| match request.method {
            "POST" => Ok(serde_json::json!({ "exists": true, "address": ADDRESS })
                .to_string()
                .into_bytes()),
            _ => Ok(b"{}".to_vec()),
        }))
    }

    fn browser_returns(url: &str, email: &str) -> std::thread::JoinHandle<()> {
        let port = url::Url::parse(url)
            .unwrap()
            .query_pairs()
            .find(|(k, _)| k == "port")
            .unwrap()
            .1
            .parse::<u16>()
            .unwrap();
        let target = format!("/?token=t&address={ADDRESS}&email={email}");
        std::thread::spawn(move || {
            use std::io::{Read, Write};
            let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
            write!(stream, "GET {target} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n").unwrap();
            let _ = stream.read_to_string(&mut String::new());
        })
    }

    #[test]
    fn a_sign_in_connects_the_account_and_its_listing() {
        let dir = tempfile::tempdir().unwrap();
        let state = Client::with_http(
            confirming(),
            dir.path().join("cache"),
            dir.path().join("arbesk_accounts.json"),
        );
        let (flow_id, url) = state.connect_start("me@example.com").unwrap();
        assert!(
            url.starts_with(&format!(
                "{ORIGIN}/api/v1/cli-auth?email=me%40example.com&port="
            )),
            "{url}"
        );
        let browser = browser_returns(&url, "me%40example.com");
        let account = state.connect_finish(&flow_id).unwrap();
        browser.join().unwrap();
        assert_eq!(
            (account.email.as_str(), account.address.as_str()),
            ("me@example.com", ADDRESS)
        );
        assert_eq!(state.accounts(), [account.clone()]);
        assert_eq!(state.site_for(ADDRESS).unwrap(), account.site());
        let asset = format!("arbesk://{ADDRESS}/1~Set/a1~Box.gltf");
        assert_eq!(
            state.studio_url(&asset).unwrap(),
            format!("{ORIGIN}/studio?asset=1&assetId=a1")
        );
        assert_eq!(
            state.connect_finish(&flow_id).unwrap_err(),
            "This sign-in attempt is no longer active."
        );

        state.disconnect(&account.id).unwrap();
        assert!(state.accounts().is_empty());
        assert_eq!(
            state.site_for(ADDRESS).unwrap_err(),
            "Connect this Arbesk account first."
        );
    }

    /// A state whose sessions the test can look into.
    fn state_with_sessions(
        http: Arc<FakeHttp>,
        dir: &std::path::Path,
    ) -> (Client, MemorySessions) {
        let sessions = MemorySessions::default();
        let state = Client::with_sessions(
            http,
            dir.join("cache"),
            dir.join("arbesk_accounts.json"),
            Box::new(sessions.clone()),
        );
        (state, sessions)
    }

    #[test]
    fn connecting_saves_the_session_and_disconnect_ends_it() {
        let dir = tempfile::tempdir().unwrap();
        let http = confirming();
        let (state, sessions) = state_with_sessions(http.clone(), dir.path());
        let (flow_id, url) = state.connect_start("me@example.com").unwrap();
        let browser = browser_returns(&url, "me%40example.com");
        let account = state.connect_finish(&flow_id).unwrap();
        browser.join().unwrap();
        let kept = sessions.load(&account.id).unwrap();
        assert_eq!(kept.token, "t");
        assert!(kept.expires_at_ms > signin::now_ms());
        assert!(
            !std::fs::read_to_string(dir.path().join("arbesk_accounts.json"))
                .unwrap()
                .contains("\"t\"")
        );
        assert!(http.requests().iter().all(|r| r.method != "DELETE"));
        let path = format!("arbesk://{ADDRESS}");
        assert!(state.signed_in(&path));
        assert_eq!(state.session_for(&path).unwrap(), "t");

        state.disconnect(&account.id).unwrap();
        let requests = http.requests();
        let last = requests.last().unwrap();
        assert_eq!(
            (last.method, last.url.as_str()),
            ("DELETE", format!("{ORIGIN}/api/v1/sessions").as_str())
        );
        assert_eq!(last.session.as_deref(), Some("t"));
        assert_eq!(sessions.load(&account.id), None);
    }

    #[test]
    fn a_missing_or_expiring_session_asks_for_a_sign_in() {
        let dir = tempfile::tempdir().unwrap();
        let (state, sessions) = state_with_sessions(confirming(), dir.path());
        let (flow_id, url) = state.connect_start("me@example.com").unwrap();
        let browser = browser_returns(&url, "me%40example.com");
        let account = state.connect_finish(&flow_id).unwrap();
        browser.join().unwrap();
        let path = format!("arbesk://{ADDRESS}/1~Set");
        let again = "Sign in to Arbesk again to copy files into it.";

        sessions.delete(&account.id);
        assert_eq!(state.session_for(&path).unwrap_err(), again);
        assert!(!state.signed_in(&path));

        let soon = Session {
            token: "t".into(),
            expires_at_ms: signin::now_ms() + 30_000,
        };
        sessions.save(&account.id, &soon).unwrap();
        assert_eq!(state.session_for(&path).unwrap_err(), again);

        let later = Session {
            expires_at_ms: signin::now_ms() + 3_600_000,
            ..soon
        };
        sessions.save(&account.id, &later).unwrap();
        assert_eq!(state.session_for(&path).unwrap(), "t");

        state.forget_session(&path);
        assert_eq!(sessions.load(&account.id), None);
        // An account that isn't connected has no session either.
        let other = "arbesk://0x00000000000000000000000000000000000000cd";
        assert_eq!(state.session_for(other).unwrap_err(), again);
    }

    #[test]
    fn a_collection_is_created_in_the_account() {
        let dir = tempfile::tempdir().unwrap();
        let mut world = testing::world();
        world.sessions.insert("t".into());
        let http = Arc::new(testing::arbesk(Arc::new(Mutex::new(world))));
        let (state, sessions) = state_with_sessions(http, dir.path());
        let account = state
            .accounts
            .lock()
            .unwrap()
            .connect("me@example.com", testing::ADDRESS, &testing::site())
            .unwrap();
        sessions
            .save(
                &account.id,
                &Session {
                    token: "t".into(),
                    expires_at_ms: signin::now_ms() + 3_600_000,
                },
            )
            .unwrap();
        let root = format!("arbesk://{}", testing::ADDRESS);
        let path = state.create_collection(&root, "My Set").unwrap();
        let token = create::named_collection_id(testing::ADDRESS, "My Set").unwrap();
        assert_eq!(
            path,
            location::collection_path(testing::ADDRESS, &token, "My Set")
        );
        assert!(
            state
                .list(&root)
                .unwrap()
                .iter()
                .any(|entry| entry.name == "My Set")
        );
        assert_eq!(
            state.create_collection(&root, "My Set").unwrap_err(),
            "A collection named “My Set” already exists."
        );
        assert_eq!(
            state.create_collection(&path, "Another").unwrap_err(),
            "Create collections in the account's folder."
        );
        assert_eq!(
            state.create_collection(&root, "   ").unwrap_err(),
            create::NO_NAME
        );
    }

    #[test]
    fn a_collection_is_shared_with_an_editor_or_viewer() {
        let dir = tempfile::tempdir().unwrap();
        let mut world = testing::world();
        world.sessions.insert("t".into());
        let http = Arc::new(testing::arbesk(Arc::new(Mutex::new(world))));
        let (state, sessions) = state_with_sessions(http, dir.path());
        let account = state
            .accounts
            .lock()
            .unwrap()
            .connect("me@example.com", testing::ADDRESS, &testing::site())
            .unwrap();
        sessions
            .save(
                &account.id,
                &Session {
                    token: "t".into(),
                    expires_at_ms: signin::now_ms() + 3_600_000,
                },
            )
            .unwrap();
        let shapes = format!("arbesk://{}/1~Shapes", testing::ADDRESS);
        let shared = state
            .share_collection(&shapes, "alice@example.com", 1)
            .unwrap();
        assert_eq!(
            (shared.address.as_str(), shared.role, shared.added),
            (testing::ALICE, 1, true)
        );
        assert_eq!(
            state.share_collection(&shapes, "alice@example.com", 1).unwrap_err(),
            "That account is already a viewer."
        );
        assert_eq!(
            state
                .share_collection(&shapes, "alice@example.com", 2)
                .unwrap()
                .role,
            2
        );
        let root = format!("arbesk://{}", testing::ADDRESS);
        assert_eq!(
            state.share_collection(&root, "alice@example.com", 1).unwrap_err(),
            "Share a collection."
        );
        assert_eq!(
            state
                .share_collection(&format!("{shapes}/a1~Box.gltf"), "alice@example.com", 1)
                .unwrap_err(),
            "Share a collection."
        );
    }

    #[test]
    fn exists_knows_a_listed_name() {
        let dir = tempfile::tempdir().unwrap();
        let state = Client::with_http(
            Arc::new(testing::arbesk(Arc::new(Mutex::new(testing::world())))),
            dir.path().join("cache"),
            dir.path().join("a.json"),
        );
        state.accounts.lock().unwrap().connect("me@example.com", testing::ADDRESS, &testing::site()).unwrap();
        let shapes = testing::shapes();
        assert!(state.exists(&format!("{shapes}/~Box.gltf")));
        assert!(!state.exists(&format!("{shapes}/~New.gltf")));
        assert!(!state.exists(&shapes));
        assert!(!state.exists("arbesk://bad/~x"));
    }

    #[test]
    fn a_401_forgets_the_session() {
        let dir = tempfile::tempdir().unwrap();
        let mut world = testing::world();
        world.relay = testing::Relay::Status(401);
        world.sessions.insert("t".into());
        let http = Arc::new(testing::arbesk(Arc::new(Mutex::new(world))));
        let (state, sessions) = state_with_sessions(http, dir.path());
        let account = state.accounts.lock().unwrap().connect("me@example.com", testing::ADDRESS, &testing::site()).unwrap();
        let kept = Session { token: "t".into(), expires_at_ms: signin::now_ms() + 3_600_000 };
        sessions.save(&account.id, &kept).unwrap();
        let file = testing::fixture("glb-box").join("input.glb");
        let to = format!("{}/~Fresh.glb", testing::shapes());
        let error = state.add(&to, &file, false, &Progress::default()).unwrap_err();
        assert_eq!(error, write::SIGN_IN);
        assert_eq!(sessions.load(&account.id), None);
        // Without a session nothing is asked.
        assert_eq!(state.add(&to, &file, false, &Progress::default()).unwrap_err(), write::SIGN_IN);
    }

    #[test]
    fn cancelling_while_arbesk_confirms_connects_nothing() {
        let dir = tempfile::tempdir().unwrap();
        // Cancel arrives while Arbesk is being asked to confirm the account.
        let cancel: Arc<std::sync::OnceLock<(Arc<Client>, String)>> = Arc::default();
        let on_confirm = cancel.clone();
        let http = Arc::new(FakeHttp::new(move |request| {
            if request.method == "POST" {
                let (state, flow_id) = on_confirm.get().unwrap();
                state.connect_cancel(flow_id);
                return Ok(serde_json::json!({ "exists": true, "address": ADDRESS })
                    .to_string()
                    .into_bytes());
            }
            Ok(b"{}".to_vec())
        }));
        let state = Arc::new(Client::with_http(
            http,
            dir.path().join("cache"),
            dir.path().join("a.json"),
        ));
        let (flow_id, url) = state.connect_start("me@example.com").unwrap();
        cancel.set((state.clone(), flow_id.clone())).ok().unwrap();
        let browser = browser_returns(&url, "me%40example.com");
        assert_eq!(
            state.connect_finish(&flow_id).unwrap_err(),
            "Sign-in was cancelled."
        );
        browser.join().unwrap();
        assert!(state.accounts().is_empty());
    }

    #[test]
    fn a_cancelled_sign_in_connects_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let state = Client::with_http(
            confirming(),
            dir.path().join("cache"),
            dir.path().join("a.json"),
        );
        let (flow_id, _) = state.connect_start("me@example.com").unwrap();
        state.connect_cancel(&flow_id);
        assert_eq!(
            state.connect_finish(&flow_id).unwrap_err(),
            "This sign-in attempt is no longer active."
        );
        assert!(state.accounts().is_empty());
    }

    #[test]
    fn an_account_that_is_not_connected_is_not_listed() {
        let dir = tempfile::tempdir().unwrap();
        let http = Arc::new(FakeHttp::new(|_| {
            panic!("nothing is asked before the account is known")
        }));
        let state =
            Client::with_http(http, dir.path().join("cache"), dir.path().join("a.json"));
        let path = "arbesk://0x00000000000000000000000000000000000000ab";
        assert_eq!(
            state.list(path).unwrap_err(),
            "Connect this Arbesk account first."
        );
        assert!(
            state
                .build(&format!("{path}/1~S/a~B.gltf"), &Progress::default())
                .is_err()
        );
        assert!(state.list("arbesk://bad").is_err());
        // Its thumbnails are empty, as for any file without one.
        assert!(
            state
                .thumbnail(&format!("{path}/1~S/a~B.gltf"), 256, &|| false)
                .is_empty()
        );
        assert!(state.thumbnail("arbesk://bad", 256, &|| false).is_empty());
    }
}
