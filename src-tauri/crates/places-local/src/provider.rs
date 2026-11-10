//! This computer's `places::Provider` (design step 6, replacing step 2's
//! adapter in the app): files on this computer and mounted volumes. A path
//! no other provider claims comes here (the registry's prefix rule, D10),
//! so anything that isn't an absolute path is refused with [`local`]'s
//! words.
//!
//! The read side answers here: listing, whether an item exists, path info,
//! opening and the preview's file (the file itself), and thumbnails through
//! the thumbnailer the app hands over (its disk cache, the desktop's shared
//! one and Quick Look stay in the app). The writes (new folders, rename,
//! copy, move, Trash, restore) stay on the app's commands, where Keep
//! Versions wraps them (D7), so the write methods say so instead of acting.

use std::path::{Path, PathBuf};

use places::{
    Capabilities, Cx, Entry, Error, Extensions, Fetch, Fetched, ListOptions, NewKind, Operation,
    PathInfo, Provider, Refused, Replace, Result, Trashed, ext,
};

use crate::fs_ops;

/// `path` as a file on this computer, or why it isn't one: anything but an
/// absolute path, which would be read from the app's working directory.
/// The app's local-only gate answers a path of this computer's with it.
pub fn local(path: &str) -> std::result::Result<&Path, String> {
    let local = Path::new(path);
    if local.is_absolute() {
        Ok(local)
    } else {
        Err(format!("“{path}” is not a local file."))
    }
}

/// One answer per path where only local paths can have one: `answer` gets
/// the local paths, in order, in a single call; every other path gets
/// `None`.
fn local_only<T>(
    paths: &[String],
    answer: impl FnOnce(Vec<String>) -> Vec<Option<T>>,
) -> Vec<Option<T>> {
    let (positions, local): (Vec<usize>, Vec<String>) = paths
        .iter()
        .enumerate()
        .filter(|(_, path)| local(path).is_ok())
        .map(|(index, path)| (index, path.clone()))
        .unzip();
    let mut out: Vec<Option<T>> = paths.iter().map(|_| None).collect();
    for (index, value) in positions.into_iter().zip(answer(local)) {
        out[index] = value;
    }
    out
}

/// Today's errors from the file operations are words; they keep them, as
/// `Other`, so a command returns exactly the same text.
fn words(message: String) -> Error {
    Error::Other(message)
}

/// A write, or a lookup no command routes here yet: each still goes through
/// its command's own dispatch, where Keep Versions wraps it (D7).
fn not_yet<T>(what: &str) -> Result<T> {
    Err(Error::Unsupported(format!("{what} doesn't go through the places registry yet.")))
}

/// `refused` with each operation that has words.
fn refusals(each: &[(Operation, Option<String>)]) -> Vec<Refused> {
    each.iter()
        .filter_map(|(operation, message)| {
            message.clone().map(|message| Refused { operation: *operation, message })
        })
        .collect()
}

/// Files on this computer and mounted volumes, with the app's thumbnailer
/// for the paths that are local.
pub struct Local {
    thumbnails: Box<dyn ext::Thumbnails>,
}

impl Local {
    /// This computer's provider; `thumbnails` makes a local file's
    /// thumbnail (it is asked only for absolute paths).
    pub fn new(thumbnails: Box<dyn ext::Thumbnails>) -> Self {
        Local { thumbnails }
    }
}

/// What a path on this computer allows: everything, unless it isn't an
/// absolute path, which every operation refuses with the same words.
fn local_capabilities(path: &str) -> Capabilities {
    let not_local = local(path).err();
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
        local(dir).map_err(words)?;
        fs_ops::list_dir(dir.to_string(), opts.show_hidden).map_err(words)
    }
    fn exists(&self, path: &str, _: &Cx) -> bool {
        local(path).is_ok_and(|path| fs_ops::path_exists(path.to_string_lossy().into()))
    }
    fn path_info(&self, paths: &[String], _: &Cx) -> Vec<Option<PathInfo>> {
        local_only(paths, fs_ops::path_info)
    }
    /// The file itself, for opening and for the preview.
    fn fetch(&self, path: &str, _: Fetch, _: &Cx) -> Result<Fetched> {
        local(path).map_err(words)?;
        Ok(Fetched::File(PathBuf::from(path)))
    }
    /// The file itself: nothing to download, so no progress to report.
    fn fetch_reports_progress(&self) -> bool {
        false
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
    fn extensions(&self) -> Extensions<'_> {
        Extensions { thumbnails: Some(self), ..Extensions::default() }
    }
}

impl ext::Thumbnails for Local {
    /// The app's thumbnail of a local file; none for a path that isn't
    /// local.
    fn thumbnail(&self, path: &str, size: u32, stop: &dyn Fn() -> bool) -> Vec<u8> {
        if local(path).is_err() {
            return Vec::new();
        }
        self.thumbnails.thumbnail(path, size, stop)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// A thumbnailer that records what it was asked for.
    #[derive(Default)]
    struct Recorded(std::sync::Arc<Mutex<Vec<String>>>);

    impl ext::Thumbnails for Recorded {
        fn thumbnail(&self, path: &str, _: u32, _: &dyn Fn() -> bool) -> Vec<u8> {
            self.0.lock().unwrap().push(path.to_string());
            vec![1, 2, 3]
        }
    }

    fn local_with_record() -> (Local, std::sync::Arc<Mutex<Vec<String>>>) {
        let asked = std::sync::Arc::default();
        (Local::new(Box::new(Recorded(std::sync::Arc::clone(&asked)))), asked)
    }

    #[test]
    fn only_an_absolute_path_is_local() {
        assert_eq!(local("/Users/me/a.txt"), Ok(Path::new("/Users/me/a.txt")));
        assert_eq!(local("/Users/me/gdrive://x"), Ok(Path::new("/Users/me/gdrive://x")));
        for path in ["a.txt", "", "file:///tmp", "GDRIVE://acct/root~x"] {
            assert_eq!(local(path), Err(format!("“{path}” is not a local file.")));
        }
    }

    /// Thumbnails come from the app's thumbnailer, for local paths only.
    #[test]
    fn thumbnails_are_the_apps_for_local_paths_only() {
        let (provider, asked) = local_with_record();
        let thumbnails = provider.extensions().thumbnails.unwrap();
        assert_eq!(thumbnails.thumbnail("/Users/me/a.png", 64, &|| false), [1, 2, 3]);
        assert!(thumbnails.thumbnail("rel.png", 64, &|| false).is_empty());
        assert_eq!(*asked.lock().unwrap(), ["/Users/me/a.png"]);
    }

    /// A batch's local paths are asked about in one call, in order; the
    /// others have no answer.
    #[test]
    fn path_info_asks_the_disk_once_for_the_local_paths() {
        let (provider, _) = local_with_record();
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.txt");
        std::fs::write(&file, "a").unwrap();
        let paths = [file.to_string_lossy().to_string(), "rel".into(), dir.path().to_string_lossy().to_string()];
        let info = provider.path_info(&paths, &Cx::default());
        assert_eq!(info.len(), 3);
        assert!(!info[0].as_ref().unwrap().is_dir);
        assert!(info[1].is_none());
        assert!(info[2].as_ref().unwrap().is_dir);
        assert!(provider.exists(&paths[0], &Cx::default()));
        assert!(!provider.exists("rel", &Cx::default()));
        let none: Vec<Option<u8>> = local_only(&[], |local| {
            assert!(local.is_empty());
            Vec::new()
        });
        assert!(none.is_empty());
    }

    /// Writes stay on the app's commands for now (Keep Versions wraps
    /// them, D7): each says so.
    #[test]
    fn writes_say_they_are_not_routed_here_yet() {
        let (provider, _) = local_with_record();
        let cx = Cx::default();
        let not_yet = |result: Result<String>| {
            matches!(result, Err(Error::Unsupported(words)) if words.ends_with("registry yet."))
        };
        assert!(not_yet(provider.create("/a", "b", NewKind::Folder, &cx)));
        assert!(not_yet(provider.rename("/a", "b", &cx)));
        assert!(not_yet(provider.restore("/a", &cx)));
        assert!(provider.stat("/a", &cx).is_err() && provider.lookup("/a", &cx).is_err());
        assert!(provider.trash(&["/a", "/b"], &cx).iter().all(|t| t.result.is_err()));
        assert!(!provider.fetch_reports_progress());
        assert_eq!(provider.scheme(), "");
    }
}
