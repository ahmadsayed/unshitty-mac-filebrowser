//! Google Drive as a `places::Provider`: listing, whether an item exists,
//! path info, opening and thumbnails through [`Calls`] (the server calls,
//! which the app's tests answer themselves), and its transfer session: the
//! app's engine runs a whole copy or move on one [`Ctx`] ([`Calls::with`]),
//! so its steps share what it learnt.
//!
//! The app's commands still make Drive's own writes (New Folder, Rename,
//! Move to Trash, Undo) through `write.rs`, so those methods say they don't
//! go through the registry yet.

use std::path::{Path, PathBuf};

use places::{
    Capabilities, Cx, Entry, Error, Extensions, Fetch, Fetched, ListOptions, NewKind, OpenTarget, Operation,
    PathInfo, Progress, Provider, Refused, Replace, Result, Trashed, ext,
};

use crate::{CloudState, Ctx, service, write};

/// Where the provider finds the app's Drive state: the app answers it with
/// its managed state.
pub trait Reach: Send + Sync + 'static {
    fn reach(&self) -> &CloudState;
}

/// Google Drive's server calls the provider makes.
pub trait Calls: Send + Sync + 'static {
    fn list(&self, dir: &str, show_hidden: bool) -> std::result::Result<Vec<Entry>, String>;
    fn exists(&self, path: &str) -> bool;
    fn path_info(&self, path: &str) -> std::result::Result<PathInfo, String>;
    fn open(&self, path: &str, cache: &Path, progress: &Progress) -> std::result::Result<OpenTarget, String>;
    fn thumbnail(&self, path: &str, size: u32) -> Vec<u8>;
    /// Run `work` on a context for the live server, reporting to `progress`.
    fn with<T>(&self, progress: &Progress, work: impl FnOnce(&Ctx) -> T) -> T;
}

/// `service`'s calls on the app's `CloudState`.
pub struct Live<S>(pub S);

impl<S: Reach> Calls for Live<S> {
    fn list(&self, dir: &str, show_hidden: bool) -> std::result::Result<Vec<Entry>, String> {
        service::list_dir(self.0.reach(), dir, show_hidden)
    }
    fn exists(&self, path: &str) -> bool {
        service::path_exists(self.0.reach(), path)
    }
    fn path_info(&self, path: &str) -> std::result::Result<PathInfo, String> {
        service::path_info(self.0.reach(), path)
    }
    fn open(&self, path: &str, cache: &Path, progress: &Progress) -> std::result::Result<OpenTarget, String> {
        service::open(self.0.reach(), path, cache, progress)
    }
    fn thumbnail(&self, path: &str, size: u32) -> Vec<u8> {
        service::thumbnail(self.0.reach(), path, size)
    }
    fn with<T>(&self, progress: &Progress, work: impl FnOnce(&Ctx) -> T) -> T {
        write::live_with(self.0.reach(), progress, work)
    }
}

/// Google Drive, through `calls`; `cache` is the app cache, where an
/// opened file's copy goes (the OS's error when it has none).
pub struct Drive<C> {
    calls: C,
    cache: std::result::Result<PathBuf, String>,
}

impl<C> Drive<C> {
    pub fn new(calls: C, cache: std::result::Result<PathBuf, String>) -> Self {
        Drive { calls, cache }
    }
}

/// Today's errors are words; they keep them, as `Other`, so a command
/// returns exactly the same text.
fn words(message: String) -> Error {
    Error::Other(message)
}

/// What a command gets back from a provider's fetch.
fn fetched(target: OpenTarget) -> Fetched {
    match target {
        OpenTarget::File { path } => Fetched::File(PathBuf::from(path)),
        OpenTarget::Url { url } => Fetched::Url(url),
    }
}

/// A write no command routes here yet.
fn not_yet<T>(what: &str) -> Result<T> {
    Err(Error::Unsupported(format!("{what} doesn't go through the places registry yet.")))
}

/// What Drive allows: everything but the features of items on this
/// computer, which it refuses with the local-only gate's words.
pub fn capabilities() -> Capabilities {
    Capabilities {
        read_only: false,
        can_create: true,
        can_rename: true,
        can_trash: true,
        can_copy_out: true,
        knows_sizes: true,
        knows_counts: false,
        searchable: false,
        has_thumbnails: true,
        // Drive checks the name first, then renames.
        rename_atomic: false,
        replace: Replace::Trash,
        names_unique: false,
        put_is_whole: true,
        refused: vec![
            // Adding other files is Arbesk's; Drive says what it says of
            // every feature it lacks.
            Refused { operation: Operation::Attach, message: service::unsupported() },
            Refused { operation: Operation::LocalOnly, message: service::unsupported() },
        ],
    }
}

impl<C: Calls> Provider for Drive<C> {
    fn scheme(&self) -> &'static str {
        "gdrive"
    }
    fn capabilities(&self, _: &str) -> Capabilities {
        capabilities()
    }
    fn list(&self, dir: &str, opts: &ListOptions, _: &Cx) -> Result<Vec<Entry>> {
        self.calls.list(dir, opts.show_hidden).map_err(words)
    }
    fn exists(&self, path: &str, _: &Cx) -> bool {
        self.calls.exists(path)
    }
    fn path_info(&self, paths: &[String], _: &Cx) -> Vec<Option<PathInfo>> {
        paths.iter().map(|path| self.calls.path_info(path).ok()).collect()
    }
    /// A file's snapshot in the app cache, or a Google Docs file's page.
    fn fetch(&self, path: &str, purpose: Fetch, cx: &Cx) -> Result<Fetched> {
        if purpose == Fetch::Preview {
            return Err(Error::Unsupported(service::unsupported()));
        }
        let cache = self.cache.clone().map_err(words)?;
        self.calls.open(path, &cache, &cx.progress).map(fetched).map_err(words)
    }
    fn stat(&self, _: &str, _: &Cx) -> Result<Option<Entry>> {
        not_yet("Looking up an item")
    }
    fn lookup(&self, _: &str, _: &Cx) -> Result<Vec<Entry>> {
        not_yet("Looking up a name")
    }
    fn create(&self, _: &str, _: &str, _: NewKind, _: &Cx) -> Result<String> {
        not_yet("Creating an item")
    }
    fn rename(&self, _: &str, _: &str, _: &Cx) -> Result<String> {
        not_yet("Renaming")
    }
    fn trash(&self, paths: &[&str], _: &Cx) -> Vec<Trashed> {
        let unrouted = |path: &&str| Trashed { path: path.to_string(), result: not_yet("Move to Trash") };
        paths.iter().map(unrouted).collect()
    }
    fn restore(&self, _: &str, _: &Cx) -> Result<String> {
        not_yet("Restoring")
    }
    /// The whole transfer on one context for the live server ([`Calls::with`]),
    /// so a step after `landing` asks Drive nothing again.
    fn transfer(&self, cx: &Cx, work: &mut dyn FnMut(&dyn places::Transfer)) {
        self.calls.with(&cx.progress, |c| work(c));
    }
    fn extensions(&self) -> Extensions<'_> {
        Extensions { thumbnails: Some(self), ..Extensions::default() }
    }
}

impl<C: Calls> ext::Thumbnails for Drive<C> {
    /// Drive's own thumbnail; a request, once sent, isn't stopped.
    fn thumbnail(&self, path: &str, size: u32, _stop: &dyn Fn() -> bool) -> Vec<u8> {
        self.calls.thumbnail(path, size)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{ROOT, file, found, heads, ok};
    use crate::test_server::serve_replies;

    /// The fake server, through the provider's own calls.
    struct OnServer {
        state: CloudState,
        base: String,
    }

    impl Calls for OnServer {
        fn list(&self, _: &str, _: bool) -> std::result::Result<Vec<Entry>, String> {
            unreachable!()
        }
        fn exists(&self, _: &str) -> bool {
            unreachable!()
        }
        fn path_info(&self, _: &str) -> std::result::Result<PathInfo, String> {
            unreachable!()
        }
        fn open(&self, _: &str, _: &Path, _: &Progress) -> std::result::Result<OpenTarget, String> {
            unreachable!()
        }
        fn thumbnail(&self, _: &str, _: u32) -> Vec<u8> {
            unreachable!()
        }
        fn with<T>(&self, progress: &Progress, work: impl FnOnce(&Ctx) -> T) -> T {
            let mut c = crate::testing::ctx(&self.state, &self.base);
            c.progress = progress.clone();
            work(&c)
        }
    }

    #[test]
    fn a_fetched_target_keeps_its_path_or_url() {
        let file = fetched(OpenTarget::File { path: "/cache/x/a.pdf".into() });
        assert_eq!(file, Fetched::File("/cache/x/a.pdf".into()));
        let url = fetched(OpenTarget::Url { url: "https://docs.google.com/d".into() });
        assert_eq!(url, Fetched::Url("https://docs.google.com/d".into()));
    }

    #[test]
    fn drive_refuses_what_only_this_computer_does_in_its_own_words() {
        let caps = capabilities();
        let refused: Vec<_> = caps.refused.iter().map(|r| (r.operation, r.message.as_str())).collect();
        let words = "This action is not available for Google Drive items yet.";
        assert_eq!(refused, [(Operation::Attach, words), (Operation::LocalOnly, words)]);
        assert_eq!((caps.rename_atomic, caps.names_unique, caps.put_is_whole), (false, false, true));
    }

    /// The provider's transfer is one session on one context: the source
    /// `landing` fetched serves the copy after it, which asks Drive nothing
    /// again, as the app's engine runs it.
    #[test]
    fn the_providers_transfer_runs_a_whole_transfer_on_one_context() {
        let (base, log, server) = serve_replies(vec![ok(file("f1", "a.txt")), found(&[]), ok(file("c1", "b.txt"))]);
        let (state, _dir) = crate::testing::state();
        let drive = Drive::new(OnServer { state, base }, Err("No app cache in tests.".into()));
        let cx = Cx::default();
        let copied = places::with_transfer(&drive, &cx, |t| {
            let landing = t.landing(Some(&format!("{ROOT}/f1~a.txt")), &format!("{ROOT}/~b.txt"), &cx).unwrap();
            assert!(landing.existing.is_empty());
            let target = t.pending(&landing.parent, &landing.name).unwrap();
            t.copy_within(&landing.source.unwrap().path, &target, &cx).unwrap()
        });
        assert_eq!(copied.unwrap().path, format!("{ROOT}/c1~b.txt"));
        server.join().unwrap();
        let heads = heads(&log);
        assert_eq!(heads.len(), 3, "{heads:?}");
        assert!(heads[0].starts_with("GET /files/f1?") && heads[2].starts_with("POST /files/f1/copy?"), "{heads:?}");
    }
}
