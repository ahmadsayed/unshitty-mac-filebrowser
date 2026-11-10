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

use crate::{Entry, Error, OpenTarget, Progress, Result};

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
/// (for Undo) or why it stayed.
#[derive(Debug, PartialEq, Eq)]
pub struct Trashed {
    pub path: String,
    pub result: Result<String>,
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

    /// For open and preview only: a local file with this item's bytes (the
    /// file itself here, a download or a build in the app cache elsewhere,
    /// marked with the `.downloaded` mark), or a URL to open instead.
    fn fetch(&self, path: &str, purpose: Fetch, cx: &Cx) -> Result<Fetched>;

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

    /// The transfer primitives the engine builds copy and move from. `None`
    /// when this provider can't be a transfer target (read only).
    fn transfer(&self) -> Option<&dyn Transfer> {
        None
    }
    /// Optional, non-POSIX parts.
    fn extensions(&self) -> Extensions<'_> {
        Extensions::default()
    }
}

/// The primitives a provider offers the transfer engine (D6), each as safely
/// as it can, declared in its [`Capabilities`]. Which pairs of places may
/// transfer, the refusals, the name-clash decision and the order of a
/// Replace are the engine's, not the provider's. A transfer between two
/// different providers meets at a local path (a write reads a local source,
/// a read fills a local target); a copy inside one provider is
/// [`Transfer::copy_within`], so it never leaves the place (Drive copies on
/// the server).
pub trait Transfer: Send + Sync {
    /// Write the local file or folder `source` to `target`, a path in this
    /// provider the engine chose (a staging name, or the final name where a
    /// file appears only whole). Skipped items are named in the result.
    fn write(&self, source: &Path, target: &str, cx: &Cx) -> Result<Put>;
    /// Read this provider's item at `path` into the local `target` the
    /// engine chose, streaming straight into it (no copy in the app cache).
    fn read_into(&self, path: &str, target: &Path, cx: &Cx) -> Result<Put>;
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
    /// Send the item a Replace replaces to this provider's Trash; returns
    /// what [`Transfer::take_back`] needs to bring it back.
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
}

// ---- Extensions ----------------------------------------------------------------

/// What isn't POSIX, one trait per feature (D4). A provider lacking one
/// leaves it `None`, and the app checks for it instead of matching on a
/// kind of place.
///
/// Only the slots are declared now. Each trait gets its methods in the
/// migration step that first implements it, shaped like the command it
/// replaces so nothing visible changes: thumbnails as PNG bytes with the
/// per-window cancel ticket, item counts as `FolderSummary` (with the
/// version badges), search with `show_hidden`, its limit, the per-window
/// stop and its summary, a folder size as bytes cancelled by walk id,
/// sharing with Arbesk's numeric role and its `Shared` result.
pub mod ext {
    /// Thumbnails of items (`thumbnail`'s per-kind arms).
    pub trait Thumbnails: Send + Sync {}

    /// What the Properties dialog shows, and a folder's total size
    /// (`file_properties`, `folder_size`).
    pub trait Properties: Send + Sync {}

    /// Item counts and version badges of folders, for the view
    /// (`item_counts`).
    pub trait Counts: Send + Sync {}

    /// A recursive search, its results streamed in batches (`search_dir`).
    pub trait Search: Send + Sync {}

    /// Sizes and item counts before a copy, for the progress total
    /// (`measure`).
    pub trait Measure: Send + Sync {}

    /// The saves of an item, oldest first (`arbesk_versions`).
    pub trait Versions: Send + Sync {}

    /// Accounts a person signs in to (each provider keeps its own sign-in
    /// step).
    pub trait Accounts: Send + Sync {}

    /// Sharing an item with someone by email (`arbesk_share_collection`).
    pub trait Sharing: Send + Sync {}

    /// The parts of a preview a provider builds itself (`arbesk_scene`,
    /// `arbesk_part`).
    pub trait Preview: Send + Sync {}

    /// Features that only make sense for items on this computer (Make Link,
    /// permissions, Batch Rename, archives, Open With, drag out, OS icons,
    /// Recent). A provider without it refuses them with its
    /// [`Operation::LocalOnly`](crate::Operation::LocalOnly) wording.
    pub trait LocalOnly: Send + Sync {}

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
        assert!(none.empty_trash.is_none());
    }
}
