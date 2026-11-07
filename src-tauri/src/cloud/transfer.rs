//! Copying and moving between local folders and Google Drive (spec D17).
//!
//! Uploads: a file goes straight to its final name, since Drive creates it
//! only when the last chunk arrives, and is checked against the local size.
//! A folder (or a Replace) is built under a staging name and renamed once
//! complete; on failure the staged item goes to Drive's Trash.
//!
//! Downloads: a file, or a folder replacing an existing one, lands in a
//! hidden staging path next to the target, is checked against Drive's sizes,
//! then renamed into place; on failure the staging path is removed. A new
//! folder downloads straight under its name so it fills as you watch (as in
//! GNOME Files); each file in it is still checked, and if the download fails
//! or is cancelled the partial folder goes to the Trash. `placement` does the
//! staging and the Replace, as for local copies.
//!
//! A move removes its source only after the copy succeeded: a local source
//! goes to the Trash, a Drive source to Drive's Trash.

use std::path::{Path, PathBuf};

use crate::cloud::CloudState;
use crate::cloud::drive::{DriveClient, DriveError, DriveFile, FileChange};
use crate::cloud::service;
use crate::cloud::write::{self, Ctx, message};
use crate::fs_ops::Measure;
use crate::placement::{self, DiscardFn, Refusal, Stage, Staged, TrashFn, Unfilled};
use crate::progress::Progress;

const UPLOAD_SUFFIX: &str = ".filebrowser-upload";

pub fn upload(
    state: &CloudState,
    from: &Path,
    to: &str,
    overwrite: bool,
    progress: &Progress,
) -> Result<String, String> {
    write::live_with(state, progress, |c| upload_in(c, from, to, overwrite))
}

pub fn download(
    state: &CloudState,
    from: &str,
    to: &Path,
    overwrite: bool,
    progress: &Progress,
    trash: TrashFn,
) -> Result<String, String> {
    write::live_with(state, progress, |c| download_in(c, from, to, overwrite, trash))
}

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

/// Copy or move between any two locations where at least one is in Drive.
/// `trash_local` is the local Trash: for a local source after a move, a
/// partial folder a download leaves and the local item a download replaces.
pub fn dispatch(
    state: &CloudState,
    from: &str,
    to: &str,
    overwrite: bool,
    moving: bool,
    progress: &Progress,
    trash_local: TrashFn,
) -> Result<String, String> {
    write::live_with(state, progress, |c| {
        dispatch_in(c, from, to, overwrite, moving, trash_local)
    })
}

pub(crate) fn dispatch_in(
    c: &Ctx,
    from: &str,
    to: &str,
    overwrite: bool,
    moving: bool,
    trash_local: TrashFn,
) -> Result<String, String> {
    let (from_cloud, to_cloud) = (crate::cloud::is_cloud(from), crate::cloud::is_cloud(to));
    match (from_cloud, to_cloud, moving) {
        (true, true, true) => write::move_in(c, from, to, overwrite),
        (true, true, false) => write::copy_in(c, from, to, overwrite),
        (false, true, true) => move_up_in(c, Path::new(from), to, overwrite, discard_to(trash_local)),
        (false, true, false) => upload_in(c, Path::new(from), to, overwrite),
        (true, false, true) => move_down_in(c, from, Path::new(to), overwrite, trash_local),
        (true, false, false) => {
            download_into(c, from, Path::new(to), overwrite, &discard_to(trash_local), trash_local)
        }
        (false, false, _) => Err("Neither location is in Google Drive.".into()),
    }
}

/// `trash` for an item that needn't come back: a moved source, a partial
/// folder.
fn discard_to<'a>(trash: TrashFn<'a>) -> impl Fn(&Path) -> Result<(), String> + 'a {
    move |path| trash.send(path).map(|_| ())
}

pub(crate) fn upload_in(c: &Ctx, from: &Path, to: &str, overwrite: bool) -> Result<String, String> {
    let (_, account) = service::writable(c.state, to)?;
    let (parent, parent_id, name) = write::pending_target(to)?;
    let meta = std::fs::symlink_metadata(from)
        .map_err(|_| String::from(Refusal::Vanished(from.display().to_string())))?;
    let client = write::client(c, &account.id)?;
    let existing = client.find_children(&parent_id, &name).map_err(message)?;
    let replaced = match existing.len() {
        0 => None,
        _ if !overwrite => return Err(Refusal::Exists(name).into()),
        1 => existing.into_iter().next(),
        _ => {
            return Err(format!(
                "Several items in this folder are named “{name}”. Rename or remove the extra ones in Google Drive first."
            ));
        }
    };
    let staged_name = if meta.is_dir() || replaced.is_some() {
        format!(".{name}{UPLOAD_SUFFIX}")
    } else {
        name.clone()
    };
    let mut skipped = Vec::new();
    let staged = if meta.is_dir() {
        let folder = client
            .create_folder(&parent_id, &staged_name)
            .map_err(message)?;
        let uploaded = upload_children(&client, from, &folder.id, &mut skipped, &c.progress);
        if let Err(error) = uploaded {
            let _ = client.update(&folder.id, &FileChange::trash(true));
            return Err(error);
        }
        c.progress.add_item();
        folder
    } else {
        upload_verified(&client, &parent_id, &staged_name, from, &c.progress)?
    };
    let uploaded = if staged_name == name {
        parent.child(&staged.id, &staged.name).to_uri()
    } else {
        write::finish_staged(&client, &parent, &name, replaced.as_ref(), &staged, true)?
    };
    match skipped.first() {
        // The rest was uploaded; the error names what was left out, so a
        // move keeps its source.
        Some(name) => Err(format!(
            "“{name}” was not copied to Google Drive: only files and folders can be."
        )),
        None => Ok(uploaded),
    }
}

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

/// `download_into` that removes a partial folder instead of trashing it; an
/// item it replaces still goes to `trash`.
pub(crate) fn download_in(
    c: &Ctx,
    from: &str,
    to: &Path,
    overwrite: bool,
    trash: TrashFn,
) -> Result<String, String> {
    download_into(c, from, to, overwrite, &placement::remove, trash)
}

/// Download a Drive item to `to`. A new folder downloads straight under its
/// name, so it fills as you watch; if that fails, the partial folder goes to
/// `discard` (the Trash). Files and replacements stage under a hidden name
/// and appear only when complete; the item a Replace sends away goes to
/// `trash` (the app's logged Trash, so Restore knows where it was), which
/// takes it back if the Replace fails.
pub(crate) fn download_into(
    c: &Ctx,
    from: &str,
    to: &Path,
    overwrite: bool,
    discard: DiscardFn,
    trash: TrashFn,
) -> Result<String, String> {
    let target = placement::check_new(to, overwrite)?;
    let location = service::drive_location(from)?;
    let id = location
        .file_id()
        .filter(|id| !id.is_empty())
        .ok_or_else(|| "This Google Drive item has no id.".to_string())?
        .to_string();
    let client = write::client(c, &location.account)?;
    let shown = location.name().unwrap_or_default().to_string();
    let file = match client.get(&id) {
        Ok(file) if !file.trashed => file,
        Ok(_) | Err(DriveError::NotFound) => return Err(Refusal::Vanished(shown).into()),
        Err(error) => return Err(message(error)),
    };
    if !file.is_dir() && !file.can_download() {
        return Err(not_downloadable(&file));
    }
    let mut skipped = Vec::new();
    let fill = |dest: &Path| download_to(&client, &file, dest, &mut skipped, &c.progress);
    target.place(Stage::by_kind(file.is_dir(), discard), fill, trash)?;
    match skipped.first() {
        // Everything else was copied; the error names what was left out.
        Some(reason) => Err(reason.clone()),
        None => Ok(to.to_string_lossy().into_owned()),
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
) -> Result<Staged<'static>, Unfilled> {
    if !file.is_dir() {
        return download_verified(client, file, dest, progress)
            .map(|()| Staged::Copy)
            .map_err(Unfilled::Partial);
    }
    // Only a folder this call created may be discarded on failure.
    std::fs::create_dir(dest).map_err(|error| Unfilled::Nothing(error.to_string()))?;
    download_children(client, &file.id, dest, skipped, progress).map_err(Unfilled::Partial)?;
    progress.add_item();
    Ok(Staged::Copy)
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

/// Upload, then hand the local source to `trash_source`.
pub(crate) fn move_up_in(
    c: &Ctx,
    from: &Path,
    to: &str,
    overwrite: bool,
    trash_source: impl FnOnce(&Path) -> Result<(), String>,
) -> Result<String, String> {
    let uploaded = upload_in(c, from, to, overwrite)?;
    trash_source(from)
        .map_err(|error| format!("Moved, but could not move the original to the Trash: {error}"))?;
    Ok(uploaded)
}

/// Download, then move the Drive item to Drive's Trash. A partial folder
/// and a replaced local item go to `trash`.
pub(crate) fn move_down_in(
    c: &Ctx,
    from: &str,
    to: &Path,
    overwrite: bool,
    trash: TrashFn,
) -> Result<String, String> {
    // Checked first: a read-only connection must not leave a copy behind
    // and then fail to remove the original.
    let (location, account) = service::writable(c.state, from)?;
    let downloaded = download_into(c, from, to, overwrite, &discard_to(trash), trash)?;
    let id = location.file_id().unwrap_or_default().to_string();
    write::client(c, &account.id)?
        .update(&id, &FileChange::trash(true))
        .map_err(|error| {
            format!(
                "Moved, but could not move the original to the Trash: {}",
                message(error)
            )
        })?;
    Ok(downloaded)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cloud::test_server::Reply;
    use crate::cloud::write::tests::{ROOT, file, folder, found, heads, ok, run, run_with};
    use crate::progress::Progress;

    fn cancelled() -> Progress {
        let progress = Progress::default();
        progress.cancel();
        progress
    }

    #[test]
    fn cancelled_download_leaves_no_staging_file() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("a.txt");
        let (result, _) = run_with(
            vec![ok(sized("f1", "a.txt", 5)), bytes(b"hello")],
            &cancelled(),
            |c| download_in(c, &format!("{ROOT}/f1~a.txt"), &target, false, &no_trash),
        );
        assert_eq!(result.unwrap_err(), "Cancelled.");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn cancelled_upload_creates_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let source = local(&dir, "a.txt", b"hello");
        let (result, log) = run_with(vec![found(&[]), session()], &cancelled(), |c| {
            upload_in(c, &source, &format!("{ROOT}/~a.txt"), false)
        });
        assert_eq!(result.unwrap_err(), "Cancelled.");
        assert!(heads(&log).iter().all(|head| !head.starts_with("PUT ")));
        assert_eq!(std::fs::read(&source).unwrap(), b"hello");
    }

    #[test]
    fn measure_sums_drive_sizes_and_counts_docs() {
        let doc = format!(r#"{{"id":"g1","name":"Plan","mimeType":"{DOC}"}}"#);
        let (result, _) = run(
            vec![ok(folder("d1", "Docs")), found(&[sized("x1", "x.txt", 100), doc])],
            |c| measure_in(c, &[format!("{ROOT}/d1~Docs")]),
        );
        assert_eq!(
            result.unwrap(),
            crate::fs_ops::Measure {
                bytes: 100,
                items: 3
            }
        );
    }

    const DOC: &str = "application/vnd.google-apps.document";

    fn session() -> Reply {
        Reply::json(200, "").header("Location", "{base}/session/1")
    }

    fn bytes(body: &[u8]) -> Reply {
        Reply {
            status: 200,
            body: body.to_vec(),
            content_type: "application/octet-stream",
            headers: Vec::new(),
        }
    }

    fn sized(id: &str, name: &str, size: u64) -> String {
        format!(r#"{{"id":"{id}","name":"{name}","mimeType":"text/plain","size":"{size}"}}"#)
    }

    fn local(dir: &tempfile::TempDir, name: &str, body: &[u8]) -> PathBuf {
        let path = dir.path().join(name);
        std::fs::write(&path, body).unwrap();
        path
    }

    #[test]
    fn dispatch_refuses_two_local_paths() {
        let (result, log) = run(vec![], |c| {
            dispatch_in(c, "/a", "/b", false, false, &no_trash)
        });
        assert!(result.is_err());
        assert!(heads(&log).is_empty());
    }

    #[test]
    fn dispatch_routes_each_direction() {
        let dir = tempfile::tempdir().unwrap();
        let source = local(&dir, "a.txt", b"hello");
        let src = source.to_string_lossy().into_owned();
        // local → Drive copy: an upload
        let (result, _) = run(
            vec![found(&[]), session(), ok(sized("u1", "a.txt", 5))],
            |c| dispatch_in(c, &src, &format!("{ROOT}/~a.txt"), false, false, &no_trash),
        );
        assert_eq!(result.unwrap(), format!("{ROOT}/u1~a.txt"));
        // Drive → Drive move: one PATCH after the checks
        let (result, log) = run(
            vec![ok(file("f1", "a.txt")), found(&[]), ok(file("f1", "a.txt"))],
            |c| {
                dispatch_in(
                    c,
                    &format!("{ROOT}/f1~a.txt"),
                    &format!("{ROOT}/p2~Other/~a.txt"),
                    false,
                    true,
                    &no_trash,
                )
            },
        );
        assert_eq!(result.unwrap(), format!("{ROOT}/p2~Other/f1~a.txt"));
        assert!(heads(&log)[2].starts_with("PATCH /files/f1?"));
        // Drive → local copy: a download
        let target = dir.path().join("b.txt");
        let (result, _) = run(vec![ok(sized("f1", "a.txt", 5)), bytes(b"hello")], |c| {
            dispatch_in(
                c,
                &format!("{ROOT}/f1~a.txt"),
                &target.to_string_lossy(),
                false,
                false,
                &no_trash,
            )
        });
        assert_eq!(result.unwrap(), target.to_string_lossy());
    }

    #[test]
    fn uploading_a_file_returns_its_drive_uri() {
        let dir = tempfile::tempdir().unwrap();
        let source = local(&dir, "a.txt", b"hello");
        let (result, _) = run(
            vec![found(&[]), session(), ok(sized("u1", "a.txt", 5))],
            |c| upload_in(c, &source, &format!("{ROOT}/~a.txt"), false),
        );
        assert_eq!(result.unwrap(), format!("{ROOT}/u1~a.txt"));
    }

    #[test]
    fn a_short_upload_is_trashed_and_reported() {
        let dir = tempfile::tempdir().unwrap();
        let source = local(&dir, "a.txt", b"hello");
        let (result, log) = run(
            vec![
                found(&[]),
                session(),
                ok(sized("u1", "a.txt", 3)),
                ok(file("u1", "a.txt")),
            ],
            |c| upload_in(c, &source, &format!("{ROOT}/~a.txt"), false),
        );
        assert_eq!(
            result.unwrap_err(),
            "The upload to Google Drive was incomplete."
        );
        let log = log.lock().unwrap();
        assert!(log[3].head.starts_with("PATCH /files/u1?"));
        assert_eq!(log[3].body_text(), r#"{"trashed":true}"#);
    }

    #[test]
    fn uploading_a_folder_is_staged_then_renamed() {
        let dir = tempfile::tempdir().unwrap();
        let tree = dir.path().join("Docs");
        std::fs::create_dir(&tree).unwrap();
        std::fs::write(tree.join("x.txt"), b"hi").unwrap();
        let (result, log) = run(
            vec![
                found(&[]),
                ok(folder("s1", ".Docs.filebrowser-upload")),
                session(),
                ok(sized("x1", "x.txt", 2)),
                ok(folder("s1", "Docs")),
            ],
            |c| upload_in(c, &tree, &format!("{ROOT}/~Docs"), false),
        );
        assert_eq!(result.unwrap(), format!("{ROOT}/s1~Docs"));
        let log = log.lock().unwrap();
        assert!(log[1].body_text().contains(".Docs.filebrowser-upload"));
        assert!(log[2].body_text().contains(r#""parents":["s1"]"#));
        assert_eq!(log[4].body_text(), r#"{"name":"Docs"}"#);
    }

    #[test]
    fn a_failed_upload_trashes_the_partial_folder() {
        let dir = tempfile::tempdir().unwrap();
        let tree = dir.path().join("Docs");
        std::fs::create_dir(&tree).unwrap();
        std::fs::write(tree.join("x.txt"), b"hi").unwrap();
        let (result, log) = run(
            vec![
                found(&[]),
                ok(folder("s1", ".Docs.filebrowser-upload")),
                Reply::json(403, r#"{"error":{"message":"quota"}}"#),
                ok(folder("s1", ".Docs.filebrowser-upload")),
            ],
            |c| upload_in(c, &tree, &format!("{ROOT}/~Docs"), false),
        );
        assert!(result.unwrap_err().contains("quota"));
        let log = log.lock().unwrap();
        assert!(log[3].head.starts_with("PATCH /files/s1?"));
        assert_eq!(log[3].body_text(), r#"{"trashed":true}"#);
    }

    #[test]
    fn downloading_a_file_stages_and_verifies_its_size() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("a.txt");
        let (result, _) = run(vec![ok(sized("f1", "a.txt", 5)), bytes(b"hello")], |c| {
            download_in(c, &format!("{ROOT}/f1~a.txt"), &target, false, &no_trash)
        });
        assert_eq!(result.unwrap(), target.to_string_lossy());
        assert_eq!(std::fs::read(&target).unwrap(), b"hello");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn a_short_download_leaves_no_file() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("a.txt");
        let (result, _) = run(vec![ok(sized("f1", "a.txt", 5)), bytes(b"hel")], |c| {
            download_in(c, &format!("{ROOT}/f1~a.txt"), &target, false, &no_trash)
        });
        assert_eq!(
            result.unwrap_err(),
            "The download from Google Drive was incomplete."
        );
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    /// A folder downloads straight under its name so it fills as you watch;
    /// a failed one is handed to `discard` (the Trash), never left looking whole.
    #[test]
    fn a_failed_folder_download_is_filled_in_place_then_discarded() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("Docs");
        let discarded = std::cell::RefCell::new(Vec::new());
        let discard = |path: &Path| {
            discarded.borrow_mut().push(path.to_path_buf());
            Ok(())
        };
        let (result, _) = run(
            vec![
                ok(folder("d1", "Docs")),
                found(&[sized("x1", "x.txt", 2), sized("y1", "y.txt", 5)]),
                bytes(b"hi"),
                bytes(b"yy"),
            ],
            |c| download_into(c, &format!("{ROOT}/d1~Docs"), &target, false, &discard, &no_trash),
        );
        assert_eq!(
            result.unwrap_err(),
            "The download from Google Drive was incomplete."
        );
        assert_eq!(*discarded.borrow(), vec![target.clone()]);
        assert_eq!(std::fs::read(target.join("x.txt")).unwrap(), b"hi");
    }

    /// Replace still stages: the old folder stays until the new one is whole.
    #[test]
    fn a_failed_replacing_folder_download_keeps_the_old_folder() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("Docs");
        std::fs::create_dir(&target).unwrap();
        std::fs::write(target.join("old.txt"), b"old").unwrap();
        let discard = |_: &Path| -> Result<(), String> { panic!("nothing to discard") };
        let (result, _) = run(
            vec![
                ok(folder("d1", "Docs")),
                found(&[sized("y1", "y.txt", 5)]),
                bytes(b"yy"),
            ],
            |c| download_into(c, &format!("{ROOT}/d1~Docs"), &target, true, &discard, &no_trash),
        );
        assert!(result.is_err());
        assert_eq!(std::fs::read(target.join("old.txt")).unwrap(), b"old");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn downloading_a_folder_copies_the_tree() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("Docs");
        let (result, _) = run(
            vec![
                ok(folder("d1", "Docs")),
                found(&[sized("x1", "x.txt", 2), folder("s", "Sub")]),
                bytes(b"hi"),
                found(&[sized("y1", "y.txt", 1)]),
                bytes(b"y"),
            ],
            |c| download_in(c, &format!("{ROOT}/d1~Docs"), &target, false, &no_trash),
        );
        result.unwrap();
        assert_eq!(std::fs::read(target.join("x.txt")).unwrap(), b"hi");
        assert_eq!(std::fs::read(target.join("Sub/y.txt")).unwrap(), b"y");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn a_google_doc_cannot_be_downloaded() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("Plan");
        let doc = format!(r#"{{"id":"g1","name":"Plan","mimeType":"{DOC}"}}"#);
        let (result, _) = run(vec![ok(doc)], |c| {
            download_in(c, &format!("{ROOT}/g1~Plan"), &target, false, &no_trash)
        });
        assert_eq!(
            result.unwrap_err(),
            "“Plan” is a Google Docs file and can only be copied within Google Drive."
        );
        assert!(!target.exists());
    }

    #[test]
    fn a_folder_with_a_google_doc_reports_it() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("Docs");
        let doc = format!(r#"{{"id":"g1","name":"Plan","mimeType":"{DOC}"}}"#);
        let (result, _) = run(
            vec![
                ok(folder("d1", "Docs")),
                found(&[doc, sized("x1", "x.txt", 2)]),
                bytes(b"hi"),
            ],
            |c| download_in(c, &format!("{ROOT}/d1~Docs"), &target, false, &no_trash),
        );
        assert!(result.unwrap_err().contains("“Plan” is a Google Docs file"));
        assert_eq!(std::fs::read(target.join("x.txt")).unwrap(), b"hi");
    }

    #[test]
    fn a_folder_with_a_shortcut_skips_it_and_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("Docs");
        let shortcut =
            r#"{"id":"k1","name":"Link","mimeType":"application/vnd.google-apps.shortcut"}"#;
        let (result, log) = run(
            vec![
                ok(folder("d1", "Docs")),
                found(&[shortcut.to_string(), sized("x1", "x.txt", 2)]),
                bytes(b"hi"),
            ],
            |c| download_in(c, &format!("{ROOT}/d1~Docs"), &target, false, &no_trash),
        );
        assert_eq!(
            result.unwrap_err(),
            "“Link” is a Google Drive shortcut and can only be copied within Google Drive."
        );
        assert_eq!(std::fs::read(target.join("x.txt")).unwrap(), b"hi");
        assert!(heads(&log).iter().all(|h| !h.contains("/files/k1")));
    }

    #[test]
    fn downloaded_names_are_made_safe() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("Docs");
        let (result, _) = run(
            vec![
                ok(folder("d1", "Docs")),
                found(&[sized("x1", "a/b", 1), sized("x2", "..", 1)]),
                bytes(b"1"),
                bytes(b"2"),
            ],
            |c| download_in(c, &format!("{ROOT}/d1~Docs"), &target, false, &no_trash),
        );
        result.unwrap();
        let mut names: Vec<String> = std::fs::read_dir(&target)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names, vec!["a_b", "item"]);
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn duplicate_drive_names_do_not_overwrite_each_other() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("Docs");
        let (result, _) = run(
            vec![
                ok(folder("d1", "Docs")),
                found(&[sized("x1", "a.txt", 1), sized("x2", "a.txt", 1)]),
                bytes(b"1"),
                bytes(b"2"),
            ],
            |c| download_in(c, &format!("{ROOT}/d1~Docs"), &target, false, &no_trash),
        );
        result.unwrap();
        assert_eq!(std::fs::read(target.join("a.txt")).unwrap(), b"1");
        assert_eq!(std::fs::read(target.join("a (2).txt")).unwrap(), b"2");
    }

    #[test]
    fn a_download_target_that_is_not_a_plain_name_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let inner = dir.path().join("Downloads");
        std::fs::create_dir(&inner).unwrap();
        std::fs::write(inner.join("keep.txt"), b"mine").unwrap();
        for target in [inner.join(".."), inner.join("."), inner.join("x/..")] {
            // A 404 stops an unguarded download before it touches the disk,
            // so this test stays harmless while it fails.
            // Not joined: when the guard holds, the reply is never asked for.
            let (base, log, _server) =
                crate::cloud::test_server::serve_replies(vec![Reply::json(404, "{}")]);
            let (state, _dir) = crate::cloud::write::tests::state();
            let result = download_in(
                &crate::cloud::write::tests::ctx(&state, &base),
                &format!("{ROOT}/f1~a.txt"),
                &target,
                true,
                &no_trash,
            );
            assert!(result.is_err(), "{}", target.display());
            assert!(heads(&log).is_empty(), "{} reached Drive", target.display());
        }
        assert_eq!(std::fs::read(inner.join("keep.txt")).unwrap(), b"mine");
    }

    #[test]
    fn an_existing_local_target_without_overwrite_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let target = local(&dir, "a.txt", b"mine");
        let (result, log) = run(vec![], |c| {
            download_in(c, &format!("{ROOT}/f1~a.txt"), &target, false, &no_trash)
        });
        assert!(result.unwrap_err().starts_with("Target already exists"));
        assert_eq!(std::fs::read(&target).unwrap(), b"mine");
        assert!(heads(&log).is_empty());
    }

    fn names_in(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    fn no_discard(_: &Path) -> Result<(), String> {
        panic!("nothing to discard")
    }

    fn no_trash(_: &Path) -> Result<Option<PathBuf>, String> {
        panic!("nothing is replaced")
    }

    /// Replace downloads under a hidden name next to the old item, sends the
    /// old one to the Trash once the new one is whole, then renames it in.
    #[test]
    fn a_replacing_download_stages_then_trashes_then_renames_into_place() {
        for is_folder in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let bin = dir.path().join("bin");
            std::fs::create_dir(&bin).unwrap();
            let (target, replies, from) = if is_folder {
                let target = dir.path().join("Docs");
                std::fs::create_dir(&target).unwrap();
                std::fs::write(target.join("old.txt"), b"old").unwrap();
                let replies = vec![ok(folder("d1", "Docs")), found(&[sized("x1", "x.txt", 2)]), bytes(b"hi")];
                (target, replies, format!("{ROOT}/d1~Docs"))
            } else {
                let target = local(&dir, "a.txt", b"old");
                (target, vec![ok(sized("f1", "a.txt", 5)), bytes(b"hello")], format!("{ROOT}/f1~a.txt"))
            };
            let seen = std::cell::RefCell::new(Vec::new());
            let trash = |p: &Path| {
                seen.borrow_mut().push((p.to_path_buf(), names_in(dir.path())));
                std::fs::rename(p, bin.join("old")).map_err(|e| e.to_string())?;
                Ok(Some(bin.join("old")))
            };
            let (result, _) = run(replies, |c| {
                download_into(c, &from, &target, true, &no_discard, &trash)
            });
            assert_eq!(result.unwrap(), target.to_string_lossy());
            let seen = seen.into_inner();
            assert_eq!(seen.len(), 1);
            assert_eq!(seen[0].0, target);
            let name = target.file_name().unwrap().to_string_lossy().into_owned();
            let staged = format!(".{name}.filebrowser-{}-0", std::process::id());
            let mut want = vec![staged, name.clone(), "bin".to_string()];
            want.sort();
            assert_eq!(seen[0].1, want);
            let mut left = vec!["bin".to_string(), name];
            left.sort();
            assert_eq!(names_in(dir.path()), left);
            if is_folder {
                assert_eq!(std::fs::read(target.join("x.txt")).unwrap(), b"hi");
                assert_eq!(std::fs::read(bin.join("old/old.txt")).unwrap(), b"old");
            } else {
                assert_eq!(std::fs::read(&target).unwrap(), b"hello");
                assert_eq!(std::fs::read(bin.join("old")).unwrap(), b"old");
            }
        }
    }

    #[test]
    fn a_replacing_download_whose_trash_fails_keeps_the_old_item() {
        let dir = tempfile::tempdir().unwrap();
        let target = local(&dir, "a.txt", b"old");
        let refuse = |_: &Path| Err("no Trash on this volume".to_string());
        let (result, _) = run(vec![ok(sized("f1", "a.txt", 5)), bytes(b"hello")], |c| {
            download_into(c, &format!("{ROOT}/f1~a.txt"), &target, true, &no_discard, &refuse)
        });
        assert_eq!(
            result.unwrap_err(),
            "Could not move the existing item to the Trash: no Trash on this volume"
        );
        assert_eq!(std::fs::read(&target).unwrap(), b"old");
        assert_eq!(names_in(dir.path()), ["a.txt"]);
    }

    /// The local refusals' words, and that the name check comes first.
    #[test]
    fn download_refusals_keep_their_wording_and_order() {
        let dir = tempfile::tempdir().unwrap();
        let target = local(&dir, "a.txt", b"mine");
        let dotdot = dir.path().join("sub/..");
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        let cases = [
            (target.clone(), format!("Target already exists: {}", target.display())),
            // `sub/..` exists too, but isn't a name.
            (dotdot.clone(), format!("“{}” is not a valid name for a local item.", dotdot.display())),
        ];
        for (to, want) in cases {
            let (result, log) = run(vec![], |c| {
                download_in(c, &format!("{ROOT}/f1~a.txt"), &to, false, &no_trash)
            });
            assert_eq!(result.unwrap_err(), want);
            assert!(heads(&log).is_empty());
        }
        assert_eq!(std::fs::read(&target).unwrap(), b"mine");
    }

    #[test]
    fn a_vanished_drive_source_is_named() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("a.txt");
        let (result, _) = run(vec![Reply::json(404, "{}")], |c| {
            download_in(c, &format!("{ROOT}/f1~a.txt"), &target, false, &no_trash)
        });
        assert_eq!(result.unwrap_err(), "“a.txt” no longer exists");
        assert_eq!(names_in(dir.path()), Vec::<String>::new());
    }

    #[test]
    fn an_existing_drive_target_without_overwrite_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let source = local(&dir, "a.txt", b"hello");
        let (result, _) = run(vec![found(&[file("x", "a.txt")])], |c| {
            upload_in(c, &source, &format!("{ROOT}/~a.txt"), false)
        });
        assert_eq!(result.unwrap_err(), "Target already exists: a.txt");
    }

    #[cfg(unix)]
    #[test]
    fn a_folder_with_items_that_cannot_be_uploaded_keeps_its_source_on_a_move() {
        let dir = tempfile::tempdir().unwrap();
        let tree = dir.path().join("Docs");
        std::fs::create_dir(&tree).unwrap();
        std::fs::write(tree.join("x.txt"), b"hi").unwrap();
        std::os::unix::fs::symlink("/etc/hosts", tree.join("link")).unwrap();
        let mut trashed = Vec::new();
        let (result, _) = run(
            vec![
                found(&[]),
                ok(folder("s1", ".Docs.filebrowser-upload")),
                session(),
                ok(sized("x1", "x.txt", 2)),
                ok(folder("s1", "Docs")),
            ],
            |c| {
                move_up_in(c, &tree, &format!("{ROOT}/~Docs"), false, |p| {
                    trashed.push(p.to_path_buf());
                    Ok(())
                })
            },
        );
        let error = result.unwrap_err();
        assert!(error.contains("“link”"), "{error}");
        assert!(trashed.is_empty());
        assert!(tree.join("x.txt").exists());
    }

    #[test]
    fn a_failed_move_keeps_the_source() {
        let dir = tempfile::tempdir().unwrap();
        let source = local(&dir, "a.txt", b"hello");
        let mut trashed = Vec::new();
        let (result, _) = run(
            vec![
                found(&[]),
                Reply::json(403, r#"{"error":{"message":"quota"}}"#),
            ],
            |c| {
                move_up_in(c, &source, &format!("{ROOT}/~a.txt"), false, |p| {
                    trashed.push(p.to_path_buf());
                    Ok(())
                })
            },
        );
        assert!(result.is_err());
        assert!(trashed.is_empty());
        assert_eq!(std::fs::read(&source).unwrap(), b"hello");
    }

    #[test]
    fn a_local_to_drive_move_trashes_the_source_after_the_upload() {
        let dir = tempfile::tempdir().unwrap();
        let source = local(&dir, "a.txt", b"hello");
        let mut trashed = Vec::new();
        let (result, _) = run(
            vec![found(&[]), session(), ok(sized("u1", "a.txt", 5))],
            |c| {
                move_up_in(c, &source, &format!("{ROOT}/~a.txt"), false, |p| {
                    trashed.push(p.to_path_buf());
                    Ok(())
                })
            },
        );
        assert_eq!(result.unwrap(), format!("{ROOT}/u1~a.txt"));
        assert_eq!(trashed, vec![source.clone()]);
    }

    #[test]
    fn a_drive_to_local_move_trashes_the_drive_item_last() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("a.txt");
        let (result, log) = run(
            vec![
                ok(sized("f1", "a.txt", 5)),
                bytes(b"hello"),
                ok(file("f1", "a.txt")),
            ],
            |c| move_down_in(c, &format!("{ROOT}/f1~a.txt"), &target, false, &no_trash),
        );
        result.unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"hello");
        let log = log.lock().unwrap();
        let last = log.last().unwrap();
        assert!(last.head.starts_with("PATCH /files/f1?"));
        assert_eq!(last.body_text(), r#"{"trashed":true}"#);
    }
}
