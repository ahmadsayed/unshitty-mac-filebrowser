//! Received shares as a `places::Provider` (design step 5): folders shared
//! with this computer, read only. Their listing and download are async
//! (iroh), so the provider runs them on the app's async runtime with
//! `block_on` (D3). Commands call providers only on blocking threads
//! (`blocking(…)` in `lib.rs`), never from an async worker, where
//! `block_on` would panic.
//!
//! Nothing is looked up for a paste, path info has nothing to say, the
//! preview refuses a shared file as any remote item, and every write is
//! refused as read only, with the words `place.rs` refuses them with.

use std::path::PathBuf;

use places::{
    Capabilities, Cx, Entry, Error, Fetch, Fetched, ListOptions, NewKind, PathInfo, Provider, Result, Trashed,
};
use tokio::runtime::Handle;

use crate::share::host::VersionsHint;
use crate::share::state::ShareState;

/// The scheme of received shares' paths, without "://".
pub const SCHEME: &str = "share";

/// Every write's refusal: nothing is written in a received share.
pub const READ_ONLY: &str = "Shared folders are read only.";

/// The local-only gate's words, for the features only items on this
/// computer have; the preview and looking an item up refuse with them too.
pub const LOCAL_ONLY: &str = "This action is not available for shared folders.";

/// What the provider reaches in the app: the managed `ShareState`, and the
/// Keep Versions hint its node starts with. `lib.rs` answers it for the app
/// handle, so nothing here holds a Tauri type.
pub trait Shares: Send + Sync + 'static {
    fn share_state(&self) -> &ShareState;
    fn versions_hint(&self) -> VersionsHint;
}

/// Received shares, over `shares`, their async calls run on `runtime`.
pub struct ReceivedShares<S> {
    shares: S,
    runtime: Handle,
}

impl<S> ReceivedShares<S> {
    /// `runtime` is the handle captured at setup (Tauri's async runtime).
    pub fn new(shares: S, runtime: Handle) -> Self {
        ReceivedShares { shares, runtime }
    }
}

/// Today's errors are words; they keep them, as `Other`, so a command
/// returns exactly the same text.
fn words(message: String) -> Error {
    Error::Other(message)
}

fn read_only<T>() -> Result<T> {
    Err(Error::Unsupported(READ_ONLY.into()))
}

fn local_only<T>() -> Result<T> {
    Err(Error::Unsupported(LOCAL_ONLY.into()))
}

impl<S: Shares> Provider for ReceivedShares<S> {
    fn scheme(&self) -> &'static str {
        SCHEME
    }
    fn capabilities(&self, _: &str) -> Capabilities {
        Capabilities::read_only_place(READ_ONLY, LOCAL_ONLY)
    }
    /// The folder as its sharer lists it, fetched on the async runtime.
    fn list(&self, dir: &str, opts: &ListOptions, _: &Cx) -> Result<Vec<Entry>> {
        let listing = self.shares.share_state().list_dir(dir, opts.show_hidden, self.shares.versions_hint());
        self.runtime.block_on(listing).map_err(words)
    }
    /// Nothing is pasted in a share, so nothing is looked up there.
    fn stat(&self, _: &str, _: &Cx) -> Result<Option<Entry>> {
        local_only()
    }
    fn lookup(&self, _: &str, _: &Cx) -> Result<Vec<Entry>> {
        local_only()
    }
    fn exists(&self, _: &str, _: &Cx) -> bool {
        false
    }
    fn path_info(&self, paths: &[String], _: &Cx) -> Vec<Option<PathInfo>> {
        paths.iter().map(|_| None).collect()
    }
    /// The file, downloaded into the app cache on the async runtime; the
    /// preview refuses it.
    fn fetch(&self, path: &str, purpose: Fetch, cx: &Cx) -> Result<Fetched> {
        if purpose == Fetch::Preview {
            return local_only();
        }
        let opening = self.shares.share_state().open(path, self.shares.versions_hint(), &cx.progress);
        let file = self.runtime.block_on(opening).map_err(words)?;
        Ok(Fetched::File(PathBuf::from(file)))
    }
    fn create(&self, _: &str, _: &str, _: NewKind, _: &Cx) -> Result<String> {
        read_only()
    }
    fn rename(&self, _: &str, _: &str, _: &Cx) -> Result<String> {
        read_only()
    }
    fn trash(&self, paths: &[&str], _: &Cx) -> Vec<Trashed> {
        paths.iter().map(|path| Trashed { path: path.to_string(), result: read_only() }).collect()
    }
    fn restore(&self, _: &str, _: &Cx) -> Result<String> {
        read_only()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use std::sync::Arc;
    use std::time::Duration;

    /// A guest's share state, as the app's managed state holds it.
    struct Guest(ShareState);

    impl Shares for Arc<Guest> {
        fn share_state(&self) -> &ShareState {
            &self.0
        }
        fn versions_hint(&self) -> VersionsHint {
            Box::new(|_, _, _| None)
        }
    }

    fn state(root: &Path, name: &str) -> ShareState {
        ShareState::new(root.join(format!("{name}-config")), root.join(format!("{name}-cache")))
    }

    /// A share's listing and download run through the registry from a
    /// blocking thread, as `list_dir` and `open_target` run them: the
    /// provider waits on the async runtime with `block_on`, which would
    /// panic on an async worker.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_share_listing_runs_through_the_registry_from_a_blocking_thread() {
        tokio::time::timeout(Duration::from_secs(60), async {
            let tmp = tempfile::tempdir().unwrap();
            let folder = tmp.path().join("Shared");
            std::fs::create_dir(&folder).unwrap();
            std::fs::write(folder.join("a.txt"), b"hi").unwrap();
            let no_hint = || -> VersionsHint { Box::new(|_, _, _| None) };
            let host = state(tmp.path(), "host");
            let info = host.share_folder(&folder, |_| false, no_hint()).await.unwrap();
            let link = host.create_link(&info.id, "anyone", None, None, no_hint()).await.unwrap();
            let guest = Arc::new(Guest(state(tmp.path(), "guest")));
            let received = guest.0.add_received(&link.link, None, no_hint()).await.unwrap();

            // The registry is built, asked and dropped on a blocking thread,
            // as the app's is used (its Arbesk client may not be built or
            // dropped on an async worker either).
            let provider = ReceivedShares::new(guest.clone(), Handle::current());
            let root = format!("share://{}", received.id);
            let cx = Cx::default();
            let progress = cx.progress.clone();
            let (listed, opened) = tokio::task::spawn_blocking(move || {
                let registry = crate::providers::testing::registry_with_shares(provider);
                let listed = registry.provider_for(&root).list(&root, &ListOptions::default(), &Cx::default());
                let file = format!("{root}/a.txt");
                let provider = registry.provider_for(&file);
                assert_eq!(provider.scheme(), SCHEME);
                assert!(provider.fetch_reports_progress());
                (listed, provider.fetch(&file, Fetch::Open, &cx))
            })
            .await
            .unwrap();
            let names = listed.unwrap().into_iter().map(|e| e.name).collect::<Vec<_>>();
            assert_eq!(names, ["a.txt"]);
            let opened = opened.unwrap();
            let Fetched::File(copy) = opened else { panic!("{opened:?}") };
            assert_eq!(std::fs::read(copy).unwrap(), b"hi");
            // The download reports the file's size and every byte.
            let update = progress.snapshot();
            assert_eq!((update.total, update.bytes), (Some(2), 2));
        })
        .await
        .expect("timed out");
    }

    /// Without a share, the provider answers in today's words and asks no
    /// node: a path that isn't a share, one no longer received, writes.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_share_refuses_in_todays_words() {
        let tmp = tempfile::tempdir().unwrap();
        let guest = Arc::new(Guest(state(tmp.path(), "guest")));
        let shares = ReceivedShares::new(guest.clone(), Handle::current());
        let answers = tokio::task::spawn_blocking(move || {
            let cx = Cx::default();
            let list = |path: &str| shares.list(path, &ListOptions::default(), &cx).unwrap_err();
            let gone = list("share://r1/Photos");
            let malformed = list("share://");
            let open = shares.fetch("share://r1/a.txt", Fetch::Open, &cx).unwrap_err();
            let preview = shares.fetch("share://r1/a.txt", Fetch::Preview, &cx).unwrap_err();
            let writes = [
                shares.create("share://r1", "b", NewKind::Folder, &cx).unwrap_err(),
                shares.rename("share://r1/a", "b", &cx).unwrap_err(),
                shares.restore("share://r1/a", &cx).unwrap_err(),
                shares.trash(&["share://r1/a"], &cx).remove(0).result.unwrap_err(),
            ];
            (gone, malformed, open, preview, writes)
        })
        .await
        .unwrap();
        let (gone, malformed, open, preview, writes) = answers;
        assert_eq!(gone, Error::Other("This folder is no longer in Network.".into()));
        assert_eq!(malformed, Error::Other("This isn't a share location.".into()));
        assert_eq!(open, Error::Other("This folder is no longer in Network.".into()));
        assert_eq!(preview, Error::Unsupported(LOCAL_ONLY.into()));
        for refused in writes {
            assert_eq!(refused, Error::Unsupported(READ_ONLY.into()));
        }
        assert!(!tmp.path().join("guest-config/share-key").exists(), "no node was started");
    }
}
