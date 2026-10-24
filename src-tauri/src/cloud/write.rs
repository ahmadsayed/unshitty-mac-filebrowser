//! Changes inside Google Drive: New Folder, Rename, Copy, Move, Move to Trash
//! and Undo. Destinations are pending paths (`…/~name`, see `location.rs`);
//! each operation returns the item's final URI.
//!
//! Data safety (spec D17): nothing is ever deleted. Replace and Undo use
//! Drive's Trash; a folder copy is built under a staging name and renamed
//! only when complete, and trashed if it fails.

use crate::cloud::CloudState;
use crate::cloud::drive::{API_BASE, DriveClient, DriveError, DriveFile, FileChange};
use crate::cloud::location::DriveLocation;
use crate::cloud::service::{self, Refresh};

/// Where requests go and how tokens refresh; tests point it at a fake server.
pub(crate) struct Ctx<'a> {
    pub(crate) state: &'a CloudState,
    pub(crate) base: &'a str,
    pub(crate) refresh: Refresh<'a>,
}

/// Run `work` with a production context.
pub(crate) fn live<T>(state: &CloudState, work: impl FnOnce(&Ctx) -> T) -> T {
    let refresh = service::google_refresh(state);
    work(&Ctx {
        state,
        base: API_BASE,
        refresh: &refresh,
    })
}

pub fn make_dir(state: &CloudState, path: &str) -> Result<String, String> {
    live(state, |c| make_dir_in(c, path))
}

pub fn rename(state: &CloudState, from: &str, to: &str) -> Result<String, String> {
    live(state, |c| rename_in(c, from, to))
}

pub fn copy(state: &CloudState, from: &str, to: &str, overwrite: bool) -> Result<String, String> {
    live(state, |c| copy_in(c, from, to, overwrite))
}

pub fn move_item(
    state: &CloudState,
    from: &str,
    to: &str,
    overwrite: bool,
) -> Result<String, String> {
    live(state, |c| move_in(c, from, to, overwrite))
}

/// Move to Drive's Trash. The URI stays valid (the id survives), so it is
/// what Undo hands back to `restore`.
pub fn trash(state: &CloudState, path: &str) -> Result<Option<String>, String> {
    live(state, |c| trash_in(c, path))
}

pub fn restore(state: &CloudState, path: &str) -> Result<String, String> {
    live(state, |c| restore_in(c, path))
}

pub(crate) fn message(error: DriveError) -> String {
    error.message()
}

pub(crate) fn client<'a>(c: &Ctx<'a>, account: &str) -> Result<DriveClient<'a>, String> {
    service::client_with(c.state, account, c.base, c.refresh)
}

fn id_of(location: &DriveLocation) -> Result<String, String> {
    location
        .file_id()
        .filter(|id| !id.is_empty())
        .map(str::to_string)
        .ok_or_else(|| "This Google Drive item has no id.".to_string())
}

/// A pending destination split into its parent and the new name.
pub(crate) fn pending_target(path: &str) -> Result<(DriveLocation, String, String), String> {
    let target = service::drive_location(path)?;
    if !target.is_pending() {
        return Err("The destination must be a new name in a Google Drive folder.".into());
    }
    let parent = target
        .parent()
        .ok_or_else(|| "The destination has no folder.".to_string())?;
    let parent_id = id_of(&parent)?;
    let name = target.name().unwrap_or_default().to_string();
    Ok((parent, parent_id, name))
}

fn staging_name(name: &str) -> String {
    format!(".{name}.filebrowser-copy")
}

pub(crate) fn make_dir_in(c: &Ctx, path: &str) -> Result<String, String> {
    let (_, account) = service::writable(c.state, path)?;
    let (parent, parent_id, name) = pending_target(path)?;
    let client = client(c, &account.id)?;
    if !client
        .find_children(&parent_id, &name)
        .map_err(message)?
        .is_empty()
    {
        return Err("A folder with that name already exists.".into());
    }
    let created = client.create_folder(&parent_id, &name).map_err(message)?;
    Ok(parent.child(&created.id, &created.name).to_uri())
}

pub(crate) fn rename_in(c: &Ctx, from: &str, to: &str) -> Result<String, String> {
    let (source, account) = service::writable(c.state, from)?;
    let id = id_of(&source)?;
    let (parent, parent_id, name) = pending_target(to)?;
    if source
        .parent()
        .and_then(|p| p.file_id().map(str::to_string))
        != Some(parent_id.clone())
    {
        return Err("Rename keeps an item in its folder; use Move instead.".into());
    }
    let client = client(c, &account.id)?;
    let clash = client
        .find_children(&parent_id, &name)
        .map_err(message)?
        .into_iter()
        .any(|other| other.id != id);
    if clash {
        return Err("A file or folder with that name already exists.".into());
    }
    let renamed = client
        .update(&id, &FileChange::rename(&name))
        .map_err(message)?;
    Ok(parent.child(&renamed.id, &renamed.name).to_uri())
}

pub(crate) fn trash_in(c: &Ctx, path: &str) -> Result<Option<String>, String> {
    let (location, account) = service::writable(c.state, path)?;
    let id = id_of(&location)?;
    client(c, &account.id)?
        .update(&id, &FileChange::trash(true))
        .map_err(message)?;
    Ok(Some(path.to_string()))
}

pub(crate) fn restore_in(c: &Ctx, path: &str) -> Result<String, String> {
    let (location, account) = service::writable(c.state, path)?;
    let id = id_of(&location)?;
    client(c, &account.id)?
        .update(&id, &FileChange::trash(false))
        .map_err(message)?;
    Ok(path.to_string())
}

/// What a checked copy or move works with.
struct Transfer<'a> {
    client: DriveClient<'a>,
    source: DriveFile,
    source_parent_id: Option<String>,
    target_parent: DriveLocation,
    target_parent_id: String,
    name: String,
    /// The existing item a Replace sends to the Trash.
    replaced: Option<DriveFile>,
}

/// The refusals of `fs_ops::check_transfer`, for Drive. Path-only checks run
/// before any request.
fn check<'a>(c: &Ctx<'a>, from: &str, to: &str, overwrite: bool) -> Result<Transfer<'a>, String> {
    let (source_location, account) = service::writable(c.state, from)?;
    let source_id = id_of(&source_location)?;
    let target = service::drive_location(to)?;
    if target.account != source_location.account {
        return Err("Copying between Google accounts is not supported.".into());
    }
    let (target_parent, target_parent_id, name) = pending_target(to)?;
    if target_parent.segments.iter().any(|s| s.id == source_id) {
        return Err("Cannot copy or move a folder into itself".into());
    }
    let source_parent_id = source_location
        .parent()
        .and_then(|p| p.file_id().map(str::to_string));
    if source_parent_id.as_deref() == Some(target_parent_id.as_str())
        && source_location.name() == Some(name.as_str())
    {
        return Err("The source and destination are the same".into());
    }

    let client = client(c, &account.id)?;
    let shown = source_location.name().unwrap_or_default().to_string();
    let source = match client.get(&source_id) {
        Ok(file) if !file.trashed => file,
        Ok(_) | Err(DriveError::NotFound) => return Err(format!("“{shown}” no longer exists")),
        Err(error) => return Err(message(error)),
    };
    let existing: Vec<DriveFile> = client
        .find_children(&target_parent_id, &name)
        .map_err(message)?
        .into_iter()
        .filter(|other| other.id != source_id)
        .collect();
    let replaced = match existing.len() {
        0 => None,
        _ if !overwrite => return Err(format!("Target already exists: {name}")),
        1 => Some(existing.into_iter().next().unwrap()),
        _ => {
            return Err(format!(
                "Several items in this folder are named “{name}”. Rename or remove the extra ones in Google Drive first."
            ));
        }
    };
    if let Some(old) = &replaced {
        if source_location.segments.iter().any(|s| s.id == old.id) {
            return Err("Cannot replace a folder with an item inside it".into());
        }
    }
    Ok(Transfer {
        client,
        source,
        source_parent_id,
        target_parent,
        target_parent_id,
        name,
        replaced,
    })
}

pub(crate) fn copy_in(c: &Ctx, from: &str, to: &str, overwrite: bool) -> Result<String, String> {
    let t = check(c, from, to, overwrite)?;
    let staged_name = if t.source.is_dir() || t.replaced.is_some() {
        staging_name(&t.name)
    } else {
        t.name.clone()
    };
    let staged = if t.source.is_dir() {
        let staged = t
            .client
            .create_folder(&t.target_parent_id, &staged_name)
            .map_err(message)?;
        if let Err(error) = copy_children(&t.client, &t.source.id, &staged.id, &staged.id) {
            let _ = t.client.update(&staged.id, &FileChange::trash(true));
            return Err(message(error));
        }
        staged
    } else {
        t.client
            .copy_file(&t.source.id, &t.target_parent_id, &staged_name)
            .map_err(message)?
    };
    if staged_name == t.name {
        return Ok(t.target_parent.child(&staged.id, &staged.name).to_uri());
    }
    finish_staged(
        &t.client,
        &t.target_parent,
        &t.name,
        t.replaced.as_ref(),
        &staged,
    )
}

/// Swap a complete staged copy into place: trash what it replaces, then give
/// it the final name. Any failure trashes the staged copy.
pub(crate) fn finish_staged(
    client: &DriveClient,
    parent: &DriveLocation,
    name: &str,
    replaced: Option<&DriveFile>,
    staged: &DriveFile,
) -> Result<String, String> {
    if let Some(old) = replaced {
        if let Err(error) = client.update(&old.id, &FileChange::trash(true)) {
            let _ = client.update(&staged.id, &FileChange::trash(true));
            return Err(format!(
                "Could not move the existing item to the Trash: {}",
                message(error)
            ));
        }
    }
    match client.update(&staged.id, &FileChange::rename(name)) {
        Ok(done) => Ok(parent.child(&done.id, &done.name).to_uri()),
        Err(error) => {
            let _ = client.update(&staged.id, &FileChange::trash(true));
            Err(message(error))
        }
    }
}

/// Copy a folder's contents into another folder, recursively. Drive copies
/// only files, so folders are recreated. `staged` (the copy being built) is
/// skipped: when the target lies inside the source, the walk meets it, and
/// copying it would never end.
fn copy_children(
    client: &DriveClient,
    from: &str,
    to: &str,
    staged: &str,
) -> Result<(), DriveError> {
    for child in client.list_all(from)? {
        if child.id == staged {
            continue;
        }
        if child.is_dir() {
            let folder = client.create_folder(to, &child.name)?;
            copy_children(client, &child.id, &folder.id, staged)?;
        } else {
            client.copy_file(&child.id, to, &child.name)?;
        }
    }
    Ok(())
}

pub(crate) fn move_in(c: &Ctx, from: &str, to: &str, overwrite: bool) -> Result<String, String> {
    let t = check(c, from, to, overwrite)?;
    let source_parent_id = t
        .source_parent_id
        .clone()
        .ok_or_else(|| "My Drive itself cannot be moved.".to_string())?;
    if let Some(old) = &t.replaced {
        t.client
            .update(&old.id, &FileChange::trash(true))
            .map_err(|error| {
                format!(
                    "Could not move the existing item to the Trash: {}",
                    message(error)
                )
            })?;
    }
    let mut change = FileChange::default();
    if t.name != t.source.name {
        change.name = Some(t.name.clone());
    }
    if t.target_parent_id != source_parent_id {
        change.add_parent = Some(t.target_parent_id.clone());
        change.remove_parent = Some(source_parent_id);
    }
    let moved = t.client.update(&t.source.id, &change).map_err(message)?;
    Ok(t.target_parent.child(&moved.id, &moved.name).to_uri())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::cloud::accounts::{Accounts, DriveAccount};
    use crate::cloud::client::ClientStore;
    use crate::cloud::oauth::SCOPE_FULL;
    use crate::cloud::test_server::{Log, Reply, serve_replies};
    use crate::cloud::token::{FileTokenStore, Token, TokenStore};

    pub(crate) const ROOT: &str = "gdrive://acct/root~My%20Drive";
    const FOLDER: &str = "application/vnd.google-apps.folder";

    pub(crate) fn state() -> (CloudState, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let mut accounts = Accounts::load(dir.path().join("accounts.json"));
        accounts
            .upsert(DriveAccount::new(
                "acct".into(),
                "me@example.com".into(),
                "Me".into(),
                "root".into(),
                vec![SCOPE_FULL.into()],
                0,
            ))
            .unwrap();
        let tokens = FileTokenStore::new(dir.path().join("tokens"));
        let token = Token::new("access".into(), None, crate::recent::now() + 3600, vec![]);
        tokens.save("acct", &token).unwrap();
        let client = ClientStore::new(dir.path().join("google-client.json"));
        (
            CloudState::with_store(accounts, Box::new(tokens), client),
            dir,
        )
    }

    fn no_refresh(_: &str) -> Result<Token, String> {
        Err("no refresh in tests".into())
    }

    pub(crate) fn ctx<'a>(state: &'a CloudState, base: &'a str) -> Ctx<'a> {
        Ctx {
            state,
            base,
            refresh: &no_refresh,
        }
    }

    pub(crate) fn file(id: &str, name: &str) -> String {
        format!(r#"{{"id":"{id}","name":"{name}","mimeType":"text/plain","size":"5"}}"#)
    }

    pub(crate) fn folder(id: &str, name: &str) -> String {
        format!(r#"{{"id":"{id}","name":"{name}","mimeType":"{FOLDER}"}}"#)
    }

    pub(crate) fn found(items: &[String]) -> Reply {
        Reply::json(200, &format!(r#"{{"files":[{}]}}"#, items.join(",")))
    }

    pub(crate) fn ok(json: String) -> Reply {
        Reply::json(200, &json)
    }

    pub(crate) fn run<T>(replies: Vec<Reply>, work: impl FnOnce(&Ctx) -> T) -> (T, Log) {
        let (base, log, server) = serve_replies(replies);
        let (state, _dir) = state();
        let result = work(&ctx(&state, &base));
        server.join().unwrap();
        (result, log)
    }

    pub(crate) fn heads(log: &Log) -> Vec<String> {
        log.lock().unwrap().iter().map(|r| r.head.clone()).collect()
    }

    #[test]
    fn make_dir_creates_a_folder_and_returns_its_uri() {
        let (result, _) = run(vec![found(&[]), ok(folder("new", "Docs"))], |c| {
            make_dir_in(c, &format!("{ROOT}/~Docs"))
        });
        assert_eq!(result.unwrap(), format!("{ROOT}/new~Docs"));
    }

    #[test]
    fn make_dir_refuses_an_existing_name() {
        let (result, log) = run(vec![found(&[folder("d1", "Docs")])], |c| {
            make_dir_in(c, &format!("{ROOT}/~Docs"))
        });
        assert_eq!(
            result.unwrap_err(),
            "A folder with that name already exists."
        );
        assert_eq!(heads(&log).len(), 1);
    }

    #[test]
    fn rename_keeps_the_id_and_returns_the_new_uri() {
        let (result, log) = run(vec![found(&[]), ok(file("f1", "b.txt"))], |c| {
            rename_in(c, &format!("{ROOT}/f1~a.txt"), &format!("{ROOT}/~b.txt"))
        });
        assert_eq!(result.unwrap(), format!("{ROOT}/f1~b.txt"));
        let log = log.lock().unwrap();
        assert!(
            log[1].head.starts_with("PATCH /files/f1?"),
            "{}",
            log[1].head
        );
        assert_eq!(log[1].body_text(), r#"{"name":"b.txt"}"#);
    }

    #[test]
    fn rename_refuses_an_existing_name() {
        let (result, _) = run(vec![found(&[file("f2", "b.txt")])], |c| {
            rename_in(c, &format!("{ROOT}/f1~a.txt"), &format!("{ROOT}/~b.txt"))
        });
        assert_eq!(
            result.unwrap_err(),
            "A file or folder with that name already exists."
        );
    }

    #[test]
    fn rename_refuses_a_local_path_on_either_side() {
        let (result, log) = run(vec![], |c| {
            (
                rename_in(c, "/Users/me/a.txt", &format!("{ROOT}/~b.txt")),
                rename_in(c, &format!("{ROOT}/f1~a.txt"), "/Users/me/b.txt"),
            )
        });
        assert!(result.0.is_err() && result.1.is_err());
        assert!(heads(&log).is_empty());
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
    fn a_move_with_replace_trashes_the_old_item_first() {
        let (result, log) = run(
            vec![
                ok(file("f1", "a.txt")),
                found(&[file("old", "a.txt")]),
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
        assert!(log[2].head.starts_with("PATCH /files/old?"));
        assert!(log[3].head.starts_with("PATCH /files/f1?"));
    }

    #[test]
    fn trash_and_restore_flip_the_trashed_flag() {
        let path = format!("{ROOT}/f1~a.txt");
        let (result, log) = run(
            vec![ok(file("f1", "a.txt")), ok(file("f1", "a.txt"))],
            |c| (trash_in(c, &path), restore_in(c, &path)),
        );
        assert_eq!(result.0.unwrap(), Some(path.clone()));
        assert_eq!(result.1.unwrap(), path);
        let log = log.lock().unwrap();
        assert_eq!(log[0].body_text(), r#"{"trashed":true}"#);
        assert_eq!(log[1].body_text(), r#"{"trashed":false}"#);
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
}
