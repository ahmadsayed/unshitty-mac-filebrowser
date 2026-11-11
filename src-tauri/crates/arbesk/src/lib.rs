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
mod attach;
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
mod limiter;
mod location;
mod memo;
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
pub use attach::{Adding, Attached, Choice, Clash, PlannedFile};
pub use location::{ArbeskPath as Location, is_arbesk, is_folder, parse, parse_target};
pub use progress::{CANCELLED, Phase, Progress, Snapshot};
pub use representations::RepresentationInfo;
pub use provider::SCHEME;
pub use scene::{Scene, SceneAt};
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
    /// Every read's permits, shared by all of them (performance plan,
    /// phase 2); writes read without it, as before.
    limiter: limiter::Limiter,
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
            limiter: limiter::Limiter::default(),
        }
    }

    fn http(&self) -> Result<&dyn Http, String> {
        self.http.as_deref().map_err(Clone::clone)
    }

    /// `read` handed the client through the app's limiter, its waits for a
    /// permit stopped by `stop`.
    fn limited<T>(
        &self,
        stop: limiter::Stop,
        read: impl FnOnce(&dyn Http) -> Result<T, String>,
    ) -> Result<T, String> {
        read(&limiter::Limited::new(self.http()?, &self.limiter, stop))
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

    /// How to reach the account `path` belongs to: its site, checking the
    /// client could be made.
    fn reach(&self, path: &str) -> Result<Site, String> {
        let site = self.site_for(parse(path)?.address())?;
        self.http()?;
        Ok(site)
    }

    /// `service::list` for the account `path` belongs to.
    /// Every listing (Reload is one) reads its collections' `tokenURI`
    /// afresh: the memo is forgotten first.
    pub fn list(&self, path: &str) -> Result<Vec<Entry>, String> {
        let site = self.reach(path)?;
        self.forget_tokens();
        self.limited(&limiter::never, |http| {
            service::list(http, &site, path, &self.cache_dir, &self.thumbnails)
        })
    }

    /// `service::thumbnail` for the account `path` belongs to, as PNG bytes:
    /// empty when the asset has none, as the `thumbnail` command answers.
    pub fn thumbnail(&self, path: &str, size: u32, stop: &dyn Fn() -> bool) -> Vec<u8> {
        let site = parse(path).and_then(|path| self.site_for(path.address()));
        let (Ok(site), Ok(http)) = (site, self.http()) else {
            return Vec::new();
        };
        // The view's stop is for this thread only: the limiter's waits hear
        // it through `bridged`.
        limiter::bridged(stop, |stop| {
            let http = limiter::Limited::new(http, &self.limiter, stop);
            service::thumbnail(
                &http,
                &site,
                path,
                size,
                &self.cache_dir,
                &self.thumbnails,
                stop,
            )
            .unwrap_or_default()
        })
    }

    /// The Studio link of the asset at `path`, on its account's deployment.
    pub fn studio_url(&self, path: &str) -> Result<String, String> {
        service::studio_url(&self.site_for(parse(path)?.address())?, path)
    }

    /// `service::build` for the account `path` belongs to.
    pub fn build(&self, path: &str, progress: &Progress) -> Result<PathBuf, String> {
        let site = self.site_for(parse(path)?.address())?;
        self.limited(&|| progress.is_cancelled(), |http| {
            service::build(http, &site, path, &self.cache_dir, progress, &self.builds)
        })
    }

    /// `service::export` for the account `path` belongs to: an asset's
    /// own file built in the app cache; for an asset's other file (spec
    /// F4), that file fetched and decoded there (`file_at`, spec F9), a
    /// pinned path refused once its entry holds another file.
    pub fn export(&self, path: &str, progress: &Progress) -> Result<PathBuf, String> {
        if let location::ArbeskPath::File { .. } = parse(path)? {
            return self.file_at(path, progress);
        }
        let site = self.site_for(parse(path)?.address())?;
        self.limited(&|| progress.is_cancelled(), |http| {
            service::export(http, &site, path, &self.cache_dir, progress, &self.builds)
        })
    }

    /// `service::representations` for the account `path` belongs to: the
    /// asset's other files (at `version`, the current save without).
    pub fn representations(
        &self,
        path: &str,
        version: Option<&str>,
    ) -> Result<Vec<RepresentationInfo>, String> {
        let site = self.reach(path)?;
        self.limited(&limiter::never, |http| {
            service::representations(http, &site, path, version, &self.cache_dir)
        })
    }

    /// What the Properties dialog shows of the asset at `path`
    /// (`service::properties`, spec F7).
    pub fn properties(&self, path: &str) -> Result<places::FileProperties, String> {
        let site = self.reach(path)?;
        self.limited(&limiter::never, |http| {
            service::properties(http, &site, path, &self.cache_dir)
        })
    }

    /// The asset at `path`'s other file `id` (at `version`, the current
    /// save without), decoded into the app cache (`service::file`).
    pub fn representation_file(
        &self,
        path: &str,
        version: Option<&str>,
        id: &str,
        progress: &Progress,
    ) -> Result<PathBuf, String> {
        let wanted = service::Wanted { asset: path, version, id, pin: None };
        self.file(&wanted, progress)
    }

    /// The file an asset's other file's path names (spec F4), decoded into
    /// the app cache: the current save's entry, refused when the path pins
    /// a CID the entry no longer has.
    pub fn file_at(&self, path: &str, progress: &Progress) -> Result<PathBuf, String> {
        let location::ArbeskPath::File { id, pin, .. } = parse(path)? else {
            return Err(format!("“{path}” is not one of an Arbesk asset's files."));
        };
        let (asset, name) = location::file_parts(path).ok_or("This file's name can't be read.")?;
        let pin = pin.as_deref().map(|cid| (cid, name.as_str()));
        let wanted = service::Wanted { asset, version: None, id: &id, pin };
        self.file(&wanted, progress)
    }

    fn file(&self, wanted: &service::Wanted, progress: &Progress) -> Result<PathBuf, String> {
        let site = self.reach(wanted.asset)?;
        self.limited(&|| progress.is_cancelled(), |http| {
            service::file(http, &site, wanted, &self.cache_dir, progress, &self.builds)
        })
    }

    /// `service::versions` for the account `path` belongs to.
    pub fn versions(&self, path: &str) -> Result<Vec<service::Version>, String> {
        let site = self.reach(path)?;
        self.limited(&limiter::never, |http| {
            service::versions(http, &site, path, &self.cache_dir)
        })
    }

    /// `service::scene_at` for the account `path` belongs to: the scene and
    /// the save's CID it was resolved from, kept for `part` under that CID
    /// only (the preview always asks for the parts by it; a part asked for
    /// without a version resolves the newest scene again).
    pub fn scene(&self, path: &str, version: Option<&str>) -> Result<scene::SceneAt, String> {
        let site = self.reach(path)?;
        let (cid, resolved) = self.limited(&limiter::never, |http| {
            service::scene_at(http, &site, path, version, &self.cache_dir)
        })?;
        let scene = self.scenes.keep(&kept(path, Some(&cid)), resolved);
        Ok(scene::SceneAt {
            cid,
            scene: scene.as_ref().clone(),
        })
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
        let site = self.reach(path)?;
        // The scene's reads hear the part's stop, so a Cancel while they
        // queue for permits is heard at once. Resolving drops what it cannot
        // read, and a read is only cancelled once the stop says so (it never
        // goes back), so a scene resolved while stopped is never kept: it
        // may be missing parts.
        let scene = self.scenes.get_or_keep(&kept(path, version), || {
            let scene = self.limited(&|| progress.is_cancelled(), |http| {
                service::scene(http, &site, path, version, &self.cache_dir)
            })?;
            if progress.is_cancelled() {
                return Err(CANCELLED.into());
            }
            Ok(scene)
        })?;
        self.limited(&|| progress.is_cancelled(), |http| {
            service::part(http, &site, &scene, key, &self.cache_dir, progress, &self.builds)
        })
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
        // A save or Replace moved the collection (or may still land).
        self.forget_tokens();
        if added.as_ref().err().is_some_and(|error| error == write::SIGN_IN) {
            self.forget_session(&account);
        }
        added
    }

    /// What adding the local `files` to the asset at `path` would do
    /// (`attach::plan`): each file as it would be recorded, and its clash.
    pub fn plan_attach(&self, path: &str, files: &[String]) -> Result<Vec<PlannedFile>, String> {
        // The planning of a write: unlimited, as the write is.
        let site = self.reach(path)?;
        attach::plan(self.http()?, &site, path, files, &self.cache_dir)
    }

    /// Add the local files `adding` to the asset at `path` as one new
    /// version (`attach::attach`, spec F10, F11). Arbesk's 401 ends the
    /// session.
    pub fn attach(&self, path: &str, adding: &[Adding], progress: &Progress) -> Result<Attached, String> {
        self.writing(path, attach::SIGN_IN, progress, |writer| attach::attach(writer, path, adding))
    }

    /// Remove the file `id` from the asset at `path` as one new version,
    /// only while it is still the file `cid` the user saw
    /// (`attach::detach`, spec F12). Arbesk's 401 ends the session.
    pub fn detach(&self, path: &str, id: &str, cid: &str, progress: &Progress) -> Result<Attached, String> {
        self.writing(path, attach::SIGN_IN_REMOVE, progress, |writer| attach::detach(writer, path, id, cid))
    }

    /// `work` as the account `path` belongs to, signed in: a missing
    /// session or Arbesk's 401 says `sign_in` and ends the session.
    fn writing<T>(
        &self,
        path: &str,
        sign_in: &str,
        progress: &Progress,
        work: impl FnOnce(&attach::Writer) -> Result<T, String>,
    ) -> Result<T, String> {
        let address = parse(path)?.address().to_string();
        let site = self.site_for(&address)?;
        let account = location::account_path(&address);
        let session = self.session_for(&account).map_err(|_| sign_in.to_string())?;
        let writer = attach::Writer {
            http: self.http()?,
            site: &site,
            session: &session,
            cache_dir: &self.cache_dir,
            locks: &self.locks,
            progress,
        };
        let done = work(&writer).map_err(|error| match error.as_str() {
            write::SIGN_IN => sign_in.to_string(),
            _ => error,
        });
        // An attach or detach moved the collection (or may still land).
        self.forget_tokens();
        if done.as_ref().err().is_some_and(|error| error == sign_in) {
            self.forget_session(&account);
        }
        done
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
        // A new collection's token now answers.
        self.forget_tokens();
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
        // A share moves the editor list, not `tokenURI`; forgotten all the
        // same, as after every write.
        self.forget_tokens();
        if shared.as_ref().err().is_some_and(|error| error == share::SIGN_IN) {
            self.forget_session(&account);
        }
        shared
    }

    /// Whether the collection of the pending name `path` lists that name:
    /// its names only (`service::lists_name`), no listing's rows or
    /// thumbnails (performance plan, phase 4).
    pub fn exists(&self, path: &str) -> bool {
        let Ok((address, token_id, name)) = parse_target(path) else {
            return false;
        };
        let collection = location::collection_path(&address, &token_id, "");
        let Ok(site) = self.reach(&collection) else {
            return false;
        };
        self.limited(&limiter::never, |http| {
            service::lists_name(http, &site, &token_id, &name, &self.cache_dir)
        })
        .unwrap_or(false)
    }

    /// Forget the `tokenURI` memo (performance plan, phase 4): after this
    /// app's own write, which may have moved a collection, and before a
    /// listing, so Reload reads every collection afresh.
    fn forget_tokens(&self) {
        self.limiter.tokens.clear();
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

    /// A client over `http` (the fake Arbesk) with `testing::ADDRESS`
    /// connected.
    fn connected(http: Arc<dyn Http>, dir: &std::path::Path) -> Client {
        let client = Client::with_http(http, dir.join("cache"), dir.join("a.json"));
        lock(&client.accounts)
            .connect("me@example.com", testing::ADDRESS, &testing::site())
            .unwrap();
        client
    }

    /// Performance plan, phase 2: a listing, a build and four thumbnails
    /// at once never have more than `limiter::READS` gateway reads in
    /// flight between them (before, 8 + 2 + 4).
    #[test]
    fn every_read_shares_one_limit() {
        let dir = tempfile::tempdir().unwrap();
        let delay = std::time::Duration::from_millis(20);
        let http = Arc::new(testing::Watched::new(testing::busy_world(), delay, &[]));
        let client = connected(http.clone(), dir.path());
        let many = format!("arbesk://{ADDRESS}/7~Many");
        let wide = format!("{many}/w~Wide.gltf");
        let (listed, built, thumbnails) = std::thread::scope(|scope| {
            let listed = scope.spawn(|| client.list(&many));
            let built = scope.spawn(|| client.build(&wide, &Progress::default()));
            let thumbnails: Vec<_> = (0..4)
                .map(|i| {
                    let (client, many) = (&client, &many);
                    scope.spawn(move || client.thumbnail(&format!("{many}/m{i}~Part{i}.gltf"), 64, &|| false))
                })
                .collect();
            let thumbnails: Vec<Vec<u8>> = thumbnails.into_iter().map(|t| t.join().unwrap()).collect();
            (listed.join().unwrap(), built.join().unwrap(), thumbnails)
        });
        assert_eq!(listed.unwrap().len(), 17);
        assert!(built.unwrap().is_file());
        assert!(thumbnails.iter().all(|png| png.starts_with(b"\x89PNG")));
        let peak = http.peak.load(std::sync::atomic::Ordering::SeqCst);
        assert!(peak <= limiter::READS, "{peak} reads in flight at once");
        assert!(peak > 2, "{peak}: the reads ran together");
        assert_eq!(client.limiter.reading(), 0);
    }

    /// A build queued behind a listing that holds every read permit hears
    /// its cancel while it waits, and reads nothing.
    #[test]
    fn a_cancel_while_waiting_for_a_permit_returns_at_once() {
        let dir = tempfile::tempdir().unwrap();
        let held: Vec<String> = (0..16).map(|i| format!("bafyMany{i}")).collect();
        let delay = std::time::Duration::ZERO;
        let http = Arc::new(testing::Watched::new(testing::busy_world(), delay, &held));
        let client = connected(http.clone(), dir.path());
        let many = format!("arbesk://{ADDRESS}/7~Many");
        std::thread::scope(|scope| {
            let listed = scope.spawn(|| client.list(&many));
            assert!(http.wait_running(limiter::READS), "the listing holds every permit");
            let progress = Progress::default();
            let cancel = progress.clone();
            let started = std::time::Instant::now();
            scope.spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(100));
                cancel.cancel();
            });
            let chair = format!("arbesk://{ADDRESS}/5~History/h1~Chair.gltf");
            assert_eq!(client.build(&chair, &progress).unwrap_err(), CANCELLED);
            assert!(started.elapsed() < std::time::Duration::from_secs(1), "{:?}", started.elapsed());
            assert_eq!(http.peak.load(std::sync::atomic::Ordering::SeqCst), limiter::READS);
            http.release();
            assert_eq!(listed.join().unwrap().unwrap().len(), 17);
        });
        assert_eq!(client.limiter.reading(), 0);
    }

    /// A listing's chain calls take their own permits: never more than
    /// `limiter::RPCS` at once, though six collections are read together.
    #[test]
    fn chain_calls_never_pass_their_own_limit() {
        let dir = tempfile::tempdir().unwrap();
        let mut world = testing::busy_world();
        world.owned = ["1", "2", "3", "4", "5", "7"].map(String::from).to_vec();
        let delay = std::time::Duration::from_millis(30);
        let http = Arc::new(testing::Watched::new(world, delay, &[]));
        let client = connected(http.clone(), dir.path());
        assert!(client.list(&format!("arbesk://{ADDRESS}")).is_ok());
        let peak = http.rpc_peak.load(std::sync::atomic::Ordering::SeqCst);
        assert!(peak <= limiter::RPCS, "{peak} chain calls at once");
        assert!(peak > 1, "{peak}: the calls ran together");
    }

    /// An asset's other file is downloaded under a read permit: queued
    /// behind a listing that holds them all, it hears its cancel and never
    /// starts its download.
    #[test]
    fn a_file_download_waits_for_a_read_permit() {
        let dir = tempfile::tempdir().unwrap();
        let held: Vec<String> = (1..16).map(|i| format!("bafyMany{i}")).collect();
        let http = Arc::new(testing::Watched::new(testing::busy_world(), std::time::Duration::ZERO, &held));
        let client = connected(http.clone(), dir.path());
        let many = format!("arbesk://{ADDRESS}/7~Many");
        let part0 = format!("{many}/m0~Part0.gltf");
        // Part0's manifest is read (and kept) before the listing holds every permit.
        assert!(client.thumbnail(&part0, 64, &|| false).starts_with(b"\x89PNG"));
        std::thread::scope(|scope| {
            let listed = scope.spawn(|| client.list(&many));
            assert!(http.wait_running(limiter::READS), "the listing holds every permit");
            let progress = Progress::default();
            let cancel = progress.clone();
            scope.spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(100));
                cancel.cancel();
            });
            let started = std::time::Instant::now();
            let file = client.representation_file(&part0, None, "notes", &progress);
            let took = started.elapsed();
            let streamed = http.streamed.load(std::sync::atomic::Ordering::SeqCst);
            // Released before anything is checked, so a failure never hangs.
            http.release();
            assert_eq!(file.unwrap_err(), CANCELLED);
            assert!(took < std::time::Duration::from_secs(1), "{took:?}");
            assert_eq!(streamed, 0);
            assert!(listed.join().unwrap().is_ok());
        });
        let file = client.representation_file(&part0, None, "notes", &Progress::default()).unwrap();
        assert_eq!(std::fs::read(file).unwrap(), b"Notes on Part0.");
        assert_eq!(client.limiter.reading(), 0);
    }

    /// A part cancelled while its scene's reads wait for a permit hears its
    /// cancel at once, behind a listing that never lets go, and keeps no
    /// scene with parts missing: its linked child's part is still there for
    /// the next ask.
    #[test]
    fn a_cancelled_part_keeps_no_scene_missing_parts() {
        let dir = tempfile::tempdir().unwrap();
        let held: Vec<String> = (0..16).map(|i| format!("bafyMany{i}")).collect();
        let http = Arc::new(testing::Watched::new(testing::busy_world(), std::time::Duration::ZERO, &held));
        let client = connected(http.clone(), dir.path());
        let rooms = format!("arbesk://{ADDRESS}/4~Rooms");
        let table = format!("{rooms}/t1~Table.gltf");
        // Rooms' and Table's manifests are read (and kept) first: only the
        // scene's own reads, its linked child's among them, queue below.
        assert!(client.list(&rooms).is_ok());
        let many = format!("arbesk://{ADDRESS}/7~Many");
        std::thread::scope(|scope| {
            let listed = scope.spawn(|| client.list(&many));
            assert!(http.wait_running(limiter::READS), "the listing holds every permit");
            let progress = Progress::default();
            let cancel = progress.clone();
            let late = http.clone();
            scope.spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(100));
                cancel.cancel();
                // Only so a part that never hears its cancel fails, not hangs.
                std::thread::sleep(std::time::Duration::from_secs(3));
                late.release();
            });
            let started = std::time::Instant::now();
            let part = client.part(&table, None, "bafySrcTable", &progress);
            let took = started.elapsed();
            // Released only once the part has answered: its cancel never
            // waited for the listing.
            http.release();
            assert_eq!(part.unwrap_err(), CANCELLED);
            let bound = std::time::Duration::from_millis(100) + 2 * crate::http::TICK;
            assert!(took < bound, "{took:?}");
            assert!(listed.join().unwrap().is_ok());
        });
        let child = client.part(&table, None, "bafySrcGltf", &Progress::default());
        assert!(child.unwrap().is_file());
        assert_eq!(client.limiter.reading(), 0);
    }

    /// One read that fails in a listing's batch still fails the listing,
    /// as before, and gives its permit back.
    #[test]
    fn one_unreachable_read_still_fails_the_listing() {
        let dir = tempfile::tempdir().unwrap();
        let mut world = testing::busy_world();
        world.failing.insert("bafyMany5".into());
        let http = Arc::new(testing::Watched::new(world, std::time::Duration::ZERO, &[]));
        let client = connected(http, dir.path());
        let error = client.list(&format!("arbesk://{ADDRESS}/7~Many")).unwrap_err();
        assert!(error.starts_with(ipfs::STORAGE_UNREACHABLE), "{error}");
        assert_eq!(client.limiter.reading(), 0);
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

    /// How many `tokenURI` calls `http` has answered.
    fn token_reads(http: &FakeHttp) -> usize {
        http.requests()
            .iter()
            .filter(|request| request.url == "https://rpc.test")
            .filter(|request| {
                request.body.as_ref().and_then(|body| body["params"][0]["data"].as_str())
                    .is_some_and(|data| data.starts_with("0xc87b56dd"))
            })
            .count()
    }

    /// A client over the fake Arbesk `world`, signed in with session "t".
    fn signed_in(world: testing::World, dir: &std::path::Path) -> (Client, Arc<FakeHttp>, Arc<Mutex<testing::World>>) {
        let world = Arc::new(Mutex::new(world));
        world.lock().unwrap().sessions.insert("t".into());
        let http = Arc::new(testing::arbesk(world.clone()));
        let (state, sessions) = state_with_sessions(http.clone(), dir);
        let account = lock(&state.accounts).connect("me@example.com", ADDRESS, &testing::site()).unwrap();
        let kept = Session { token: "t".into(), expires_at_ms: signin::now_ms() + 3_600_000 };
        sessions.save(&account.id, &kept).unwrap();
        (state, http, world)
    }

    /// Performance plan, phase 4: an operation's calls for one collection
    /// (the copy-out plan's assets, a preview's scene, saves and files)
    /// share one `tokenURI` read; each before read it again.
    #[test]
    fn repeated_calls_for_one_collection_read_its_token_once() {
        let dir = tempfile::tempdir().unwrap();
        let (state, http, _) = signed_in(testing::world(), dir.path());
        let shapes = testing::shapes();
        for asset in ["a1~Box.gltf", "a2~Box.3mf", "a4~Odd"] {
            assert!(state.representations(&format!("{shapes}/{asset}"), None).is_ok(), "{asset}");
        }
        assert_eq!(token_reads(&http), 1);
        let box_ = format!("{shapes}/a1~Box.gltf");
        let scene = state.scene(&box_, None).unwrap();
        assert!(state.versions(&box_).is_ok());
        assert!(state.properties(&box_).is_ok());
        assert_eq!(state.representations(&box_, Some(&scene.cid)).unwrap(), []);
        assert_eq!(token_reads(&http), 1);
    }

    /// A scene's links to one collection, and the scene's own collection,
    /// cost one `tokenURI` read each, though the links are read together.
    #[test]
    fn a_scenes_links_to_one_collection_read_its_token_once() {
        let dir = tempfile::tempdir().unwrap();
        let (state, http, _) = signed_in(testing::world(), dir.path());
        let table = format!("arbesk://{ADDRESS}/4~Rooms/t1~Table.gltf");
        let scene = state.scene(&table, None).unwrap();
        assert!(scene.parts.len() >= 2, "{:?}", scene.parts.len());
        // Rooms, and Shapes through the links' own contract.
        assert_eq!(token_reads(&http), 2);
        assert!(state.versions(&table).is_ok());
        assert_eq!(token_reads(&http), 2);
    }

    /// Reload (any listing) reads `tokenURI` afresh, and so does a pending
    /// name's check; neither is answered from the memo.
    #[test]
    fn a_listing_and_a_name_check_read_the_token_afresh() {
        let dir = tempfile::tempdir().unwrap();
        let (state, http, world) = signed_in(testing::world(), dir.path());
        let shapes = testing::shapes();
        let box_ = format!("{shapes}/a1~Box.gltf");
        assert!(state.representations(&box_, None).is_ok());
        // Another app adds an asset: the memo still answers the old save.
        {
            let mut w = world.lock().unwrap();
            let mut collection: serde_json::Value = serde_json::from_slice(&w.files["bafyC1"]).unwrap();
            collection["assets"]["a9"] = serde_json::json!("bafyA1");
            w.put("bafyC1b", collection);
            w.tokens.insert("1".into(), "bafyC1b".into());
        }
        assert!(state.representations(&box_, None).is_ok());
        assert_eq!(token_reads(&http), 1);
        assert!(state.exists(&format!("{shapes}/~Box.gltf")));
        assert_eq!(token_reads(&http), 2, "the name check reads afresh");
        let listed = state.list(&shapes).unwrap();
        assert_eq!(token_reads(&http), 3, "Reload reads afresh");
        assert!(listed.iter().any(|entry| entry.path.contains("/a9~")), "{listed:?}");
        // The listing's fresh answer serves the next calls.
        assert!(state.representations(&box_, None).is_ok());
        assert_eq!(token_reads(&http), 3);
    }

    /// A write never trusts the memo: its compare-and-swap reads
    /// `tokenURI` under the collection hold and builds on a save another
    /// app made meanwhile; and it clears the memo, so the next read sees
    /// what it wrote.
    #[test]
    fn a_write_reads_the_token_itself_and_clears_the_memo() {
        let dir = tempfile::tempdir().unwrap();
        let (state, http, world) = signed_in(testing::world(), dir.path());
        let shapes = testing::shapes();
        let box_ = format!("{shapes}/a1~Box.gltf");
        assert_eq!(state.representations(&box_, None).unwrap(), []);
        {
            let mut w = world.lock().unwrap();
            let mut collection: serde_json::Value = serde_json::from_slice(&w.files["bafyC1"]).unwrap();
            collection["assets"]["a9"] = serde_json::json!("bafyA2");
            w.put("bafyC1b", collection);
            w.tokens.insert("1".into(), "bafyC1b".into());
        }
        // Reads still have the memo's answer from before that save.
        assert_eq!(state.representations(&box_, None).unwrap(), []);
        assert_eq!(token_reads(&http), 1);
        let before = token_reads(&http);
        let file = dir.path().join("Box.stl");
        std::fs::write(&file, b"solid").unwrap();
        let adding = [Adding { path: file.to_string_lossy().into_owned(), choice: None }];
        state.attach(&box_, &adding, &Progress::default()).unwrap();
        assert!(token_reads(&http) > before, "the write read tokenURI itself");
        let (tip, collection) = {
            let w = world.lock().unwrap();
            let tip = w.tokens["1"].clone();
            let collection: serde_json::Value = serde_json::from_slice(&w.files[&tip]).unwrap();
            (tip, collection)
        };
        assert_ne!(tip, "bafyC1b");
        assert_eq!(collection["prev_asset_manifest_cid"], "bafyC1b", "built on the other app's save");
        assert_eq!(collection["assets"]["a9"], "bafyA2");
        let after = token_reads(&http);
        let files = state.representations(&box_, None).unwrap();
        assert_eq!(token_reads(&http), after + 1, "the write cleared the memo");
        assert_eq!(files.len(), 1);
    }
}
