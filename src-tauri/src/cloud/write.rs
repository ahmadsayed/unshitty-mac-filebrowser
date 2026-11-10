//! Changes inside Google Drive: New Folder, Rename, Move to Trash and Undo.
//! Destinations are pending paths (`…/~name`, see `location.rs`); each
//! operation returns the item's final URI. Copies and moves are the app's
//! transfer engine's, built from the primitives in `transfer.rs`.
//!
//! Data safety (spec D17): nothing is ever deleted. Undo uses Drive's Trash.

use std::cell::RefCell;
use std::collections::HashMap;

use crate::cloud::CloudState;
use crate::cloud::drive::{API_BASE, DriveClient, DriveError, DriveFile, FileChange};
use crate::cloud::location::DriveLocation;
use crate::cloud::service::{self, Refresh};
use crate::progress::Progress;

/// Where requests go and how tokens refresh; tests point it at a fake server.
/// One transfer runs on one context (`transfer.rs`): its primitives are
/// Drive's `places::Transfer`.
pub(crate) struct Ctx<'a> {
    pub(crate) state: &'a CloudState,
    pub(crate) base: &'a str,
    pub(crate) refresh: Refresh<'a>,
    /// Where copies count bytes and items, and check for a cancel.
    pub(crate) progress: Progress,
    /// What this transfer learnt of its items, by id, so a later step asks
    /// Drive nothing again (the source `landing` fetched, an item as a move
    /// left it).
    pub(crate) seen: RefCell<HashMap<String, DriveFile>>,
}

impl<'a> Ctx<'a> {
    pub(crate) fn new(state: &'a CloudState, base: &'a str, refresh: Refresh<'a>, progress: Progress) -> Self {
        Ctx { state, base, refresh, progress, seen: RefCell::default() }
    }
}

/// Run `work` with a production context.
pub(crate) fn live<T>(state: &CloudState, work: impl FnOnce(&Ctx) -> T) -> T {
    live_with(state, &Progress::default(), work)
}

/// `live`, reporting to `progress`.
pub(crate) fn live_with<T>(
    state: &CloudState,
    progress: &Progress,
    work: impl FnOnce(&Ctx) -> T,
) -> T {
    let refresh = service::google_refresh(state);
    work(&Ctx::new(state, API_BASE, &refresh, progress.clone()))
}

pub fn make_dir(state: &CloudState, path: &str) -> Result<String, String> {
    live(state, |c| make_dir_in(c, path))
}

pub fn rename(state: &CloudState, from: &str, to: &str) -> Result<String, String> {
    live(state, |c| rename_in(c, from, to))
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

pub(crate) fn id_of(location: &DriveLocation) -> Result<String, String> {
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

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::progress::Progress;
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
        Ctx::new(state, base, &no_refresh, Progress::default())
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
        run_with(replies, &Progress::default(), work)
    }

    /// `run` with the operation reporting to `progress`.
    pub(crate) fn run_with<T>(
        replies: Vec<Reply>,
        progress: &Progress,
        work: impl FnOnce(&Ctx) -> T,
    ) -> (T, Log) {
        let (base, log, server) = serve_replies(replies);
        let (state, _dir) = state();
        let mut c = ctx(&state, &base);
        c.progress = progress.clone();
        let result = work(&c);
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
}
