//! What tests of Drive's code share, in this crate and in the app (the
//! `testing` feature): a connected account with a fresh token, a context on
//! the fake server (`test_server`), Drive's replies, and the requests a test
//! sent.

use places::Progress;

use crate::CloudState;
use crate::accounts::{Accounts, DriveAccount};
use crate::client::ClientStore;
use crate::oauth::SCOPE_FULL;
use crate::test_server::{Log, Reply, serve_replies};
use crate::token::{FileTokenStore, Token, TokenStore};
use crate::write::Ctx;

pub const ROOT: &str = "gdrive://acct/root~My%20Drive";
const FOLDER: &str = "application/vnd.google-apps.folder";

pub fn state() -> (CloudState, tempfile::TempDir) {
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
    let token = Token::new("access".into(), None, places::now() + 3600, vec![]);
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

pub fn ctx<'a>(state: &'a CloudState, base: &'a str) -> Ctx<'a> {
    Ctx::new(state, base, &no_refresh, Progress::default())
}

pub fn file(id: &str, name: &str) -> String {
    format!(r#"{{"id":"{id}","name":"{name}","mimeType":"text/plain","size":"5"}}"#)
}

pub fn folder(id: &str, name: &str) -> String {
    format!(r#"{{"id":"{id}","name":"{name}","mimeType":"{FOLDER}"}}"#)
}

pub fn found(items: &[String]) -> Reply {
    Reply::json(200, &format!(r#"{{"files":[{}]}}"#, items.join(",")))
}

pub fn ok(json: String) -> Reply {
    Reply::json(200, &json)
}

pub fn run<T>(replies: Vec<Reply>, work: impl FnOnce(&Ctx) -> T) -> (T, Log) {
    run_with(replies, &Progress::default(), work)
}

/// `run` with the operation reporting to `progress`.
pub fn run_with<T>(
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

/// The head of each request the server got, in order.
pub fn heads(log: &Log) -> Vec<String> {
    log.lock().unwrap().iter().map(|r| r.head.clone()).collect()
}
