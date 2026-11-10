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

    /// One answer per path, in order, for a batch that spans places: each
    /// provider gets its own paths, in order, in a single call to `answer`
    /// (providers in the order their first path comes). A provider that
    /// answers fewer paths than it got leaves the rest `None`.
    pub fn answer_each<T>(
        &self,
        paths: &[String],
        mut answer: impl FnMut(&dyn Provider, &[String]) -> Vec<Option<T>>,
    ) -> Vec<Option<T>> {
        let mut groups: Vec<(&dyn Provider, Vec<usize>)> = Vec::new();
        for (index, path) in paths.iter().enumerate() {
            let provider = self.provider_for(path);
            match groups.iter_mut().find(|(known, _)| same(*known, provider)) {
                Some((_, indices)) => indices.push(index),
                None => groups.push((provider, vec![index])),
            }
        }
        let mut out: Vec<Option<T>> = paths.iter().map(|_| None).collect();
        for (provider, indices) in groups {
            let batch: Vec<String> = indices.iter().map(|&index| paths[index].clone()).collect();
            for (index, value) in indices.into_iter().zip(answer(provider, &batch)) {
                out[index] = value;
            }
        }
        out
    }
}

/// Whether `a` and `b` are the same registered provider: each scheme is
/// registered once, so the scheme names it.
fn same(a: &dyn Provider, b: &dyn Provider) -> bool {
    a.scheme() == b.scheme()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    use crate::{Capabilities, Cx, Entry, Error, Fetch, Fetched, ListOptions, NewKind, PathInfo, Replace, Result, Trashed};

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
        fn exists(&self, path: &str, _: &Cx) -> bool {
            path.ends_with(self.0)
        }
        /// A folder whose device is the length of the provider's scheme, one
        /// answer short: the batch's last path gets none.
        fn path_info(&self, paths: &[String], _: &Cx) -> Vec<Option<PathInfo>> {
            let info = || Some(PathInfo { is_dir: true, device: self.0.len() as u64 });
            paths.iter().skip(1).map(|_| info()).collect()
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
    fn a_batch_reaches_each_provider_once_with_its_own_paths_in_order() {
        let registry = registry();
        let paths: Vec<String> = ["/a", "gdrive://x", "rel", "sftp://h/a", "gdrive://y", "/b", "gdrive://z"]
            .map(String::from)
            .to_vec();
        let calls = RefCell::new(Vec::new());
        let out = registry.answer_each(&paths, |provider, batch| {
            calls.borrow_mut().push((provider.scheme(), batch.to_vec()));
            provider.path_info(batch, &Cx::default())
        });
        assert_eq!(
            calls.into_inner(),
            vec![
                ("", vec!["/a".to_string(), "rel".into(), "/b".into()]),
                ("gdrive", vec!["gdrive://x".to_string(), "gdrive://y".into(), "gdrive://z".into()]),
                ("sftp", vec!["sftp://h/a".to_string()]),
            ]
        );
        // Each provider's short answer leaves its last path without one.
        let devices: Vec<Option<u64>> = out.iter().map(|info| info.as_ref().map(|i| i.device)).collect();
        assert_eq!(devices, vec![Some(0), Some(6), Some(0), None, Some(6), None, None]);
    }

    #[test]
    fn an_empty_batch_asks_nobody() {
        let out: Vec<Option<u8>> = registry().answer_each(&[], |_, _| panic!("nobody to ask"));
        assert!(out.is_empty());
    }

    #[test]
    fn exists_is_the_providers_own_answer() {
        let registry = registry();
        assert!(registry.provider_for("share://r1/share").exists("share://r1/share", &Cx::default()));
        assert!(!registry.provider_for("share://r1/a").exists("share://r1/a", &Cx::default()));
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
