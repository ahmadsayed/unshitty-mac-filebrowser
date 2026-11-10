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
/// Replace are the engine's, not the provider's. Every cross-place transfer
/// meets at a local path: a write reads a local source, a read fills a local
/// target.
pub trait Transfer: Send + Sync {
    /// Write the local file or folder `source` to `target`, a path in this
    /// provider the engine chose (a staging name, or the final name where a
    /// file appears only whole). Skipped items are named in the result.
    fn write(&self, source: &Path, target: &str, cx: &Cx) -> Result<Put>;
    /// Read this provider's item at `path` into the local `target` the
    /// engine chose, streaming straight into it (no copy in the app cache).
    fn read_into(&self, path: &str, target: &Path, cx: &Cx) -> Result<Put>;
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
    /// Discard a staged or partial copy the engine made (never the user's
    /// own item).
    fn discard(&self, staged: &str, cx: &Cx) -> Result<()>;
}

// ---- Extensions ----------------------------------------------------------------

/// What isn't POSIX, one trait per feature (D4). A provider lacking one
/// leaves it `None`, and the app checks for it instead of matching on a
/// kind of place. The traits are declared small here and grow with the
/// first provider that implements each.
pub mod ext {
    use std::path::{Path, PathBuf};

    use serde::Serialize;

    use crate::{Cx, Entry, FileProperties, Result};

    /// Thumbnails of items, as image files on this computer.
    pub trait Thumbnails: Send + Sync {
        /// A picture of `path` at most `size` pixels on its long side, or
        /// `None` when there is none.
        fn thumbnail(&self, path: &str, size: u32, cx: &Cx) -> Result<Option<PathBuf>>;
    }

    /// What the Properties dialog shows, and a folder's total size.
    pub trait Properties: Send + Sync {
        fn properties(&self, path: &str, cx: &Cx) -> Result<FileProperties>;
        /// Everything under `dir`, cancellable through `cx`.
        fn folder_size(&self, dir: &str, cx: &Cx) -> Result<crate::Measure>;
    }

    /// Item counts of folders, for the view.
    pub trait Counts: Send + Sync {
        /// One count per folder, `None` for one that can't be read.
        fn item_counts(&self, dirs: &[&str], show_hidden: bool) -> Vec<Option<u32>>;
    }

    /// A recursive search, its results handed over in batches.
    pub trait Search: Send + Sync {
        fn search(&self, dir: &str, query: &str, found: &mut dyn FnMut(Vec<Entry>), cx: &Cx)
        -> Result<()>;
    }

    /// Sizes and item counts before a copy, for the progress total.
    pub trait Measure: Send + Sync {
        fn measure(&self, paths: &[&str], cx: &Cx) -> Result<crate::Measure>;
    }

    /// One saved version of an item.
    #[derive(Clone, Debug, PartialEq, Eq, Serialize)]
    pub struct Version {
        pub id: String,
        pub timestamp: Option<i64>,
    }

    /// The saves of an item, oldest first.
    pub trait Versions: Send + Sync {
        fn versions(&self, path: &str, cx: &Cx) -> Result<Vec<Version>>;
    }

    /// Accounts a person signs in to (each provider has its own sign-in
    /// step, so only what they share is here).
    pub trait Accounts: Send + Sync {
        /// The root path of each connected account.
        fn accounts(&self) -> Vec<String>;
        /// Forget the account whose root is `root`, and its credential.
        fn disconnect(&self, root: &str) -> Result<()>;
    }

    /// Sharing an item with someone by email.
    pub trait Sharing: Send + Sync {
        fn share(&self, path: &str, email: &str, role: &str, cx: &Cx) -> Result<()>;
    }

    /// The parts of a preview a provider builds itself (an Arbesk scene).
    pub trait Preview: Send + Sync {
        /// The scene of `path` (at `version`, if given), as the preview reads it.
        fn scene(&self, path: &str, version: Option<&str>, cx: &Cx) -> Result<serde_json::Value>;
        /// One part of that scene, as a local file.
        fn part(&self, path: &str, version: Option<&str>, key: &str, cx: &Cx) -> Result<PathBuf>;
    }

    /// Features that only make sense for items on this computer (Make Link,
    /// permissions, Batch Rename, archives, Open With, drag out, OS icons,
    /// Recent): the place's local path for the app's own code.
    pub trait LocalOnly: Send + Sync {
        fn local_path<'a>(&self, path: &'a str) -> Result<&'a Path>;
    }

    /// Delete Permanently, only inside the Trash.
    pub trait PermanentDelete: Send + Sync {
        fn delete_permanently(&self, paths: &[&str], cx: &Cx) -> Vec<Result<()>>;
    }

    /// Empty Trash.
    pub trait EmptyTrash: Send + Sync {
        fn empty_trash(&self, cx: &Cx) -> Result<()>;
    }
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
    fn no_extension_is_there_by_default() {
        let none = Extensions::default();
        assert!(none.thumbnails.is_none() && none.properties.is_none() && none.counts.is_none());
        assert!(none.search.is_none() && none.measure.is_none() && none.versions.is_none());
        assert!(none.accounts.is_none() && none.sharing.is_none() && none.preview.is_none());
        assert!(none.local_only.is_none() && none.permanent_delete.is_none());
        assert!(none.empty_trash.is_none());
    }
}
