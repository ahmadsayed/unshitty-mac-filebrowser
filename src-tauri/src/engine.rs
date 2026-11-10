//! The transfer engine's routes to, from and inside Google Drive (design
//! D6, spec D17): copy and move built from Drive's `places::Transfer`
//! primitives, with `placement.rs` placing what lands on this computer.
//!
//! The engine decides what Drive's primitives don't: the refusals' order
//! (the place's own, then this computer's, then what is at the target),
//! when a new item is staged under a hidden name, and Replace's order —
//! stage, send the old item to the Trash, rename the new one in, and take
//! the old one back out if that last rename fails. A staged copy that
//! failed is discarded (Drive's Trash); a staged original (the user's own
//! item, during a move that replaces) is only ever moved back.
//!
//! Per route, as before:
//! - local → Drive: a file goes straight to its final name (Drive creates
//!   it only once its upload is whole); a folder, or a Replace, is uploaded
//!   under `.<name>.filebrowser-upload` and renamed in. A move sends the
//!   local source to the Trash only after the whole upload.
//! - Drive → local: `placement` stages a file (and a Replace) and fills a
//!   new folder in place, a partial one going to the logged local Trash;
//!   the item a Replace replaces goes to the logged Trash and comes back if
//!   the Replace fails. A move sends the Drive source to Drive's Trash only
//!   after the whole download, and is refused first if it couldn't.
//! - Drive → Drive (one account): a server-side copy (a folder, or a
//!   Replace, under `.<name>.filebrowser-copy`), or one move (under
//!   `.<name>.filebrowser-move` when it replaces).
//!
//! Skipped items (a Google Doc in a downloaded folder, a link in an
//! uploaded one) make the transfer an error once the rest is in place, so a
//! move keeps its source.

use std::cell::RefCell;
use std::path::Path;

use places::{Cx, Error, Landing, Transfer, Unread};

use crate::cloud::is_cloud;
use crate::placement::{self, DiscardFn, Refusal, Stage, Staged, TrashFn, Unfilled};

/// The staging names of each route, next to the target.
fn upload_staging(name: &str) -> String {
    format!(".{name}.filebrowser-upload")
}

fn copy_staging(name: &str) -> String {
    format!(".{name}.filebrowser-copy")
}

fn move_staging(name: &str) -> String {
    format!(".{name}.filebrowser-move")
}

/// Copy or move between any two locations where at least one is in Drive,
/// through `drive`'s primitives. `trash_local` is the local Trash: for a
/// local source after a move, a partial folder a download leaves and the
/// local item a download replaces.
pub fn dispatch(
    drive: &dyn Transfer,
    cx: &Cx,
    from: &str,
    to: &str,
    overwrite: bool,
    moving: bool,
    trash_local: TrashFn,
) -> Result<String, String> {
    match (is_cloud(from), is_cloud(to), moving) {
        (true, true, true) => move_within(drive, cx, from, to, overwrite),
        (true, true, false) => copy_within(drive, cx, from, to, overwrite),
        (false, true, true) => move_up(drive, cx, Path::new(from), to, overwrite, discard_to(trash_local)),
        (false, true, false) => upload(drive, cx, Path::new(from), to, overwrite),
        (true, false, true) => move_down(drive, cx, from, Path::new(to), overwrite, trash_local),
        (true, false, false) => {
            download(drive, cx, from, Path::new(to), overwrite, &discard_to(trash_local), trash_local)
        }
        (false, false, _) => Err("Neither location is in Google Drive.".into()),
    }
}

/// `trash` for an item that needn't come back: a moved source, a partial
/// folder.
fn discard_to<'a>(trash: TrashFn<'a>) -> impl Fn(&Path) -> Result<(), String> + 'a {
    move |path| trash.send(path).map(|_| ())
}

// ---- What is at the target ---------------------------------------------------------

/// The item a transfer replaces, if any: a taken name is refused without
/// `overwrite`, and where names aren't unique, so is an ambiguous one;
/// then a folder can't be replaced with an item inside it.
fn replaced(landing: &Landing, overwrite: bool) -> Result<Option<String>, String> {
    let name = &landing.name;
    let old = match landing.existing.as_slice() {
        [] => return Ok(None),
        _ if !overwrite => return Err(Refusal::Exists(name.clone()).into()),
        [old] => old,
        _ => {
            return Err(format!(
                "Several items in this folder are named “{name}”. Rename or remove the extra ones in Google Drive first."
            ));
        }
    };
    if old.holds_source {
        return Err(Refusal::ReplaceWithChild.into());
    }
    Ok(Some(old.path.clone()))
}

/// The transfer's path, or the first item it skipped.
fn finished(path: String, skipped: &[String]) -> Result<String, String> {
    match skipped.first() {
        Some(reason) => Err(reason.clone()),
        None => Ok(path),
    }
}

/// Whose item is at a staging name in Drive, which decides what a failure
/// does with it.
#[derive(Clone, Copy)]
enum Placing<'a> {
    /// A copy the engine made: discarded (Drive's Trash).
    Copy,
    /// The user's own item, moved there: moved back to this pending path.
    Original(&'a str),
}

/// Swap a complete staged item into place in Drive: trash what it
/// replaces, then give it the final name. If the rename fails, the replaced
/// item comes back out of the Trash. A staged copy is discarded on failure;
/// a staged original is moved back where it was.
fn swap_in(
    t: &dyn Transfer,
    cx: &Cx,
    landing: &Landing,
    staged: &str,
    replaced: Option<&str>,
    placing: Placing,
) -> Result<String, String> {
    let give_back = || match placing {
        Placing::Copy => {
            let _ = t.discard(staged, cx);
        }
        Placing::Original(home) => {
            let _ = t.move_to(staged, home, cx);
        }
    };
    let target = match t.pending(&landing.parent, &landing.name) {
        Ok(target) => target,
        Err(error) => {
            give_back();
            return Err(error.into());
        }
    };
    let mut trashed = None;
    if let Some(old) = replaced {
        match t.trash_for_replace(old, cx) {
            Ok(path) => trashed = Some((path, old)),
            Err(error) => {
                give_back();
                return Err(format!("Could not move the existing item to the Trash: {error}"));
            }
        }
    }
    t.rename_new(staged, &target, cx).map_err(|error| {
        if let Some((path, old)) = &trashed {
            let _ = t.take_back(path, old, cx);
        }
        give_back();
        error.into()
    })
}

// ---- Inside Drive --------------------------------------------------------------------

/// A server-side copy; a folder, or a Replace, is built under a staging
/// name and renamed in once complete.
pub fn copy_within(t: &dyn Transfer, cx: &Cx, from: &str, to: &str, overwrite: bool) -> Result<String, String> {
    t.check(Some(from), Some(to), cx)?;
    let landing = t.landing(Some(from), to, cx)?;
    let replaced = replaced(&landing, overwrite)?;
    let source = landing.source.as_ref().ok_or("The source is not in Google Drive.")?;
    let staged_name = if source.is_dir || replaced.is_some() {
        copy_staging(&landing.name)
    } else {
        landing.name.clone()
    };
    let put = t.copy_within(&source.path, &t.pending(&landing.parent, &staged_name)?, cx)?;
    if staged_name == landing.name {
        return Ok(put.path);
    }
    swap_in(t, cx, &landing, &put.path, replaced.as_deref(), Placing::Copy)
}

/// One move; with Replace, the item moves in under a staging name first
/// (stage, trash the old item, then rename into place), and goes back where
/// and as it was if that fails.
pub fn move_within(t: &dyn Transfer, cx: &Cx, from: &str, to: &str, overwrite: bool) -> Result<String, String> {
    t.check(Some(from), Some(to), cx)?;
    let landing = t.landing(Some(from), to, cx)?;
    let replaced = replaced(&landing, overwrite)?;
    let source = landing.source.as_ref().ok_or("The source is not in Google Drive.")?;
    let staged_name = match replaced {
        Some(_) => move_staging(&landing.name),
        None => landing.name.clone(),
    };
    let moved = t.move_to(&source.path, &t.pending(&landing.parent, &staged_name)?, cx)?;
    let Some(old) = replaced else {
        return Ok(moved);
    };
    // A source at the top of My Drive was refused by the move itself.
    let home = source.home.as_deref().unwrap_or(&source.path);
    swap_in(t, cx, &landing, &moved, Some(&old), Placing::Original(home))
}

// ---- This computer to Drive ----------------------------------------------------------

/// Upload `from` to the pending `to`.
pub fn upload(t: &dyn Transfer, cx: &Cx, from: &Path, to: &str, overwrite: bool) -> Result<String, String> {
    t.check(None, Some(to), cx)?;
    let meta = std::fs::symlink_metadata(from)
        .map_err(|_| String::from(Refusal::Vanished(from.display().to_string())))?;
    let landing = t.landing(None, to, cx)?;
    let replaced = replaced(&landing, overwrite)?;
    let staged_name = if meta.is_dir() || replaced.is_some() {
        upload_staging(&landing.name)
    } else {
        landing.name.clone()
    };
    let put = t.write(from, &t.pending(&landing.parent, &staged_name)?, false, cx)?;
    let uploaded = if staged_name == landing.name {
        put.path
    } else {
        swap_in(t, cx, &landing, &put.path, replaced.as_deref(), Placing::Copy)?
    };
    finished(uploaded, &put.skipped)
}

/// Upload, then hand the local source to `trash_source`.
pub fn move_up(
    t: &dyn Transfer,
    cx: &Cx,
    from: &Path,
    to: &str,
    overwrite: bool,
    trash_source: impl FnOnce(&Path) -> Result<(), String>,
) -> Result<String, String> {
    let uploaded = upload(t, cx, from, to, overwrite)?;
    trash_source(from)
        .map_err(|error| format!("Moved, but could not move the original to the Trash: {error}"))?;
    Ok(uploaded)
}

// ---- Drive to this computer ----------------------------------------------------------

/// What `placement` makes of a failed read.
fn unfilled(unread: Unread) -> Unfilled {
    match unread {
        Unread::Nothing(error) => Unfilled::Nothing(error.to_string()),
        Unread::Partial(error) => Unfilled::Partial(error.to_string()),
    }
}

/// Download the Drive item `from` to `to`. A new folder downloads straight
/// under its name, so it fills as you watch; if that fails, the partial
/// folder goes to `discard`. Files and replacements stage under a hidden
/// name and appear only when complete; the item a Replace sends away goes
/// to `trash` (the app's logged Trash, so Restore knows where it was),
/// which takes it back if the Replace fails.
pub fn download(
    t: &dyn Transfer,
    cx: &Cx,
    from: &str,
    to: &Path,
    overwrite: bool,
    discard: DiscardFn,
    trash: TrashFn,
) -> Result<String, String> {
    let target = placement::check_new(to, overwrite)?;
    let skipped = RefCell::new(Vec::new());
    t.read(
        from,
        &mut |item| {
            let fill = |dest: &Path| {
                let read = item.fill(dest, cx).map_err(unfilled)?;
                *skipped.borrow_mut() = read;
                Ok(Staged::Copy)
            };
            let stage = Stage::by_kind(item.is_dir(), discard);
            target.place(stage, fill, trash).map(|_| ()).map_err(Error::Other)
        },
        cx,
    )?;
    finished(to.to_string_lossy().into_owned(), &skipped.into_inner())
}

/// Download, then move the Drive item to Drive's Trash. A partial folder
/// and a replaced local item go to `trash`.
pub fn move_down(
    t: &dyn Transfer,
    cx: &Cx,
    from: &str,
    to: &Path,
    overwrite: bool,
    trash: TrashFn,
) -> Result<String, String> {
    // Checked first: a read-only connection must not leave a copy behind
    // and then fail to remove the original.
    t.check(Some(from), None, cx)?;
    let downloaded = download(t, cx, from, to, overwrite, &discard_to(trash), trash)?;
    t.trash_for_replace(from, cx).map_err(|error| match error {
        // No request was sent: the account's own words.
        Error::AuthRequired(words) => words,
        error => format!("Moved, but could not move the original to the Trash: {error}"),
    })?;
    Ok(downloaded)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    use crate::cloud::test_server::Reply;
    use crate::cloud::write::Ctx;
    use crate::cloud::write::tests::{ROOT, file, folder, found, heads, ok, run, run_with};
    use crate::progress::Progress;

    // ---- Drive's own tests, through the engine -------------------------------
    //
    // Each runs the engine on Drive's real primitives against a fake server
    // and asserts the requests Drive is sent, as before the engine took
    // these routes over. The names they call are the routes' old names.

    fn cx_of(c: &Ctx) -> Cx {
        Cx { progress: c.progress.clone(), ..Cx::default() }
    }

    fn upload_in(c: &Ctx, from: &Path, to: &str, overwrite: bool) -> Result<String, String> {
        upload(c, &cx_of(c), from, to, overwrite)
    }

    /// `download_into` that removes a partial folder instead of trashing it.
    fn download_in(c: &Ctx, from: &str, to: &Path, overwrite: bool, trash: TrashFn) -> Result<String, String> {
        download(c, &cx_of(c), from, to, overwrite, &placement::remove, trash)
    }

    fn download_into(
        c: &Ctx,
        from: &str,
        to: &Path,
        overwrite: bool,
        discard: DiscardFn,
        trash: TrashFn,
    ) -> Result<String, String> {
        download(c, &cx_of(c), from, to, overwrite, discard, trash)
    }

    fn dispatch_in(
        c: &Ctx,
        from: &str,
        to: &str,
        overwrite: bool,
        moving: bool,
        trash_local: TrashFn,
    ) -> Result<String, String> {
        dispatch(c, &cx_of(c), from, to, overwrite, moving, trash_local)
    }

    fn move_up_in(
        c: &Ctx,
        from: &Path,
        to: &str,
        overwrite: bool,
        trash_source: impl FnOnce(&Path) -> Result<(), String>,
    ) -> Result<String, String> {
        move_up(c, &cx_of(c), from, to, overwrite, trash_source)
    }

    fn move_down_in(c: &Ctx, from: &str, to: &Path, overwrite: bool, trash: TrashFn) -> Result<String, String> {
        move_down(c, &cx_of(c), from, to, overwrite, trash)
    }

    fn copy_in(c: &Ctx, from: &str, to: &str, overwrite: bool) -> Result<String, String> {
        copy_within(c, &cx_of(c), from, to, overwrite)
    }

    fn move_in(c: &Ctx, from: &str, to: &str, overwrite: bool) -> Result<String, String> {
        move_within(c, &cx_of(c), from, to, overwrite)
    }


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

    #[test]
    fn drive_copy_counts_items() {
        let progress = Progress::default();
        let (result, _) = run_with(
            vec![
                ok(folder("d1", "Docs")),
                found(&[]),
                ok(folder("s1", ".Docs.filebrowser-copy")),
                found(&[file("c", "x.txt")]),
                ok(file("c2", "x.txt")),
                ok(folder("s1", "Docs")),
            ],
            &progress,
            |c| {
                copy_in(
                    c,
                    &format!("{ROOT}/d1~Docs"),
                    &format!("{ROOT}/p2~Other/~Docs"),
                    false,
                )
            },
        );
        result.unwrap();
        assert_eq!(progress.snapshot().items, 2);
    }

    #[test]
    fn copy_of_a_file_is_server_side_and_gets_a_new_id() {
        let (result, log) = run(
            vec![ok(file("f1", "a.txt")), found(&[]), ok(file("c1", "a.txt"))],
            |c| {
                copy_in(
                    c,
                    &format!("{ROOT}/f1~a.txt"),
                    &format!("{ROOT}/p2~Other/~a.txt"),
                    false,
                )
            },
        );
        assert_eq!(result.unwrap(), format!("{ROOT}/p2~Other/c1~a.txt"));
        assert!(heads(&log)[2].starts_with("POST /files/f1/copy?"));
    }

    #[test]
    fn copy_of_a_folder_is_staged_then_renamed() {
        let (result, log) = run(
            vec![
                ok(folder("d1", "Docs")),
                found(&[]),
                ok(folder("s1", ".Docs.filebrowser-copy")),
                found(&[file("c", "x.txt")]),
                ok(file("c2", "x.txt")),
                ok(folder("s1", "Docs")),
            ],
            |c| {
                copy_in(
                    c,
                    &format!("{ROOT}/d1~Docs"),
                    &format!("{ROOT}/p2~Other/~Docs"),
                    false,
                )
            },
        );
        assert_eq!(result.unwrap(), format!("{ROOT}/p2~Other/s1~Docs"));
        let log = log.lock().unwrap();
        assert!(
            log[2]
                .body_text()
                .contains(r#""name":".Docs.filebrowser-copy""#)
        );
        assert!(log[4].head.starts_with("POST /files/c/copy?"));
        assert!(log[4].body_text().contains(r#""parents":["s1"]"#));
        assert!(log[5].head.starts_with("PATCH /files/s1?"));
        assert_eq!(log[5].body_text(), r#"{"name":"Docs"}"#);
    }

    #[test]
    fn copying_a_folder_into_its_own_subfolder_does_not_copy_the_copy() {
        // The target "sub" really is inside d1, but a pasted Drive link
        // gives a one-segment path, so only the tree walk can notice.
        let (result, log) = run(
            vec![
                ok(folder("d1", "Docs")),
                found(&[]),
                ok(folder("s1", ".Docs.filebrowser-copy")),
                found(&[folder("sub", "Sub")]),
                ok(folder("n1", "Sub")),
                found(&[folder("s1", ".Docs.filebrowser-copy")]),
                ok(folder("s1", "Docs")),
            ],
            |c| {
                copy_in(
                    c,
                    &format!("{ROOT}/d1~Docs"),
                    "gdrive://acct/sub~Sub/~Docs",
                    false,
                )
            },
        );
        assert_eq!(result.unwrap(), "gdrive://acct/sub~Sub/s1~Docs");
        let log = log.lock().unwrap();
        assert_eq!(log.len(), 7);
        assert_eq!(log[6].body_text(), r#"{"name":"Docs"}"#);
    }

    #[test]
    fn a_failed_folder_copy_trashes_the_staged_folder() {
        let (result, log) = run(
            vec![
                ok(folder("d1", "Docs")),
                found(&[]),
                ok(folder("s1", ".Docs.filebrowser-copy")),
                found(&[file("c", "x.txt")]),
                Reply::json(403, r#"{"error":{"message":"nope"}}"#),
                ok(folder("s1", ".Docs.filebrowser-copy")),
            ],
            |c| {
                copy_in(
                    c,
                    &format!("{ROOT}/d1~Docs"),
                    &format!("{ROOT}/p2~Other/~Docs"),
                    false,
                )
            },
        );
        assert!(result.unwrap_err().contains("nope"));
        let log = log.lock().unwrap();
        let last = log.last().unwrap();
        assert!(last.head.starts_with("PATCH /files/s1?"), "{}", last.head);
        assert_eq!(last.body_text(), r#"{"trashed":true}"#);
    }

    #[test]
    fn move_updates_parents_in_one_call() {
        let (result, log) = run(
            vec![ok(file("f1", "a.txt")), found(&[]), ok(file("f1", "a.txt"))],
            |c| {
                move_in(
                    c,
                    &format!("{ROOT}/f1~a.txt"),
                    &format!("{ROOT}/p2~Other/~a.txt"),
                    false,
                )
            },
        );
        assert_eq!(result.unwrap(), format!("{ROOT}/p2~Other/f1~a.txt"));
        let head = heads(&log)[2].clone();
        assert!(head.starts_with("PATCH /files/f1?"), "{head}");
        assert!(head.contains("addParents=p2") && head.contains("removeParents=root"));
    }

    #[test]
    fn moving_into_its_own_subfolder_is_refused() {
        let (result, log) = run(vec![], |c| {
            move_in(
                c,
                &format!("{ROOT}/d1~Docs"),
                &format!("{ROOT}/d1~Docs/s~Sub/~Docs"),
                false,
            )
        });
        assert_eq!(
            result.unwrap_err(),
            "Cannot copy or move a folder into itself"
        );
        assert!(heads(&log).is_empty());
    }

    #[test]
    fn the_same_path_is_refused() {
        let (result, log) = run(vec![], |c| {
            move_in(
                c,
                &format!("{ROOT}/f1~a.txt"),
                &format!("{ROOT}/~a.txt"),
                false,
            )
        });
        assert_eq!(
            result.unwrap_err(),
            "The source and destination are the same"
        );
        assert!(heads(&log).is_empty());
    }

    #[test]
    fn an_existing_target_without_overwrite_is_refused() {
        let (result, _) = run(
            vec![ok(file("f1", "a.txt")), found(&[file("x", "a.txt")])],
            |c| {
                copy_in(
                    c,
                    &format!("{ROOT}/f1~a.txt"),
                    &format!("{ROOT}/p2~Other/~a.txt"),
                    false,
                )
            },
        );
        assert_eq!(result.unwrap_err(), "Target already exists: a.txt");
    }

    #[test]
    fn replacing_a_folder_with_its_own_child_is_refused() {
        let (result, _) = run(
            vec![ok(file("f1", "Docs")), found(&[folder("d1", "Docs")])],
            |c| {
                move_in(
                    c,
                    &format!("{ROOT}/d1~Docs/f1~Docs"),
                    &format!("{ROOT}/~Docs"),
                    true,
                )
            },
        );
        assert_eq!(
            result.unwrap_err(),
            "Cannot replace a folder with an item inside it"
        );
    }

    #[test]
    fn replace_refuses_an_ambiguous_name() {
        let (result, _) = run(
            vec![
                ok(file("f1", "a.txt")),
                found(&[file("x", "a.txt"), file("y", "a.txt")]),
            ],
            |c| {
                copy_in(
                    c,
                    &format!("{ROOT}/f1~a.txt"),
                    &format!("{ROOT}/p2~Other/~a.txt"),
                    true,
                )
            },
        );
        assert_eq!(
            result.unwrap_err(),
            "Several items in this folder are named “a.txt”. Rename or remove the extra ones in Google Drive first."
        );
    }

    #[test]
    fn replace_trashes_the_old_item_and_never_deletes() {
        let (result, log) = run(
            vec![
                ok(file("f1", "a.txt")),
                found(&[file("old", "a.txt")]),
                ok(file("c1", ".a.txt.filebrowser-copy")),
                ok(file("old", "a.txt")),
                ok(file("c1", "a.txt")),
            ],
            |c| {
                copy_in(
                    c,
                    &format!("{ROOT}/f1~a.txt"),
                    &format!("{ROOT}/p2~Other/~a.txt"),
                    true,
                )
            },
        );
        assert_eq!(result.unwrap(), format!("{ROOT}/p2~Other/c1~a.txt"));
        let log = log.lock().unwrap();
        assert!(log.iter().all(|r| !r.head.starts_with("DELETE")));
        assert!(log[3].head.starts_with("PATCH /files/old?"));
        assert_eq!(log[3].body_text(), r#"{"trashed":true}"#);
        assert_eq!(log[4].body_text(), r#"{"name":"a.txt"}"#);
    }

    #[test]
    fn a_move_with_replace_is_staged_before_the_old_item_goes() {
        let (result, log) = run(
            vec![
                ok(file("f1", "a.txt")),
                found(&[file("old", "a.txt")]),
                ok(file("f1", ".a.txt.filebrowser-move")),
                ok(file("old", "a.txt")),
                ok(file("f1", "a.txt")),
            ],
            |c| {
                move_in(
                    c,
                    &format!("{ROOT}/f1~a.txt"),
                    &format!("{ROOT}/p2~Other/~a.txt"),
                    true,
                )
            },
        );
        assert_eq!(result.unwrap(), format!("{ROOT}/p2~Other/f1~a.txt"));
        let log = log.lock().unwrap();
        assert!(
            log[2].head.starts_with("PATCH /files/f1?") && log[2].head.contains("addParents=p2")
        );
        assert_eq!(log[2].body_text(), r#"{"name":".a.txt.filebrowser-move"}"#);
        assert!(log[3].head.starts_with("PATCH /files/old?"));
        assert_eq!(log[3].body_text(), r#"{"trashed":true}"#);
        assert_eq!(log[4].body_text(), r#"{"name":"a.txt"}"#);
    }

    #[test]
    fn a_failed_rename_after_replace_restores_the_old_item() {
        let (result, log) = run(
            vec![
                ok(file("f1", "a.txt")),
                found(&[file("old", "a.txt")]),
                ok(file("c1", ".a.txt.filebrowser-copy")),
                ok(file("old", "a.txt")),
                Reply::json(403, r#"{"error":{"message":"nope"}}"#),
                ok(file("old", "a.txt")),
                ok(file("c1", ".a.txt.filebrowser-copy")),
            ],
            |c| {
                copy_in(
                    c,
                    &format!("{ROOT}/f1~a.txt"),
                    &format!("{ROOT}/p2~Other/~a.txt"),
                    true,
                )
            },
        );
        assert!(result.unwrap_err().contains("nope"));
        let log = log.lock().unwrap();
        assert!(log[5].head.starts_with("PATCH /files/old?"));
        assert_eq!(log[5].body_text(), r#"{"trashed":false}"#);
        assert!(log[6].head.starts_with("PATCH /files/c1?"));
        assert_eq!(log[6].body_text(), r#"{"trashed":true}"#);
    }

    #[test]
    fn a_failed_move_with_replace_puts_everything_back() {
        let (result, log) = run(
            vec![
                ok(file("f1", "a.txt")),
                found(&[file("old", "a.txt")]),
                ok(file("f1", ".a.txt.filebrowser-move")),
                ok(file("old", "a.txt")),
                Reply::json(403, r#"{"error":{"message":"nope"}}"#),
                ok(file("old", "a.txt")),
                ok(file("f1", "a.txt")),
            ],
            |c| {
                move_in(
                    c,
                    &format!("{ROOT}/f1~a.txt"),
                    &format!("{ROOT}/p2~Other/~a.txt"),
                    true,
                )
            },
        );
        assert!(result.unwrap_err().contains("nope"));
        let log = log.lock().unwrap();
        assert_eq!(log[5].body_text(), r#"{"trashed":false}"#);
        let back = &log[6];
        assert!(back.head.starts_with("PATCH /files/f1?"), "{}", back.head);
        assert!(back.head.contains("addParents=root") && back.head.contains("removeParents=p2"));
        assert_eq!(back.body_text(), r#"{"name":"a.txt"}"#);
        assert!(log.iter().all(
            |r| !(r.head.starts_with("PATCH /files/f1?") && r.body_text().contains("trashed"))
        ));
    }

    #[test]
    fn a_vanished_source_is_refused() {
        let (result, _) = run(vec![Reply::json(404, "{}")], |c| {
            copy_in(
                c,
                &format!("{ROOT}/f1~a.txt"),
                &format!("{ROOT}/p2~Other/~a.txt"),
                false,
            )
        });
        assert_eq!(result.unwrap_err(), "“a.txt” no longer exists");

        let trashed = r#"{"id":"f1","name":"a.txt","mimeType":"text/plain","trashed":true}"#;
        let (result, _) = run(vec![Reply::json(200, trashed)], |c| {
            copy_in(
                c,
                &format!("{ROOT}/f1~a.txt"),
                &format!("{ROOT}/p2~Other/~a.txt"),
                false,
            )
        });
        assert_eq!(result.unwrap_err(), "“a.txt” no longer exists");
    }

    #[test]
    fn copying_between_accounts_is_refused() {
        let (result, log) = run(vec![], |c| {
            copy_in(
                c,
                &format!("{ROOT}/f1~a.txt"),
                "gdrive://other/root~My%20Drive/~a.txt",
                false,
            )
        });
        assert_eq!(
            result.unwrap_err(),
            "Copying between Google accounts is not supported."
        );
        assert!(heads(&log).is_empty());
    }

    // ---- Steps the engine adds for D6 -------------------------------------------

    /// A Drive download's partial folder goes to the local Trash the
    /// transfer was given (the app's logged one), never removed for good.
    #[test]
    fn a_partial_folder_from_a_drive_download_goes_to_the_logged_trash() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("Docs");
        let sent = std::cell::RefCell::new(Vec::new());
        let trash_local = |p: &Path| -> Result<Option<PathBuf>, String> {
            sent.borrow_mut().push(p.to_path_buf());
            Ok(None)
        };
        let (result, _) = run(
            vec![
                ok(folder("d1", "Docs")),
                found(&[sized("x1", "x.txt", 2), sized("y1", "y.txt", 5)]),
                bytes(b"hi"),
                bytes(b"yy"),
            ],
            |c| dispatch_in(c, &format!("{ROOT}/d1~Docs"), &target.to_string_lossy(), false, false, &trash_local),
        );
        assert_eq!(result.unwrap_err(), "The download from Google Drive was incomplete.");
        assert_eq!(*sent.borrow(), vec![target.clone()]);
    }
}

/// The engine against a fake Drive: every D6 primitive it uses, in the
/// order it calls them, when each succeeds and when each fails, with what a
/// failure leaves behind.
#[cfg(test)]
mod fake_drive_tests {
    use super::*;
    use std::cell::{Cell, RefCell};
    use std::collections::{BTreeMap, HashMap};
    use std::path::PathBuf;

    use places::{Existing, Put, Reading, Source};

    const ROOT: &str = "gdrive://acct/root~R";

    /// An item of the fake Drive.
    #[derive(Clone, Debug, PartialEq)]
    struct Item {
        parent: String,
        name: String,
        is_dir: bool,
        trashed: bool,
    }

    /// Drive as the primitives see it: items by path (`parent/id~name`),
    /// each primitive call logged, and any primitive made to fail.
    #[derive(Default)]
    struct FakeDrive {
        items: RefCell<BTreeMap<String, Item>>,
        log: RefCell<Vec<String>>,
        fail: RefCell<HashMap<&'static str, Error>>,
        next: Cell<u32>,
        /// What a write or a read skips.
        skips: Vec<String>,
        /// A read fills only part of a folder, then fails.
        read_fails: bool,
    }

    fn parent_of(path: &str) -> String {
        path.rsplit_once('/').map(|(p, _)| p.to_string()).unwrap_or_default()
    }

    fn name_of(path: &str) -> String {
        path.rsplit('/').next().and_then(|s| s.split_once('~')).map(|(_, n)| n.to_string()).unwrap_or_default()
    }

    impl FakeDrive {
        fn with(items: &[(&str, bool)]) -> Self {
            let drive = FakeDrive::default();
            for (path, is_dir) in items {
                drive.items.borrow_mut().insert(
                    path.to_string(),
                    Item { parent: parent_of(path), name: name_of(path), is_dir: *is_dir, trashed: false },
                );
            }
            drive
        }

        fn failing(self, primitive: &'static str) -> Self {
            self.fail.borrow_mut().insert(primitive, Error::Other(format!("{primitive} failed")));
            self
        }

        fn failing_with(self, primitive: &'static str, error: Error) -> Self {
            self.fail.borrow_mut().insert(primitive, error);
            self
        }

        /// Log the call; the error a test asked for, if any.
        fn call(&self, primitive: &'static str, args: &[&str]) -> places::Result<()> {
            self.log.borrow_mut().push(format!("{primitive} {}", args.join(" ")).trim_end().to_string());
            match self.fail.borrow().get(primitive) {
                Some(error) => Err(error.clone()),
                None => Ok(()),
            }
        }

        fn calls(&self) -> Vec<String> {
            self.log.borrow().clone()
        }

        fn item(&self, path: &str) -> Option<Item> {
            self.items.borrow().get(path).cloned()
        }

        /// The live items, as `parent/name` with what they are.
        fn shown(&self) -> Vec<String> {
            let items = self.items.borrow();
            let mut shown: Vec<String> = items
                .values()
                .filter(|i| !i.trashed)
                .map(|i| format!("{}/{}", i.parent, i.name))
                .collect();
            shown.sort();
            shown
        }

        fn add(&self, parent: &str, name: &str, is_dir: bool) -> String {
            self.next.set(self.next.get() + 1);
            let path = format!("{parent}/n{}~{name}", self.next.get());
            let item = Item { parent: parent.into(), name: name.into(), is_dir, trashed: false };
            self.items.borrow_mut().insert(path.clone(), item);
            path
        }

        /// Give the item at `from` a new folder and name; its path follows.
        fn relocate(&self, from: &str, to: &str) -> places::Result<String> {
            let (parent, name) = to.rsplit_once("/~").unwrap();
            let mut item = self.items.borrow_mut().remove(from).ok_or(Error::NotFound("gone".into()))?;
            item.parent = parent.into();
            item.name = name.into();
            let id = from.rsplit('/').next().unwrap().split('~').next().unwrap();
            let path = format!("{parent}/{id}~{name}");
            self.items.borrow_mut().insert(path.clone(), item);
            Ok(path)
        }

        fn set_trashed(&self, path: &str, trashed: bool) -> places::Result<()> {
            let mut items = self.items.borrow_mut();
            let item = items.get_mut(path).ok_or(Error::NotFound("gone".into()))?;
            item.trashed = trashed;
            Ok(())
        }
    }

    /// A read of the fake Drive: a file "hello", or a folder with two.
    struct FakeRead<'a> {
        is_dir: bool,
        fails: bool,
        skips: &'a [String],
    }

    impl Reading for FakeRead<'_> {
        fn is_dir(&self) -> bool {
            self.is_dir
        }
        fn fill(&self, dest: &Path, cx: &Cx) -> Result<Vec<String>, Unread> {
            cx.check().map_err(Unread::Nothing)?;
            if !self.is_dir {
                std::fs::write(dest, "hello").unwrap();
                return match self.fails {
                    true => Err(Unread::Partial(Error::Other("short read".into()))),
                    false => Ok(self.skips.to_vec()),
                };
            }
            std::fs::create_dir(dest).map_err(|e| Unread::Nothing(Error::Other(e.to_string())))?;
            std::fs::write(dest.join("x.txt"), "x").unwrap();
            if self.fails {
                return Err(Unread::Partial(Error::Other("short read".into())));
            }
            std::fs::write(dest.join("y.txt"), "y").unwrap();
            Ok(self.skips.to_vec())
        }
    }

    impl Transfer for FakeDrive {
        fn check(&self, from: Option<&str>, to: Option<&str>, _: &Cx) -> places::Result<()> {
            self.call("check", &[from.unwrap_or("-"), to.unwrap_or("-")])
        }
        fn landing(&self, from: Option<&str>, to: &str, _: &Cx) -> places::Result<Landing> {
            self.call("landing", &[to])?;
            let (parent, name) = to.rsplit_once("/~").unwrap();
            let items = self.items.borrow();
            let existing = items
                .iter()
                .filter(|(path, i)| i.parent == parent && i.name == name && !i.trashed && Some(path.as_str()) != from)
                .map(|(path, _)| Existing {
                    path: path.clone(),
                    holds_source: from.is_some_and(|f| f.starts_with(&format!("{path}/"))),
                })
                .collect();
            let source = from.map(|from| Source {
                path: from.into(),
                is_dir: items[from].is_dir,
                home: Some(format!("{}/~{}", items[from].parent, items[from].name)),
            });
            Ok(Landing { parent: parent.into(), name: name.into(), existing, source })
        }
        fn pending(&self, parent: &str, name: &str) -> places::Result<String> {
            Ok(format!("{parent}/~{name}"))
        }
        fn write(&self, source: &Path, target: &str, _: bool, cx: &Cx) -> places::Result<Put> {
            self.call("write", &[target])?;
            cx.check()?;
            let (parent, name) = target.rsplit_once("/~").unwrap();
            let path = self.add(parent, name, source.is_dir());
            Ok(Put { path, skipped: self.skips.clone() })
        }
        fn read_into(&self, _: &str, _: &Path, _: &Cx) -> places::Result<Put> {
            unreachable!("the engine reads in two steps")
        }
        fn read(&self, path: &str, place: &mut dyn FnMut(&dyn Reading) -> places::Result<()>, _: &Cx) -> places::Result<()> {
            self.call("read", &[path])?;
            let item = self.item(path).ok_or(Error::NotFound("gone".into()))?;
            place(&FakeRead { is_dir: item.is_dir, fails: self.read_fails, skips: &self.skips })
        }
        fn export(&self, _: &str, _: &Cx) -> places::Result<PathBuf> {
            unreachable!("Drive reads straight into the target")
        }
        fn copy_within(&self, from: &str, to: &str, cx: &Cx) -> places::Result<Put> {
            self.call("copy_within", &[from, to])?;
            cx.check()?;
            let (parent, name) = to.rsplit_once("/~").unwrap();
            let path = self.add(parent, name, self.item(from).unwrap().is_dir);
            Ok(Put { path, skipped: Vec::new() })
        }
        fn rename_new(&self, from: &str, to: &str, _: &Cx) -> places::Result<String> {
            self.call("rename_new", &[from, to])?;
            self.relocate(from, to)
        }
        fn move_to(&self, from: &str, to: &str, _: &Cx) -> places::Result<String> {
            self.call("move_to", &[from, to])?;
            self.relocate(from, to)
        }
        fn trash_for_replace(&self, path: &str, _: &Cx) -> places::Result<String> {
            self.call("trash_for_replace", &[path])?;
            self.set_trashed(path, true)?;
            Ok(path.into())
        }
        fn take_back(&self, trashed: &str, original: &str, _: &Cx) -> places::Result<()> {
            self.call("take_back", &[trashed, original])?;
            self.set_trashed(trashed, false)
        }
        fn discard(&self, staged: &str, _: &Cx) -> places::Result<()> {
            self.call("discard", &[staged])?;
            self.set_trashed(staged, true)
        }
    }

    fn cx() -> Cx {
        Cx::default()
    }

    fn cancelled() -> Cx {
        let cx = Cx::default();
        cx.progress.cancel();
        cx
    }

    fn local_file(dir: &tempfile::TempDir, name: &str) -> PathBuf {
        let path = dir.path().join(name);
        std::fs::write(&path, "mine").unwrap();
        path
    }

    fn no_trash(_: &Path) -> Result<Option<PathBuf>, String> {
        panic!("nothing is replaced on this computer")
    }

    fn no_discard(_: &Path) -> Result<(), String> {
        panic!("nothing to discard")
    }

    fn names_in(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> =
            std::fs::read_dir(dir).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        names
    }

    const OLD: &str = "gdrive://acct/root~R/o1~a.txt";

    // ---- write, with and without Replace ----------------------------------------

    #[test]
    fn a_new_file_is_written_straight_under_its_name() {
        let dir = tempfile::tempdir().unwrap();
        let drive = FakeDrive::default();
        let put = upload(&drive, &cx(), &local_file(&dir, "a.txt"), &format!("{ROOT}/~a.txt"), false).unwrap();
        assert_eq!(put, format!("{ROOT}/n1~a.txt"));
        assert_eq!(drive.calls(), [format!("check - {ROOT}/~a.txt"), format!("landing {ROOT}/~a.txt"), format!("write {ROOT}/~a.txt")]);
    }

    #[test]
    fn a_new_folder_is_written_under_a_staging_name_then_renamed_in() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("Docs")).unwrap();
        let drive = FakeDrive::default();
        let put = upload(&drive, &cx(), &dir.path().join("Docs"), &format!("{ROOT}/~Docs"), false).unwrap();
        assert_eq!(put, format!("{ROOT}/n1~Docs"));
        let staged = format!("{ROOT}/n1~.Docs.filebrowser-upload");
        assert_eq!(drive.calls()[2..], [
            format!("write {ROOT}/~.Docs.filebrowser-upload"),
            format!("rename_new {staged} {ROOT}/~Docs"),
        ]);
    }

    #[test]
    fn replace_writes_staged_trashes_the_old_item_then_renames_in() {
        let dir = tempfile::tempdir().unwrap();
        let drive = FakeDrive::with(&[(OLD, false)]);
        let put = upload(&drive, &cx(), &local_file(&dir, "a.txt"), &format!("{ROOT}/~a.txt"), true).unwrap();
        assert_eq!(put, format!("{ROOT}/n1~a.txt"));
        let staged = format!("{ROOT}/n1~.a.txt.filebrowser-upload");
        assert_eq!(drive.calls()[2..], [
            format!("write {ROOT}/~.a.txt.filebrowser-upload"),
            format!("trash_for_replace {OLD}"),
            format!("rename_new {staged} {ROOT}/~a.txt"),
        ]);
        assert!(drive.item(OLD).unwrap().trashed, "in Drive's Trash, never deleted");
        assert_eq!(drive.shown(), [format!("{ROOT}/a.txt")]);
    }

    #[test]
    fn a_write_that_fails_midway_leaves_the_old_item_alone() {
        let dir = tempfile::tempdir().unwrap();
        let drive = FakeDrive::with(&[(OLD, false)]).failing("write");
        let error = upload(&drive, &cx(), &local_file(&dir, "a.txt"), &format!("{ROOT}/~a.txt"), true).unwrap_err();
        assert_eq!(error, "write failed");
        assert_eq!(drive.calls().len(), 3, "nothing after the write");
        assert!(!drive.item(OLD).unwrap().trashed);
    }

    #[test]
    fn a_replace_whose_trash_fails_discards_the_staged_copy() {
        let dir = tempfile::tempdir().unwrap();
        let drive = FakeDrive::with(&[(OLD, false)]).failing("trash_for_replace");
        let error = upload(&drive, &cx(), &local_file(&dir, "a.txt"), &format!("{ROOT}/~a.txt"), true).unwrap_err();
        assert_eq!(error, "Could not move the existing item to the Trash: trash_for_replace failed");
        let staged = format!("{ROOT}/n1~.a.txt.filebrowser-upload");
        assert_eq!(drive.calls()[3..], [format!("trash_for_replace {OLD}"), format!("discard {staged}")]);
        assert_eq!(drive.shown(), [format!("{ROOT}/a.txt")]);
        assert!(drive.item(&staged).unwrap().trashed, "discarded to Drive's Trash, never deleted");
    }

    /// The last rename fails once the old item is in the Trash: it comes
    /// back out first, then the staged copy is discarded.
    #[test]
    fn a_failed_last_rename_takes_the_old_item_back_then_discards_the_copy() {
        let dir = tempfile::tempdir().unwrap();
        let drive = FakeDrive::with(&[(OLD, false)]).failing("rename_new");
        let error = upload(&drive, &cx(), &local_file(&dir, "a.txt"), &format!("{ROOT}/~a.txt"), true).unwrap_err();
        assert_eq!(error, "rename_new failed");
        let staged = format!("{ROOT}/n1~.a.txt.filebrowser-upload");
        assert_eq!(drive.calls()[3..], [
            format!("trash_for_replace {OLD}"),
            format!("rename_new {staged} {ROOT}/~a.txt"),
            format!("take_back {OLD} {OLD}"),
            format!("discard {staged}"),
        ]);
        assert!(!drive.item(OLD).unwrap().trashed);
        assert_eq!(drive.shown(), [format!("{ROOT}/a.txt")]);
    }

    /// Drive's take-back is best effort, as before: when it fails too, the
    /// rename's error is the transfer's and the old item stays in the Trash.
    #[test]
    fn a_take_back_that_fails_leaves_the_old_item_in_drives_trash() {
        let dir = tempfile::tempdir().unwrap();
        let drive = FakeDrive::with(&[(OLD, false)]).failing("rename_new").failing("take_back");
        let error = upload(&drive, &cx(), &local_file(&dir, "a.txt"), &format!("{ROOT}/~a.txt"), true).unwrap_err();
        assert_eq!(error, "rename_new failed");
        assert!(drive.item(OLD).unwrap().trashed);
        assert!(drive.calls().last().unwrap().starts_with("discard "));
    }

    #[test]
    fn skipped_items_fail_the_write_once_the_rest_is_in_place() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("Docs")).unwrap();
        let drive = FakeDrive { skips: vec!["“link” was not copied.".into()], ..FakeDrive::default() };
        let error = upload(&drive, &cx(), &dir.path().join("Docs"), &format!("{ROOT}/~Docs"), false).unwrap_err();
        assert_eq!(error, "“link” was not copied.");
        assert_eq!(drive.shown(), [format!("{ROOT}/Docs")]);
    }

    #[test]
    fn a_cancelled_write_trashes_and_renames_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let drive = FakeDrive::with(&[(OLD, false)]);
        let error = upload(&drive, &cancelled(), &local_file(&dir, "a.txt"), &format!("{ROOT}/~a.txt"), true).unwrap_err();
        assert_eq!(error, "Cancelled.");
        assert!(drive.calls().last().unwrap().starts_with("write "));
        assert_eq!(drive.shown(), [format!("{ROOT}/a.txt")]);
        assert!(!drive.item(OLD).unwrap().trashed);
    }

    // ---- The refusals, in order -------------------------------------------------

    #[test]
    fn the_places_own_refusal_comes_before_anything_else() {
        let drive = FakeDrive::default().failing_with("check", Error::Other("read only".into()));
        // The source doesn't exist either: the place's refusal is first.
        let error = upload(&drive, &cx(), Path::new("/nonexistent/a.txt"), &format!("{ROOT}/~a.txt"), false);
        assert_eq!(error.unwrap_err(), "read only");
        assert_eq!(drive.calls().len(), 1);
    }

    #[test]
    fn a_vanished_local_source_is_refused_before_drive_is_asked() {
        let drive = FakeDrive::default();
        let error = upload(&drive, &cx(), Path::new("/nonexistent/a.txt"), &format!("{ROOT}/~a.txt"), false);
        assert_eq!(error.unwrap_err(), "“/nonexistent/a.txt” no longer exists");
        assert_eq!(drive.calls().len(), 1, "only the place's own check");
    }

    #[test]
    fn a_taken_name_an_ambiguous_one_and_a_folder_holding_the_source_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let source = local_file(&dir, "a.txt");
        let to = format!("{ROOT}/~a.txt");
        let drive = FakeDrive::with(&[(OLD, false)]);
        assert_eq!(upload(&drive, &cx(), &source, &to, false).unwrap_err(), "Target already exists: a.txt");
        let drive = FakeDrive::with(&[(OLD, false), ("gdrive://acct/root~R/o2~a.txt", false)]);
        assert_eq!(upload(&drive, &cx(), &source, &to, false).unwrap_err(), "Target already exists: a.txt");
        assert_eq!(
            upload(&drive, &cx(), &source, &to, true).unwrap_err(),
            "Several items in this folder are named “a.txt”. Rename or remove the extra ones in Google Drive first."
        );
        let docs = "gdrive://acct/root~R/d1~Docs";
        let inner = "gdrive://acct/root~R/d1~Docs/f1~Docs";
        let drive = FakeDrive::with(&[(docs, true), (inner, false)]);
        assert_eq!(move_within(&drive, &cx(), inner, &format!("{ROOT}/~Docs"), true).unwrap_err(), "Cannot replace a folder with an item inside it");
        for drive in [&drive] {
            assert!(drive.calls().iter().all(|c| c.starts_with("check") || c.starts_with("landing")));
        }
    }

    // ---- Inside Drive -------------------------------------------------------------

    #[test]
    fn a_copy_within_of_a_file_lands_under_its_name() {
        let src = "gdrive://acct/root~R/f1~a.txt";
        let drive = FakeDrive::with(&[(src, false)]);
        let put = copy_within(&drive, &cx(), src, &format!("{ROOT}/p~P/~a.txt"), false).unwrap();
        assert_eq!(put, format!("{ROOT}/p~P/n1~a.txt"));
        assert_eq!(drive.calls()[2], format!("copy_within {src} {ROOT}/p~P/~a.txt"));
    }

    #[test]
    fn a_copy_within_of_a_folder_is_staged_and_a_failed_rename_discards_it() {
        let src = "gdrive://acct/root~R/d1~Docs";
        let drive = FakeDrive::with(&[(src, true)]).failing("rename_new");
        let error = copy_within(&drive, &cx(), src, &format!("{ROOT}/p~P/~Docs"), false).unwrap_err();
        assert_eq!(error, "rename_new failed");
        let staged = format!("{ROOT}/p~P/n1~.Docs.filebrowser-copy");
        assert_eq!(drive.calls()[2..], [
            format!("copy_within {src} {ROOT}/p~P/~.Docs.filebrowser-copy"),
            format!("rename_new {staged} {ROOT}/p~P/~Docs"),
            format!("discard {staged}"),
        ]);
        assert_eq!(drive.shown(), [format!("{ROOT}/Docs")]);
    }

    #[test]
    fn a_cancelled_copy_within_copies_nothing() {
        let src = "gdrive://acct/root~R/f1~a.txt";
        let drive = FakeDrive::with(&[(src, false)]);
        let error = copy_within(&drive, &cancelled(), src, &format!("{ROOT}/p~P/~a.txt"), false).unwrap_err();
        assert_eq!(error, "Cancelled.");
        assert_eq!(drive.shown(), [format!("{ROOT}/a.txt")]);
    }

    #[test]
    fn a_move_within_that_replaces_nothing_is_one_move() {
        let src = "gdrive://acct/root~R/f1~a.txt";
        let drive = FakeDrive::with(&[(src, false)]);
        let moved = move_within(&drive, &cx(), src, &format!("{ROOT}/p~P/~a.txt"), false).unwrap();
        assert_eq!(moved, format!("{ROOT}/p~P/f1~a.txt"));
        assert_eq!(drive.calls()[2..], [format!("move_to {src} {ROOT}/p~P/~a.txt")]);
    }

    /// The user's own item, staged during a Replace, is only ever moved back:
    /// never discarded, whichever step fails.
    #[test]
    fn a_staged_original_goes_back_home_whichever_step_fails() {
        let src = "gdrive://acct/root~R/f1~a.txt";
        let old = "gdrive://acct/root~R/p~P/o1~a.txt";
        for failing in ["trash_for_replace", "rename_new"] {
            let drive = FakeDrive::with(&[(src, false), (old, false)]).failing(failing);
            assert!(move_within(&drive, &cx(), src, &format!("{ROOT}/p~P/~a.txt"), true).is_err());
            let staged = format!("{ROOT}/p~P/f1~.a.txt.filebrowser-move");
            let calls = drive.calls();
            assert_eq!(calls[2], format!("move_to {src} {ROOT}/p~P/~.a.txt.filebrowser-move"));
            assert_eq!(calls.last().unwrap(), &format!("move_to {staged} {ROOT}/~a.txt"), "{failing}");
            assert!(calls.iter().all(|c| !c.starts_with("discard")), "{failing}");
            assert_eq!(drive.shown(), [format!("{ROOT}/a.txt"), format!("{ROOT}/p~P/a.txt")], "{failing}");
            assert!(!drive.item(old).unwrap().trashed, "{failing}");
        }
    }

    // ---- Drive to this computer ---------------------------------------------------

    const FILE: &str = "gdrive://acct/root~R/f1~a.txt";
    const FOLDER: &str = "gdrive://acct/root~R/d1~Docs";

    #[test]
    fn a_read_file_is_staged_then_placed() {
        let dir = tempfile::tempdir().unwrap();
        let drive = FakeDrive::with(&[(FILE, false)]);
        let to = dir.path().join("a.txt");
        assert_eq!(download(&drive, &cx(), FILE, &to, false, &no_discard, &no_trash).unwrap(), to.to_string_lossy());
        assert_eq!(std::fs::read_to_string(&to).unwrap(), "hello");
        assert_eq!(names_in(dir.path()), ["a.txt"]);
    }

    #[test]
    fn a_partial_folder_read_goes_to_discard() {
        let dir = tempfile::tempdir().unwrap();
        let drive = FakeDrive { read_fails: true, ..FakeDrive::with(&[(FOLDER, true)]) };
        let to = dir.path().join("Docs");
        let discarded = RefCell::new(Vec::new());
        let discard = |p: &Path| {
            discarded.borrow_mut().push(p.to_path_buf());
            Ok(())
        };
        assert_eq!(download(&drive, &cx(), FOLDER, &to, false, &discard, &no_trash).unwrap_err(), "short read");
        assert_eq!(*discarded.borrow(), [to]);
    }

    #[test]
    fn a_partial_file_read_is_removed_and_the_old_file_kept() {
        let dir = tempfile::tempdir().unwrap();
        let to = local_file(&dir, "a.txt");
        let drive = FakeDrive { read_fails: true, ..FakeDrive::with(&[(FILE, false)]) };
        assert_eq!(download(&drive, &cx(), FILE, &to, true, &no_discard, &no_trash).unwrap_err(), "short read");
        assert_eq!(std::fs::read_to_string(&to).unwrap(), "mine");
        assert_eq!(names_in(dir.path()), ["a.txt"]);
    }

    #[test]
    fn a_read_that_replaces_sends_the_old_item_to_the_local_trash() {
        let dir = tempfile::tempdir().unwrap();
        let to = local_file(&dir, "a.txt");
        let bin = tempfile::tempdir().unwrap();
        let trash = |p: &Path| -> Result<Option<PathBuf>, String> {
            std::fs::copy(p, bin.path().join("a.txt")).unwrap();
            std::fs::remove_file(p).unwrap();
            Ok(Some(bin.path().join("a.txt")))
        };
        let drive = FakeDrive::with(&[(FILE, false)]);
        download(&drive, &cx(), FILE, &to, true, &no_discard, &trash).unwrap();
        assert_eq!(std::fs::read_to_string(&to).unwrap(), "hello");
        assert_eq!(std::fs::read_to_string(bin.path().join("a.txt")).unwrap(), "mine");
    }

    #[test]
    fn a_cancelled_read_leaves_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let drive = FakeDrive::with(&[(FILE, false)]);
        let to = dir.path().join("a.txt");
        assert_eq!(download(&drive, &cancelled(), FILE, &to, false, &no_discard, &no_trash).unwrap_err(), "Cancelled.");
        assert_eq!(names_in(dir.path()), Vec::<String>::new());
    }

    #[test]
    fn a_move_down_trashes_the_drive_source_only_after_the_whole_read() {
        let dir = tempfile::tempdir().unwrap();
        let drive = FakeDrive::with(&[(FILE, false)]);
        move_down(&drive, &cx(), FILE, &dir.path().join("a.txt"), false, &no_trash).unwrap();
        assert_eq!(drive.calls(), [format!("check {FILE} -"), format!("read {FILE}"), format!("trash_for_replace {FILE}")]);
        assert!(drive.item(FILE).unwrap().trashed);

        let drive = FakeDrive { read_fails: true, ..FakeDrive::with(&[(FILE, false)]) };
        assert!(move_down(&drive, &cx(), FILE, &dir.path().join("b.txt"), false, &no_trash).is_err());
        assert!(!drive.item(FILE).unwrap().trashed, "a failed copy keeps its source");
    }

    #[test]
    fn a_move_down_from_a_source_that_cant_move_reads_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let drive = FakeDrive::with(&[(FILE, false)]).failing_with("check", Error::Other("read only".into()));
        let error = move_down(&drive, &cx(), FILE, &dir.path().join("a.txt"), false, &no_trash).unwrap_err();
        assert_eq!(error, "read only");
        assert_eq!(drive.calls().len(), 1);
        assert_eq!(names_in(dir.path()), Vec::<String>::new());
    }

    /// The original's Trash failing keeps today's words: a request that
    /// failed says the item moved; an account that can't ask says why.
    #[test]
    fn a_move_down_whose_source_cant_go_to_the_trash_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let drive = FakeDrive::with(&[(FILE, false)]).failing("trash_for_replace");
        let error = move_down(&drive, &cx(), FILE, &dir.path().join("a.txt"), false, &no_trash).unwrap_err();
        assert_eq!(error, "Moved, but could not move the original to the Trash: trash_for_replace failed");
        let signed_out = Error::AuthRequired("Google Drive is not connected.".into());
        let drive = FakeDrive::with(&[(FILE, false)]).failing_with("trash_for_replace", signed_out);
        let error = move_down(&drive, &cx(), FILE, &dir.path().join("b.txt"), false, &no_trash).unwrap_err();
        assert_eq!(error, "Google Drive is not connected.");
    }

    // ---- This computer to Drive, moving -------------------------------------------

    #[test]
    fn a_move_up_trashes_the_local_source_only_after_the_whole_write() {
        let dir = tempfile::tempdir().unwrap();
        let source = local_file(&dir, "a.txt");
        let trashed = RefCell::new(Vec::new());
        let drive = FakeDrive::default().failing("write");
        let failed = move_up(&drive, &cx(), &source, &format!("{ROOT}/~a.txt"), false, |p| {
            trashed.borrow_mut().push(p.to_path_buf());
            Ok(())
        });
        assert!(failed.is_err());
        assert!(trashed.borrow().is_empty());
        let drive = FakeDrive::default();
        move_up(&drive, &cx(), &source, &format!("{ROOT}/~a.txt"), false, |p| {
            trashed.borrow_mut().push(p.to_path_buf());
            Ok(())
        })
        .unwrap();
        assert_eq!(*trashed.borrow(), [source]);
    }
}
