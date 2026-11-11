//! The interface every place implements (design D3–D6): a small blocking
//! [`Provider`] with POSIX-like verbs on path strings, the [`Transfer`]
//! primitives the app's transfer engine builds copy and move from, and the
//! optional, non-POSIX [`Extensions`].
//!
//! Commands call providers only on blocking threads (`blocking(…)` in the
//! app), so nothing here is async. A provider parses its own scheme and may
//! keep ids inside its paths; paths it returns are never rewritten.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::{Entry, Error, OpenTarget, PathInfo, Progress, Result};

// ---- What a place allows -------------------------------------------------------

/// How Replace treats the item it replaces.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Replace {
    /// The old item goes to the Trash first and comes back if the new one
    /// can't take its place (this computer, Google Drive).
    Trash,
    /// The new content is saved as a new version of the item (Arbesk).
    NewVersion,
}

/// An operation a place may refuse, named for the wording it refuses with.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Operation {
    Create,
    Rename,
    Trash,
    CopyIn,
    CopyOut,
    /// Adding files to an item as its other files (an Arbesk asset's
    /// representations, spec F13): `can_attach` is whether it isn't refused.
    Attach,
    /// A feature only items on this computer have (the local-only gate,
    /// D10): its message is the place's “This action is not available for
    /// …” text, as `place::local` words it today.
    LocalOnly,
}

/// One refused operation and the backend's words for it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Refused {
    pub operation: Operation,
    pub message: String,
}

/// What a path allows and how its operations behave (D5). Answered without
/// the network; the same for every path of a kind of place, so the app can
/// hand the frontend one table per provider.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Capabilities {
    pub read_only: bool,
    pub can_create: bool,
    pub can_rename: bool,
    pub can_trash: bool,
    pub can_copy_out: bool,
    pub knows_sizes: bool,
    pub knows_counts: bool,
    pub searchable: bool,
    pub has_thumbnails: bool,
    /// A rename refuses a taken name atomically (on disk), rather than
    /// checking first and then renaming (Drive).
    pub rename_atomic: bool,
    pub replace: Replace,
    /// Names are unique in a folder (false on Drive, where the engine asks
    /// [`Provider::lookup`] and refuses an ambiguous name).
    pub names_unique: bool,
    /// A written file appears only once whole (a Drive upload).
    pub put_is_whole: bool,
    /// The backend's wording for each operation it refuses here.
    pub refused: Vec<Refused>,
}

impl Capabilities {
    /// A read-only place (an SSH server, a received share): sizes are
    /// known, nothing else is offered, every write is refused with
    /// `read_only`'s words and the features of items on this computer with
    /// `local_only`'s. Nothing is renamed or replaced there; the behaviour
    /// values are a disk's.
    pub fn read_only_place(read_only: &str, local_only: &str) -> Self {
        let refused = |operation| Refused { operation, message: read_only.to_string() };
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
            rename_atomic: false,
            replace: Replace::Trash,
            names_unique: true,
            put_is_whole: false,
            refused: vec![
                refused(Operation::Create),
                refused(Operation::Rename),
                refused(Operation::Trash),
                refused(Operation::CopyIn),
                refused(Operation::CopyOut),
                refused(Operation::Attach),
                Refused { operation: Operation::LocalOnly, message: local_only.to_string() },
            ],
        }
    }
}

/// The words a place that keeps no other files of an item refuses adding
/// them with (spec F13): only an Arbesk asset takes them.
pub const ADD_TO_AN_ASSET: &str = "Add files to an asset.";

// ---- Arguments and results ---------------------------------------------------

/// How to list a folder.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ListOptions {
    /// Include items whose name starts with a dot.
    pub show_hidden: bool,
}

/// The context of one call: the progress (and cancel) it reports into, and
/// the app cache folder where a provider may leave downloads and builds.
#[derive(Clone, Debug, Default)]
pub struct Cx {
    pub progress: Progress,
    pub cache_dir: PathBuf,
}

impl Cx {
    /// `Err(Error::Cancelled)` once the user cancelled; call between chunks
    /// and items.
    pub fn check(&self) -> Result<()> {
        if self.progress.is_cancelled() {
            Err(Error::cancelled())
        } else {
            Ok(())
        }
    }
}

/// Why an item's bytes are wanted from [`Provider::fetch`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fetch {
    /// To hand to the default app or Open With.
    Open,
    /// To show in the Space preview.
    Preview,
}

/// What [`Provider::fetch`] gives: a local file with the item's bytes (the
/// file itself on this computer, a marked copy in the app cache elsewhere),
/// or a URL to open instead.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Fetched {
    File(PathBuf),
    Url(String),
}

impl From<Fetched> for OpenTarget {
    fn from(fetched: Fetched) -> Self {
        match fetched {
            Fetched::File(path) => OpenTarget::File {
                path: path.to_string_lossy().into_owned(),
            },
            Fetched::Url(url) => OpenTarget::Url { url },
        }
    }
}

/// What [`Provider::create`] makes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NewKind {
    Folder,
    EmptyFile,
}

/// One outcome of Move to Trash: the item as asked for, and where it went
/// (for Undo; `None` when the place can't tell) or why it stayed.
#[derive(Debug, PartialEq, Eq)]
pub struct Trashed {
    pub path: String,
    pub result: Result<Option<String>>,
}

/// What a [`Transfer`] write or read left: the final path, and the items it
/// skipped, named as the user sees them (a Google Doc in a downloaded
/// folder). The engine makes a transfer with skipped items an error, so a
/// move keeps its source.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Put {
    pub path: String,
    pub skipped: Vec<String>,
}

// ---- The provider --------------------------------------------------------------

/// One kind of place, addressed by its paths. Every method a provider can't
/// do returns [`Error::Unsupported`] (or [`Error::ReadOnly`]) with its own
/// wording.
pub trait Provider: Send + Sync {
    /// "" for this computer, else the scheme without "://".
    fn scheme(&self) -> &'static str;
    /// What this path allows, and how. Cheap: no network.
    fn capabilities(&self, path: &str) -> Capabilities;

    fn list(&self, dir: &str, opts: &ListOptions, cx: &Cx) -> Result<Vec<Entry>>;
    /// The item at `path`, or `None` when nothing is there.
    fn stat(&self, path: &str, cx: &Cx) -> Result<Option<Entry>>;
    /// Every item at a pending `parent/~name`: more than one where names
    /// aren't unique (Drive), so Replace can refuse instead of guessing.
    fn lookup(&self, pending: &str, cx: &Cx) -> Result<Vec<Entry>>;
    /// Whether an item is at `path` (for a pending `parent/~name`, whether
    /// its folder has an item of that name), as `path_exists` answers; false
    /// when the place can't tell, or failed to.
    fn exists(&self, path: &str, cx: &Cx) -> bool;
    /// What drag and drop needs to know of each of `paths` (all this
    /// provider's), one answer per path in order: `None` where the place has
    /// nothing to say. One call per batch, so this computer asks the disk
    /// once.
    fn path_info(&self, paths: &[String], cx: &Cx) -> Vec<Option<PathInfo>>;

    /// For open and preview only: a local file with this item's bytes (the
    /// file itself here, a download or a build in the app cache elsewhere,
    /// marked with the `.downloaded` mark), or a URL to open instead.
    fn fetch(&self, path: &str, purpose: Fetch, cx: &Cx) -> Result<Fetched>;
    /// Whether [`fetch`](Provider::fetch) may take a while and reports into
    /// its `Cx`'s progress (a download or a build), so opening an item here
    /// is an operation the app tracks and the user can cancel. False where
    /// `fetch` hands back the item itself (this computer), which starts no
    /// operation.
    fn fetch_reports_progress(&self) -> bool {
        true
    }

    /// New folder or empty file at `parent/name`, never over an existing
    /// name. Returns the final path (Drive learns its id only now).
    fn create(&self, parent: &str, name: &str, kind: NewKind, cx: &Cx) -> Result<String>;
    /// Rename in place, never over another item; returns the new path.
    fn rename(&self, path: &str, new_name: &str, cx: &Cx) -> Result<String>;
    /// Move to Trash, as the user asks for it; one outcome per path.
    fn trash(&self, paths: &[&str], cx: &Cx) -> Vec<Trashed>;
    /// Put a trashed item back where it came from, never over another item;
    /// returns where it went.
    fn restore(&self, trashed: &str, cx: &Cx) -> Result<String>;

    /// Run `work` once on the transfer primitives the engine builds copy
    /// and move from, for one whole transfer: every step runs on the same
    /// session, so what an early step learnt serves the later ones (Drive:
    /// one context, so the steps after `landing` ask the server nothing
    /// again). A provider that can't be a transfer target (read only) never
    /// runs `work`. Callers use [`with_transfer`], which hands back what
    /// `work` returned.
    fn transfer(&self, _cx: &Cx, _work: &mut dyn FnMut(&dyn Transfer)) {}
    /// Optional, non-POSIX parts.
    fn extensions(&self) -> Extensions<'_> {
        Extensions::default()
    }
}

/// `work` on `provider`'s transfer session ([`Provider::transfer`]): its
/// answer, or `None` when the provider has no transfer primitives.
pub fn with_transfer<T>(provider: &dyn Provider, cx: &Cx, work: impl FnOnce(&dyn Transfer) -> T) -> Option<T> {
    let mut work = Some(work);
    let mut out = None;
    provider.transfer(cx, &mut |t| {
        if let Some(work) = work.take() {
            out = Some(work(t));
        }
    });
    out
}

/// What is at a transfer's pending target in a place, as [`Transfer::landing`]
/// found it: the target's folder and name, the items already there under
/// that name, and the source when it is this place's own item. The engine
/// decides from it: a taken name without `overwrite`, an ambiguous one
/// where names aren't unique, a folder replaced by its own child.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Landing {
    /// The target's folder, as a path of this place.
    pub parent: String,
    /// The new name.
    pub name: String,
    /// The items already called `name` in `parent`, the source excepted.
    pub existing: Vec<Existing>,
    /// The source, for a transfer inside this place.
    pub source: Option<Source>,
}

/// An item already at a transfer's target name.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Existing {
    pub path: String,
    /// The source is inside it: replacing it would replace a folder with
    /// its own child.
    pub holds_source: bool,
}

/// The source of a transfer inside one place, as the place sees it now.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Source {
    pub path: String,
    pub is_dir: bool,
    /// Where a move puts it back, as it was, when a Replace fails after it
    /// moved: a pending path in its old folder under its own name. `None`
    /// at the top of a place, where nothing moves.
    pub home: Option<String>,
}

/// An item [`Transfer::read`] is reading, handed to the engine once the
/// place knows what it is, so the engine can pick the local target (a new
/// folder fills in place, a file is staged) before any byte is read.
pub trait Reading {
    fn is_dir(&self) -> bool;
    /// Write the item at `dest`, which the call creates (a file, or a
    /// folder with everything in it). Returns the items it skipped, each as
    /// the message the transfer fails with.
    fn fill(&self, dest: &Path, cx: &Cx) -> std::result::Result<Vec<String>, Unread>;
}

/// A [`Reading::fill`] that failed, and whether it left anything at its
/// path for the engine to clean up.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Unread {
    /// Nothing of the read is there (a folder that could not be created).
    Nothing(Error),
    /// A partial copy is there.
    Partial(Error),
}

/// The primitives a provider offers the transfer engine (D6), each as safely
/// as it can, declared in its [`Capabilities`]. Which pairs of places may
/// transfer, the refusals, the name-clash decision and the order of a
/// Replace are the engine's, not the provider's. A transfer between two
/// different providers meets at a local path (a write reads a local source,
/// a read fills a local target); a copy inside one provider is
/// [`Transfer::copy_within`], so it never leaves the place (Drive copies on
/// the server).
///
/// Not `Send + Sync`: the engine uses one on the blocking thread that runs
/// the transfer (Drive's is the context of that one transfer).
pub trait Transfer {
    /// The place's own refusals for a transfer, before any request: of a
    /// source this place must later move away (`from` alone: a move to
    /// another place), of a target in it (`to` alone: a copy in), or of a
    /// transfer inside it (both: its ids decide what is the same item or
    /// inside another), in the order the user meets them.
    fn check(&self, from: Option<&str>, to: Option<&str>, cx: &Cx) -> Result<()>;
    /// What is at the pending target `to`, for a copy in (`from` is `None`)
    /// or a transfer of this place's own `from` (which must still be there).
    fn landing(&self, from: Option<&str>, to: &str, cx: &Cx) -> Result<Landing>;
    /// The pending path of `name` in this place's folder `parent`.
    fn pending(&self, parent: &str, name: &str) -> Result<String>;
    /// Read this provider's item at `path` into a local target the engine
    /// picks: `place` gets the item once the place knows what it is, and
    /// fills the target through it ([`Reading::fill`]), streaming straight
    /// into it (no copy in the app cache). What `place` returns is the
    /// read's result.
    fn read(&self, path: &str, place: &mut dyn FnMut(&dyn Reading) -> Result<()>, cx: &Cx) -> Result<()>;
    /// Write the local file or folder `source` to `target`, a path in this
    /// provider the engine chose (a staging name, or the final name where a
    /// file appears only whole). `replace` says the engine decided to
    /// replace an item at `target`: where [`Capabilities::replace`] is
    /// [`Replace::NewVersion`] (Arbesk) the write saves a new version of it;
    /// where it is [`Replace::Trash`] the engine has moved the old item
    /// away itself and passes `false`. Skipped items are named in the
    /// result.
    fn write(&self, source: &Path, target: &str, replace: bool, cx: &Cx) -> Result<Put>;
    /// Read this provider's item at `path` into the local `target` as it
    /// is, without staging: [`Transfer::read`] with `target` filled
    /// whatever the item is. A provider that can only [`Transfer::export`]
    /// returns [`Error::Unsupported`].
    fn read_into(&self, path: &str, target: &Path, cx: &Cx) -> Result<Put>;
    /// A local file with the item's bytes that the engine copies from as a
    /// local source, where the place builds the file before anything can
    /// be read (Arbesk: the asset's own `.gltf`/`.3mf`, built in the app
    /// cache and reused while unchanged). A provider that reads straight
    /// into a target ([`Transfer::read_into`]) returns
    /// [`Error::Unsupported`].
    fn export(&self, path: &str, cx: &Cx) -> Result<PathBuf>;
    /// Copy this provider's own item at `from` to `to`, a path in this
    /// provider the engine chose (a staging name for a folder or a Replace,
    /// as for [`Transfer::write`]), without its bytes passing through this
    /// computer: Drive's server-side `files.copy`, which also copies Google
    /// Docs and shortcuts. A provider whose same-place copy the engine makes
    /// itself (this computer: its copy is the engine's staged local copy)
    /// returns [`Error::Unsupported`].
    fn copy_within(&self, from: &str, to: &str, cx: &Cx) -> Result<Put>;
    /// Rename `from` to `to` in this provider, never replacing an item at
    /// `to`; returns the final path.
    fn rename_new(&self, from: &str, to: &str, cx: &Cx) -> Result<String>;
    /// Move this provider's own item to any path in it, never replacing;
    /// moving back is the same call the other way. Returns the final path.
    fn move_to(&self, from: &str, to: &str, cx: &Cx) -> Result<String>;
    /// Send an item to this provider's Trash for the engine: the item a
    /// Replace replaces, or a source a move to another place has copied
    /// away. Returns what [`Transfer::take_back`] needs to bring it back.
    fn trash_for_replace(&self, path: &str, cx: &Cx) -> Result<String>;
    /// Bring an item [`Transfer::trash_for_replace`] trashed back to
    /// `original`, without making parent folders, unless something took
    /// its place.
    fn take_back(&self, trashed: &str, original: &str, cx: &Cx) -> Result<()>;
    /// Discard a staged or partial copy the engine made in this provider
    /// with its own primitives (never the user's own item): removed for good
    /// here, `trashed=true` on Drive. Policies that differ by route stay the
    /// engine's, as `DiscardFn` is today: a Drive download's partial local
    /// folder goes to the logged local Trash through the app's Trash, not
    /// through this method.
    fn discard(&self, staged: &str, cx: &Cx) -> Result<()>;
    /// Run `run`, a whole transfer from `from` to `to` inside this place
    /// (a copy, or a move when `moving`), as the place keeps it: what it
    /// keeps about its items stays consistent around it (on this computer,
    /// Keep Versions closes the histories inside both ends for it and
    /// reopens them after), and its primitives word their failures as the
    /// transfer's (a move's or a copy's). Returns what `run` returns; by
    /// default, just `run`.
    fn within(&self, _from: &str, _to: &str, _moving: bool, run: &mut dyn FnMut() -> Result<String>) -> Result<String> {
        run()
    }
}

// ---- Extensions ----------------------------------------------------------------

/// What isn't POSIX, one trait per feature (D4). A provider lacking one
/// leaves it `None`, and the app checks for it instead of matching on a
/// kind of place.
///
/// Each trait gets its methods in the migration step that first implements
/// it, shaped like the command it replaces so nothing visible changes:
/// thumbnails as PNG bytes with the per-window cancel ticket (step 2);
/// Arbesk's versions, accounts, sharing (its numeric role and its
/// [`Shared`](crate::Shared) result) and preview (step 3); item counts as
/// `FolderSummary` (with the version badges), search with `show_hidden`,
/// its limit, the per-window stop and its summary, a folder size as bytes
/// cancelled by walk id.
pub mod ext {
    use std::path::PathBuf;

    use crate::{Cx, FileProperties, Json, Result, Shared, SignIn, Version};

    /// Thumbnails of items (`thumbnail`'s per-kind arms).
    pub trait Thumbnails: Send + Sync {
        /// PNG bytes of a thumbnail of `path`, at most `size` pixels square;
        /// empty when it has none, or once `stop` says the window's request
        /// (its cancel ticket) was cancelled. A place that can't stop
        /// half-way may ignore `stop`.
        fn thumbnail(&self, path: &str, size: u32, stop: &dyn Fn() -> bool) -> Vec<u8>;
    }

    /// What the Properties dialog shows, and a folder's total size
    /// (`file_properties`, `folder_size`). Only a place whose items this
    /// computer's and Drive's module don't describe fills it (Arbesk's
    /// assets, spec F7); folder sizes stay with their command.
    pub trait Properties: Send + Sync {
        /// What the Properties dialog shows of the item at `path`.
        fn properties(&self, path: &str) -> Result<FileProperties>;
    }

    /// Item counts and version badges of folders, for the view
    /// (`item_counts`).
    pub trait Counts: Send + Sync {}

    /// A recursive search, its results streamed in batches (`search_dir`).
    pub trait Search: Send + Sync {}

    /// Sizes and item counts before a copy, for the progress total
    /// (`measure`).
    pub trait Measure: Send + Sync {}

    /// The saves of an item, oldest first (`arbesk_versions`).
    pub trait Versions: Send + Sync {
        /// The saves of the item at `path`, oldest first.
        fn versions(&self, path: &str) -> Result<Vec<Version>>;
    }

    /// Accounts a person signs in to (`arbesk_accounts`,
    /// `arbesk_connect_*`, `arbesk_signed_in`, `arbesk_disconnect`). Each
    /// provider keeps its own sign-in step and its own account record,
    /// handed on as it serialises it. Shaped as Arbesk signs in (an email,
    /// then the browser); the next provider to implement it widens it.
    pub trait Accounts: Send + Sync {
        /// The connected accounts.
        fn accounts(&self) -> Vec<Json>;
        /// Start a sign-in for `email`.
        fn sign_in_start(&self, email: &str) -> Result<SignIn>;
        /// Wait for the browser, then connect the account it signed in.
        fn sign_in_finish(&self, flow_id: &str) -> Result<Json>;
        /// Stop a sign-in; nothing is connected.
        fn sign_in_cancel(&self, flow_id: &str);
        /// Whether writing to the account `path` belongs to can start
        /// without a new sign-in.
        fn signed_in(&self, path: &str) -> bool;
        /// Forget the account `account` (its id) and end its session.
        fn disconnect(&self, account: &str) -> Result<()>;
    }

    /// Sharing an item with someone by email (`arbesk_share_collection`).
    pub trait Sharing: Send + Sync {
        /// Add the account behind `email` to the item at `path` with
        /// `role` (Arbesk: `1` viewer, `2` editor), or change its role.
        fn share(&self, path: &str, email: &str, role: u8) -> Result<Shared>;
    }

    /// The parts of a preview a provider builds itself (`arbesk_scene`,
    /// `arbesk_part`).
    pub trait Preview: Send + Sync {
        /// The scene of the item at `path` (at the save `version`, the
        /// newest without one): its parts and where each goes, as the
        /// provider describes it.
        fn scene(&self, path: &str, version: Option<&str>) -> Result<Json>;
        /// The file of the scene's part `key`, built on this computer.
        fn part(&self, path: &str, version: Option<&str>, key: &str, cx: &Cx) -> Result<PathBuf>;
    }

    /// An item's other files (an Arbesk asset's representations:
    /// `arbesk_representations`, and fetching one): what only one place
    /// has today. Each provider describes its files in its own record,
    /// handed on as it serialises it.
    pub trait Representations: Send + Sync {
        /// The other files of the item at `path`, at the save `version`
        /// (the current one without).
        fn representations(&self, path: &str, version: Option<&str>) -> Result<Json>;
        /// The other file `id` of the item at `path`, at the save
        /// `version` (the current one without), on this computer.
        fn file(&self, path: &str, version: Option<&str>, id: &str, cx: &Cx) -> Result<PathBuf>;
        /// What adding the local `files` to the item at `path` would do,
        /// before anything is asked or written: each file as it would be
        /// recorded and any clash with one the item has.
        fn plan_attach(&self, path: &str, files: &[String]) -> Result<Json>;
        /// Add local files to the item at `path` as one new version: each
        /// file and what was answered for it, in the provider's shape
        /// (`files`); the item's files afterwards.
        fn attach(&self, path: &str, files: &serde_json::Value, cx: &Cx) -> Result<Json>;
        /// Remove the other file `id` from the item at `path` as one new
        /// version, only while it is still the file `cid` the user saw;
        /// the item's files afterwards.
        fn detach(&self, path: &str, id: &str, cid: &str, cx: &Cx) -> Result<Json>;
    }

    /// Features that only make sense for items on this computer (Make Link,
    /// permissions, Batch Rename, archives, Open With, drag out, OS icons,
    /// Recent). A provider without it refuses them with its
    /// [`Operation::LocalOnly`](crate::Operation::LocalOnly) wording.
    /// Batch Rename has its method since migration step 6b; the others
    /// stay on their commands.
    pub trait LocalOnly: Send + Sync {
        /// Batch Rename: each `(from, to)` renamed in its folder, all or
        /// nothing, never over another item.
        fn rename_many(&self, renames: &[(String, String)]) -> Result<()>;
    }

    /// Delete Permanently, only inside the Trash.
    pub trait PermanentDelete: Send + Sync {}

    /// Empty Trash.
    pub trait EmptyTrash: Send + Sync {}
}

/// The extensions a provider has; absent ones are `None`.
#[derive(Clone, Copy, Default)]
pub struct Extensions<'a> {
    pub thumbnails: Option<&'a dyn ext::Thumbnails>,
    pub properties: Option<&'a dyn ext::Properties>,
    pub counts: Option<&'a dyn ext::Counts>,
    pub search: Option<&'a dyn ext::Search>,
    pub measure: Option<&'a dyn ext::Measure>,
    pub versions: Option<&'a dyn ext::Versions>,
    pub accounts: Option<&'a dyn ext::Accounts>,
    pub sharing: Option<&'a dyn ext::Sharing>,
    pub preview: Option<&'a dyn ext::Preview>,
    pub representations: Option<&'a dyn ext::Representations>,
    pub local_only: Option<&'a dyn ext::LocalOnly>,
    pub permanent_delete: Option<&'a dyn ext::PermanentDelete>,
    pub empty_trash: Option<&'a dyn ext::EmptyTrash>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fetched_item_opens_as_today() {
        let file: OpenTarget = Fetched::File("/cache/a.pdf".into()).into();
        let url: OpenTarget = Fetched::Url("https://docs.google.com/d".into()).into();
        assert_eq!(serde_json::to_string(&file).unwrap(), r#"{"kind":"file","path":"/cache/a.pdf"}"#);
        assert_eq!(serde_json::to_string(&url).unwrap(), r#"{"kind":"url","url":"https://docs.google.com/d"}"#);
    }

    #[test]
    fn a_context_stops_once_cancelled() {
        let cx = Cx::default();
        assert_eq!(cx.check(), Ok(()));
        cx.progress.cancel();
        assert_eq!(cx.check(), Err(Error::cancelled()));
    }

    #[test]
    fn the_local_only_gate_has_its_own_operation() {
        let refused = Refused {
            operation: Operation::LocalOnly,
            message: "This action is not available for SSH servers.".into(),
        };
        assert_eq!(
            serde_json::to_string(&refused).unwrap(),
            r#"{"operation":"local_only","message":"This action is not available for SSH servers."}"#
        );
    }

    #[test]
    fn no_extension_is_there_by_default() {
        let none = Extensions::default();
        assert!(none.thumbnails.is_none() && none.properties.is_none() && none.counts.is_none());
        assert!(none.search.is_none() && none.measure.is_none() && none.versions.is_none());
        assert!(none.accounts.is_none() && none.sharing.is_none() && none.preview.is_none());
        assert!(none.local_only.is_none() && none.permanent_delete.is_none());
        assert!(none.representations.is_none());
        assert!(none.empty_trash.is_none());
    }
}
