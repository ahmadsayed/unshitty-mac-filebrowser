//! Arbesk (promptscad.com) accounts in Network, read only: sign in with an
//! email and the code Arbesk emails, then browse the account's collections
//! as folders and their assets as glTF and 3MF files. Arbesk's read path is
//! rebuilt here from public sources (the backend's config and token index,
//! the chain's `tokenURI`, the IPFS gateway); nothing is written to Arbesk.

pub mod accounts;
pub mod chain;
pub mod compose;
pub mod http;
pub mod ipfs;
pub mod location;
pub mod service;
pub mod signin;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};

use crate::arbesk::accounts::{Accounts, ArbeskAccount};
use crate::arbesk::chain::Site;
use crate::arbesk::http::{Http, ReqwestHttp};
use crate::arbesk::location::parse;
use crate::arbesk::signin::{CALLBACK_TIMEOUT, Flow};
use crate::fs_ops::Entry;
use crate::progress::Progress;

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

/// What the app keeps for Arbesk: one HTTP client, the cache folder, the
/// connected accounts and the sign-ins waiting for the browser.
pub struct ArbeskState {
    http: Result<Arc<dyn Http>, String>,
    cache_dir: PathBuf,
    accounts: Mutex<Accounts>,
    flows: Mutex<HashMap<String, Arc<Flow>>>,
    builds: service::Builds,
    thumbnails: service::Thumbnails,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl ArbeskState {
    pub fn new(cache_dir: PathBuf, accounts_file: PathBuf) -> Self {
        let http = ReqwestHttp::new().map(|http| Arc::new(http) as Arc<dyn Http>);
        Self::with(http, cache_dir, accounts_file)
    }

    #[cfg(test)]
    pub fn with_http(http: Arc<dyn Http>, cache_dir: PathBuf, accounts_file: PathBuf) -> Self {
        Self::with(Ok(http), cache_dir, accounts_file)
    }

    fn with(
        http: Result<Arc<dyn Http>, String>,
        cache_dir: PathBuf,
        accounts_file: PathBuf,
    ) -> Self {
        let accounts = Mutex::new(Accounts::load(accounts_file));
        Self {
            http,
            cache_dir,
            accounts,
            flows: Mutex::new(HashMap::new()),
            builds: service::Builds::default(),
            thumbnails: service::Thumbnails::default(),
        }
    }

    fn http(&self) -> Result<&dyn Http, String> {
        self.http.as_deref().map_err(Clone::clone)
    }

    pub fn accounts(&self) -> Vec<ArbeskAccount> {
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

    /// `service::list` for the account `path` belongs to.
    pub fn list(&self, path: &str) -> Result<Vec<Entry>, String> {
        let site = self.site_for(parse(path)?.address())?;
        service::list(self.http()?, &site, path, &self.cache_dir, &self.thumbnails)
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
    pub fn connect_finish(&self, flow_id: &str) -> Result<ArbeskAccount, String> {
        let flow = lock(&self.flows)
            .get(flow_id)
            .cloned()
            .ok_or("This sign-in attempt is no longer active.")?;
        flow.claim()?;
        let site = sign_in_site();
        let signed_in = signin::finish(&flow, self.http()?, &site, CALLBACK_TIMEOUT);
        lock(&self.flows).remove(flow_id);
        let (email, address) = signed_in?;
        // Cancel may have come while Arbesk was confirming: it still cancels.
        if flow.is_cancelled() {
            return Err("Sign-in was cancelled.".into());
        }
        lock(&self.accounts).connect(&email, &address, &site)
    }

    pub fn connect_cancel(&self, flow_id: &str) {
        if let Some(flow) = lock(&self.flows).remove(flow_id) {
            flow.cancel();
        }
    }

    /// Forget the account; nothing on Arbesk changes.
    pub fn disconnect(&self, id: &str) -> Result<(), String> {
        lock(&self.accounts).remove(id)?;
        // Where its assets' thumbnails are is no longer needed.
        self.thumbnails.clear();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arbesk::http::fake::FakeHttp;

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
        let state = ArbeskState::with_http(
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

    #[test]
    fn cancelling_while_arbesk_confirms_connects_nothing() {
        let dir = tempfile::tempdir().unwrap();
        // Cancel arrives while Arbesk is being asked to confirm the account.
        let cancel: Arc<std::sync::OnceLock<(Arc<ArbeskState>, String)>> = Arc::default();
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
        let state = Arc::new(ArbeskState::with_http(
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
        let state = ArbeskState::with_http(
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
            ArbeskState::with_http(http, dir.path().join("cache"), dir.path().join("a.json"));
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
