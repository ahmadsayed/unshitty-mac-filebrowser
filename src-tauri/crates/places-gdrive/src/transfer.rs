//! Google Drive's transfer primitives (design D6, spec D17): what the app's
//! transfer engine (`engine.rs`) builds a copy or move to, from or inside
//! Drive from. The engine decides the staging names, the Replace order
//! (trash the old item, rename the new one in, take the old one back if
//! that fails) and what a failure undoes; the primitives here only do one
//! step each, as safely as Drive allows, and never delete anything.
//!
//! - `write` uploads a local file or folder: a file appears only once its
//!   upload is whole and is checked against the local size; a folder is
//!   created and filled. Whatever a failed write made goes to Drive's Trash.
//! - `read` downloads into a local target the engine picks once it knows
//!   the item's kind: Google Docs and shortcuts are skipped and named,
//!   Drive's duplicate names become unique local ones, every file is
//!   checked against Drive's size.
//! - `copy_within` copies on the server (`files.copy`; folders are made
//!   again, since Drive copies only files); a failed folder copy goes to
//!   Drive's Trash.
//! - `rename_new`, `move_to`, `trash_for_replace`, `take_back` and
//!   `discard` are one PATCH each; the names they take were looked up by
//!   `landing` just before (check, then act: Drive has no rename that
//!   refuses a taken name).
//!
//! A [`Ctx`] is the context of one transfer: the primitives run on it, and
//! it remembers what `landing` learnt of the source, so the steps after it
//! ask Drive nothing again.

use std::path::{Path, PathBuf};

use places::{Cx, Error, Existing, Landing, Put, Reading, Refusal, Source, Unread};

use crate::CloudState;
use crate::drive::{DriveClient, DriveError, DriveFile, FileChange};
use crate::location::DriveLocation;
use crate::service;
use crate::write::{self, Ctx, id_of, message, pending_target};
use places::Measure;
use places::Progress;

/// Total size and item count of Drive items, for an operation's progress.
pub fn measure(state: &CloudState, paths: &[String]) -> Result<Measure, String> {
    write::live(state, |c| measure_in(c, paths))
}

pub(crate) fn measure_in(c: &Ctx, paths: &[String]) -> Result<Measure, String> {
    let mut total = Measure::default();
    for path in paths {
        let location = service::drive_location(path)?;
        let id = location
            .file_id()
            .filter(|id| !id.is_empty())
            .ok_or_else(|| "This Google Drive item has no id.".to_string())?;
        let client = write::client(c, &location.account)?;
        let file = client.get(id).map_err(message)?;
        measure_file(&client, &file, &mut total)?;
    }
    Ok(total)
}

/// Google Docs and shortcuts count as items without bytes.
fn measure_file(client: &DriveClient, file: &DriveFile, total: &mut Measure) -> Result<(), String> {
    total.items += 1;
    if file.is_dir() {
        for child in client.list_all(&file.id).map_err(message)? {
            measure_file(client, &child, total)?;
        }
    } else {
        total.bytes += drive_size(file).unwrap_or(0);
    }
    Ok(())
}

// ---- The primitives ----------------------------------------------------------

/// Today's errors are words; they keep them, so the engine's error is the
/// same text.
fn words(message: String) -> Error {
    Error::Other(message)
}

fn drive_words(error: DriveError) -> Error {
    words(message(error))
}

/// The account's client. Its failures are the credential's (not connected,
/// no token, a refused refresh): `AuthRequired`, so the engine can tell
/// them from a failed request.
fn client_of<'a>(c: &Ctx<'a>, account: &str) -> places::Result<DriveClient<'a>> {
    write::client(c, account).map_err(Error::AuthRequired)
}

/// What the path-only refusals of a transfer inside Drive found: the
/// refusals of `placement::check_transfer`, for Drive, by id.
struct Within {
    source: DriveLocation,
    source_id: String,
    target_parent: DriveLocation,
    target_parent_id: String,
    name: String,
}

fn within(c: &Ctx, from: &str, to: &str) -> Result<Within, String> {
    let (source, _) = service::writable(c.state, from)?;
    let source_id = id_of(&source)?;
    let target = service::drive_location(to)?;
    if target.account != source.account {
        return Err("Copying between Google accounts is not supported.".into());
    }
    let (target_parent, target_parent_id, name) = pending_target(to)?;
    if target_parent.segments.iter().any(|s| s.id == source_id) {
        return Err(Refusal::IntoItself.into());
    }
    let source_parent_id = source.parent().and_then(|p| p.file_id().map(str::to_string));
    if source_parent_id.as_deref() == Some(target_parent_id.as_str()) && source.name() == Some(name.as_str()) {
        return Err(Refusal::SamePath.into());
    }
    Ok(Within { source, source_id, target_parent, target_parent_id, name })
}

/// The pending path of `name` in `parent`, when a path can say it (a
/// name with a `/` can't).
fn pending_child(parent: &DriveLocation, name: &str) -> Option<String> {
    let uri = parent.child("", name).to_uri();
    DriveLocation::parse(&uri).ok().map(|_| uri)
}

/// Where a moved item goes back to: its old folder, under the name Drive
/// gave it, or the name its path gave it where a path can't say Drive's
/// (`move_to` then sends Drive's own name, from `Ctx::homes`).
fn home_of(source: &DriveLocation, name: &str) -> Option<String> {
    let parent = source.parent()?;
    pending_child(&parent, name).or_else(|| pending_child(&parent, source.name().unwrap_or_default()))
}

impl Ctx<'_> {
    /// The source `landing` fetched, by id, or Drive's answer now.
    fn file(&self, client: &DriveClient, id: &str) -> places::Result<DriveFile> {
        if let Some(file) = self.seen.borrow().get(id) {
            return Ok(file.clone());
        }
        client.get(id).map_err(drive_words)
    }

    fn remember(&self, file: &DriveFile) {
        self.seen.borrow_mut().insert(file.id.clone(), file.clone());
    }

    /// One PATCH of the item at `path`.
    fn patch(&self, path: &str, change: &FileChange) -> places::Result<DriveFile> {
        let location = service::drive_location(path).map_err(words)?;
        let id = id_of(&location).map_err(words)?;
        client_of(self, &location.account)?.update(&id, change).map_err(drive_words)
    }
}

impl places::Transfer for Ctx<'_> {
    fn check(&self, from: Option<&str>, to: Option<&str>, _: &Cx) -> places::Result<()> {
        let checked = match (from, to) {
            (Some(from), Some(to)) => within(self, from, to).map(|_| ()),
            (None, Some(to)) => service::writable(self.state, to).and_then(|_| pending_target(to)).map(|_| ()),
            (Some(from), None) => service::writable(self.state, from).map(|_| ()),
            (None, None) => Ok(()),
        };
        checked.map_err(words)
    }

    fn landing(&self, from: Option<&str>, to: &str, _: &Cx) -> places::Result<Landing> {
        let Some(from) = from else {
            let (parent, parent_id, name) = pending_target(to).map_err(words)?;
            let client = client_of(self, &parent.account)?;
            let found = client.find_children(&parent_id, &name).map_err(drive_words)?;
            let existing = found
                .iter()
                .map(|other| Existing { path: parent.child(&other.id, &other.name).to_uri(), holds_source: false })
                .collect();
            return Ok(Landing { parent: parent.to_uri(), name, existing, source: None });
        };
        let w = within(self, from, to).map_err(words)?;
        let client = client_of(self, &w.source.account)?;
        let shown = w.source.name().unwrap_or_default().to_string();
        let source = match client.get(&w.source_id) {
            Ok(file) if !file.trashed => file,
            Ok(_) | Err(DriveError::NotFound) => return Err(Refusal::Vanished(shown).into()),
            Err(error) => return Err(drive_words(error)),
        };
        self.remember(&source);
        let home = home_of(&w.source, &source.name);
        if let Some(home) = &home {
            self.homes.borrow_mut().insert(home.clone(), source.name.clone());
        }
        let existing = client
            .find_children(&w.target_parent_id, &w.name)
            .map_err(drive_words)?
            .into_iter()
            .filter(|other| other.id != w.source_id)
            .map(|other| Existing {
                path: w.target_parent.child(&other.id, &other.name).to_uri(),
                holds_source: w.source.segments.iter().any(|s| s.id == other.id),
            })
            .collect();
        Ok(Landing {
            parent: w.target_parent.to_uri(),
            name: w.name,
            existing,
            source: Some(Source {
                path: w.source.to_uri(),
                is_dir: source.is_dir(),
                home,
            }),
        })
    }

    fn pending(&self, parent: &str, name: &str) -> places::Result<String> {
        let parent = service::drive_location(parent).map_err(words)?;
        pending_child(&parent, name).ok_or_else(|| words("Google Drive name contains an invalid character".into()))
    }

    /// Upload `source` under the pending `target`; a folder is created and
    /// filled, and goes to Drive's Trash if that fails.
    fn write(&self, source: &Path, target: &str, _replace: bool, cx: &Cx) -> places::Result<Put> {
        let (parent, parent_id, name) = pending_target(target).map_err(words)?;
        let meta = std::fs::symlink_metadata(source)
            .map_err(|_| Error::from(Refusal::Vanished(source.display().to_string())))?;
        let client = client_of(self, &parent.account)?;
        let mut skipped = Vec::new();
        let made = if meta.is_dir() {
            let folder = client.create_folder(&parent_id, &name).map_err(drive_words)?;
            let uploaded = upload_children(&client, source, &folder.id, &mut skipped, &cx.progress);
            if let Err(error) = uploaded {
                let _ = client.update(&folder.id, &FileChange::trash(true));
                return Err(words(error));
            }
            cx.progress.add_item();
            folder
        } else {
            upload_verified(&client, &parent_id, &name, source, &cx.progress).map_err(words)?
        };
        // The rest was uploaded; each skipped item fails the transfer with
        // its name, so a move keeps its source.
        let skipped = skipped
            .iter()
            .map(|name| format!("“{name}” was not copied to Google Drive: only files and folders can be."))
            .collect();
        Ok(Put { path: parent.child(&made.id, &made.name).to_uri(), skipped })
    }

    /// The item read whole into `target`, without staging.
    fn read_into(&self, path: &str, target: &Path, cx: &Cx) -> places::Result<Put> {
        let mut skipped = Vec::new();
        self.read(
            path,
            &mut |item| {
                skipped = item.fill(target, cx).map_err(|(Unread::Nothing(e) | Unread::Partial(e))| e)?;
                Ok(())
            },
            cx,
        )?;
        Ok(Put { path: target.to_string_lossy().into_owned(), skipped })
    }

    fn read(&self, path: &str, place: &mut dyn FnMut(&dyn Reading) -> places::Result<()>, _: &Cx) -> places::Result<()> {
        let location = service::drive_location(path).map_err(words)?;
        let id = location
            .file_id()
            .filter(|id| !id.is_empty())
            .ok_or_else(|| words("This Google Drive item has no id.".into()))?
            .to_string();
        let client = client_of(self, &location.account)?;
        let shown = location.name().unwrap_or_default().to_string();
        let file = match client.get(&id) {
            Ok(file) if !file.trashed => file,
            Ok(_) | Err(DriveError::NotFound) => return Err(Refusal::Vanished(shown).into()),
            Err(error) => return Err(drive_words(error)),
        };
        if !file.is_dir() && !file.can_download() {
            return Err(words(not_downloadable(&file)));
        }
        place(&Download { client: &client, file: &file })
    }

    fn export(&self, _: &str, _: &Cx) -> places::Result<PathBuf> {
        Err(Error::Unsupported("A Google Drive item is read straight into its target.".into()))
    }

    /// A server-side copy of `from` under the pending `to`; a folder is
    /// made again and filled, and goes to Drive's Trash if that fails.
    fn copy_within(&self, from: &str, to: &str, cx: &Cx) -> places::Result<Put> {
        let source = service::drive_location(from).map_err(words)?;
        let id = id_of(&source).map_err(words)?;
        let (parent, parent_id, name) = pending_target(to).map_err(words)?;
        let client = client_of(self, &source.account)?;
        let file = self.file(&client, &id)?;
        let made = if file.is_dir() {
            let staged = client.create_folder(&parent_id, &name).map_err(drive_words)?;
            let copied = copy_children(&client, &file.id, &staged.id, &staged.id, &cx.progress);
            if let Err(error) = copied {
                let _ = client.update(&staged.id, &FileChange::trash(true));
                return Err(drive_words(error));
            }
            staged
        } else {
            cx.progress.check().map_err(words)?;
            client.copy_file(&file.id, &parent_id, &name).map_err(drive_words)?
        };
        cx.progress.add_item();
        Ok(Put { path: parent.child(&made.id, &made.name).to_uri(), skipped: Vec::new() })
    }

    /// Give `from` the pending `to`'s name, without asking first.
    fn rename_new(&self, from: &str, to: &str, _: &Cx) -> places::Result<String> {
        let (parent, _, name) = pending_target(to).map_err(words)?;
        let done = self.patch(from, &FileChange::rename(&name))?;
        Ok(parent.child(&done.id, &done.name).to_uri())
    }

    /// One PATCH: the name when it changes, the parents when the folder
    /// does. The top of My Drive can't move. Moved back to its home, an
    /// item gets Drive's exact name again, which its path may not say.
    fn move_to(&self, from: &str, to: &str, _: &Cx) -> places::Result<String> {
        let item = service::drive_location(from).map_err(words)?;
        let id = id_of(&item).map_err(words)?;
        let from_parent_id = item
            .parent()
            .and_then(|p| p.file_id().map(str::to_string))
            .ok_or_else(|| words("My Drive itself cannot be moved.".into()))?;
        let (parent, parent_id, name) = pending_target(to).map_err(words)?;
        let name = self.homes.borrow().get(to).cloned().unwrap_or(name);
        let client = client_of(self, &item.account)?;
        let current = match self.seen.borrow().get(&id) {
            Some(file) => file.name.clone(),
            None => item.name().unwrap_or_default().to_string(),
        };
        let mut change = FileChange::default();
        if name != current {
            change.name = Some(name);
        }
        if parent_id != from_parent_id {
            change.add_parent = Some(parent_id);
            change.remove_parent = Some(from_parent_id);
        }
        let moved = client.update(&id, &change).map_err(drive_words)?;
        self.remember(&moved);
        Ok(parent.child(&moved.id, &moved.name).to_uri())
    }

    /// `trashed=true`; the path stays valid (the id survives).
    fn trash_for_replace(&self, path: &str, _: &Cx) -> places::Result<String> {
        self.patch(path, &FileChange::trash(true))?;
        Ok(path.to_string())
    }

    /// `trashed=false`: back in the folder it was trashed from.
    fn take_back(&self, trashed: &str, _original: &str, _: &Cx) -> places::Result<()> {
        self.patch(trashed, &FileChange::trash(false)).map(|_| ())
    }

    /// `trashed=true`, never deleted.
    fn discard(&self, staged: &str, _: &Cx) -> places::Result<()> {
        self.patch(staged, &FileChange::trash(true)).map(|_| ())
    }
}

// ---- Uploads -----------------------------------------------------------------

/// Upload one file and check Drive kept every byte; a short file is trashed.
fn upload_verified(
    client: &DriveClient,
    parent_id: &str,
    name: &str,
    local: &Path,
    progress: &Progress,
) -> Result<DriveFile, String> {
    let expected = std::fs::metadata(local)
        .map_err(|error| error.to_string())?
        .len();
    let uploaded = client
        .upload(parent_id, name, local, progress)
        .map_err(message)?;
    if drive_size(&uploaded) != Some(expected) {
        let _ = client.update(&uploaded.id, &FileChange::trash(true));
        return Err("The upload to Google Drive was incomplete.".into());
    }
    progress.add_item();
    Ok(uploaded)
}

/// Upload a folder's contents. Anything that is not a plain file or folder
/// (symlinks, FIFOs, sockets, devices) and anything unreadable is skipped
/// and named in `skipped`.
fn upload_children(
    client: &DriveClient,
    dir: &Path,
    parent_id: &str,
    skipped: &mut Vec<String>,
    progress: &Progress,
) -> Result<(), String> {
    let mut children = Vec::new();
    for entry in std::fs::read_dir(dir).map_err(|error| error.to_string())? {
        match entry {
            Ok(entry) => children.push(entry),
            Err(_) => skipped.push(format!("{} (an unreadable item)", dir.display())),
        }
    }
    children.sort_by_key(|entry| entry.file_name());
    for entry in children {
        progress.check()?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            skipped.push(name);
            continue;
        };
        if meta.is_dir() {
            let folder = client.create_folder(parent_id, &name).map_err(message)?;
            upload_children(client, &path, &folder.id, skipped, progress)?;
            progress.add_item();
        } else if meta.is_file() {
            upload_verified(client, parent_id, &name, &path, progress)?;
        } else {
            skipped.push(name);
        }
    }
    Ok(())
}

fn drive_size(file: &DriveFile) -> Option<u64> {
    file.size.as_deref().and_then(|size| size.parse().ok())
}

// ---- Server-side copies --------------------------------------------------------

/// Copy a folder's contents into another folder, recursively. Drive copies
/// only files, so folders are recreated. `staged` (the copy being built) is
/// skipped: when the target lies inside the source, the walk meets it, and
/// copying it would never end. Each item is counted, and a cancel stops the
/// walk before the next one.
fn copy_children(
    client: &DriveClient,
    from: &str,
    to: &str,
    staged: &str,
    progress: &Progress,
) -> Result<(), DriveError> {
    for child in client.list_all(from)? {
        if child.id == staged {
            continue;
        }
        if progress.is_cancelled() {
            return Err(DriveError::Cancelled);
        }
        if child.is_dir() {
            let folder = client.create_folder(to, &child.name)?;
            copy_children(client, &child.id, &folder.id, staged, progress)?;
        } else {
            client.copy_file(&child.id, to, &child.name)?;
        }
        progress.add_item();
    }
    Ok(())
}

// ---- Downloads -----------------------------------------------------------------

/// Why a Drive item has no bytes to download.
fn not_downloadable(file: &DriveFile) -> String {
    let name = &file.name;
    if file.is_shortcut() {
        format!("“{name}” is a Google Drive shortcut and can only be copied within Google Drive.")
    } else if file.is_google_doc() {
        format!("“{name}” is a Google Docs file and can only be copied within Google Drive.")
    } else {
        format!("“{name}” can't be downloaded: its owner turned downloads off.")
    }
}

/// A Drive item being read into a local target.
struct Download<'a, 'c> {
    client: &'a DriveClient<'c>,
    file: &'a DriveFile,
}

impl Reading for Download<'_, '_> {
    fn is_dir(&self) -> bool {
        self.file.is_dir()
    }

    /// A file whole, a folder with everything in it that has bytes (the
    /// others are named in the result).
    fn fill(&self, dest: &Path, cx: &Cx) -> Result<Vec<String>, Unread> {
        let mut skipped = Vec::new();
        download_to(self.client, self.file, dest, &mut skipped, &cx.progress)?;
        Ok(skipped)
    }
}

/// Download `file` to `dest`: a file whole, a folder with everything in it
/// that has bytes (the others are listed in `skipped`).
fn download_to(
    client: &DriveClient,
    file: &DriveFile,
    dest: &Path,
    skipped: &mut Vec<String>,
    progress: &Progress,
) -> Result<(), Unread> {
    let partial = |error: String| Unread::Partial(words(error));
    if !file.is_dir() {
        return download_verified(client, file, dest, progress).map_err(partial);
    }
    // Only a folder this call created may be discarded on failure.
    std::fs::create_dir(dest).map_err(|error| Unread::Nothing(words(error.to_string())))?;
    download_children(client, &file.id, dest, skipped, progress).map_err(partial)?;
    progress.add_item();
    Ok(())
}

/// Download one file to `dest` and check its size against Drive's.
fn download_verified(
    client: &DriveClient,
    file: &DriveFile,
    dest: &Path,
    progress: &Progress,
) -> Result<(), String> {
    client.download(&file.id, dest, progress).map_err(message)?;
    let written = std::fs::metadata(dest).map(|meta| meta.len()).ok();
    if drive_size(file).is_some_and(|size| Some(size) != written) {
        let _ = std::fs::remove_file(dest);
        return Err("The download from Google Drive was incomplete.".into());
    }
    progress.add_item();
    Ok(())
}

/// Download a Drive folder's contents into `dir`. Items without bytes
/// (Google Docs, shortcuts) are skipped, with the reason in `skipped`;
/// Drive names become safe, unique local names.
fn download_children(
    client: &DriveClient,
    folder_id: &str,
    dir: &Path,
    skipped: &mut Vec<String>,
    progress: &Progress,
) -> Result<(), String> {
    for child in client.list_all(folder_id).map_err(message)? {
        progress.check()?;
        if !child.is_dir() && !child.can_download() {
            skipped.push(not_downloadable(&child));
            continue;
        }
        let path = unique_child(dir, &service::safe_name(&child.name));
        if child.is_dir() {
            std::fs::create_dir(&path).map_err(|error| error.to_string())?;
            download_children(client, &child.id, &path, skipped, progress)?;
            progress.add_item();
        } else {
            download_verified(client, &child, &path, progress)?;
        }
    }
    Ok(())
}

/// `name` in `dir`, or "name (2).ext", "name (3).ext"… when Drive had several
/// items with one name.
fn unique_child(dir: &Path, name: &str) -> PathBuf {
    let first = dir.join(name);
    if std::fs::symlink_metadata(&first).is_err() {
        return first;
    }
    let dot = name.rfind('.').filter(|&i| i > 0);
    let (stem, ext) = dot.map_or((name, ""), |i| name.split_at(i));
    (2..)
        .map(|n| dir.join(format!("{stem} ({n}){ext}")))
        .find(|path| std::fs::symlink_metadata(path).is_err())
        .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_server::Reply;
    use crate::testing::{ROOT, file, folder, found, heads, ok, run};
    use places::Transfer;

    #[test]
    fn measure_sums_drive_sizes_and_counts_docs() {
        let doc = format!(r#"{{"id":"g1","name":"Plan","mimeType":"{DOC}"}}"#);
        let (result, _) = run(
            vec![ok(folder("d1", "Docs")), found(&[sized("x1", "x.txt", 100), doc])],
            |c| measure_in(c, &[format!("{ROOT}/d1~Docs")]),
        );
        assert_eq!(
            result.unwrap(),
            places::Measure {
                bytes: 100,
                items: 3
            }
        );
    }

    const DOC: &str = "application/vnd.google-apps.document";

    fn sized(id: &str, name: &str, size: u64) -> String {
        format!(r#"{{"id":"{id}","name":"{name}","mimeType":"text/plain","size":"{size}"}}"#)
    }

    // ---- Each primitive on its own: the requests it sends --------------------------

    fn cx() -> Cx {
        Cx::default()
    }

    fn bodies(log: &crate::test_server::Log) -> Vec<String> {
        log.lock().unwrap().iter().map(|r| r.body_text()).collect()
    }

    #[test]
    fn check_refuses_by_path_without_a_request() {
        let (result, log) = run(vec![], |c| {
            (
                c.check(Some(&format!("{ROOT}/d1~Docs")), Some(&format!("{ROOT}/d1~Docs/s~Sub/~Docs")), &cx()),
                c.check(None, Some(&format!("{ROOT}/f1~a.txt")), &cx()),
                c.check(Some("gdrive://"), None, &cx()),
                c.check(Some(&format!("{ROOT}/f1~a.txt")), None, &cx()),
            )
        });
        assert_eq!(result.0.unwrap_err().to_string(), "Cannot copy or move a folder into itself");
        assert_eq!(
            result.1.unwrap_err().to_string(),
            "The destination must be a new name in a Google Drive folder."
        );
        assert!(result.2.is_err());
        assert_eq!(result.3, Ok(()));
        assert!(heads(&log).is_empty());
    }

    #[test]
    fn landing_looks_the_name_up_and_says_what_is_there() {
        let (result, log) = run(vec![found(&[file("x", "a.txt")])], |c| c.landing(None, &format!("{ROOT}/~a.txt"), &cx()));
        let landing = result.unwrap();
        assert_eq!(landing.parent, ROOT);
        assert_eq!(landing.name, "a.txt");
        assert_eq!(landing.existing, [Existing { path: format!("{ROOT}/x~a.txt"), holds_source: false }]);
        assert_eq!(landing.source, None);
        assert!(heads(&log)[0].starts_with("GET /files?"));

        let from = format!("{ROOT}/d1~Docs/f1~Docs");
        let (result, log) = run(vec![ok(file("f1", "Renamed")), found(&[folder("d1", "Docs")])], |c| {
            let landing = c.landing(Some(&from), &format!("{ROOT}/~Docs"), &cx());
            (landing, c.seen.borrow().contains_key("f1"))
        });
        let (landing, remembered) = result;
        let landing = landing.unwrap();
        assert!(remembered, "the source is remembered for the steps after");
        assert_eq!(landing.existing, [Existing { path: format!("{ROOT}/d1~Docs"), holds_source: true }]);
        let source = landing.source.unwrap();
        assert_eq!((source.path, source.is_dir), (from, false));
        assert_eq!(source.home.as_deref(), Some(format!("{ROOT}/d1~Docs/~Renamed").as_str()));
        assert!(heads(&log)[0].starts_with("GET /files/f1?"));
    }

    #[test]
    fn a_vanished_or_trashed_source_lands_nowhere() {
        let trashed = r#"{"id":"f1","name":"a.txt","mimeType":"text/plain","trashed":true}"#;
        for reply in [Reply::json(404, "{}"), Reply::json(200, trashed)] {
            let (result, log) =
                run(vec![reply], |c| c.landing(Some(&format!("{ROOT}/f1~a.txt")), &format!("{ROOT}/p~P/~a.txt"), &cx()));
            assert_eq!(result.unwrap_err().to_string(), "“a.txt” no longer exists");
            assert_eq!(heads(&log).len(), 1);
        }
    }

    #[test]
    fn a_pending_path_is_one_a_path_can_say() {
        let (result, log) = run(vec![], |c| (c.pending(ROOT, "a b~c.txt"), c.pending(ROOT, "a/b")));
        assert_eq!(result.0.unwrap(), format!("{ROOT}/~a%20b%7Ec.txt"));
        assert!(result.1.is_err());
        assert!(heads(&log).is_empty());
    }

    #[test]
    fn rename_trash_take_back_and_discard_are_one_patch_each() {
        let staged = format!("{ROOT}/s1~.a.txt.filebrowser-copy");
        let (result, log) = run(
            vec![ok(file("s1", "a.txt")), ok(file("o1", "a.txt")), ok(file("o1", "a.txt")), ok(file("s1", "x"))],
            |c| {
                (
                    c.rename_new(&staged, &format!("{ROOT}/~a.txt"), &cx()),
                    c.trash_for_replace(&format!("{ROOT}/o1~a.txt"), &cx()),
                    c.take_back(&format!("{ROOT}/o1~a.txt"), &format!("{ROOT}/o1~a.txt"), &cx()),
                    c.discard(&staged, &cx()),
                )
            },
        );
        assert_eq!(result.0.unwrap(), format!("{ROOT}/s1~a.txt"));
        assert_eq!(result.1.unwrap(), format!("{ROOT}/o1~a.txt"));
        assert_eq!((result.2, result.3), (Ok(()), Ok(())));
        let heads = heads(&log);
        for (head, id) in heads.iter().zip(["s1", "o1", "o1", "s1"]) {
            assert!(head.starts_with(&format!("PATCH /files/{id}?")), "{head}");
        }
        assert_eq!(
            bodies(&log),
            [r#"{"name":"a.txt"}"#, r#"{"trashed":true}"#, r#"{"trashed":false}"#, r#"{"trashed":true}"#]
        );
    }

    #[test]
    fn a_failed_patch_is_the_primitives_error() {
        let (result, _) = run(vec![Reply::json(403, r#"{"error":{"message":"nope"}}"#)], |c| {
            c.trash_for_replace(&format!("{ROOT}/o1~a.txt"), &cx())
        });
        assert!(result.unwrap_err().to_string().contains("nope"));
    }

    #[test]
    fn move_to_names_and_reparents_only_what_changes() {
        // Same folder, new name: the name only.
        let (result, log) =
            run(vec![ok(file("f1", "b.txt"))], |c| c.move_to(&format!("{ROOT}/p~P/f1~a.txt"), &format!("{ROOT}/p~P/~b.txt"), &cx()));
        assert_eq!(result.unwrap(), format!("{ROOT}/p~P/f1~b.txt"));
        assert!(!heads(&log)[0].contains("Parents"));
        assert_eq!(bodies(&log), [r#"{"name":"b.txt"}"#]);
        // Another folder, same name: the parents only.
        let (result, log) =
            run(vec![ok(file("f1", "a.txt"))], |c| c.move_to(&format!("{ROOT}/p~P/f1~a.txt"), &format!("{ROOT}/~a.txt"), &cx()));
        assert_eq!(result.unwrap(), format!("{ROOT}/f1~a.txt"));
        assert!(heads(&log)[0].contains("addParents=root") && heads(&log)[0].contains("removeParents=p"));
        assert_eq!(bodies(&log), ["{}"]);
        // The top of My Drive doesn't move, and nothing is asked.
        let (result, log) = run(vec![], |c| c.move_to("gdrive://acct/f1~a.txt", &format!("{ROOT}/~a.txt"), &cx()));
        assert_eq!(result.unwrap_err().to_string(), "My Drive itself cannot be moved.");
        assert!(heads(&log).is_empty());
    }

    /// The name Drive gave the item, when this transfer learnt it, decides
    /// whether the move renames, not the name its path still shows.
    #[test]
    fn move_to_compares_with_the_name_drive_has() {
        let (result, log) = run(vec![ok(file("f1", "a.txt"))], |c| {
            c.remember(&serde_json::from_str(&file("f1", "renamed.txt")).unwrap());
            c.move_to(&format!("{ROOT}/f1~a.txt"), &format!("{ROOT}/~a.txt"), &cx())
        });
        result.unwrap();
        assert_eq!(bodies(&log), [r#"{"name":"a.txt"}"#]);
    }

    #[test]
    fn copy_within_asks_for_the_source_only_when_it_wasnt_seen() {
        let (result, log) = run(vec![ok(file("f1", "a.txt")), ok(file("c1", "a.txt"))], |c| {
            c.copy_within(&format!("{ROOT}/f1~a.txt"), &format!("{ROOT}/p~P/~a.txt"), &cx())
        });
        assert_eq!(result.unwrap().path, format!("{ROOT}/p~P/c1~a.txt"));
        let heads = heads(&log);
        assert!(heads[0].starts_with("GET /files/f1?") && heads[1].starts_with("POST /files/f1/copy?"));
    }

    #[test]
    fn a_google_doc_is_refused_before_the_engine_places_anything() {
        let doc = format!(r#"{{"id":"g1","name":"Plan","mimeType":"{DOC}"}}"#);
        let mut placed = false;
        let (result, _) = run(vec![ok(doc)], |c| {
            c.read(&format!("{ROOT}/g1~Plan"), &mut |_| {
                placed = true;
                Ok(())
            }, &cx())
        });
        assert_eq!(result.unwrap_err().to_string(), "“Plan” is a Google Docs file and can only be copied within Google Drive.");
        assert!(!placed);
    }

    #[test]
    fn read_into_fills_the_target_as_it_is() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("a.txt");
        let bytes = Reply { status: 200, body: b"hello".to_vec(), content_type: "application/octet-stream", headers: Vec::new() };
        let (result, _) = run(vec![ok(sized("f1", "a.txt", 5)), bytes], |c| c.read_into(&format!("{ROOT}/f1~a.txt"), &target, &cx()));
        assert_eq!(result.unwrap(), Put { path: target.to_string_lossy().into_owned(), skipped: Vec::new() });
        assert_eq!(std::fs::read(&target).unwrap(), b"hello");
    }

    #[test]
    fn a_folder_write_that_fails_trashes_what_it_made() {
        let dir = tempfile::tempdir().unwrap();
        let tree = dir.path().join("Docs");
        std::fs::create_dir(&tree).unwrap();
        std::fs::write(tree.join("x.txt"), b"hi").unwrap();
        let (result, log) = run(
            vec![
                ok(folder("s1", "Docs")),
                Reply::json(403, r#"{"error":{"message":"quota"}}"#),
                ok(folder("s1", "Docs")),
            ],
            |c| c.write(&tree, &format!("{ROOT}/~Docs"), false, &cx()),
        );
        assert!(result.unwrap_err().to_string().contains("quota"));
        assert!(heads(&log)[2].starts_with("PATCH /files/s1?"));
        assert_eq!(bodies(&log)[2], r#"{"trashed":true}"#);
    }

    #[test]
    fn drive_reads_straight_into_targets_and_exports_nothing() {
        let (result, log) = run(vec![], |c| c.export(&format!("{ROOT}/f1~a.txt"), &cx()));
        assert!(matches!(result, Err(Error::Unsupported(_))));
        assert!(heads(&log).is_empty());
    }
}
