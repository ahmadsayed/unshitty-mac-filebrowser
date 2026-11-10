//! One `places::Provider` per kind of place, and the registry that holds
//! them (design `2026-11-10-provider-crates-design.md`, step 2).
//!
//! Each provider here is a thin adapter over today's code, where it is: this
//! computer's over `fs_ops` and `thumbnails`, Google Drive's over
//! `cloud::service`, Arbesk's over the `arbesk` crate's `Client`, SSH's over
//! `sftp::SftpState`. The read side goes through them: listing, whether an
//! item exists, path info, thumbnails and opening. Writes (new folders,
//! rename, copy, move, Trash, restore) stay on their commands' own dispatch
//! until the steps that move them, so each adapter's write methods say so
//! instead of acting.
//!
//! Received shares are the one async place (D3): their listing and download
//! are still awaited on the async runtime by `list_dir` and `open_target`
//! (`share_listing`, `share_target` in `lib.rs`), until step 5 moves them
//! into `blocking(…)`. Their provider answers everything else.
//!
//! Each adapter reaches the state it wraps through [`Reach`], which `lib.rs`
//! answers with the app's managed state, and gets its folders at startup
//! ([`Folders`]): nothing here holds a Tauri type. Drive's and Arbesk's
//! server calls go through [`DriveCalls`] and [`ArbeskCalls`], so a test
//! answers them itself and still runs each adapter and the registry.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use places::{
    Capabilities, Cx, Entry, Error, Extensions, Fetch, Fetched, ListOptions, NewKind, OpenTarget,
    Operation, PathInfo, Progress, Provider, Refused, Registry, Replace, Result, Trashed, ext,
};

use crate::cloud::{self, CloudState};
use crate::{fs_ops, place, sftp, thumbnails};

/// The scheme of received shares, whose listing and download `lib.rs` still
/// awaits itself (see the module comment).
pub const SHARE: &str = "share";

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

/// The registry of every place the app shows, its providers reaching their
/// state through `states`.
pub fn registry<S>(states: S, folders: Folders) -> Registry
where
    S: Reach<CloudState> + Reach<arbesk::Client> + Reach<sftp::SftpState> + Clone,
{
    let Folders { cache, thumbnails, shared_thumbnails } = folders;
    registry_with(
        Local { thumbnails, shared_thumbnails },
        Drive { calls: LiveDrive(states.clone()), cache },
        Arbesk { calls: LiveArbesk(states.clone()) },
        Sftp { states },
    )
}

/// The registry over these providers, with received shares'.
fn registry_with(
    local: Local,
    drive: Drive<impl DriveCalls>,
    arbesk: Arbesk<impl ArbeskCalls>,
    sftp: Sftp<impl Reach<sftp::SftpState>>,
) -> Registry {
    Registry::new(Arc::new(local))
        .register(Arc::new(drive))
        .register(Arc::new(arbesk))
        .register(Arc::new(sftp))
        .register(Arc::new(Share))
}

// ---- What every adapter shares ---------------------------------------------------

/// Today's errors are words; they keep them, as `Other`, so a command
/// returns exactly the same text.
fn words(message: String) -> Error {
    Error::Other(message)
}

/// A write, or a read no command routes here yet: each still goes through
/// its command's own dispatch until the step that moves it.
fn not_yet<T>(what: &str) -> Result<T> {
    Err(Error::Other(format!("{what} doesn't go through the places registry yet.")))
}

/// The methods no command routes through a provider yet (see [`not_yet`]).
macro_rules! not_wired_yet {
    () => {
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
    };
}

/// The local-only gate's words for `path` (`place::local`), which every
/// place but this computer refuses Make Link, permissions and the like with.
fn local_only_refusal(path: &str) -> Option<String> {
    place::local(path).err()
}

/// A path's local-only refusal as an error: what opening a remote item
/// for the preview answers, as `preview_bytes` does.
fn not_previewed<T>(path: &str) -> Result<T> {
    Err(words(local_only_refusal(path).unwrap_or_default()))
}

/// `refused` with each operation that has words.
fn refusals(each: &[(Operation, Option<String>)]) -> Vec<Refused> {
    each.iter()
        .filter_map(|(operation, message)| {
            message.clone().map(|message| Refused { operation: *operation, message })
        })
        .collect()
}

/// A read-only place: SSH servers, received shares and, with its own
/// differences, Arbesk. Nothing is written there; `read_only` names the
/// refusal of every write, `local_only` the local-only gate's.
fn read_only(read_only: &str, local_only: Option<String>) -> Capabilities {
    let refused = || Some(read_only.to_string());
    Capabilities {
        read_only: true,
        can_create: false,
        can_rename: false,
        can_trash: false,
        can_copy_out: false,
        knows_sizes: true,
        knows_counts: false,
        searchable: false,
        has_thumbnails: false,
        // Nothing is renamed or replaced here; the values are a disk's.
        rename_atomic: false,
        replace: Replace::Trash,
        names_unique: true,
        put_is_whole: false,
        refused: refusals(&[
            (Operation::Create, refused()),
            (Operation::Rename, refused()),
            (Operation::Trash, refused()),
            (Operation::CopyIn, refused()),
            (Operation::CopyOut, refused()),
            (Operation::LocalOnly, local_only),
        ]),
    }
}

/// What a command gets back from a provider's fetch.
fn fetched(target: OpenTarget) -> Fetched {
    match target {
        OpenTarget::File { path } => Fetched::File(PathBuf::from(path)),
        OpenTarget::Url { url } => Fetched::Url(url),
    }
}

// ---- This computer ---------------------------------------------------------------

/// Files on this computer and mounted volumes. A path no other provider
/// claims comes here, so anything that isn't an absolute path is refused
/// with `place::local`'s words.
pub struct Local {
    thumbnails: Option<PathBuf>,
    shared_thumbnails: Option<PathBuf>,
}

/// What a path on this computer allows: everything, unless it isn't an
/// absolute path, which every operation refuses with the same words.
fn local_capabilities(path: &str) -> Capabilities {
    let not_local = local_only_refusal(path);
    Capabilities {
        read_only: false,
        can_create: true,
        can_rename: true,
        can_trash: true,
        can_copy_out: true,
        knows_sizes: true,
        knows_counts: true,
        searchable: true,
        has_thumbnails: true,
        rename_atomic: true,
        replace: Replace::Trash,
        names_unique: true,
        // A copy is staged by the transfer engine, not written whole.
        put_is_whole: false,
        refused: refusals(&[
            (Operation::Create, not_local.clone()),
            (Operation::Rename, not_local.clone()),
            (Operation::Trash, not_local.clone()),
            (Operation::CopyIn, not_local.clone()),
            (Operation::CopyOut, not_local.clone()),
            (Operation::LocalOnly, not_local),
        ]),
    }
}

impl Provider for Local {
    fn scheme(&self) -> &'static str {
        ""
    }
    fn capabilities(&self, path: &str) -> Capabilities {
        local_capabilities(path)
    }
    fn list(&self, dir: &str, opts: &ListOptions, _: &Cx) -> Result<Vec<Entry>> {
        place::local(dir).map_err(words)?;
        fs_ops::list_dir(dir.to_string(), opts.show_hidden).map_err(words)
    }
    fn exists(&self, path: &str, _: &Cx) -> bool {
        place::local(path).is_ok_and(|path| fs_ops::path_exists(path.to_string_lossy().into()))
    }
    fn path_info(&self, paths: &[String], _: &Cx) -> Vec<Option<PathInfo>> {
        place::local_only(paths, fs_ops::path_info)
    }
    /// The file itself, for opening and for the preview.
    fn fetch(&self, path: &str, _: Fetch, _: &Cx) -> Result<Fetched> {
        place::local(path).map_err(words)?;
        Ok(Fetched::File(PathBuf::from(path)))
    }
    not_wired_yet!();
    fn extensions(&self) -> Extensions<'_> {
        Extensions { thumbnails: Some(self), ..Extensions::default() }
    }
}

impl ext::Thumbnails for Local {
    /// The desktop's own thumbnail (Linux), Quick Look or the image
    /// decoder, through the disk cache; none for a path that isn't local.
    fn thumbnail(&self, path: &str, size: u32, stop: &dyn Fn() -> bool) -> Vec<u8> {
        if place::local(path).is_err() {
            return Vec::new();
        }
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

/// Google Drive's server calls the provider makes.
pub trait DriveCalls: Send + Sync + 'static {
    fn list(&self, dir: &str, show_hidden: bool) -> std::result::Result<Vec<Entry>, String>;
    fn exists(&self, path: &str) -> bool;
    fn path_info(&self, path: &str) -> std::result::Result<PathInfo, String>;
    fn open(&self, path: &str, cache: &Path, progress: &Progress) -> std::result::Result<OpenTarget, String>;
    fn thumbnail(&self, path: &str, size: u32) -> Vec<u8>;
}

/// `cloud::service`'s calls on the app's `CloudState`.
pub struct LiveDrive<S>(S);

impl<S: Reach<CloudState>> DriveCalls for LiveDrive<S> {
    fn list(&self, dir: &str, show_hidden: bool) -> std::result::Result<Vec<Entry>, String> {
        cloud::service::list_dir(self.0.reach(), dir, show_hidden)
    }
    fn exists(&self, path: &str) -> bool {
        cloud::service::path_exists(self.0.reach(), path)
    }
    fn path_info(&self, path: &str) -> std::result::Result<PathInfo, String> {
        cloud::service::path_info(self.0.reach(), path)
    }
    fn open(&self, path: &str, cache: &Path, progress: &Progress) -> std::result::Result<OpenTarget, String> {
        cloud::service::open(self.0.reach(), path, cache, progress)
    }
    fn thumbnail(&self, path: &str, size: u32) -> Vec<u8> {
        cloud::service::thumbnail(self.0.reach(), path, size)
    }
}

/// Google Drive, over `cloud::service` (through `calls`).
pub struct Drive<C> {
    calls: C,
    cache: std::result::Result<PathBuf, String>,
}

fn drive_capabilities(path: &str) -> Capabilities {
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
        refused: refusals(&[(Operation::LocalOnly, local_only_refusal(path))]),
    }
}

impl<C: DriveCalls> Provider for Drive<C> {
    fn scheme(&self) -> &'static str {
        "gdrive"
    }
    fn capabilities(&self, path: &str) -> Capabilities {
        drive_capabilities(path)
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
            return not_previewed(path);
        }
        let cache = self.cache.clone().map_err(words)?;
        self.calls.open(path, &cache, &cx.progress).map(fetched).map_err(words)
    }
    not_wired_yet!();
    fn extensions(&self) -> Extensions<'_> {
        Extensions { thumbnails: Some(self), ..Extensions::default() }
    }
}

impl<C: DriveCalls> ext::Thumbnails for Drive<C> {
    /// Drive's own thumbnail; a request, once sent, isn't stopped.
    fn thumbnail(&self, path: &str, size: u32, _stop: &dyn Fn() -> bool) -> Vec<u8> {
        self.calls.thumbnail(path, size)
    }
}

// ---- Arbesk ----------------------------------------------------------------------

/// The `arbesk` crate's calls the provider makes.
pub trait ArbeskCalls: Send + Sync + 'static {
    fn list(&self, dir: &str) -> std::result::Result<Vec<arbesk::Entry>, String>;
    fn exists(&self, path: &str) -> bool;
    fn studio_url(&self, path: &str) -> std::result::Result<String, String>;
    fn build(&self, path: &str, progress: &Progress) -> std::result::Result<PathBuf, String>;
    fn thumbnail(&self, path: &str, size: u32, stop: &dyn Fn() -> bool) -> Vec<u8>;
}

/// The app's `arbesk::Client`.
pub struct LiveArbesk<S>(S);

impl<S: Reach<arbesk::Client>> ArbeskCalls for LiveArbesk<S> {
    fn list(&self, dir: &str) -> std::result::Result<Vec<arbesk::Entry>, String> {
        self.0.reach().list(dir)
    }
    fn exists(&self, path: &str) -> bool {
        self.0.reach().exists(path)
    }
    fn studio_url(&self, path: &str) -> std::result::Result<String, String> {
        self.0.reach().studio_url(path)
    }
    fn build(&self, path: &str, progress: &Progress) -> std::result::Result<PathBuf, String> {
        self.0.reach().build(path, progress)
    }
    fn thumbnail(&self, path: &str, size: u32, stop: &dyn Fn() -> bool) -> Vec<u8> {
        self.0.reach().thumbnail(path, size, stop)
    }
}

/// Arbesk accounts, over the `arbesk` crate's `Client` (through `calls`).
pub struct Arbesk<C> {
    calls: C,
}

/// Arbesk is read only but for copies in: a local model file into a
/// collection (`place::copy`), and an asset out (built first). Its sizes are
/// known only once an asset is built, and a save is a new version.
fn arbesk_capabilities(path: &str) -> Capabilities {
    const READ_ONLY: &str = "Arbesk is read only.";
    let parsed = arbesk::parse(path);
    let is_asset = matches!(parsed, Ok(arbesk::Location::Asset { .. }));
    let copy_in = match &parsed {
        Ok(arbesk::Location::Collection { .. }) => None,
        Ok(_) => Some("Paste into a collection.".to_string()),
        Err(error) => Some(error.clone()),
    };
    let copy_out = match &parsed {
        Ok(arbesk::Location::Asset { .. }) => None,
        Ok(_) => Some("Copying Arbesk collections comes next.".to_string()),
        Err(error) => Some(error.clone()),
    };
    Capabilities {
        can_copy_out: is_asset,
        knows_sizes: false,
        has_thumbnails: true,
        replace: Replace::NewVersion,
        // A save lands whole, once Arbesk has it.
        put_is_whole: true,
        refused: refusals(&[
            (Operation::Create, Some(READ_ONLY.into())),
            (Operation::Rename, Some(READ_ONLY.into())),
            (Operation::Trash, Some(READ_ONLY.into())),
            (Operation::CopyIn, copy_in),
            (Operation::CopyOut, copy_out),
            (Operation::LocalOnly, local_only_refusal(path)),
        ]),
        ..read_only(READ_ONLY, None)
    }
}

impl<C: ArbeskCalls> Provider for Arbesk<C> {
    fn scheme(&self) -> &'static str {
        "arbesk"
    }
    fn capabilities(&self, path: &str) -> Capabilities {
        arbesk_capabilities(path)
    }
    fn list(&self, dir: &str, _: &ListOptions, _: &Cx) -> Result<Vec<Entry>> {
        let entries = self.calls.list(dir).map_err(words)?;
        Ok(entries.into_iter().map(crate::arbesk_entry).collect())
    }
    /// A name pending in a collection exists when the collection lists it.
    fn exists(&self, path: &str, _: &Cx) -> bool {
        self.calls.exists(path)
    }
    /// Accounts and collections are folders, assets files; nothing for a
    /// path Arbesk can't read.
    fn path_info(&self, paths: &[String], _: &Cx) -> Vec<Option<PathInfo>> {
        let info = |path: &String| arbesk::is_folder(path).map(|is_dir| PathInfo { is_dir, device: 0 });
        paths.iter().map(info).collect()
    }
    /// An asset opens in Arbesk's Studio, in the browser; the preview gets
    /// its file built in the app cache.
    fn fetch(&self, path: &str, purpose: Fetch, cx: &Cx) -> Result<Fetched> {
        match purpose {
            Fetch::Open => self.calls.studio_url(path).map(Fetched::Url).map_err(words),
            Fetch::Preview => self.calls.build(path, &cx.progress).map(Fetched::File).map_err(words),
        }
    }
    not_wired_yet!();
    fn extensions(&self) -> Extensions<'_> {
        Extensions { thumbnails: Some(self), ..Extensions::default() }
    }
}

impl<C: ArbeskCalls> ext::Thumbnails for Arbesk<C> {
    /// The thumbnail the asset was saved with, if any.
    fn thumbnail(&self, path: &str, size: u32, stop: &dyn Fn() -> bool) -> Vec<u8> {
        self.calls.thumbnail(path, size, stop)
    }
}

// ---- SSH servers -----------------------------------------------------------------

/// SSH servers, read only, over `sftp::SftpState`.
pub struct Sftp<S> {
    states: S,
}

impl<S: Reach<sftp::SftpState>> Provider for Sftp<S> {
    fn scheme(&self) -> &'static str {
        "sftp"
    }
    fn capabilities(&self, path: &str) -> Capabilities {
        read_only("SSH servers are read only for now.", local_only_refusal(path))
    }
    fn list(&self, dir: &str, opts: &ListOptions, _: &Cx) -> Result<Vec<Entry>> {
        self.states.reach().list(dir, opts.show_hidden).map_err(words)
    }
    /// Nothing is looked up on a server: nothing is pasted there.
    fn exists(&self, _: &str, _: &Cx) -> bool {
        false
    }
    fn path_info(&self, paths: &[String], _: &Cx) -> Vec<Option<PathInfo>> {
        paths.iter().map(|_| None).collect()
    }
    /// The file, downloaded into the app cache.
    fn fetch(&self, path: &str, purpose: Fetch, cx: &Cx) -> Result<Fetched> {
        if purpose == Fetch::Preview {
            return not_previewed(path);
        }
        let file = self.states.reach().open(path, &cx.progress).map_err(words)?;
        Ok(Fetched::File(PathBuf::from(file)))
    }
    not_wired_yet!();
}

// ---- Received shares -------------------------------------------------------------

/// Folders shared with this computer, read only. Their listing and download
/// are async and stay in `lib.rs` for now (see the module comment).
pub struct Share;

impl Provider for Share {
    fn scheme(&self) -> &'static str {
        SHARE
    }
    fn capabilities(&self, path: &str) -> Capabilities {
        read_only("Shared folders are read only.", local_only_refusal(path))
    }
    fn list(&self, _: &str, _: &ListOptions, _: &Cx) -> Result<Vec<Entry>> {
        not_yet("Listing a shared folder")
    }
    fn exists(&self, _: &str, _: &Cx) -> bool {
        false
    }
    fn path_info(&self, paths: &[String], _: &Cx) -> Vec<Option<PathInfo>> {
        paths.iter().map(|_| None).collect()
    }
    fn fetch(&self, path: &str, purpose: Fetch, _: &Cx) -> Result<Fetched> {
        match purpose {
            Fetch::Open => not_yet("Opening a shared file"),
            Fetch::Preview => not_previewed(path),
        }
    }
    not_wired_yet!();
}

/// Stand-ins for the servers, so tests run the adapters and the registry
/// without Tauri or a network.
#[cfg(test)]
pub(crate) mod testing {
    use super::*;

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
    }

    /// Arbesk: whether an item exists and an asset's build, as the test
    /// says; anything else isn't asked in tests.
    pub struct StubArbesk {
        pub exists: Answer<bool>,
        pub build: Answer<std::result::Result<PathBuf, String>>,
    }

    impl Default for StubArbesk {
        fn default() -> Self {
            StubArbesk { exists: unexpected("Arbesk"), build: unexpected("Arbesk") }
        }
    }

    impl ArbeskCalls for StubArbesk {
        fn list(&self, dir: &str) -> std::result::Result<Vec<arbesk::Entry>, String> {
            unexpected("Arbesk")(dir)
        }
        fn exists(&self, path: &str) -> bool {
            (self.exists)(path)
        }
        fn studio_url(&self, path: &str) -> std::result::Result<String, String> {
            unexpected("Arbesk")(path)
        }
        fn build(&self, path: &str, _: &Progress) -> std::result::Result<PathBuf, String> {
            (self.build)(path)
        }
        fn thumbnail(&self, path: &str, _: u32, _: &dyn Fn() -> bool) -> Vec<u8> {
            unexpected("Arbesk")(path)
        }
    }

    /// No SSH server: the tests never reach one.
    pub struct NoServer;

    impl Reach<sftp::SftpState> for NoServer {
        fn reach(&self) -> &sftp::SftpState {
            panic!("no SSH server in tests")
        }
    }

    /// The registry with Drive and Arbesk answered by `drive` and `arbesk`.
    pub fn registry(drive: StubDrive, arbesk: StubArbesk) -> Registry {
        registry_with(
            Local { thumbnails: None, shared_thumbnails: None },
            Drive { calls: drive, cache: Err("No app cache in tests.".into()) },
            Arbesk { calls: arbesk },
            Sftp { states: NoServer },
        )
    }

    /// The registry, with no server expected to be asked anything.
    pub fn plain() -> Registry {
        registry(StubDrive::default(), StubArbesk::default())
    }
}

#[cfg(test)]
mod tests {
    use super::testing::{StubArbesk, StubDrive};
    use super::*;
    use crate::place::Kind;

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
            Kind::Share => SHARE,
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
            for operation in [Operation::Create, Operation::Rename, Operation::Trash] {
                assert_eq!(refusal(&caps, operation), write, "{path} {operation:?}");
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
    fn a_fetched_target_keeps_its_path_or_url() {
        let file = fetched(OpenTarget::File { path: "/cache/x/a.pdf".into() });
        assert_eq!(file, Fetched::File("/cache/x/a.pdf".into()));
        let url = fetched(OpenTarget::Url { url: "https://docs.google.com/d".into() });
        assert_eq!(url, Fetched::Url("https://docs.google.com/d".into()));
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
            assert_eq!(refused.unwrap_err().to_string(), words, "{path}");
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

    /// The writes and the lookups stay on their commands' dispatch in this
    /// step; a provider says so rather than acting.
    #[test]
    fn writes_are_not_routed_through_providers_yet() {
        let registry = testing::plain();
        let cx = Cx::default();
        for path in ["/Users/me/a", "gdrive://acct/root~x", ARBESK, "sftp://me@host/a", "share://r1/a"] {
            let provider = registry.provider_for(path);
            let not_yet = |result: Result<String>| result.unwrap_err().to_string().ends_with("registry yet.");
            assert!(not_yet(provider.create(path, "b", NewKind::Folder, &cx)), "{path}");
            assert!(not_yet(provider.rename(path, "b", &cx)), "{path}");
            assert!(not_yet(provider.restore(path, &cx)), "{path}");
            assert!(provider.stat(path, &cx).is_err() && provider.lookup(path, &cx).is_err(), "{path}");
            let trashed = provider.trash(&[path], &cx);
            assert_eq!(trashed.len(), 1);
            assert!(trashed[0].result.is_err(), "{path}");
        }
        assert!(registry.provider_for("share://r1/a").list("share://r1/a", &ListOptions::default(), &cx).is_err());
        assert!(registry.provider_for("share://r1/a").fetch("share://r1/a", Fetch::Open, &cx).is_err());
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
        let registry = testing::registry(drive, StubArbesk::default());
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

    /// Arbesk's adapter asks its client whether a name pending in a
    /// collection exists, and has an asset built for the preview.
    #[test]
    fn arbesk_asks_its_client_for_pending_names_and_preview_builds() {
        const PENDING: &str = "arbesk://0x00000000000000000000000000000000000000ab/12~Set/~Box.gltf";
        let arbesk = StubArbesk {
            exists: Box::new(|path| path == PENDING),
            build: Box::new(|path| match path {
                ASSET => Ok(PathBuf::from("/cache/arbesk/Box.gltf")),
                _ => Err("Not an Arbesk asset.".into()),
            }),
        };
        let registry = testing::registry(StubDrive::default(), arbesk);
        let (provider, cx) = (registry.provider_for(PENDING), Cx::default());
        assert!(provider.exists(PENDING, &cx));
        assert!(!provider.exists(&format!("{ARBESK}/~Other.glb"), &cx));
        let built = provider.fetch(ASSET, Fetch::Preview, &cx).unwrap();
        assert_eq!(built, Fetched::File("/cache/arbesk/Box.gltf".into()));
        assert_eq!(provider.fetch(ARBESK, Fetch::Preview, &cx).unwrap_err().to_string(), "Not an Arbesk asset.");
    }

    /// The app's own states, as `lib.rs`' managed state holds them.
    struct States {
        cloud: CloudState,
        arbesk: arbesk::Client,
        sftp: sftp::SftpState,
    }

    impl Reach<CloudState> for Arc<States> {
        fn reach(&self) -> &CloudState {
            &self.cloud
        }
    }

    impl Reach<arbesk::Client> for Arc<States> {
        fn reach(&self) -> &arbesk::Client {
            &self.arbesk
        }
    }

    impl Reach<sftp::SftpState> for Arc<States> {
        fn reach(&self) -> &sftp::SftpState {
            &self.sftp
        }
    }

    /// The app's registry, over its real states: each kind reaches its
    /// provider, and a malformed Drive or Arbesk path is answered without
    /// a server, as `cloud::service` and the `arbesk` client answer it.
    #[test]
    fn the_apps_registry_reaches_each_kinds_state() {
        let dir = tempfile::tempdir().unwrap();
        let states = Arc::new(States {
            cloud: CloudState::new(dir.path()),
            arbesk: arbesk::Client::new(dir.path().join("cache"), dir.path().join("a.json"), dir.path().join("s")),
            sftp: sftp::SftpState::new(dir.path().join("known_hosts"), dir.path().into(), dir.path().join("sftp")),
        });
        let folders = Folders { cache: Ok(dir.path().join("cache")), thumbnails: None, shared_thumbnails: None };
        let registry = registry(states, folders);
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
}
