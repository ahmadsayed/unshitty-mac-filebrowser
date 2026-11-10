//! Sharing a collection: add an Arbesk account as a viewer or an editor, by
//! the email it signed in with. Arbesk keeps the editor list on IPFS and only
//! its Merkle root on chain; the Studio (and `besk`) add a member by resolving
//! the email, uploading the list under a new root, and relaying
//! `updateEditors` with the caller's proof for the current tree.

use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::chain::Site;
use crate::decompose::stored_json;
use crate::http::{Http, HttpError};
use crate::merkle;
use crate::service::Reader;
use crate::write::{BUSY, CollectionLocks, Outcome, Uploader};

/// No session, an expired one, or Arbesk's 401.
pub const SIGN_IN: &str = "Sign in to Arbesk again to share a collection.";
/// The caller isn't an editor of this collection (or the relay refused it).
pub const FORBIDDEN: &str = "Only an editor can share this collection.";
/// Arbesk's email lookup isn't configured on this deployment.
pub const LOOKUP_UNAVAILABLE: &str = "Email lookup is not available on this Arbesk server.";
pub const NO_ACCOUNT: &str =
    "No Arbesk account found for that email. They need to sign in to Arbesk at least once.";
pub const LOOKUP_FORBIDDEN: &str = "Arbesk refused to look up that email.";
pub const NO_WALLET: &str = "That Arbesk account has no wallet yet.";
pub const UNREADABLE: &str = "The collection's editor list can't be read.";
const UNREADABLE_ANSWER: &str = "Arbesk sent an answer File Browser can't read.";
pub const NO_LIST: &str = "This collection has no editor list.";
pub const BAD_ROLE: &str = "Choose Viewer or Editor.";
/// The relay's 502: most often an expired CDP delegation.
pub const RELAY_FAILED: &str =
    "Arbesk couldn't save the collaborator. Sign in again to renew its permission.";

/// One entry of a collection's editor list, as asset-core writes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Editor {
    pub address: String,
    pub role: u8,
    /// The email the member was invited by (display only, for Arbesk's own
    /// email sign-in); absent for wallet-only accounts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
}

/// What sharing did: the member's address and role, and whether it was added
/// (`false` when an existing member's role changed).
pub use places::Shared;

fn already(role: u8) -> String {
    format!("That account is already a{}.", if role == merkle::EDITOR { "n editor" } else { " viewer" })
}

fn still_saving() -> String {
    "Arbesk is still saving the collaborator. Try again in a minute.".into()
}

/// Add the account behind `email` to `token_id`'s editor list as `role`
/// (`merkle::VIEWER` or `merkle::EDITOR`), or change an existing member's
/// role. `caller` is the session's wallet address, which must be an editor.
#[allow(clippy::too_many_arguments)]
pub fn add(
    http: &dyn Http,
    site: &Site,
    session: &str,
    caller: &str,
    token_id: &str,
    email: &str,
    role: u8,
    cache_dir: &Path,
    locks: &CollectionLocks,
) -> Result<Shared, String> {
    if role != merkle::VIEWER && role != merkle::EDITOR {
        return Err(BAD_ROLE.into());
    }
    let address = resolve_email(http, site, session, email)?;
    // One editor-list change at a time per collection from this app, so two
    // shares don't both build on the version before the other lands.
    let _held = locks.take(caller, token_id);
    let reader = Reader::open_fresh(http, site, cache_dir)?;
    let (version, _, value) = reader.editor_list(token_id)?;
    let mut list: Vec<Editor> =
        serde_json::from_value(value).map_err(|_| UNREADABLE.to_string())?;
    if list.is_empty() {
        return Err(NO_LIST.into());
    }
    if list.len() > merkle::MAX_EDITORS {
        return Err("This collection has reached its editor limit.".into());
    }
    let caller_index = list
        .iter()
        .position(|editor| editor.address.eq_ignore_ascii_case(caller))
        .ok_or(FORBIDDEN)?;
    if list[caller_index].role != merkle::EDITOR {
        return Err(FORBIDDEN.into());
    }
    let caller_role = list[caller_index].role;
    let old_leaves = leaves(&list, token_id, version)?;
    let caller_proof: Vec<String> = merkle::proof(&old_leaves, caller_index)
        .iter()
        .map(|node| merkle::hex(node))
        .collect();

    if address.eq_ignore_ascii_case(caller) {
        // Never let the caller turn their own editor role off: the last
        // editor doing that would leave the collection with no one to manage it.
        return Err(if role == merkle::EDITOR {
            already(role)
        } else {
            "You can't change your own role.".into()
        });
    }
    let member = list
        .iter()
        .position(|editor| editor.address.eq_ignore_ascii_case(&address));
    let added = match member {
        Some(index) if list[index].role == role => return Err(already(role)),
        Some(index) => {
            list[index].role = role;
            false
        }
        None if list.len() >= merkle::MAX_EDITORS => {
            return Err("This collection has reached its editor limit.".into());
        }
        None => {
            list.push(Editor {
                address: address.clone(),
                role,
                email: Some(email.trim().to_ascii_lowercase()),
            });
            true
        }
    };

    let next_version = version
        .checked_add(1)
        .ok_or("The collection's editor version is too large.")?;
    let new_leaves = leaves(&list, token_id, next_version)?;
    let new_root = merkle::hex(&merkle::root(&new_leaves));
    let mut uploader = Uploader::new(http, site, session, 1, "the editor list")?;
    let list = serde_json::to_value(&list).map_err(|_| UNREADABLE.to_string())?;
    let list_cid = uploader.upload(
        &format!("editors_token_{token_id}_v{next_version}.json"),
        &stored_json(&list, true),
    )?;
    let outcome = relay(
        http,
        site,
        session,
        token_id,
        &new_root,
        &list_cid,
        caller_role,
        &caller_proof,
    );
    match outcome {
        Outcome::Landed => Ok(Shared {
            address,
            role,
            added,
        }),
        Outcome::Unsettled(error) | Outcome::Refused(error) => Err(error),
    }
}

/// The account `email` belongs to, as Arbesk resolves it.
pub fn resolve_email(
    http: &dyn Http,
    site: &Site,
    session: &str,
    email: &str,
) -> Result<String, String> {
    let email = email.trim().to_ascii_lowercase();
    let valid = !email.is_empty()
        && email.len() <= 254
        && !email.chars().any(char::is_whitespace)
        && matches!(email.split_once('@'), Some((local, domain)) if !local.is_empty() && domain.contains('.'));
    if !valid {
        return Err("Enter an email address.".into());
    }
    let url = format!("{}/api/v1/users/resolve-email", site.origin);
    let answer = http
        .post_json_slow(&url, &json!({ "email": email }), Some(session))
        .map_err(|error| match error.status {
            Some(401) => SIGN_IN.to_string(),
            Some(403) => LOOKUP_FORBIDDEN.to_string(),
            Some(429) => BUSY.to_string(),
            Some(503) => LOOKUP_UNAVAILABLE.to_string(),
            _ => format!("Couldn't look up that email: {}", error.message),
        })?;
    let body: Value = serde_json::from_slice(&answer).map_err(|_| UNREADABLE_ANSWER.to_string())?;
    if body["exists"] != true {
        return Err(NO_ACCOUNT.into());
    }
    let address = body["address"]
        .as_str()
        .filter(|address| merkle::address_bytes(address).is_ok())
        .ok_or(NO_WALLET)?;
    Ok(address.to_ascii_lowercase())
}

fn leaves(list: &[Editor], token_id: &str, version: u64) -> Result<Vec<[u8; 32]>, String> {
    list.iter()
        .map(|editor| merkle::leaf(&editor.address, editor.role, token_id, version))
        .collect()
}

/// Relay the new root and list URI, with the caller's proof for the current
/// tree. The contract bumps the version itself.
#[allow(clippy::too_many_arguments)]
fn relay(
    http: &dyn Http,
    site: &Site,
    session: &str,
    token_id: &str,
    new_root: &str,
    list_cid: &str,
    caller_role: u8,
    caller_proof: &[String],
) -> Outcome {
    let body = json!({
        "op": "updateEditors",
        "tokenId": token_id,
        // The relay authorizes from the top level, the contract call from
        // `params`; both need the proof for a caller who isn't the owner.
        "requiredRole": caller_role,
        "proof": caller_proof,
        "params": {
            "newRoot": new_root,
            "newListUri": list_cid,
            "callerRole": caller_role,
            "callerProof": caller_proof,
        },
    });
    let url = format!("{}/api/v1/wallet/relay", site.origin);
    match http.post_json_slow(&url, &body, Some(session)) {
        Ok(answer) => receipt_outcome(&answer),
        Err(error) => relay_failure(error),
    }
}

fn receipt_outcome(answer: &[u8]) -> Outcome {
    let answer: Value = serde_json::from_slice(answer).unwrap_or_default();
    match answer["receipt"]["status"].as_bool() {
        Some(true) => Outcome::Landed,
        Some(false) => Outcome::Refused("Arbesk couldn't save the collaborator.".into()),
        None => Outcome::Unsettled(still_saving()),
    }
}

fn relay_failure(error: HttpError) -> Outcome {
    match error.status {
        Some(401) => Outcome::Refused(SIGN_IN.into()),
        Some(403) => Outcome::Refused(FORBIDDEN.into()),
        Some(429) => Outcome::Refused(BUSY.into()),
        Some(400..=499) => Outcome::Refused(format!(
            "Arbesk couldn't save the collaborator: {}",
            error.message
        )),
        Some(502) => Outcome::Unsettled(RELAY_FAILED.into()),
        Some(_) | None => Outcome::Unsettled(still_saving()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::fake::FakeHttp;
    use crate::testing::{self, ADDRESS, ALICE, arbesk, site, world};
    use std::sync::{Arc, Mutex};

    const SESSION: &str = "s3ss10n";
    const TOKEN: &str = "1";

    fn signed_in() -> (Arc<Mutex<testing::World>>, FakeHttp, tempfile::TempDir) {
        let mut w = world();
        w.sessions.insert(SESSION.into());
        let state = Arc::new(Mutex::new(w));
        (state.clone(), arbesk(state), tempfile::tempdir().unwrap())
    }

    fn share(
        http: &dyn Http,
        dir: &std::path::Path,
        email: &str,
        role: u8,
    ) -> Result<Shared, String> {
        let locks = crate::write::CollectionLocks::default();
        add(
            http,
            &site(),
            SESSION,
            ADDRESS,
            TOKEN,
            email,
            role,
            dir,
            &locks,
        )
    }

    fn list(state: &Arc<Mutex<testing::World>>) -> Vec<Editor> {
        let world = state.lock().unwrap();
        let (cid, _) = &world.editor_lists[TOKEN];
        let bytes = crate::ipfs::unpack(world.files[cid].clone(), 1 << 20).unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[test]
    fn adds_a_viewer_by_email_and_updates_the_root() {
        let (state, http, dir) = signed_in();
        assert_eq!(
            share(&http, dir.path(), "Alice@Example.com", merkle::VIEWER).unwrap(),
            Shared {
                address: ALICE.into(),
                role: merkle::VIEWER,
                added: true
            }
        );
        let list = list(&state);
        assert_eq!(
            list,
            [
                Editor {
                    address: ADDRESS.into(),
                    role: merkle::EDITOR,
                    email: None
                },
                Editor {
                    address: ALICE.into(),
                    role: merkle::VIEWER,
                    email: Some("alice@example.com".into())
                }
            ]
        );
        let world = state.lock().unwrap();
        assert_eq!(world.editor_lists[TOKEN].1, 2);
        let relay = http
            .requests()
            .into_iter()
            .find(|request| request.url.ends_with("/api/v1/wallet/relay"))
            .unwrap();
        let body = relay.body.unwrap();
        assert_eq!(body["op"], "updateEditors");
        assert_eq!(body["params"]["newRoot"], "0xf58ca0e3ca724d9273d426fa2379359cce31792983aca8ae94f6ddef9dc4f18b");
        assert_eq!(body["params"]["newListUri"], "bafyUp0");
        assert_eq!(body["params"]["callerRole"], merkle::EDITOR);
        assert_eq!(body["params"]["callerProof"], json!([]));
        // The relay's own authorization reads the proof from the top level.
        assert_eq!(body["requiredRole"], merkle::EDITOR);
        assert_eq!(body["proof"], json!([]));
    }

    #[test]
    fn an_existing_members_role_can_change_without_adding_a_second_entry() {
        let (state, http, dir) = signed_in();
        share(&http, dir.path(), "alice@example.com", merkle::VIEWER).unwrap();
        assert_eq!(
            share(&http, dir.path(), "alice@example.com", merkle::EDITOR).unwrap(),
            Shared {
                address: ALICE.into(),
                role: merkle::EDITOR,
                added: false
            }
        );
        assert_eq!(list(&state).len(), 2);
        assert_eq!(list(&state)[1].role, merkle::EDITOR);
        assert_eq!(state.lock().unwrap().editor_lists[TOKEN].1, 3);
        assert_eq!(
            share(&http, dir.path(), "alice@example.com", merkle::EDITOR).unwrap_err(),
            "That account is already an editor."
        );
    }

    #[test]
    fn a_shared_editor_sends_a_proof_for_a_second_editor() {
        let (state, http, dir) = signed_in();
        {
            let mut world = state.lock().unwrap();
            world.put(
                "bafyEditorsShared",
                json!([
                    { "address": crate::testing::ALICE, "role": merkle::EDITOR },
                    { "address": ADDRESS, "role": merkle::EDITOR }
                ]),
            );
            world
                .editor_lists
                .insert(TOKEN.into(), ("bafyEditorsShared".into(), 1));
        }
        let shared = add(
            &http,
            &site(),
            SESSION,
            crate::testing::ALICE,
            TOKEN,
            "bob@example.com",
            merkle::VIEWER,
            dir.path(),
            &crate::write::CollectionLocks::default(),
        )
        .unwrap();
        assert_eq!(shared.address, crate::testing::BOB);
        let relay = http
            .requests()
            .into_iter()
            .find(|request| request.url.ends_with("/api/v1/wallet/relay"))
            .unwrap();
        let body = relay.body.unwrap();
        assert_eq!(body["requiredRole"], merkle::EDITOR);
        assert_eq!(body["proof"].as_array().unwrap().len(), 1);
        assert_eq!(body["params"]["callerProof"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn the_caller_cannot_change_their_own_role() {
        let (_state, http, dir) = signed_in();
        assert_eq!(
            share(&http, dir.path(), "me@example.com", merkle::VIEWER).unwrap_err(),
            "You can't change your own role."
        );
        assert_eq!(
            share(&http, dir.path(), "me@example.com", merkle::EDITOR).unwrap_err(),
            "That account is already an editor."
        );
    }

    #[test]
    fn an_unknown_email_is_refused_before_any_upload() {
        let (_state, http, dir) = signed_in();
        assert_eq!(
            share(&http, dir.path(), "nobody@example.com", merkle::VIEWER).unwrap_err(),
            NO_ACCOUNT
        );
        assert!(http
            .requests()
            .iter()
            .all(|request| !request.url.contains("/upload-urls")));
        for bad in ["not-an-email", "a@b", "a @b.com"] {
            assert_eq!(
                share(&http, dir.path(), bad, merkle::VIEWER).unwrap_err(),
                "Enter an email address."
            );
        }
        assert_eq!(
            share(&http, dir.path(), "alice@example.com", 0).unwrap_err(),
            BAD_ROLE
        );
    }

    #[test]
    fn only_an_editor_can_share() {
        let (state, http, dir) = signed_in();
        {
            let mut world = state.lock().unwrap();
            world.put(
                "bafyEditorsViewer",
                json!([{ "address": ADDRESS, "role": merkle::VIEWER }]),
            );
            world
                .editor_lists
                .insert(TOKEN.into(), ("bafyEditorsViewer".into(), 1));
        }
        assert_eq!(
            share(&http, dir.path(), "alice@example.com", merkle::VIEWER).unwrap_err(),
            FORBIDDEN
        );
        assert!(http
            .requests()
            .iter()
            .all(|request| !request.url.contains("/upload-urls")));
    }

    #[test]
    fn a_collection_without_an_editor_list_is_refused() {
        let (state, http, dir) = signed_in();
        state.lock().unwrap().editor_lists.remove(TOKEN);
        assert_eq!(
            share(&http, dir.path(), "alice@example.com", merkle::VIEWER).unwrap_err(),
            NO_LIST
        );
    }

    #[test]
    fn a_missing_session_asks_to_sign_in() {
        let dir = tempfile::tempdir().unwrap();
        let mut w = world();
        w.sessions.clear();
        let state = Arc::new(Mutex::new(w));
        let http = arbesk(state);
        assert_eq!(
            share(&http, dir.path(), "alice@example.com", merkle::VIEWER).unwrap_err(),
            SIGN_IN
        );
    }

    #[test]
    fn a_relay_that_stalls_or_fails_is_an_error() {
        for (relay, expected) in [
            (testing::Relay::Pending, still_saving()),
            (testing::Relay::Failed, "Arbesk couldn't save the collaborator.".to_string()),
        ] {
            let (state, http, dir) = signed_in();
            state.lock().unwrap().relay = relay;
            assert_eq!(
                share(&http, dir.path(), "alice@example.com", merkle::VIEWER).unwrap_err(),
                expected
            );
            // The list was uploaded, but the chain didn't move.
            assert_eq!(state.lock().unwrap().editor_lists[TOKEN].1, 1);
        }
    }

    #[test]
    fn a_relays_refusal_is_said_in_words() {
        for (status, expected) in [
            (401, SIGN_IN.to_string()),
            (403, FORBIDDEN.to_string()),
            (429, BUSY.to_string()),
        ] {
            let (state, http, dir) = signed_in();
            state.lock().unwrap().relay = testing::Relay::Status(status);
            assert_eq!(
                share(&http, dir.path(), "alice@example.com", merkle::VIEWER).unwrap_err(),
                expected,
                "{status}"
            );
        }
    }
}


