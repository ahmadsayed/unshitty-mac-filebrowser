//! This computer's `places::Provider` (design step 6, replacing step 2's
//! adapter in the app): files on this computer and mounted volumes. A path
//! no other provider claims comes here (the registry's prefix rule, D10),
//! so anything that isn't an absolute path is refused with [`local`]'s
//! words.
//!
//! It answers listing, whether an item exists, path info, opening and the
//! preview's file (the file itself), and thumbnails through the thumbnailer
//! the app hands over (its disk cache, the desktop's shared one and Quick
//! Look stay in the app). Since migration step 6b it also writes: a new
//! folder or empty file (`fs_ops::make_dir`, `fs_ops::create_file`), Rename
//! (`rename_in_place`), Batch Rename (its `LocalOnly` extension), Move to
//! Trash (`trash_ops::trash_many`, logged) and Put Back (`Trash::restore`),
//! and hands the app's transfer engine its `places::Transfer`
//! (`transfer::LocalTransfer`, with the logged Trash for Replace). The app's
//! Trash and home folder are handed in ([`TrashReach`]); Keep Versions is
//! the app's decorator around this provider (D7), so nothing here touches a
//! history.

use std::path::{Path, PathBuf};

use places::{
    Capabilities, Cx, Entry, Error, Extensions, Fetch, Fetched, ListOptions, NewKind, Operation,
    PathInfo, Provider, Refused, Replace, Result, Trashed, ext,
};

use crate::fs_ops;
use crate::transfer::{LocalTransfer, ReplaceTrash};
use crate::trash_ops::{self, Trash};

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

/// One answer per path where only some paths can have one: `answer` gets
/// the paths `keep` accepts, in order, in a single call; every other path
/// gets `None`. The one batch splitter: this provider's [`local_only`] and
/// the app's local-only gate (`place::local_only`) both scatter through it.
pub fn answer_kept<T>(
    paths: &[String],
    keep: impl Fn(&str) -> bool,
    answer: impl FnOnce(Vec<String>) -> Vec<Option<T>>,
) -> Vec<Option<T>> {
    let (positions, kept): (Vec<usize>, Vec<String>) = paths
        .iter()
        .enumerate()
        .filter(|(_, path)| keep(path))
        .map(|(index, path)| (index, path.clone()))
        .unzip();
    let mut out: Vec<Option<T>> = paths.iter().map(|_| None).collect();
    for (index, value) in positions.into_iter().zip(answer(kept)) {
        out[index] = value;
    }
    out
}

/// One answer per path where only local paths can have one: `answer` gets
/// the local paths, in order, in a single call; every other path gets
/// `None`.
fn local_only<T>(
    paths: &[String],
    answer: impl FnOnce(Vec<String>) -> Vec<Option<T>>,
) -> Vec<Option<T>> {
    answer_kept(paths, |path| local(path).is_ok(), answer)
}

/// Today's errors from the file operations are words; they keep them, as
/// `Other`, so a command returns exactly the same text.
fn words(message: String) -> Error {
    Error::Other(message)
}

/// The path of `name` in the folder `parent`, joined as the app's paths
/// split at their last slash, so `parent` and `name` of a path give it back
/// exactly.
pub fn child_path(parent: &str, name: &str) -> String {
    format!("{parent}/{name}")
}

/// Why Rename refuses to take an item out of its folder: a name with a
/// slash, or a new path in another folder. Google Drive's words.
pub const RENAME_STAYS: &str = "Rename keeps an item in its folder; use Move instead.";

/// Where Rename puts `path` given `new_name`: the name in `path`'s folder.
/// A `new_name` with a slash in it isn't a name, and would take the item
/// to another folder past the move's refusals, so it is refused
/// ([`RENAME_STAYS`]).
pub fn renamed_path(path: &str, new_name: &str) -> std::result::Result<String, String> {
    if new_name.contains('/') {
        return Err(RENAME_STAYS.to_string());
    }
    Ok(child_path(path.rsplit_once('/').map_or("", |(parent, _)| parent), new_name))
}

/// What the provider needs of the app for the Trash: the app's Trash and
/// its log (`trash_ops::Trash`, one in the app's state), and the home
/// folder whose Trash Move to Trash uses, each with the app's words when
/// it is unknown.
pub trait TrashReach: Send + Sync {
    fn trash(&self) -> std::result::Result<&Trash, String>;
    fn home(&self) -> std::result::Result<PathBuf, String>;
}

/// The app's logged Trash for a Replace ([`ReplaceTrash`]): `Trash::send`,
/// so Restore knows where the old item came from, and `Trash::untrash`,
/// which forgets an item taken back.
pub struct LoggedTrash<'a>(pub &'a dyn TrashReach);

impl ReplaceTrash for LoggedTrash<'_> {
    fn send(&self, path: &Path) -> std::result::Result<Option<PathBuf>, String> {
        self.0.trash()?.send(path)
    }

    fn take_back(&self, trashed: &Path, origin: &Path) -> std::result::Result<(), String> {
        self.0.trash()?.untrash(trashed, origin)
    }
}

/// A path's text, as the app's strings carry it.
fn text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
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
/// for the paths that are local and the app's Trash.
pub struct Local {
    thumbnails: Box<dyn ext::Thumbnails>,
    trash: Box<dyn TrashReach>,
}

impl Local {
    /// This computer's provider; `thumbnails` makes a local file's
    /// thumbnail (it is asked only for absolute paths), and `trash` reaches
    /// the app's Trash.
    pub fn new(thumbnails: Box<dyn ext::Thumbnails>, trash: Box<dyn TrashReach>) -> Self {
        Local { thumbnails, trash }
    }

    /// The home folder Move to Trash uses, or why it is unknown.
    pub fn trash_home(&self) -> std::result::Result<PathBuf, String> {
        self.trash.home()
    }

    /// Move the absolute `paths` to the Trash in one batch and log where
    /// each went.
    fn trash_local(&self, paths: &[String], cx: &Cx) -> Vec<Option<Result<Option<String>>>> {
        let origins: Vec<PathBuf> = paths.iter().map(PathBuf::from).collect();
        let results = match self.trash.home() {
            Ok(home) => trash_ops::trash_many(&origins, &home, &cx.progress),
            Err(error) => origins.iter().map(|_| Err(error.clone())).collect(),
        };
        let moves: Vec<(PathBuf, PathBuf)> = origins
            .iter()
            .zip(&results)
            .filter_map(|(origin, result)| match result {
                Ok(Some(trashed)) => Some((trashed.clone(), origin.clone())),
                _ => None,
            })
            .collect();
        if let Ok(trash) = self.trash.trash() {
            trash.record(&moves);
        }
        let place = |result: trash_ops::TrashResult| result.map(|at| at.as_deref().map(text)).map_err(words);
        results.into_iter().map(|result| Some(place(result))).collect()
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
            // A folder's files keep no other files of their own.
            (Operation::Attach, Some(places::ADD_TO_AN_ASSET.to_string())),
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
    /// The item at `path`, or `None` when nothing is there.
    fn stat(&self, path: &str, _: &Cx) -> Result<Option<Entry>> {
        Ok(fs_ops::entry_for(local(path).map_err(words)?))
    }
    /// Names are unique on a disk: the item at the path, if any.
    fn lookup(&self, pending: &str, cx: &Cx) -> Result<Vec<Entry>> {
        Ok(self.stat(pending, cx)?.into_iter().collect())
    }
    /// `fs_ops::make_dir` or `fs_ops::create_file` at `parent/name`, never
    /// over an existing item.
    fn create(&self, parent: &str, name: &str, kind: NewKind, _: &Cx) -> Result<String> {
        let path = child_path(parent, name);
        local(&path).map_err(words)?;
        match kind {
            NewKind::Folder => fs_ops::make_dir(path.clone()),
            NewKind::EmptyFile => fs_ops::create_file(path.clone()),
        }
        .map_err(words)?;
        Ok(path)
    }
    /// `fs_ops::rename_entry` (`rename_in_place`), to [`renamed_path`].
    fn rename(&self, path: &str, new_name: &str, _: &Cx) -> Result<String> {
        local(path).map_err(words)?;
        let to = renamed_path(path, new_name).map_err(words)?;
        fs_ops::rename_entry(path.to_string(), to.clone()).map_err(words)?;
        Ok(to)
    }
    /// Move to Trash as one batch of the absolute paths (one Finder request
    /// on macOS), logged so Restore knows where each came from; any other
    /// path is refused in `local`'s words.
    fn trash(&self, paths: &[&str], cx: &Cx) -> Vec<Trashed> {
        let paths: Vec<String> = paths.iter().map(|path| path.to_string()).collect();
        let results = answer_kept(&paths, |path| local(path).is_ok(), |kept| self.trash_local(&kept, cx));
        paths
            .into_iter()
            .zip(results)
            .map(|(path, result)| {
                let result = result.unwrap_or_else(|| Err(words(local(&path).unwrap_err())));
                Trashed { path, result }
            })
            .collect()
    }
    /// Put Back (`Trash::restore`): never over an item at its old place.
    fn restore(&self, trashed: &str, _: &Cx) -> Result<String> {
        let path = local(trashed).map_err(words)?;
        let trash = self.trash.trash().map_err(words)?;
        trash.restore(path).map(|origin| text(&origin)).map_err(words)
    }
    /// A copy or a move on this computer, with the app's logged Trash for
    /// Replace (`transfer::LocalTransfer`).
    fn transfer(&self, _: &Cx, work: &mut dyn FnMut(&dyn places::Transfer)) {
        let trash = LoggedTrash(&*self.trash);
        work(&LocalTransfer::new(&trash));
    }
    fn extensions(&self) -> Extensions<'_> {
        Extensions { thumbnails: Some(self), local_only: Some(self), ..Extensions::default() }
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

impl ext::LocalOnly for Local {
    /// `fs_ops::rename_many`: all or nothing, never over another item.
    fn rename_many(&self, renames: &[(String, String)]) -> Result<()> {
        let items = renames
            .iter()
            .map(|(from, to)| fs_ops::RenameItem { from: from.clone(), to: to.clone() })
            .collect();
        fs_ops::rename_many(items).map_err(words)
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

    /// The app's Trash as a test hands it over: `trash` when given, and
    /// `home` (or the app's words when it is unknown).
    struct Reached {
        trash: Option<Trash>,
        home: Option<PathBuf>,
    }

    impl TrashReach for Reached {
        fn trash(&self) -> std::result::Result<&Trash, String> {
            self.trash.as_ref().ok_or_else(|| "The home folder is unknown".to_string())
        }
        fn home(&self) -> std::result::Result<PathBuf, String> {
            self.home.clone().ok_or_else(|| "no home in tests".to_string())
        }
    }

    fn no_trash() -> Box<dyn TrashReach> {
        Box::new(Reached { trash: None, home: None })
    }

    fn local_with_record() -> (Local, std::sync::Arc<Mutex<Vec<String>>>) {
        let asked = std::sync::Arc::default();
        (Local::new(Box::new(Recorded(std::sync::Arc::clone(&asked))), no_trash()), asked)
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

    fn text_of(path: &Path) -> String {
        path.to_string_lossy().into_owned()
    }

    /// A path splits at its last slash and joins back exactly; Rename puts
    /// a name in the item's folder, and refuses anything with a slash.
    #[test]
    fn names_join_their_folder_as_paths_split() {
        for path in ["/a/b", "/x", "/a/b/", "//x"] {
            let (parent, name) = path.rsplit_once('/').unwrap();
            assert_eq!(child_path(parent, name), path);
        }
        assert_eq!(renamed_path("/a/b.txt", "c.txt"), Ok("/a/c.txt".into()));
        assert_eq!(renamed_path("/b.txt", "c.txt"), Ok("/c.txt".into()));
        for name in ["/elsewhere/c.txt", "sub/c.txt"] {
            assert_eq!(renamed_path("/a/b.txt", name), Err(RENAME_STAYS.into()), "{name}");
        }
    }

    /// New folders and files are made at their name and never over an
    /// item; a path that isn't local is refused in `local`'s words.
    #[test]
    fn creates_folders_and_files_never_over_an_item() {
        let (provider, _) = local_with_record();
        let (dir, cx) = (tempfile::tempdir().unwrap(), Cx::default());
        let parent = text_of(dir.path());
        let folder = provider.create(&parent, "New Folder", NewKind::Folder, &cx).unwrap();
        assert_eq!(folder, text_of(&dir.path().join("New Folder")));
        assert!(Path::new(&folder).is_dir());
        let file = provider.create(&parent, "a.txt", NewKind::EmptyFile, &cx).unwrap();
        assert_eq!(std::fs::read(&file).unwrap(), b"");
        assert_eq!(
            provider.create(&parent, "New Folder", NewKind::Folder, &cx),
            Err(Error::Other(format!("Path already exists: {folder}")))
        );
        assert!(provider.create(&parent, "a.txt", NewKind::EmptyFile, &cx).unwrap_err().to_string().starts_with("Could not create the file: "));
        let refused = Error::Other("“rel/b” is not a local file.".into());
        assert_eq!(provider.create("rel", "b", NewKind::Folder, &cx), Err(refused));
    }

    /// Rename renames in place, never over another item; Batch Rename is
    /// its `LocalOnly` extension, all or nothing.
    #[test]
    fn renames_never_over_another_item() {
        let (provider, _) = local_with_record();
        let (dir, cx) = (tempfile::tempdir().unwrap(), Cx::default());
        let (a, b) = (dir.path().join("a.txt"), dir.path().join("b.txt"));
        std::fs::write(&a, "a").unwrap();
        std::fs::write(&b, "b").unwrap();
        let taken = Err(Error::Other(format!("Target already exists: {}", text_of(&b))));
        assert_eq!(provider.rename(&text_of(&a), "b.txt", &cx), taken);
        assert_eq!(provider.rename(&text_of(&a), "c.txt", &cx), Ok(text_of(&dir.path().join("c.txt"))));
        assert_eq!(provider.rename("rel", "c.txt", &cx), Err(Error::Other("“rel” is not a local file.".into())));
        let elsewhere = text_of(&dir.path().join("sub/e.txt"));
        assert_eq!(provider.rename(&text_of(&dir.path().join("c.txt")), &elsewhere, &cx), Err(Error::Other(RENAME_STAYS.into())));
        assert!(dir.path().join("c.txt").exists());
        let batch = provider.extensions().local_only.unwrap();
        let c = text_of(&dir.path().join("c.txt"));
        let d = text_of(&dir.path().join("d.txt"));
        assert_eq!(batch.rename_many(&[(c.clone(), d.clone())]), Ok(()));
        let twice = [(d.clone(), c.clone()), (text_of(&b), c.clone())];
        assert_eq!(batch.rename_many(&twice), Err(Error::Other("Two items would have the same name.".into())));
        assert_eq!(std::fs::read_to_string(&d).unwrap(), "a");
    }

    /// Looking an item up is the item at the path.
    #[test]
    fn stat_and_lookup_answer_the_item_at_the_path() {
        let (provider, _) = local_with_record();
        let (dir, cx) = (tempfile::tempdir().unwrap(), Cx::default());
        let a = text_of(&dir.path().join("a.txt"));
        assert!(provider.stat(&a, &cx).unwrap().is_none());
        assert!(provider.lookup(&a, &cx).unwrap().is_empty());
        std::fs::write(&a, "a").unwrap();
        assert_eq!(provider.stat(&a, &cx).unwrap().unwrap().name, "a.txt");
        assert_eq!(provider.lookup(&a, &cx).unwrap().len(), 1);
        assert!(provider.stat("rel", &cx).is_err());
    }

    /// Move to Trash refuses a path that isn't local and, without a home
    /// folder, says why for the rest; Put Back without the app's Trash
    /// says so, and with it never guesses where an unknown item came from.
    #[test]
    fn trash_and_restore_say_why_they_cannot() {
        let (provider, _) = local_with_record();
        let cx = Cx::default();
        let trashed = provider.trash(&["/nowhere/a.txt", "rel"], &cx);
        assert_eq!(
            trashed,
            [
                Trashed { path: "/nowhere/a.txt".into(), result: Err(Error::Other("no home in tests".into())) },
                Trashed { path: "rel".into(), result: Err(Error::Other("“rel” is not a local file.".into())) },
            ]
        );
        assert_eq!(provider.trash_home(), Err("no home in tests".into()));
        let unknown = Err(Error::Other("The home folder is unknown".into()));
        assert_eq!(provider.restore("/nowhere/a.txt", &cx), unknown);
        assert_eq!(provider.restore("rel", &cx), Err(Error::Other("“rel” is not a local file.".into())));

        let dir = tempfile::tempdir().unwrap();
        let (trash, home, items) = trash_ops::temp_trash(dir.path());
        let provider = Local::new(Box::new(Recorded::default()), Box::new(Reached { trash: Some(trash), home: Some(home) }));
        std::fs::write(items.join("old.txt"), "old").unwrap();
        let old = text_of(&items.join("old.txt"));
        assert_eq!(provider.restore(&old, &cx), Err(Error::Other("The original location is unknown".into())));
        assert!(provider.trash(&[], &cx).is_empty());
    }

    /// Move to Trash through the provider logs where each item came from
    /// (`Trash::record`), and Put Back then returns it there. Runs on the
    /// host's layout (`.trashinfo` on Linux, the app's log beside Finder's
    /// on macOS), with the real Trash of the home folder; macOS without Full
    /// Disk Access can't list `~/.Trash`, so there the item may go
    /// unlocated, and is then neither logged nor put back.
    #[test]
    fn trash_logs_where_each_item_came_from_and_put_back_returns_it() {
        let dir = tempfile::tempdir().unwrap();
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| dir.path().to_path_buf());
        let log = dir.path().join("config/trash_log.json");
        let trash = Trash::new(home.clone(), Some(log.clone()));
        let provider = Local::new(Box::new(Recorded::default()), Box::new(Reached { trash: Some(trash), home: Some(home) }));
        let (a, b, cx) = (dir.path().join("a.txt"), dir.path().join("b.txt"), Cx::default());
        std::fs::write(&a, "a").unwrap();
        std::fs::write(&b, "b").unwrap();
        let (a, b) = (text_of(&a), text_of(&b));
        let gone = text_of(&dir.path().join("gone.txt"));
        let trashed = provider.trash(&[&a, "rel", &gone, &b], &cx);
        assert_eq!(trashed.iter().map(|t| t.path.as_str()).collect::<Vec<_>>(), [&*a, "rel", &*gone, &*b]);
        assert_eq!(trashed[2].result, Err(Error::Other(format!("“{gone}” no longer exists"))));
        assert!(!Path::new(&a).exists() && !Path::new(&b).exists());
        let logged = std::fs::read_to_string(&log)
            .map(|text| serde_json::from_str::<trash_ops::TrashLog>(&text).unwrap().items)
            .unwrap_or_default();
        for (index, origin, bytes) in [(0, &a, "a"), (3, &b, "b")] {
            match trashed[index].result.clone().unwrap() {
                Some(at) => {
                    assert_eq!(logged.get(&at).map(|item| &item.origin), Some(origin), "{at}");
                    assert_eq!(std::fs::read_to_string(&at).unwrap(), bytes);
                    assert_eq!(provider.restore(&at, &cx), Ok(origin.clone()));
                    assert!(Path::new(origin).exists() && !Path::new(&at).exists());
                }
                None if cfg!(target_os = "macos") => {}
                None => panic!("only macOS may not say where {origin} went"),
            }
        }
        assert_eq!(logged.len(), trashed.iter().filter(|t| matches!(t.result, Ok(Some(_)))).count());
    }

    /// This computer hands the engine its transfer session, with the logged
    /// Trash for Replace.
    #[test]
    fn hands_the_engine_its_transfer_session() {
        let (provider, _) = local_with_record();
        let cx = Cx::default();
        let refused = places::with_transfer(&provider, &cx, |t| t.trash_for_replace("/nowhere/a.txt", &cx)).unwrap();
        assert_eq!(refused, Err(Error::Other("The home folder is unknown".into())));
        let back = places::with_transfer(&provider, &cx, |t| t.take_back("/t/a.txt", "/nowhere/a.txt", &cx)).unwrap();
        assert_eq!(back, Err(Error::Other("The home folder is unknown".into())));
        assert!(!provider.fetch_reports_progress());
        assert_eq!(provider.scheme(), "");
    }
}
