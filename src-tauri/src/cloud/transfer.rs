//! Copying and moving between local folders and Google Drive (spec D17).
//!
//! Uploads: a file goes straight to its final name, since Drive creates it
//! only when the last chunk arrives, and is checked against the local size.
//! A folder (or a Replace) is built under a staging name and renamed once
//! complete; on failure the staged item goes to Drive's Trash.
//!
//! Downloads: everything lands in a hidden staging path next to the target,
//! is checked against Drive's sizes, then renamed into place. On failure the
//! staging path, the app's own temporary copy, is removed.
//!
//! A move removes its source only after the copy succeeded: a local source
//! goes to the Trash, a Drive source to Drive's Trash.

use std::path::{Path, PathBuf};

use crate::cloud::CloudState;
use crate::cloud::drive::{DriveClient, DriveError, DriveFile, FileChange};
use crate::cloud::service;
use crate::cloud::write::{self, Ctx, message};

const UPLOAD_SUFFIX: &str = ".filebrowser-upload";

pub fn upload(
    state: &CloudState,
    from: &Path,
    to: &str,
    overwrite: bool,
) -> Result<String, String> {
    write::live(state, |c| upload_in(c, from, to, overwrite))
}

pub fn download(
    state: &CloudState,
    from: &str,
    to: &Path,
    overwrite: bool,
) -> Result<String, String> {
    write::live(state, |c| download_in(c, from, to, overwrite))
}

/// Copy or move between any two locations where at least one is in Drive.
/// `trash_local` sends a local source to the Trash after a move.
pub fn dispatch(
    state: &CloudState,
    from: &str,
    to: &str,
    overwrite: bool,
    moving: bool,
    trash_local: impl FnOnce(&Path) -> Result<(), String>,
) -> Result<String, String> {
    write::live(state, |c| {
        dispatch_in(c, from, to, overwrite, moving, trash_local)
    })
}

pub(crate) fn dispatch_in(
    c: &Ctx,
    from: &str,
    to: &str,
    overwrite: bool,
    moving: bool,
    trash_local: impl FnOnce(&Path) -> Result<(), String>,
) -> Result<String, String> {
    let (from_cloud, to_cloud) = (crate::cloud::is_cloud(from), crate::cloud::is_cloud(to));
    match (from_cloud, to_cloud, moving) {
        (true, true, true) => write::move_in(c, from, to, overwrite),
        (true, true, false) => write::copy_in(c, from, to, overwrite),
        (false, true, true) => move_up_in(c, Path::new(from), to, overwrite, trash_local),
        (false, true, false) => upload_in(c, Path::new(from), to, overwrite),
        (true, false, true) => move_down_in(c, from, Path::new(to), overwrite),
        (true, false, false) => download_in(c, from, Path::new(to), overwrite),
        (false, false, _) => Err("Neither location is in Google Drive.".into()),
    }
}

pub(crate) fn upload_in(c: &Ctx, from: &Path, to: &str, overwrite: bool) -> Result<String, String> {
    let (_, account) = service::writable(c.state, to)?;
    let (parent, parent_id, name) = write::pending_target(to)?;
    let meta = std::fs::symlink_metadata(from)
        .map_err(|_| format!("“{}” no longer exists", from.display()))?;
    let client = write::client(c, &account.id)?;
    let existing = client.find_children(&parent_id, &name).map_err(message)?;
    let replaced = match existing.len() {
        0 => None,
        _ if !overwrite => return Err(format!("Target already exists: {name}")),
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
    let staged = if meta.is_dir() {
        let folder = client
            .create_folder(&parent_id, &staged_name)
            .map_err(message)?;
        if let Err(error) = upload_children(&client, from, &folder.id) {
            let _ = client.update(&folder.id, &FileChange::trash(true));
            return Err(error);
        }
        folder
    } else {
        upload_verified(&client, &parent_id, &staged_name, from)?
    };
    if staged_name == name {
        return Ok(parent.child(&staged.id, &staged.name).to_uri());
    }
    write::finish_staged(&client, &parent, &name, replaced.as_ref(), &staged)
}

/// Upload one file and check Drive kept every byte; a short file is trashed.
fn upload_verified(
    client: &DriveClient,
    parent_id: &str,
    name: &str,
    local: &Path,
) -> Result<DriveFile, String> {
    let expected = std::fs::metadata(local)
        .map_err(|error| error.to_string())?
        .len();
    let uploaded = client.upload(parent_id, name, local).map_err(message)?;
    if drive_size(&uploaded) != Some(expected) {
        let _ = client.update(&uploaded.id, &FileChange::trash(true));
        return Err("The upload to Google Drive was incomplete.".into());
    }
    Ok(uploaded)
}

/// Upload a folder's contents. Symlinks are skipped, as local copies do.
fn upload_children(client: &DriveClient, dir: &Path, parent_id: &str) -> Result<(), String> {
    let mut children: Vec<_> = std::fs::read_dir(dir)
        .map_err(|error| error.to_string())?
        .filter_map(Result::ok)
        .collect();
    children.sort_by_key(|entry| entry.file_name());
    for entry in children {
        let path = entry.path();
        let meta = std::fs::symlink_metadata(&path).map_err(|error| error.to_string())?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if meta.file_type().is_symlink() {
            continue;
        } else if meta.is_dir() {
            let folder = client.create_folder(parent_id, &name).map_err(message)?;
            upload_children(client, &path, &folder.id)?;
        } else {
            upload_verified(client, parent_id, &name, &path)?;
        }
    }
    Ok(())
}

fn drive_size(file: &DriveFile) -> Option<u64> {
    file.size.as_deref().and_then(|size| size.parse().ok())
}

fn google_doc_refusal(name: &str) -> String {
    format!("“{name}” is a Google Docs file and can only be copied within Google Drive.")
}

pub(crate) fn download_in(
    c: &Ctx,
    from: &str,
    to: &Path,
    overwrite: bool,
) -> Result<String, String> {
    let replacing = std::fs::symlink_metadata(to).is_ok();
    if replacing && !overwrite {
        return Err(format!("Target already exists: {}", to.display()));
    }
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
        Ok(_) | Err(DriveError::NotFound) => return Err(format!("“{shown}” no longer exists")),
        Err(error) => return Err(message(error)),
    };
    if !file.is_dir() && !file.can_download() {
        return Err(google_doc_refusal(&file.name));
    }

    let staging = crate::fs_ops::staging_path(to);
    let mut skipped = Vec::new();
    let result = if file.is_dir() {
        std::fs::create_dir(&staging)
            .map_err(|error| error.to_string())
            .and_then(|()| download_children(&client, &file.id, &staging, &mut skipped))
    } else {
        download_verified(&client, &file, &staging)
    };
    if let Err(error) = result {
        let _ = crate::fs_ops::remove_any(&staging);
        return Err(error);
    }
    if replacing {
        crate::fs_ops::replace_with_staged(&staging, to)?;
    } else if let Err(error) = std::fs::rename(&staging, to) {
        let _ = crate::fs_ops::remove_any(&staging);
        return Err(format!("Failed to copy: {error}"));
    }
    match skipped.first() {
        // Everything else was copied; the error names what was left out.
        Some(name) => Err(google_doc_refusal(name)),
        None => Ok(to.to_string_lossy().into_owned()),
    }
}

/// Download one file to `dest` and check its size against Drive's.
fn download_verified(client: &DriveClient, file: &DriveFile, dest: &Path) -> Result<(), String> {
    client.download(&file.id, dest).map_err(message)?;
    let written = std::fs::metadata(dest).map(|meta| meta.len()).ok();
    if drive_size(file).is_some_and(|size| Some(size) != written) {
        let _ = std::fs::remove_file(dest);
        return Err("The download from Google Drive was incomplete.".into());
    }
    Ok(())
}

/// Download a Drive folder's contents into `dir`. Google-native files are
/// collected in `skipped`; Drive names become safe, unique local names.
fn download_children(
    client: &DriveClient,
    folder_id: &str,
    dir: &Path,
    skipped: &mut Vec<String>,
) -> Result<(), String> {
    for child in client.list_all(folder_id).map_err(message)? {
        if !child.is_dir() && !child.can_download() {
            skipped.push(child.name.clone());
            continue;
        }
        let path = unique_child(dir, &service::safe_name(&child.name));
        if child.is_dir() {
            std::fs::create_dir(&path).map_err(|error| error.to_string())?;
            download_children(client, &child.id, &path, skipped)?;
        } else {
            download_verified(client, &child, &path)?;
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

/// Download, then move the Drive item to Drive's Trash.
pub(crate) fn move_down_in(
    c: &Ctx,
    from: &str,
    to: &Path,
    overwrite: bool,
) -> Result<String, String> {
    // Checked first: a read-only connection must not leave a copy behind
    // and then fail to remove the original.
    let (location, account) = service::writable(c.state, from)?;
    let downloaded = download_in(c, from, to, overwrite)?;
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
    use crate::cloud::write::tests::{ROOT, file, folder, found, heads, ok, run};

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
            dispatch_in(c, "/a", "/b", false, false, |_| Ok(()))
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
            |c| dispatch_in(c, &src, &format!("{ROOT}/~a.txt"), false, false, |_| Ok(())),
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
                    |_| Ok(()),
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
                |_| Ok(()),
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
            download_in(c, &format!("{ROOT}/f1~a.txt"), &target, false)
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
            download_in(c, &format!("{ROOT}/f1~a.txt"), &target, false)
        });
        assert_eq!(
            result.unwrap_err(),
            "The download from Google Drive was incomplete."
        );
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
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
            |c| download_in(c, &format!("{ROOT}/d1~Docs"), &target, false),
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
            download_in(c, &format!("{ROOT}/g1~Plan"), &target, false)
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
            |c| download_in(c, &format!("{ROOT}/d1~Docs"), &target, false),
        );
        assert!(result.unwrap_err().contains("“Plan” is a Google Docs file"));
        assert_eq!(std::fs::read(target.join("x.txt")).unwrap(), b"hi");
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
            |c| download_in(c, &format!("{ROOT}/d1~Docs"), &target, false),
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
            |c| download_in(c, &format!("{ROOT}/d1~Docs"), &target, false),
        );
        result.unwrap();
        assert_eq!(std::fs::read(target.join("a.txt")).unwrap(), b"1");
        assert_eq!(std::fs::read(target.join("a (2).txt")).unwrap(), b"2");
    }

    #[test]
    fn an_existing_local_target_without_overwrite_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let target = local(&dir, "a.txt", b"mine");
        let (result, log) = run(vec![], |c| {
            download_in(c, &format!("{ROOT}/f1~a.txt"), &target, false)
        });
        assert!(result.unwrap_err().starts_with("Target already exists"));
        assert_eq!(std::fs::read(&target).unwrap(), b"mine");
        assert!(heads(&log).is_empty());
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
            |c| move_down_in(c, &format!("{ROOT}/f1~a.txt"), &target, false),
        );
        result.unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"hello");
        let log = log.lock().unwrap();
        let last = log.last().unwrap();
        assert!(last.head.starts_with("PATCH /files/f1?"));
        assert_eq!(last.body_text(), r#"{"trashed":true}"#);
    }
}
