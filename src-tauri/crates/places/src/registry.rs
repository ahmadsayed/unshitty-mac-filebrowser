//! Which provider a path belongs to (D10): one provider per scheme, picked
//! by the exact, case-sensitive prefix `<scheme>://` at the start of the
//! path. Every other path (`file://…`, `smb://…`, `GDRIVE://…`, a relative
//! path, a local path that merely contains `gdrive://`) goes to the local
//! provider, which refuses what isn't an absolute local path in its own
//! words. A malformed path with a registered prefix still reaches its
//! provider, whose parser refuses it.

use std::sync::Arc;

use crate::Provider;

/// The providers by scheme, with this computer's as the fallback.
pub struct Registry {
    local: Arc<dyn Provider>,
    /// `(prefix, provider)`, the prefix being `<scheme>://`.
    remote: Vec<(String, Arc<dyn Provider>)>,
}

impl Registry {
    /// A registry with only this computer's provider, whose scheme is "".
    ///
    /// # Panics
    /// When `local`'s scheme isn't "": a wiring mistake, caught at startup.
    pub fn new(local: Arc<dyn Provider>) -> Self {
        assert_eq!(local.scheme(), "", "the local provider has the empty scheme");
        Registry { local, remote: Vec::new() }
    }

    /// Add `provider` for paths starting with `<its scheme>://`.
    ///
    /// # Panics
    /// When its scheme is "" or already registered: a wiring mistake, caught
    /// at startup.
    pub fn register(mut self, provider: Arc<dyn Provider>) -> Self {
        let scheme = provider.scheme();
        assert!(!scheme.is_empty(), "only the local provider has the empty scheme");
        let prefix = format!("{scheme}://");
        assert!(
            self.remote.iter().all(|(known, _)| *known != prefix),
            "{scheme} is registered twice"
        );
        self.remote.push((prefix, provider));
        self
    }

    /// The provider for `path`.
    pub fn provider_for(&self, path: &str) -> &dyn Provider {
        self.remote
            .iter()
            .find(|(prefix, _)| path.starts_with(prefix.as_str()))
            .map_or(&*self.local, |(_, provider)| &**provider)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Capabilities, Cx, Entry, Error, Fetch, Fetched, ListOptions, NewKind, Replace, Result, Trashed};

    /// A provider that only knows its scheme; every call says which it is.
    struct Named(&'static str);

    impl Named {
        fn refuse<T>(&self) -> Result<T> {
            Err(Error::Unsupported(format!("not available for {:?}", self.0)))
        }
    }

    impl Provider for Named {
        fn scheme(&self) -> &'static str {
            self.0
        }
        fn capabilities(&self, _: &str) -> Capabilities {
            Capabilities {
                read_only: true,
                can_create: false,
                can_rename: false,
                can_trash: false,
                can_copy_out: false,
                knows_sizes: false,
                knows_counts: false,
                searchable: false,
                has_thumbnails: false,
                rename_atomic: false,
                replace: Replace::Trash,
                names_unique: true,
                put_is_whole: false,
                refused: Vec::new(),
            }
        }
        fn list(&self, _: &str, _: &ListOptions, _: &Cx) -> Result<Vec<Entry>> {
            self.refuse()
        }
        fn stat(&self, _: &str, _: &Cx) -> Result<Option<Entry>> {
            self.refuse()
        }
        fn lookup(&self, _: &str, _: &Cx) -> Result<Vec<Entry>> {
            self.refuse()
        }
        fn fetch(&self, _: &str, _: Fetch, _: &Cx) -> Result<Fetched> {
            self.refuse()
        }
        fn create(&self, _: &str, _: &str, _: NewKind, _: &Cx) -> Result<String> {
            self.refuse()
        }
        fn rename(&self, _: &str, _: &str, _: &Cx) -> Result<String> {
            self.refuse()
        }
        fn trash(&self, paths: &[&str], _: &Cx) -> Vec<Trashed> {
            paths
                .iter()
                .map(|path| Trashed { path: path.to_string(), result: self.refuse() })
                .collect()
        }
        fn restore(&self, _: &str, _: &Cx) -> Result<String> {
            self.refuse()
        }
    }

    fn registry() -> Registry {
        Registry::new(Arc::new(Named("")))
            .register(Arc::new(Named("gdrive")))
            .register(Arc::new(Named("arbesk")))
            .register(Arc::new(Named("sftp")))
            .register(Arc::new(Named("share")))
    }

    fn scheme_for(path: &str) -> &'static str {
        registry().provider_for(path).scheme()
    }

    #[test]
    fn a_registered_scheme_reaches_its_provider() {
        assert_eq!(scheme_for("gdrive://acct/root~My%20Drive"), "gdrive");
        assert_eq!(scheme_for("arbesk://0xab/1~Set"), "arbesk");
        assert_eq!(scheme_for("sftp://me@host/home"), "sftp");
        assert_eq!(scheme_for("share://r1/a.txt"), "share");
    }

    #[test]
    fn local_paths_and_unregistered_prefixes_reach_the_local_provider() {
        for path in ["/home/me", "/", "file:///home/me", "smb://nas/share", "relative/path", ""] {
            assert_eq!(scheme_for(path), "", "{path}");
        }
    }

    #[test]
    fn the_prefix_is_case_sensitive_and_needs_both_slashes() {
        for path in ["GDRIVE://acct/root", "Gdrive://acct", "gdrive:/acct", "gdrive:acct", "gdrive"] {
            assert_eq!(scheme_for(path), "", "{path}");
        }
    }

    #[test]
    fn a_malformed_path_with_a_registered_prefix_still_reaches_its_provider() {
        assert_eq!(scheme_for("gdrive://"), "gdrive");
        assert_eq!(scheme_for("share://r1/%2E%2E"), "share");
        assert_eq!(scheme_for("arbesk://"), "arbesk");
    }

    #[test]
    fn a_remote_prefix_inside_a_local_path_stays_local() {
        assert_eq!(scheme_for("/home/me/gdrive://acct/root"), "");
        assert_eq!(scheme_for(" gdrive://acct/root"), "");
    }

    #[test]
    fn the_local_provider_refuses_in_its_own_words() {
        let registry = registry();
        let error = registry.provider_for("file:///a").list("file:///a", &ListOptions::default(), &Cx::default());
        assert_eq!(error.unwrap_err().to_string(), "not available for \"\"");
    }

    #[test]
    #[should_panic(expected = "registered twice")]
    fn a_scheme_registers_once() {
        let _ = registry().register(Arc::new(Named("gdrive")));
    }

    #[test]
    #[should_panic(expected = "empty scheme")]
    fn only_the_local_provider_has_the_empty_scheme() {
        let _ = Registry::new(Arc::new(Named("gdrive")));
    }

    #[test]
    #[should_panic(expected = "empty scheme")]
    fn a_remote_provider_needs_a_scheme() {
        let _ = registry().register(Arc::new(Named("")));
    }
}
