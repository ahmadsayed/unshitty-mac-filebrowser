//! Arbesk (promptscad.com) accounts in Network, read only: sign in with an
//! email and the code Arbesk emails, then browse the account's collections
//! as folders and their assets as glTF and 3MF files. Arbesk's read path is
//! rebuilt here from public sources (the backend's config and token index,
//! the chain's `tokenURI`, the IPFS gateway); nothing is written to Arbesk.

pub mod chain;
pub mod compose;
pub mod http;
pub mod ipfs;
pub mod location;
pub mod service;

use std::path::PathBuf;
use std::sync::Arc;

use crate::arbesk::chain::Site;
use crate::arbesk::http::{Http, ReqwestHttp};
use crate::arbesk::location::parse;
use crate::fs_ops::Entry;
use crate::progress::Progress;

/// What the app keeps for Arbesk: one HTTP client and the cache folder.
pub struct ArbeskState {
    http: Result<Arc<dyn Http>, String>,
    cache_dir: PathBuf,
}

impl ArbeskState {
    pub fn new(cache_dir: PathBuf) -> Self {
        let http = ReqwestHttp::new().map(|http| Arc::new(http) as Arc<dyn Http>);
        Self { http, cache_dir }
    }

    #[cfg(test)]
    pub fn with_http(http: Arc<dyn Http>, cache_dir: PathBuf) -> Self {
        Self {
            http: Ok(http),
            cache_dir,
        }
    }

    fn http(&self) -> Result<&dyn Http, String> {
        self.http.as_deref().map_err(Clone::clone)
    }

    /// The deployment and chain of the account `address`.
    fn site_for(&self, _address: &str) -> Result<Site, String> {
        Err("Connect this Arbesk account first.".into())
    }

    /// `service::list` for the account `path` belongs to.
    pub fn list(&self, path: &str) -> Result<Vec<Entry>, String> {
        let site = self.site_for(parse(path)?.address())?;
        service::list(self.http()?, &site, path, &self.cache_dir)
    }

    /// `service::build` for the account `path` belongs to.
    pub fn build(&self, path: &str, progress: &Progress) -> Result<PathBuf, String> {
        let site = self.site_for(parse(path)?.address())?;
        service::build(self.http()?, &site, path, &self.cache_dir, progress)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arbesk::http::fake::FakeHttp;

    #[test]
    fn an_account_that_is_not_connected_is_not_listed() {
        let dir = tempfile::tempdir().unwrap();
        let http = Arc::new(FakeHttp::new(|_| {
            panic!("nothing is asked before the account is known")
        }));
        let state = ArbeskState::with_http(http, dir.path().into());
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
    }
}
