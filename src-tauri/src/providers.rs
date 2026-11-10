//! One `places::Provider` per kind of place, and the registry that holds
//! them (design `2026-11-10-provider-crates-design.md`, step 2).
//!
//! This computer's provider is its crate's (`places_local::Local`, step 6),
//! handed the app's thumbnails (`LocalThumbnails`) and Trash
//! (`places_local::TrashReach`), inside Keep Versions' decorator
//! (`versions::versioned::Versioned`, step 6b), which is what the registry
//! holds for this computer. Google Drive (`places_gdrive::Drive`,
//! step 4), Arbesk (`arbesk::Client`, step 3) and SSH servers
//! (`places_sftp::SftpState`, step 5) are their own crates' providers, and
//! received shares the app's own (`share::provider::ReceivedShares`, step
//! 5), registered here as they are.
//! The read side goes through them: listing, whether an item exists, path
//! info, thumbnails, opening and the preview's file. This computer's writes
//! (new folders and files, Rename, Batch Rename, copy and move, Move to
//! Trash, Put Back) go through its provider too (step 6b); Drive's writes
//! (new folders, rename, Trash, restore) stay on their commands' own
//! dispatch, so its write methods say so instead of acting, while its
//! copies and moves run on its transfer session; Arbesk's copies in and out, new
//! collections, sharing, saves, scenes and accounts already go through its
//! provider, and SSH servers and shares refuse every write as read only.
//!
//! Received shares are the one async place (D3): their provider waits on the
//! app's async runtime with `block_on`, so `list_dir` and `open_target` call
//! it from `blocking(…)` like every other provider.
//!
//! Drive and shares reach the app's state through [`Reach`] and
//! `share::provider::Shares`, which `lib.rs` answers with the app's managed
//! state; this computer gets its folders at startup ([`Folders`]): nothing
//! here holds a Tauri type. Drive's server calls go through its crate's
//! `Calls`, so a test answers them itself and still runs each provider and
//! the registry; Arbesk's tests answer through its crate's own fake server,
//! and here the registry runs the real `arbesk::Client`, and the real
//! `SftpState` with no server connected.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use places::{Registry, ext};

use places_gdrive::{Calls as DriveCalls, CloudState, Drive, Live};
use places_local::{Local, TrashReach};
use places_sftp::SftpState;
use crate::share::provider::{ReceivedShares, Shares};
use crate::thumbnails;
use crate::versions::state::VersionsState;
use crate::versions::versioned::Versioned;

/// Where an adapter finds the state it wraps: `lib.rs` answers it for the
/// app handle with the managed state, as the commands did.
pub trait Reach<T>: Send + Sync + 'static {
    fn reach(&self) -> &T;
}

/// The folders the providers keep files in, from the app's paths.
pub struct Folders {
    /// The app cache, where Drive puts the copies it opens; the OS's error
    /// when it has none.
    pub cache: std::result::Result<PathBuf, String>,
    /// The app's thumbnail cache.
    pub thumbnails: Option<PathBuf>,
    /// The desktop's shared thumbnail cache, read only (Linux).
    pub shared_thumbnails: Option<PathBuf>,
}

/// The places that are their own crates' providers, held by the registry
/// and by the app's commands that belong to them: the app's Arbesk client
/// and its SSH connections.
pub struct Held {
    pub arbesk: Arc<arbesk::Client>,
    pub sftp: Arc<SftpState>,
}

/// The registry of every place the app shows, its providers reaching their
/// state through `states` (this computer: Keep Versions' histories), this
/// computer's provider the app's Trash through `trash` (`lib.rs` answers
/// `places_local::TrashReach` for the app handle); received shares wait on
/// `runtime`, the app's async runtime.
pub fn registry<S>(
    states: S,
    trash: Box<dyn TrashReach>,
    folders: Folders,
    held: Held,
    runtime: tokio::runtime::Handle,
) -> Registry
where
    S: Reach<CloudState> + Reach<VersionsState> + Shares + Clone,
{
    let Folders { cache, thumbnails, shared_thumbnails } = folders;
    registry_with(
        local(thumbnails, shared_thumbnails, trash, Box::new(states.clone())),
        Drive::new(Live(DriveState(states.clone())), cache),
        held,
        ReceivedShares::new(states, runtime),
    )
}

/// The registry over these providers.
fn registry_with(
    local: Versioned,
    drive: Drive<impl DriveCalls>,
    held: Held,
    shares: ReceivedShares<impl Shares>,
) -> Registry {
    Registry::new(Arc::new(local))
        .register(Arc::new(drive))
        .register(held.arbesk)
        .register(held.sftp)
        .register(Arc::new(shares))
}

// ---- This computer ---------------------------------------------------------------

/// This computer's provider is its crate's (`places_local::Local`); its
/// thumbnails are the app's: the desktop's own thumbnail (Linux), Quick Look
/// or the image decoder, through the disk cache.
pub struct LocalThumbnails {
    thumbnails: Option<PathBuf>,
    shared_thumbnails: Option<PathBuf>,
}

/// This computer's provider, with the app's thumbnails in these folders and
/// the app's Trash, inside Keep Versions' decorator over `versions`.
fn local(
    thumbnails: Option<PathBuf>,
    shared_thumbnails: Option<PathBuf>,
    trash: Box<dyn TrashReach>,
    versions: Box<dyn Reach<VersionsState>>,
) -> Versioned {
    let thumbnails = Box::new(LocalThumbnails { thumbnails, shared_thumbnails });
    Versioned::new(Local::new(thumbnails, trash), versions)
}

impl ext::Thumbnails for LocalThumbnails {
    /// Asked only for an absolute path: `places_local::Local` refuses any
    /// other before it asks.
    fn thumbnail(&self, path: &str, size: u32, stop: &dyn Fn() -> bool) -> Vec<u8> {
        let shared = self.shared_thumbnails.as_deref();
        self.thumbnails
            .as_deref()
            .and_then(|dir| {
                thumbnails::thumbnail_unless(Path::new(path), size, dir, shared, stop)
                    .ok()
                    .flatten()
            })
            .and_then(|p| std::fs::read(p).ok())
            .unwrap_or_default()
    }
}

// ---- Google Drive ----------------------------------------------------------------

/// Google Drive is its own crate's provider (`places_gdrive::Drive`); it
/// reaches the app's `CloudState` through this.
pub struct DriveState<S>(S);

impl<S: Reach<CloudState>> places_gdrive::Reach for DriveState<S> {
    fn reach(&self) -> &CloudState {
        self.0.reach()
    }
}

// ---- What the frontend reads (design D5, step 7) --------------------------------

/// The booleans of a place's `places::Capabilities` the frontend reads,
/// for one kind of place it tells apart. Only the booleans: the frontend
/// keeps its own toasts (D5), so no refusal wording goes with them, and
/// `can_copy_in` is whether the place has no copy-in refusal.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub struct KindCapabilities {
    pub read_only: bool,
    pub can_create: bool,
    pub can_rename: bool,
    pub can_trash: bool,
    pub can_copy_in: bool,
    pub can_copy_out: bool,
    pub knows_sizes: bool,
    pub knows_counts: bool,
    pub searchable: bool,
    pub has_thumbnails: bool,
}

impl KindCapabilities {
    fn of(caps: &places::Capabilities) -> Self {
        KindCapabilities {
            read_only: caps.read_only,
            can_create: caps.can_create,
            can_rename: caps.can_rename,
            can_trash: caps.can_trash,
            can_copy_in: !caps.refused.iter().any(|r| r.operation == places::Operation::CopyIn),
            can_copy_out: caps.can_copy_out,
            knows_sizes: caps.knows_sizes,
            knows_counts: caps.knows_counts,
            searchable: caps.searchable,
            has_thumbnails: caps.has_thumbnails,
        }
    }
}

/// Each kind of place the frontend tells apart (`src/capabilities.ts`'
/// `CapabilityRow`, picked by `locations.ts`' `capabilityRow`), with a path
/// of that kind its provider answers for. Every place answers the same for
/// all its paths but Arbesk, whose account, collection and asset differ
/// (New Collection in an account, copies into a collection, out of an
/// asset), so it has a row each. Recent, Starred, Network and the Trash are
/// the frontend's own views, not places: they have no row.
pub const KINDS: [(&str, &str); 7] = [
    ("folder", "/"),
    ("drive", "gdrive://acct/root~x"),
    ("share", "share://r1/Photos"),
    ("sftp", "sftp://me@host/home"),
    ("arbesk-account", "arbesk://0x00000000000000000000000000000000000000ab"),
    ("arbesk-collection", "arbesk://0x00000000000000000000000000000000000000ab/12~Set"),
    ("arbesk-asset", "arbesk://0x00000000000000000000000000000000000000ab/12~Set/a1~Box.gltf"),
];

/// What each kind of place allows, asked of the registered providers
/// themselves (`Provider::capabilities`, no network), so the frontend's
/// table can't drift from the backend's. The `capabilities` command's
/// answer; `tests/capabilities.json` pins it, and the frontend's built-in
/// copy and the mock against it.
pub fn capability_table(registry: &Registry) -> std::collections::BTreeMap<&'static str, KindCapabilities> {
    KINDS
        .iter()
        .map(|&(kind, path)| (kind, KindCapabilities::of(&registry.provider_for(path).capabilities(path))))
        .collect()
}

/// Stand-ins for the servers, so tests run the adapters and the registry
/// without Tauri or a network.
#[cfg(test)]
pub(crate) mod testing {
    use super::*;
    use places::{Entry, OpenTarget, PathInfo, Progress};

    /// A test's answer for one path.
    pub type Answer<T> = Box<dyn Fn(&str) -> T + Send + Sync>;

    /// What a stand-in says when a test sends it a call it didn't expect.
    fn unexpected<T>(server: &str) -> Answer<T> {
        let server = server.to_string();
        Box::new(move |path| panic!("{path} asked {server}, which this test doesn't expect"))
    }

    /// Drive's server: whether an item exists and its path info, as the
    /// test says; anything else isn't asked in tests.
    pub struct StubDrive {
        pub exists: Answer<bool>,
        pub path_info: Answer<std::result::Result<PathInfo, String>>,
    }

    impl Default for StubDrive {
        fn default() -> Self {
            StubDrive { exists: unexpected("Drive"), path_info: unexpected("Drive") }
        }
    }

    impl DriveCalls for StubDrive {
        fn list(&self, dir: &str, _: bool) -> std::result::Result<Vec<Entry>, String> {
            unexpected("Drive")(dir)
        }
        fn exists(&self, path: &str) -> bool {
            (self.exists)(path)
        }
        fn path_info(&self, path: &str) -> std::result::Result<PathInfo, String> {
            (self.path_info)(path)
        }
        fn open(&self, path: &str, _: &Path, _: &Progress) -> std::result::Result<OpenTarget, String> {
            unexpected("Drive")(path)
        }
        fn thumbnail(&self, path: &str, _: u32) -> Vec<u8> {
            unexpected("Drive")(path)
        }
        fn with<T>(&self, _: &Progress, _: impl FnOnce(&places_gdrive::Ctx) -> T) -> T {
            unexpected("Drive")("a transfer")
        }
    }

    /// No received share: the tests that use it never reach one.
    pub struct NoShares;

    impl Shares for NoShares {
        fn share_state(&self) -> &crate::share::state::ShareState {
            panic!("no received share in tests")
        }
        fn versions_hint(&self) -> crate::share::host::VersionsHint {
            panic!("no received share in tests")
        }
    }

    /// An async runtime of the tests' own, for the shares provider of a
    /// registry no share is listed through.
    pub fn runtime() -> tokio::runtime::Handle {
        static RUNTIME: std::sync::OnceLock<tokio::runtime::Runtime> = std::sync::OnceLock::new();
        let build = || tokio::runtime::Builder::new_current_thread().build().expect("a test runtime");
        RUNTIME.get_or_init(build).handle().clone()
    }

    /// The real SSH provider, with no server connected: it answers every
    /// path without a network (“Connect to this server first.”, or its
    /// parser's refusal), and writes nothing (its folders don't exist).
    pub fn sftp() -> Arc<SftpState> {
        let nowhere = PathBuf::from("/nonexistent-filebrowser-tests/sftp");
        Arc::new(SftpState::new(nowhere.join("known_hosts"), nowhere.clone(), nowhere.join("cache")))
    }

    /// The real Arbesk client, with no account connected: it answers every
    /// path without a server (“Connect this Arbesk account first.”, or its
    /// parser's refusal), and writes nothing (its folders don't exist).
    pub fn arbesk() -> Arc<arbesk::Client> {
        let nowhere = PathBuf::from("/nonexistent-filebrowser-tests/arbesk");
        Arc::new(arbesk::Client::new(nowhere.join("cache"), nowhere.join("accounts.json"), nowhere.join("sessions")))
    }

    /// No Trash: the tests that use the registry trash nothing on this
    /// computer.
    pub struct NoTrash;

    impl TrashReach for NoTrash {
        fn trash(&self) -> std::result::Result<&places_local::trash_ops::Trash, String> {
            Err("The home folder is unknown".into())
        }
        fn home(&self) -> std::result::Result<PathBuf, String> {
            Err("No home folder in tests.".into())
        }
    }

    /// Keep Versions with no history registered.
    pub struct NoHistories(VersionsState);

    impl Reach<VersionsState> for NoHistories {
        fn reach(&self) -> &VersionsState {
            &self.0
        }
    }

    /// This computer's provider with no Trash and no history.
    fn plain_local() -> Versioned {
        let versions = VersionsState::new(PathBuf::from("/nonexistent-filebrowser-tests/versioned.json"));
        local(None, None, Box::new(NoTrash), Box::new(NoHistories(versions)))
    }

    /// The registry with Drive answered by `drive`, the real Arbesk
    /// provider with no account and the real SSH provider with no server.
    pub fn registry(drive: StubDrive) -> Registry {
        registry_with(
            plain_local(),
            Drive::new(drive, Err("No app cache in tests.".into())),
            Held { arbesk: arbesk(), sftp: sftp() },
            ReceivedShares::new(NoShares, runtime()),
        )
    }

    /// The registry with received shares answered by `shares`, and no
    /// other server expected to be asked anything.
    pub fn registry_with_shares(shares: ReceivedShares<impl Shares>) -> Registry {
        registry_with(
            plain_local(),
            Drive::new(StubDrive::default(), Err("No app cache in tests.".into())),
            Held { arbesk: arbesk(), sftp: sftp() },
            shares,
        )
    }

    /// The registry, with no server expected to be asked anything.
    pub fn plain() -> Registry {
        registry(StubDrive::default())
    }
}

#[cfg(test)]
mod tests {
    use super::testing::StubDrive;
    use super::*;
    use places::{
        Capabilities, Cx, Entry, Error, Fetch, Fetched, ListOptions, NewKind, OpenTarget, Operation,
        PathInfo, Replace, Result, Trashed,
    };
    use crate::place::{self, Kind};

    const ARBESK: &str = "arbesk://0x00000000000000000000000000000000000000ab/12~Set";
    const ACCOUNT: &str = "arbesk://0x00000000000000000000000000000000000000ab";
    const ASSET: &str = "arbesk://0x00000000000000000000000000000000000000ab/12~Set/a1~Box.gltf";

    /// The rows of `place.rs`' table test, by path, plus an Arbesk account
    /// and asset.
    const PATHS: [&str; 26] = [
        "/Users/me/a.txt",
        "/",
        "/Users/me/gdrive://x",
        "Documents/a.txt",
        "",
        "pseudo:recent",
        "file:///Users/me",
        "smb://host/share",
        "GDRIVE://acct/root~x",
        "gdrive:/acct",
        "share:/r1",
        "sftp:/me@host",
        "gdrive://acct/root~My%20Drive",
        "gdrive://acct/root~x/~New%20Folder",
        "gdrive://",
        "gdrive://../root~x",
        "share://r1/Photos",
        "share://",
        "share://r1/%2E%2E",
        "sftp://me@host/home",
        "sftp://",
        ARBESK,
        "arbesk://",
        "arbesk:/0xab",
        ACCOUNT,
        ASSET,
    ];

    fn scheme_of(kind: Kind) -> &'static str {
        match kind {
            Kind::Local => "",
            Kind::Drive => "gdrive",
            Kind::Share => crate::share::provider::SCHEME,
            Kind::Sftp => "sftp",
            Kind::Arbesk => "arbesk",
        }
    }

    fn refusal(caps: &Capabilities, operation: Operation) -> Option<String> {
        caps.refused.iter().find(|r| r.operation == operation).map(|r| r.message.clone())
    }

    /// Each row reaches the provider of its kind, and its capabilities give
    /// the backend's own refusals for it, word for word: the local-only
    /// gate's, the read-only writes', and a copy's either way.
    #[test]
    fn each_path_reaches_its_kinds_provider_with_the_backends_refusals() {
        let registry = testing::plain();
        for path in PATHS {
            let provider = registry.provider_for(path);
            let kind = place::kind(path);
            assert_eq!(provider.scheme(), scheme_of(kind), "{path}");
            let caps = provider.capabilities(path);
            assert_eq!(refusal(&caps, Operation::LocalOnly), place::local(path).err(), "{path}");
            assert_eq!(caps.read_only, place::writable(path).is_err(), "{path}");
            let write = match kind {
                Kind::Drive => None,
                Kind::Arbesk => place::writable(path).err(),
                _ => place::local_write(path).err(),
            };
            for operation in [Operation::Rename, Operation::Trash] {
                assert_eq!(refusal(&caps, operation), write, "{path} {operation:?}");
            }
            // Arbesk creates only a collection, in an account (New
            // Collection), and refuses anywhere else in its `create`'s words.
            let create = match kind {
                Kind::Arbesk if path == ACCOUNT => None,
                Kind::Arbesk => provider
                    .create(path, "x", places::NewKind::Folder, &Cx::default())
                    .err()
                    .map(|error| error.to_string()),
                _ => write,
            };
            assert_eq!(refusal(&caps, Operation::Create), create, "{path} Create");
            if kind == Kind::Arbesk {
                assert_eq!(caps.can_create, create.is_none(), "{path}");
            }
            let model = "/Users/me/Box.glb";
            let copy_in = match kind {
                Kind::Arbesk => place::copy(model, &format!("{path}/~Box.glb")).err(),
                Kind::Drive => None,
                _ => place::local_write(path).err(),
            };
            assert_eq!(refusal(&caps, Operation::CopyIn), copy_in, "{path}");
            let copy_out = match kind {
                Kind::Drive => None,
                _ => place::copy(path, "/Users/me/b").err(),
            };
            assert_eq!(refusal(&caps, Operation::CopyOut), copy_out, "{path}");
            // A path that isn't local keeps its kind's booleans; only its
            // refusals say it isn't one.
            if kind != Kind::Local || place::local(path).is_ok() {
                assert_eq!(caps.can_copy_out, copy_out.is_none(), "{path}");
            }
        }
    }

    /// What each kind of place allows and how its operations behave.
    #[test]
    fn each_kind_has_its_capabilities() {
        let registry = testing::plain();
        // (path, read_only, create, rename, trash, sizes, counts, search, thumbnails)
        let rows = [
            ("/Users/me", false, true, true, true, true, true, true, true),
            ("gdrive://acct/root~x", false, true, true, true, true, false, false, true),
            ("share://r1/Photos", true, false, false, false, true, false, false, false),
            ("sftp://me@host/home", true, false, false, false, true, false, false, false),
            (ARBESK, true, false, false, false, false, false, false, true),
        ];
        for (path, read_only, create, rename, trash, sizes, counts, search, thumbs) in rows {
            let caps = registry.provider_for(path).capabilities(path);
            assert_eq!(
                (caps.read_only, caps.can_create, caps.can_rename, caps.can_trash),
                (read_only, create, rename, trash),
                "{path}"
            );
            assert_eq!((caps.knows_sizes, caps.knows_counts, caps.searchable), (sizes, counts, search), "{path}");
            let has = registry.provider_for(path).extensions().thumbnails.is_some();
            assert_eq!((caps.has_thumbnails, has), (thumbs, thumbs), "{path}");
        }
        let behaviour = |path: &str| {
            let caps = registry.provider_for(path).capabilities(path);
            (caps.rename_atomic, caps.replace, caps.names_unique, caps.put_is_whole)
        };
        assert_eq!(behaviour("/Users/me"), (true, Replace::Trash, true, false));
        assert_eq!(behaviour("gdrive://acct/root~x"), (false, Replace::Trash, false, true));
        assert_eq!(behaviour(ARBESK), (false, Replace::NewVersion, true, true));
    }

    #[test]
    fn this_computer_lists_local_folders_and_refuses_anything_else() {
        let registry = testing::plain();
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "abc").unwrap();
        std::fs::write(dir.path().join(".b"), "").unwrap();
        let folder = dir.path().to_string_lossy().to_string();
        let list = |path: &str, show_hidden| {
            registry.provider_for(path).list(path, &ListOptions { show_hidden }, &Cx::default())
        };
        let names = |entries: Vec<Entry>| entries.into_iter().map(|e| e.name).collect::<Vec<_>>();
        assert_eq!(names(list(&folder, false).unwrap()), ["a.txt"]);
        assert_eq!(list(&folder, true).unwrap().len(), 2);
        for path in ["src", "", "file:///tmp", "GDRIVE://acct/root~x"] {
            assert_eq!(list(path, false).unwrap_err().to_string(), format!("“{path}” is not a local file."));
        }
    }

    #[test]
    fn a_local_file_opens_and_previews_as_itself() {
        let registry = testing::plain();
        let local = registry.provider_for("/Users/me/a b.txt");
        for purpose in [Fetch::Open, Fetch::Preview] {
            let opened = local.fetch("/Users/me/a b.txt/", purpose, &Cx::default()).unwrap();
            assert_eq!(opened, Fetched::File("/Users/me/a b.txt/".into()));
            let refused = local.fetch("a.txt", purpose, &Cx::default()).unwrap_err();
            assert_eq!(refused.to_string(), "“a.txt” is not a local file.");
        }
        let target: OpenTarget = Fetched::File("/Users/me/a b.txt/".into()).into();
        assert_eq!(serde_json::to_string(&target).unwrap(), r#"{"kind":"file","path":"/Users/me/a b.txt/"}"#);
    }

    #[test]
    fn remote_items_are_not_previewed_but_arbesk_assets() {
        let registry = testing::plain();
        for (path, words) in [
            ("gdrive://acct/f1~a.txt", "This action is not available for Google Drive items yet."),
            ("sftp://me@host/a.txt", "This action is not available for SSH servers."),
            ("share://r1/a.txt", "This action is not available for shared folders."),
        ] {
            let refused = registry.provider_for(path).fetch(path, Fetch::Preview, &Cx::default());
            assert_eq!(refused.unwrap_err(), Error::Unsupported(words.into()), "{path}");
        }
    }

    #[test]
    fn shares_and_ssh_servers_have_no_path_info_existence_or_thumbnails() {
        let registry = testing::plain();
        for path in ["share://r1/Photos", "share://", "sftp://me@host/home", "sftp://"] {
            let provider = registry.provider_for(path);
            assert!(!provider.exists(path, &Cx::default()), "{path}");
            assert_eq!(provider.path_info(&[path.to_string(), path.to_string()], &Cx::default()), vec![None, None]);
            assert!(provider.extensions().thumbnails.is_none(), "{path}");
        }
    }

    #[test]
    fn a_local_thumbnail_of_a_path_that_isnt_local_is_empty() {
        let registry = testing::plain();
        let thumbnails = registry.provider_for("rel.png").extensions().thumbnails.unwrap();
        assert!(thumbnails.thumbnail("rel.png", 64, &|| false).is_empty());
        assert!(thumbnails.thumbnail("file:///a.png", 64, &|| false).is_empty());
    }

    /// Drive's writes and lookups stay on their commands' dispatch; its
    /// provider says so rather than acting. This computer's act (step 6b:
    /// `places_local::provider`'s tests, and `versions::versioned`'s).
    #[test]
    fn drives_writes_are_not_routed_through_providers_yet() {
        let registry = testing::plain();
        let cx = Cx::default();
        // Arbesk refuses them as read only, in its own crate's tests, and
        // so do SSH servers and shares (below).
        for path in ["gdrive://acct/root~x"] {
            let provider = registry.provider_for(path);
            let not_yet = |result: Result<String>| {
                matches!(result, Err(Error::Unsupported(words)) if words.ends_with("registry yet."))
            };
            assert!(not_yet(provider.create(path, "b", NewKind::Folder, &cx)), "{path}");
            assert!(not_yet(provider.rename(path, "b", &cx)), "{path}");
            assert!(not_yet(provider.restore(path, &cx)), "{path}");
            assert!(provider.stat(path, &cx).is_err() && provider.lookup(path, &cx).is_err(), "{path}");
            let trashed = provider.trash(&[path], &cx);
            assert_eq!(trashed.len(), 1);
            assert!(trashed[0].result.is_err(), "{path}");
        }
    }

    /// SSH servers and received shares refuse every write in the words
    /// `place::writable` refuses it with, and looking an item up in the
    /// local-only gate's.
    #[test]
    fn ssh_servers_and_shares_refuse_writes_as_read_only() {
        let registry = testing::plain();
        let cx = Cx::default();
        for path in ["sftp://me@host/a", "share://r1/a"] {
            let provider = registry.provider_for(path);
            let read_only = Error::Unsupported(place::writable(path).unwrap_err());
            assert_eq!(provider.create(path, "b", NewKind::Folder, &cx).unwrap_err(), read_only, "{path}");
            assert_eq!(provider.rename(path, "b", &cx).unwrap_err(), read_only, "{path}");
            assert_eq!(provider.restore(path, &cx).unwrap_err(), read_only, "{path}");
            assert_eq!(provider.trash(&[path], &cx), [Trashed { path: path.into(), result: Err(read_only) }]);
            let local_only = Error::Unsupported(place::local(path).unwrap_err());
            assert_eq!(provider.stat(path, &cx).unwrap_err(), local_only, "{path}");
            assert_eq!(provider.lookup(path, &cx).unwrap_err(), local_only, "{path}");
            assert!(places::with_transfer(provider, &cx, |_| ()).is_none(), "{path}");
        }
    }

    /// Opening a file starts a tracked operation everywhere but on this
    /// computer, where the file itself opens (`open_target`).
    #[test]
    fn only_this_computer_opens_without_reporting_progress() {
        let registry = testing::plain();
        for path in PATHS {
            let provider = registry.provider_for(path);
            assert_eq!(provider.fetch_reports_progress(), place::kind(path) != Kind::Local, "{path}");
        }
    }

    /// Drive's adapter asks its server whether an item exists, and for each
    /// path's info, with nothing for a path the server can't describe.
    #[test]
    fn drive_asks_its_server_for_existence_and_path_info() {
        const FOLDER: &str = "gdrive://acct/f1~Docs";
        let drive = StubDrive {
            exists: Box::new(|path| path == FOLDER),
            path_info: Box::new(|path| match path {
                FOLDER => Ok(PathInfo { is_dir: true, device: 0 }),
                _ => Err("File not found.".into()),
            }),
        };
        let registry = testing::registry(drive);
        let (provider, cx) = (registry.provider_for(FOLDER), Cx::default());
        assert!(provider.exists(FOLDER, &cx));
        assert!(!provider.exists("gdrive://acct/f2~gone", &cx));
        let paths = [FOLDER.to_string(), "gdrive://acct/f2~gone".into()];
        assert_eq!(provider.path_info(&paths, &cx), vec![Some(PathInfo { is_dir: true, device: 0 }), None]);
    }

    /// Without an app cache, Drive opens nothing and says why, before it
    /// asks the server.
    #[test]
    fn drive_opens_nothing_without_an_app_cache() {
        let registry = testing::plain();
        let path = "gdrive://acct/f1~a.txt";
        let opened = registry.provider_for(path).fetch(path, Fetch::Open, &Cx::default());
        assert_eq!(opened.unwrap_err().to_string(), "No app cache in tests.");
    }

    /// The registry runs the real Arbesk provider, the crate's `Client`: with
    /// no account connected it asks no server, and says so in its words.
    #[test]
    fn arbesk_is_its_own_crates_provider() {
        const PENDING: &str = "arbesk://0x00000000000000000000000000000000000000ab/12~Set/~Box.gltf";
        let registry = testing::plain();
        let (provider, cx) = (registry.provider_for(PENDING), Cx::default());
        assert_eq!(provider.scheme(), arbesk::SCHEME);
        assert!(registry.provider(arbesk::SCHEME).is_some_and(|p| p.scheme() == "arbesk"));
        assert!(!provider.exists(PENDING, &cx));
        let unknown = Error::Other("Connect this Arbesk account first.".into());
        assert_eq!(provider.fetch(ASSET, Fetch::Preview, &cx).unwrap_err(), unknown);
        assert_eq!(provider.list(ARBESK, &ListOptions::default(), &cx).unwrap_err(), unknown);
        let extensions = provider.extensions();
        assert!(extensions.thumbnails.is_some() && extensions.versions.is_some() && extensions.preview.is_some());
        assert!(extensions.sharing.is_some() && extensions.accounts.is_some());
        assert!(extensions.accounts.unwrap().accounts().is_empty());
        let exported = places::with_transfer(provider, &cx, |t| t.export(ASSET, &cx)).unwrap();
        assert_eq!(exported.unwrap_err(), unknown);
    }

    /// The app's own states, as `lib.rs`' managed state holds them.
    struct States {
        cloud: CloudState,
        shares: crate::share::state::ShareState,
    }

    impl Reach<CloudState> for Arc<States> {
        fn reach(&self) -> &CloudState {
            &self.cloud
        }
    }

    /// Keep Versions, as `lib.rs` answers it; the test asks none.
    impl Reach<VersionsState> for Arc<States> {
        fn reach(&self) -> &VersionsState {
            panic!("no history in this test")
        }
    }

    impl Shares for Arc<States> {
        fn share_state(&self) -> &crate::share::state::ShareState {
            &self.shares
        }
        fn versions_hint(&self) -> crate::share::host::VersionsHint {
            Box::new(|_, _, _| None)
        }
    }

    /// The app's registry, over its real states: each kind reaches its
    /// provider, and a malformed Drive or Arbesk path is answered without
    /// a server, as `places_gdrive::service` and the `arbesk` client answer it.
    #[test]
    fn the_apps_registry_reaches_each_kinds_state() {
        let dir = tempfile::tempdir().unwrap();
        let states = Arc::new(States {
            cloud: CloudState::new(dir.path()),
            shares: crate::share::state::ShareState::new(dir.path().join("config"), dir.path().join("cache")),
        });
        let sftp = Arc::new(SftpState::new(dir.path().join("known_hosts"), dir.path().into(), dir.path().join("sftp")));
        let folders = Folders { cache: Ok(dir.path().join("cache")), thumbnails: None, shared_thumbnails: None };
        let arbesk = Arc::new(arbesk::Client::new(dir.path().join("cache"), dir.path().join("a.json"), dir.path().join("s")));
        let registry = registry(states, Box::new(testing::NoTrash), folders, Held { arbesk, sftp }, testing::runtime());
        for path in PATHS {
            assert_eq!(registry.provider_for(path).scheme(), scheme_of(place::kind(path)), "{path}");
        }
        let cx = Cx::default();
        for path in ["gdrive://", "gdrive://../root~x", "arbesk://", "arbesk:/0xab"] {
            let provider = registry.provider_for(path);
            assert!(!provider.exists(path, &cx), "{path}");
        }
        assert_eq!(registry.provider_for("gdrive://").path_info(&["gdrive://".into()], &cx), vec![None]);
    }

    /// The table the frontend reads (design D5) is the providers' own
    /// answers, and `tests/capabilities.json` holds it: the frontend's
    /// built-in copy and the mock are tested against the same file
    /// (`tests/capabilities.test.ts`), so none of the three can drift.
    #[test]
    fn the_frontends_capability_table_is_the_shared_fixture() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../../tests/capabilities.json")).unwrap();
        let table = serde_json::to_value(capability_table(&testing::plain())).unwrap();
        assert_eq!(table, fixture["table"]);
        let paths: serde_json::Map<String, serde_json::Value> =
            KINDS.iter().map(|&(kind, path)| (kind.to_string(), path.into())).collect();
        assert_eq!(serde_json::Value::Object(paths), fixture["paths"]);
        // Each row's path is of its kind: its place, and for Arbesk the
        // account, collection or asset.
        let registry = testing::plain();
        let schemes: Vec<&str> = KINDS.iter().map(|&(_, path)| registry.provider_for(path).scheme()).collect();
        assert_eq!(schemes, ["", "gdrive", "share", "sftp", "arbesk", "arbesk", "arbesk"]);
        assert_eq!([KINDS[4].1, KINDS[5].1, KINDS[6].1], [ACCOUNT, ARBESK, ASSET]);
    }

    /// The frontend picks an Arbesk path's row by its number of segments
    /// (`locations.ts`' `capabilityRow`): the root, a single segment that
    /// isn't an address and four or more segments get the account's row,
    /// and it reads a path's row for its copy flags only (`copiesOf`).
    /// Those agree with what the provider says for such a path; New
    /// Collection doesn't (the account allows it, these paths refuse it).
    #[test]
    fn the_arbesk_root_and_a_malformed_path_copy_as_the_account() {
        let registry = testing::plain();
        let account = KindCapabilities::of(&registry.provider_for(ACCOUNT).capabilities(ACCOUNT));
        for path in ["arbesk://", "arbesk://not-an-address", "arbesk://0x00000000000000000000000000000000000000ab/12~Set/a1~Box.gltf/x"] {
            assert_eq!(registry.provider_for(path).scheme(), "arbesk", "{path}");
            let caps = KindCapabilities::of(&registry.provider_for(path).capabilities(path));
            assert_eq!((caps.can_copy_in, caps.can_copy_out), (account.can_copy_in, account.can_copy_out), "{path}");
            assert!(!caps.can_create && account.can_create, "{path}");
        }
    }
}
