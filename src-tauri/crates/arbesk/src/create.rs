//! Creating a named collection, as the Studio's New Collection and `besk
//! create` do: the token id is the hash of the account and the name, the
//! first collection manifest and the one-editor list go to IPFS, and the
//! wallet relay mints the token (`publish`). Ids decide, as everywhere in
//! Arbesk, so a name already minted for the account is one collection.

use std::path::Path;

use serde_json::{Value, json};
use sha3::{Digest, Keccak256};

use crate::chain::{self, Site};
use crate::decompose::{sanitize_file_name, stored_json};
use crate::http::{Http, HttpError};
use crate::location::collection_path;
use crate::service::Reader;
use crate::signin::now_ms;
use crate::write::{BUSY, Outcome, Uploader};

/// No session, an expired one, or Arbesk's 401.
pub const SIGN_IN: &str = "Sign in to Arbesk again to create a collection.";
pub const NO_NAME: &str = "Enter a name for the collection.";
pub const FORBIDDEN: &str = "You can't create collections in this account.";
/// The relay's 502: most often an expired CDP delegation.
pub const RELAY_FAILED: &str =
    "Arbesk couldn't create the collection. Sign in again to renew its permission.";

/// The role the account has in a new collection: `CollaboratorRole.Editor`.
const EDITOR_ROLE: u8 = 2;

fn exists(name: &str) -> String {
    format!("A collection named “{name}” already exists.")
}

fn not_created(name: &str) -> String {
    format!("Arbesk couldn't create “{name}”.")
}

fn still_creating(name: &str) -> String {
    format!("Arbesk is still creating “{name}”. It appears once the change is confirmed.")
}

/// Create the named collection of `address`; the token id is decimal, and
/// Arbesk's answer is what `Client::create_collection` returns.
pub fn create(
    http: &dyn Http,
    site: &Site,
    session: &str,
    address: &str,
    name: &str,
    cache_dir: &Path,
) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err(NO_NAME.into());
    }
    let token_id = named_collection_id(address, name)?;
    let reader = Reader::open(http, site, cache_dir)?;
    match reader.collection_manifest_with_cid(&token_id) {
        Ok(_) => return Err(exists(name)),
        // `tokenURI` reverts for a token that was never minted.
        Err(error) if error.starts_with(chain::TOKEN_REFUSED) => {}
        Err(error) => return Err(error),
    }
    let manifest = collection_manifest(name);
    let asset_id = manifest["asset_id"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    let editor_list = json!([{ "address": address.to_ascii_lowercase(), "role": EDITOR_ROLE }]);
    let editor_root = editor_root(address, &token_id)?;
    let mut uploader = Uploader::new(http, site, session, 2, name)?;
    let collection_cid = uploader.upload(
        &format!("collect_{}.json", sanitize_file_name(&asset_id)),
        &stored_json(&manifest, false),
    )?;
    let editor_list_cid = uploader.upload(
        &format!("editors_token_{token_id}_v1.json"),
        &stored_json(&editor_list, true),
    )?;
    match publish(
        http,
        site,
        session,
        name,
        &token_id,
        &collection_cid,
        &editor_root,
        &editor_list_cid,
    ) {
        Outcome::Landed => Ok(collection_path(address, &token_id, name)),
        Outcome::Unsettled(error) | Outcome::Refused(error) => Err(error),
    }
}

/// The canonical first collection manifest (asset-core's
/// `buildCollectionManifest`).
fn collection_manifest(name: &str) -> Value {
    let timestamp = now_ms();
    json!({
        "type": "collection",
        "name": name,
        "asset_id": format!("collection_{timestamp}"),
        "version": 1,
        "timestamp": timestamp,
        "assets": {},
        "prev_asset_manifest_cid": null,
    })
}

/// The token id of the account's collection named `name`:
/// `keccak256(abi.encodePacked(address, string))` as a decimal number, as
/// asset-core's `deriveNamedCollectionId` makes it.
pub fn named_collection_id(address: &str, name: &str) -> Result<String, String> {
    let mut packed = address_bytes(address)?.to_vec();
    packed.extend_from_slice(name.as_bytes());
    Ok(chain::decimal(&keccak(&packed)))
}

/// The one-editor Merkle root for a new collection: a single leaf is the
/// root (`computeRoot([...], token, 1)`).
pub fn editor_root(address: &str, token_id: &str) -> Result<String, String> {
    let mut leaf = address_bytes(address)?.to_vec();
    leaf.push(EDITOR_ROLE);
    leaf.extend_from_slice(&chain::uint256_word(token_id)?);
    // Collection-wide editor scope: bytes32(0).
    leaf.extend_from_slice(&[0; 32]);
    // Editor-set version 1.
    leaf.extend_from_slice(&chain::uint256_word("1")?);
    Ok(hex(&keccak(&leaf)))
}

/// A wallet address's 20 bytes.
fn address_bytes(address: &str) -> Result<[u8; 20], String> {
    let digits = address
        .strip_prefix("0x")
        .filter(|digits| digits.len() == 40)
        .ok_or_else(|| format!("“{address}” is not a wallet address."))?;
    let mut bytes = [0u8; 20];
    for (i, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&digits[i * 2..i * 2 + 2], 16)
            .map_err(|_| format!("“{address}” is not a wallet address."))?;
    }
    Ok(bytes)
}

fn keccak(bytes: &[u8]) -> [u8; 32] {
    Keccak256::digest(bytes).into()
}

fn hex(bytes: &[u8]) -> String {
    let mut hex = String::with_capacity(2 + bytes.len() * 2);
    hex.push_str("0x");
    for byte in bytes {
        hex.push_str(&format!("{byte:02x}"));
    }
    hex
}

/// Mint the collection through Arbesk's wallet relay, as the Studio's
/// `publishAsset` does: the session's account is its first editor.
#[allow(clippy::too_many_arguments)]
fn publish(
    http: &dyn Http,
    site: &Site,
    session: &str,
    name: &str,
    token_id: &str,
    collection_cid: &str,
    editor_root: &str,
    editor_list_cid: &str,
) -> Outcome {
    let body = json!({
        "op": "publish",
        "tokenId": token_id,
        "params": {
            "uri": collection_cid,
            "editorRoot": editor_root,
            "editorListUri": editor_list_cid,
        },
    });
    let url = format!("{}/api/v1/wallet/relay", site.origin);
    match http.post_json_slow(&url, &body, Some(session)) {
        Ok(answer) => receipt_outcome(&answer, name),
        Err(error) => relay_failure(error, name),
    }
}

/// What a relay's receipt says: `status` true mined, false reverted, null
/// (or no receipt to read) not known yet.
fn receipt_outcome(answer: &[u8], name: &str) -> Outcome {
    let answer: Value = serde_json::from_slice(answer).unwrap_or_default();
    match answer["receipt"]["status"].as_bool() {
        Some(true) => Outcome::Landed,
        Some(false) => Outcome::Refused(not_created(name)),
        None => Outcome::Unsettled(still_creating(name)),
    }
}

/// What a failed relay request means. Only a 4xx is a refusal before
/// anything was sent; anything else may still land.
fn relay_failure(error: HttpError, name: &str) -> Outcome {
    match error.status {
        Some(401) => Outcome::Refused(SIGN_IN.into()),
        Some(403) => Outcome::Refused(FORBIDDEN.into()),
        Some(409) => Outcome::Refused(exists(name)),
        Some(429) => Outcome::Refused(BUSY.into()),
        Some(400..=499) => Outcome::Refused(format!(
            "Arbesk couldn't create the collection: {}",
            error.message
        )),
        Some(502) => Outcome::Unsettled(RELAY_FAILED.into()),
        Some(_) => Outcome::Unsettled(still_creating(name)),
        None => Outcome::Unsettled(format!("Couldn't create “{name}”: {}", error.message)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::fake::FakeHttp;
    use crate::testing::{self, ADDRESS, arbesk, site, world};
    use std::sync::{Arc, Mutex};

    const SESSION: &str = "s3ss10n";

    fn signed_in() -> (Arc<Mutex<testing::World>>, FakeHttp, tempfile::TempDir) {
        let mut w = world();
        w.sessions.insert(SESSION.into());
        let state = Arc::new(Mutex::new(w));
        (state.clone(), arbesk(state), tempfile::tempdir().unwrap())
    }

    fn created(http: &dyn Http, dir: &std::path::Path) -> Result<String, String> {
        create(http, &site(), SESSION, ADDRESS, "My Set", dir)
    }

    #[test]
    fn a_named_collection_id_is_the_hash_of_the_account_and_name() {
        // asset-core's `deriveNamedCollectionId`, via viem.
        assert_eq!(
            named_collection_id(ADDRESS, "My Set").unwrap(),
            "22573643048518722397328738209259211178527102262436760332319665845902767721290"
        );
        assert_eq!(
            named_collection_id(ADDRESS, "Shapes").unwrap(),
            "88876693295894923046239674319859492167942286336121656163869179723618577151363"
        );
        // The name is hashed as given; trimming is `create`'s job.
        assert_ne!(
            named_collection_id(ADDRESS, "Shapes ").unwrap(),
            named_collection_id(ADDRESS, "Shapes").unwrap()
        );
    }

    #[test]
    fn a_wallet_address_is_twenty_bytes() {
        assert!(address_bytes(ADDRESS).is_ok());
        for bad in [
            "",
            "0x",
            "0X00000000000000000000000000000000000000ab",
            "0x1234",
            "0xzz000000000000000000000000000000000000ab",
        ] {
            assert!(address_bytes(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_new_collections_editor_root_is_its_one_editor_leaf() {
        // keccak256(abi.encodePacked(address, uint8 2, uint256 token, bytes32 0, uint256 1)).
        assert_eq!(
            editor_root(
                ADDRESS,
                "22573643048518722397328738209259211178527102262436760332319665845902767721290"
            )
            .unwrap(),
            "0x6c081bedc9d6258f65f62c8ea77ab875934a0be7c921203a44b683ff8ba8429f"
        );
    }

    #[test]
    fn the_first_collection_manifest_has_no_assets() {
        let manifest = collection_manifest("My Set");
        assert_eq!(manifest["type"], "collection");
        assert_eq!(manifest["name"], "My Set");
        assert_eq!(manifest["version"], 1);
        assert_eq!(manifest["assets"], json!({}));
        assert!(
            manifest["asset_id"]
                .as_str()
                .unwrap()
                .starts_with("collection_")
        );
        assert!(manifest["timestamp"].as_i64().is_some());
    }

    #[test]
    fn creating_a_collection_writes_the_manifest_the_editors_and_mints_it() {
        let (state, http, dir) = signed_in();
        let token_id = named_collection_id(ADDRESS, "My Set").unwrap();
        let path = created(&http, dir.path()).unwrap();
        assert_eq!(path, collection_path(ADDRESS, &token_id, "My Set"));

        let world = state.lock().unwrap();
        assert!(world.owned.contains(&token_id));
        let collection_cid = world.tokens[&token_id].clone();
        let manifest: Value = serde_json::from_slice(&world.files[&collection_cid]).unwrap();
        assert_eq!(manifest["type"], "collection");
        assert_eq!(manifest["name"], "My Set");
        assert_eq!(manifest["version"], 1);
        assert_eq!(manifest["assets"], json!({}));
        let editor_list = crate::ipfs::unpack(world.files["bafyUp1"].clone(), 1 << 20).unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&editor_list).unwrap(),
            json!([{ "address": ADDRESS, "role": 2 }])
        );

        let relay = http
            .requests()
            .into_iter()
            .find(|request| request.url.ends_with("/api/v1/wallet/relay"))
            .unwrap();
        let body = relay.body.unwrap();
        assert_eq!(body["op"], "publish");
        assert_eq!(body["tokenId"], token_id.as_str());
        assert_eq!(body["params"]["uri"], collection_cid);
        assert_eq!(body["params"]["editorListUri"], "bafyUp1");
        assert_eq!(
            body["params"]["editorRoot"],
            editor_root(ADDRESS, &token_id).unwrap()
        );
    }

    #[test]
    fn a_name_already_minted_is_refused_before_any_upload() {
        let (state, http, dir) = signed_in();
        let token_id = named_collection_id(ADDRESS, "My Set").unwrap();
        {
            let mut world = state.lock().unwrap();
            world.tokens.insert(token_id.clone(), "bafyExisting".into());
            world.put(
                "bafyExisting",
                json!({ "type": "collection", "name": "My Set" }),
            );
        }
        assert_eq!(
            created(&http, dir.path()).unwrap_err(),
            "A collection named “My Set” already exists."
        );
        assert!(
            http.requests()
                .iter()
                .all(|request| !request.url.contains("/upload-urls"))
        );
    }

    #[test]
    fn a_name_is_trimmed_and_an_empty_one_is_refused() {
        let (_state, http, dir) = signed_in();
        assert_eq!(
            create(&http, &site(), SESSION, ADDRESS, "", dir.path()).unwrap_err(),
            NO_NAME
        );
        assert_eq!(
            create(&http, &site(), SESSION, ADDRESS, "   ", dir.path()).unwrap_err(),
            NO_NAME
        );
        let path = create(&http, &site(), SESSION, ADDRESS, "  My Set  ", dir.path()).unwrap();
        assert!(path.ends_with("~My%20Set"), "{path}");
    }

    #[test]
    fn a_missing_session_asks_to_sign_in_again() {
        let mut w = world();
        w.sessions.clear();
        let state = Arc::new(Mutex::new(w));
        let http = arbesk(state);
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            create(&http, &site(), "gone", ADDRESS, "My Set", dir.path()).unwrap_err(),
            "Sign in to Arbesk again to copy files into it."
        );
    }

    #[test]
    fn a_relay_that_stalls_or_reverts_is_an_error() {
        for (relay, expected) in [
            (testing::Relay::Pending, still_creating("My Set")),
            (testing::Relay::Failed, not_created("My Set")),
        ] {
            let (state, http, dir) = signed_in();
            state.lock().unwrap().relay = relay;
            assert_eq!(created(&http, dir.path()).unwrap_err(), expected);
        }
    }

    #[test]
    fn a_relays_refusal_is_said_in_words() {
        for (status, expected) in [
            (401, SIGN_IN.to_string()),
            (403, FORBIDDEN.to_string()),
            (409, exists("My Set")),
            (429, BUSY.to_string()),
        ] {
            let (state, http, dir) = signed_in();
            state.lock().unwrap().relay = testing::Relay::Status(status);
            assert_eq!(
                created(&http, dir.path()).unwrap_err(),
                expected,
                "{status}"
            );
        }
    }
}
