//! A fake Arbesk for tests: its config, token index, chain, IPFS gateway,
//! and the write side (upload URLs, the upload target, the wallet relay),
//! all served from one [`World`] through [`FakeHttp`].

use super::chain::Site;
use super::chain::testing::abi_string;
use super::http::HttpError;
use super::http::fake::{FakeHttp, Request, not_found};
use super::location::collection_path;
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, Mutex};

pub(crate) const ORIGIN: &str = "https://arbesk.test";
pub(crate) const ADDRESS: &str = "0x00000000000000000000000000000000000000ab";
pub(crate) const ALICE: &str = "0x1111111111111111111111111111111111111111";
pub(crate) const BOB: &str = "0x2222222222222222222222222222222222222222";
pub(crate) const STAMP_MS: i64 = 1_762_000_000_000;

pub(crate) fn site() -> Site {
    Site {
        origin: ORIGIN.into(),
        chain_id: 84532,
    }
}

/// What the fake Arbesk holds: IPFS content by CID, each token's
/// manifest CID, the index's answer, and CIDs whose fetch fails.
#[derive(Default)]
pub(crate) struct World {
    pub(crate) files: HashMap<String, Vec<u8>>,
    pub(crate) tokens: HashMap<String, String>,
    pub(crate) owned: Vec<String>,
    pub(crate) failing: HashSet<String>,
    pub(crate) rpc_down: bool,
    /// Tokens the chain answers only through this contract.
    pub(crate) only_via: HashMap<String, String>,
    /// Session tokens Arbesk accepts on its write endpoints.
    pub(crate) sessions: HashSet<String>,
    /// What the wallet relay does with a write.
    pub(crate) relay: Relay,
    /// Upload URLs are Kubo's API instead of pre-signed PUTs.
    pub(crate) kubo: bool,
    /// Arbesk's rate limit is reached: upload URLs answer 429.
    pub(crate) busy: bool,
    /// How many `tokenURI` reads after a confirmed relay still answer the
    /// token's CID from before it, as a node a block behind does.
    pub(crate) lag: usize,
    /// Tokens whose reads still lag: the CID before, and how many reads.
    pub(crate) lagging: HashMap<String, (String, usize)>,
    /// Upload URLs handed out so far; the next is `https://up.test/<minted>`.
    pub(crate) minted: usize,
    /// Upload URLs already used (a second use is refused, as Pinata does).
    pub(crate) spent: HashSet<usize>,
    /// Each token's editor list: its CID and current set version.
    pub(crate) editor_lists: HashMap<String, (String, u64)>,
    /// Emails Arbesk's `resolve-email` knows, and their wallet addresses.
    pub(crate) users: HashMap<String, String>,
}

/// How the fake wallet relay answers an accepted write.
#[derive(Default, Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Relay {
    /// Confirmed: the token's URI moves to the new one.
    #[default]
    Ok,
    /// A receipt with no outcome yet; nothing moves.
    Pending,
    /// A receipt that says the transaction failed; nothing moves.
    Failed,
    /// An HTTP error instead of a receipt.
    Status(u16),
}

pub(crate) fn fixture(case: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/arbesk")
        .join(case)
}

impl World {
    pub(crate) fn put(&mut self, cid: &str, value: Value) {
        self.files
            .insert(cid.into(), value.to_string().into_bytes());
    }

    /// A fixture's stored source and parts, under `cid` for the source.
    /// A source that holds all its parts has no `cids/` (git keeps no
    /// empty folder).
    pub(crate) fn put_fixture(&mut self, case: &str, cid: &str) {
        self.files.insert(
            cid.into(),
            std::fs::read(fixture(case).join("source.json")).unwrap(),
        );
        for part in std::fs::read_dir(fixture(case).join("cids"))
            .into_iter()
            .flatten()
            .flatten()
        {
            let name = part.file_name().to_string_lossy().into_owned();
            self.files.insert(name, std::fs::read(part.path()).unwrap());
        }
    }

    /// Gives the asset manifest at `asset` a thumbnail stored at `cid`, as
    /// the Studio saves one: a WebP picture and its metadata.
    pub(crate) fn put_thumbnail(&mut self, asset: &str, cid: &str, width: u32, height: u32) {
        let mut manifest: Value = serde_json::from_slice(&self.files[asset]).unwrap();
        manifest["thumbnail"] = json!({ "cid": cid, "mime": "image/webp", "path": "thumbnail.webp", "width": width, "height": height });
        self.put(asset, manifest);
        self.files.insert(cid.into(), webp(width, height));
    }

    /// An asset manifest that wraps the source at `source`.
    pub(crate) fn put_asset(&mut self, cid: &str, name: Option<&str>, source: &str) {
        let mut manifest = json!({ "type": "asset", "timestamp": STAMP_MS, "scene": { "nodes": [{ "source": { "cid": source } }] } });
        if let Some(name) = name {
            manifest["name"] = json!(name);
        }
        self.put(cid, manifest);
    }
}

/// Shapes (token 1): Box as glTF and as 3MF, a live link, an asset of an
/// unknown format and one whose manifest is gone; an untitled collection
/// (token 2); token 3 points at a manifest that can't be read.
pub(crate) fn world() -> World {
    let mut w = World::default();
    w.put_fixture("glb-box", "bafySrcGltf");
    w.put_fixture("3mf-box", "bafySrc3mf");
    w.put(
        "bafySrcOdd",
        json!({ "arbesk_format": "composite-example" }),
    );
    w.put_asset("bafyA1", Some("Box"), "bafySrcGltf");
    w.put_asset("bafyA2", Some("Box"), "bafySrc3mf");
    w.put_asset("bafyA4", Some("Odd"), "bafySrcOdd");
    w.put(
        "bafyA3",
        json!({ "name": "Linked", "scene": { "nodes": [{ "child_ref": { "assetID": "x" } }] } }),
    );
    w.put_thumbnail("bafyA1", "bafyThumb", 512, 384);
    w.put(
        "bafyC1",
        json!({ "type": "collection", "name": "Shapes", "timestamp": STAMP_MS, "assets": {
            "a1": "bafyA1", "a2": "bafyA2", "a3": "bafyA3", "a4": "bafyA4", "a5": "bafyGone"
        } }),
    );
    w.put(
        "bafyC2",
        json!({ "type": "collection", "assets": { "u1": "bafyA1" } }),
    );
    w.tokens.insert("1".into(), "bafyC1".into());
    w.tokens.insert("2".into(), "bafyC2".into());
    w.tokens.insert("3".into(), "bafyGone".into());
    w.owned = vec!["1".into(), "2".into(), "3".into()];
    // Shapes' editor list: the account alone, as the Studio starts one.
    w.put(
        "bafyEditors1",
        json!([{ "address": ADDRESS, "role": 2 }]),
    );
    w.editor_lists.insert("1".into(), ("bafyEditors1".into(), 1));
    w.users.insert("alice@example.com".into(), ALICE.into());
    w.users.insert("bob@example.com".into(), BOB.into());
    w.users.insert("me@example.com".into(), ADDRESS.into());
    // Rooms (token 4, not the account's own): a table scene, its own
    // model raised and scaled, and two links to Shapes' Box.
    w.put_fixture("gltf-image", "bafySrcTable");
    let link = |x: f64| {
        json!({ "transform_matrix": [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, x, 1.95, 0.0, 1.0],
        "child_ref": { "collection": { "chainId": 84532, "contractAddress": "0xLINK", "tokenId": "1" }, "assetID": "a1" } })
    };
    w.put("bafyTable", json!({ "name": "Table", "timestamp": STAMP_MS, "scene": { "nodes": [
        { "source": { "cid": "bafySrcTable", "format": "gltf" }, "transform_matrix": [4.0, 0.0, 0.0, 0.0, 0.0, 4.0, 0.0, 0.0, 0.0, 0.0, 4.0, 0.0, 0.0, 1.27, 0.0, 1.0] },
        link(-2.57), link(2.08),
        { "child_ref": { "collection": { "chainId": 1, "tokenId": "1" }, "assetID": "a1" } }
    ] } }));
    w.put(
        "bafyC4",
        json!({ "type": "collection", "name": "Rooms", "assets": { "t1": "bafyTable" } }),
    );
    w.tokens.insert("4".into(), "bafyC4".into());
    // A collection someone else owns, where the account is not an editor.
    w.put(
        "bafyEditors4",
        json!([{ "address": ALICE, "role": 2 }]),
    );
    w.editor_lists.insert("4".into(), ("bafyEditors4".into(), 1));
    // History (token 5): asset h1 saved three times, the first save a 3MF.
    let save = |source: &str, timestamp: i64, prev: Option<&str>| {
        let mut manifest = json!({ "name": "Chair", "timestamp": timestamp, "scene": { "nodes": [{ "source": { "cid": source } }] } });
        if let Some(prev) = prev {
            manifest["prev_asset_manifest_cid"] = json!(prev);
        }
        manifest
    };
    w.put("bafyH1", save("bafySrc3mf", 1000, None));
    w.put("bafyH2", save("bafySrcGltf", 2000, Some("bafyH1")));
    w.put("bafyH3", save("bafySrcGltf", 3000, Some("bafyH2")));
    w.put(
        "bafyC5",
        json!({ "type": "collection", "name": "History", "assets": { "h1": "bafyH3" } }),
    );
    w.tokens.insert("5".into(), "bafyC5".into());
    w
}

pub(crate) fn arbesk(world: Arc<Mutex<World>>) -> FakeHttp {
    FakeHttp::new(move |request| {
        let mut w = world.lock().unwrap();
        let url = request.url.as_str();
        if let Some(answer) = write_side(&mut w, request) {
            return answer;
        }
        if url == format!("{ORIGIN}/api/v1/config") {
            let config = json!({
                "ipfsGatewayUrl": "https://gw.test/ipfs/",
                "networkConfigs": { "84532": { "contractAddress": "0xC0", "rpcUrl": "https://rpc.test" } }
            });
            return Ok(config.to_string().into_bytes());
        }
        if url.starts_with(&format!("{ORIGIN}/api/v1/indexer/owned")) {
            return Ok(json!({ "owned": w.owned }).to_string().into_bytes());
        }
        if url.starts_with(&format!("{ORIGIN}/api/v1/indexer/shared")) {
            return Ok(json!({ "shared": [] }).to_string().into_bytes());
        }
        if url == format!("{ORIGIN}/api/v1/users/resolve-email") {
            signed_in(&w, request)?;
            let email = request
                .body
                .as_ref()
                .and_then(|body| body["email"].as_str())
                .unwrap_or_default();
            let answer = match w.users.get(email) {
                Some(address) => json!({ "exists": true, "address": address }),
                None => json!({ "exists": false }),
            };
            return Ok(answer.to_string().into_bytes());
        }
        if url == "https://rpc.test" && w.rpc_down {
            return Err(HttpError::new(None, "connection refused"));
        }
        if url == "https://rpc.test" {
            let call = &request.body.as_ref().unwrap()["params"][0];
            let data = call["data"].as_str().unwrap().to_string();
            let token = word_decimal(&data[data.len() - 64..]);
            let through = w
                .only_via
                .get(&token)
                .is_none_or(|contract| call["to"] == contract.as_str());
            let selector = &data[..10];
            if selector == "0x51d7d6e1" {
                let answer = match w.editor_lists.get(&token).filter(|_| through) {
                    Some((cid, _)) => {
                        json!({ "result": abi_string(&format!("ipfs://{cid}")) })
                    }
                    // A real token whose list was never set returns "".
                    None if w.tokens.contains_key(&token) => {
                        json!({ "result": abi_string("") })
                    }
                    None => json!({ "error": { "message": "execution reverted" } }),
                };
                return Ok(answer.to_string().into_bytes());
            }
            if selector == "0xb0f762fa" {
                let version = w
                    .editor_lists
                    .get(&token)
                    .filter(|_| through)
                    .map_or(0, |(_, version)| *version);
                let answer = json!({ "result": format!("0x{version:064x}") });
                return Ok(answer.to_string().into_bytes());
            }
            let stale = w.lagging.get_mut(&token).filter(|(_, reads)| *reads > 0);
            let stale = stale.map(|(cid, reads)| {
                *reads -= 1;
                cid.clone()
            });
            let answer = match stale.as_ref().or(w.tokens.get(&token)).filter(|_| through) {
                Some(cid) => json!({ "result": abi_string(&format!("ipfs://{cid}")) }),
                None => json!({ "error": { "message": "execution reverted" } }),
            };
            return Ok(answer.to_string().into_bytes());
        }
        let cid = url
            .strip_prefix("https://gw.test/ipfs/")
            .ok_or_else(not_found)?;
        if w.failing.contains(cid) {
            return Err(HttpError::new(Some(502), "Bad gateway"));
        }
        w.files.get(cid).cloned().ok_or_else(not_found)
    })
}

/// The write side: upload URLs, the upload target and the wallet relay;
/// `None` for any other request.
fn write_side(w: &mut World, request: &Request) -> Option<Result<Vec<u8>, HttpError>> {
    let url = request.url.as_str();
    if url == format!("{ORIGIN}/api/v1/ipfs/upload-urls") {
        return Some(upload_urls(w, request));
    }
    if let Some(n) = url.strip_prefix("https://up.test/") {
        return Some(upload(w, n.parse().ok()?, request));
    }
    if url == format!("{ORIGIN}/api/v1/wallet/relay") {
        return Some(wallet_relay(w, request));
    }
    None
}

fn json_answer(value: Value) -> Result<Vec<u8>, HttpError> {
    Ok(value.to_string().into_bytes())
}

fn signed_in(w: &World, request: &Request) -> Result<(), HttpError> {
    match &request.session {
        Some(token) if w.sessions.contains(token) => Ok(()),
        _ => Err(HttpError::new(
            Some(401),
            "The server answered 401 Unauthorized.",
        )),
    }
}

/// `{count}` upload credentials: pre-signed PUT URLs, or one reusable Kubo API.
fn upload_urls(w: &mut World, request: &Request) -> Result<Vec<u8>, HttpError> {
    signed_in(w, request)?;
    if w.busy {
        return Err(HttpError::new(
            Some(429),
            "The server answered 429 Too Many Requests.",
        ));
    }
    let count = request
        .body
        .as_ref()
        .and_then(|b| b["count"].as_u64())
        .unwrap_or(1);
    if w.kubo {
        let credential = json!({ "strategy": "kubo-api", "apiUrl": "https://kubo.test",
            "gateway": "https://gw.test/ipfs/", "reusable": true });
        return json_answer(json!({ "credentials": [credential] }));
    }
    let credentials: Vec<Value> = (0..count)
        .map(|_| {
            let url = format!("https://up.test/{}", w.minted);
            w.minted += 1;
            json!({ "strategy": "presigned-put", "url": url,
                "gateway": "https://gw.test/ipfs/", "reusable": false })
        })
        .collect();
    json_answer(json!({ "credentials": credentials }))
}

/// A pre-signed upload: the file is stored under `bafyUp<n>`; a second
/// upload to the same URL is refused as Pinata does.
fn upload(w: &mut World, n: usize, request: &Request) -> Result<Vec<u8>, HttpError> {
    if n >= w.minted {
        return Err(not_found());
    }
    if !w.spent.insert(n) {
        return Err(HttpError::new(
            Some(409),
            "The server answered 409 Conflict.",
        ));
    }
    let cid = format!("bafyUp{n}");
    w.files
        .insert(cid.clone(), request.data.clone().unwrap_or_default());
    json_answer(json!({ "data": { "cid": cid } }))
}

/// The wallet relay: the session must be Arbesk's; an `updateUri` must name
/// the account's own token, a `publish` a token that doesn't exist yet.
/// What happens next is `w.relay`.
fn wallet_relay(w: &mut World, request: &Request) -> Result<Vec<u8>, HttpError> {
    signed_in(w, request)?;
    let body = request.body.as_ref().cloned().unwrap_or_default();
    let token = body["tokenId"]
        .as_str()
        .map(String::from)
        .or_else(|| body["tokenId"].as_u64().map(|n| n.to_string()))
        .unwrap_or_default();
    let op = body["op"].as_str().unwrap_or("updateUri");
    let publish = op == "publish";
    if publish {
        // Minting: the token must not exist yet.
        if w.tokens.contains_key(&token) {
            return Err(HttpError::new(
                Some(409),
                "The server answered 409 Conflict.",
            ));
        }
    } else if !w.owned.contains(&token) {
        return Err(HttpError::new(
            Some(403),
            "The server answered 403 Forbidden.",
        ));
    }
    let receipt = |status: Option<bool>| {
        let mut receipt = json!({ "transactionHash": "0x1" });
        if let Some(status) = status {
            receipt["status"] = json!(status);
        }
        json_answer(json!({ "receipt": receipt }))
    };
    match w.relay {
        Relay::Status(code) => Err(HttpError::new(
            Some(code),
            format!("The server answered {code}."),
        )),
        Relay::Pending => receipt(None),
        Relay::Failed => receipt(Some(false)),
        Relay::Ok if publish => {
            let uri = body["params"]["uri"].as_str().unwrap_or_default();
            let cid = uri.strip_prefix("ipfs://").unwrap_or(uri);
            w.tokens.insert(token.clone(), cid.to_string());
            if !w.owned.contains(&token) {
                w.owned.push(token);
            }
            receipt(Some(true))
        }
        Relay::Ok if op == "updateEditors" => {
            let uri = body["params"]["newListUri"].as_str().unwrap_or_default();
            let cid = uri.strip_prefix("ipfs://").unwrap_or(uri);
            let entry = w
                .editor_lists
                .entry(token.clone())
                .or_insert_with(|| (String::new(), 1));
            entry.0 = cid.to_string();
            entry.1 += 1;
            receipt(Some(true))
        }
        Relay::Ok => {
            let uri = body["params"]["newUri"].as_str().unwrap_or_default();
            let cid = uri.strip_prefix("ipfs://").unwrap_or(uri);
            if let Some(before) = w.tokens.insert(token.clone(), cid.to_string())
                && w.lag > 0
            {
                let lag = w.lag;
                w.lagging.insert(token, (before, lag));
            }
            receipt(Some(true))
        }
    }
}

/// A 32-byte word of hex as a decimal number, as `tokenURI` gets it.
fn word_decimal(hex: &str) -> String {
    let bytes = data_encoding::HEXLOWER_PERMISSIVE
        .decode(hex.as_bytes())
        .unwrap_or_default();
    crate::chain::decimal(&bytes)
}

/// A WebP picture, the format the Studio stores thumbnails in.
pub(crate) fn webp(width: u32, height: u32) -> Vec<u8> {
    let picture = image::DynamicImage::new_rgb8(width, height);
    let mut bytes = Vec::new();
    picture
        .write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::WebP,
        )
        .unwrap();
    bytes
}

pub(crate) fn shapes() -> String {
    collection_path(ADDRESS, "1", "Shapes")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chain;
    use crate::http::Http;

    fn with_session() -> (Arc<Mutex<World>>, FakeHttp) {
        let state = Arc::new(Mutex::new(world()));
        state.lock().unwrap().sessions.insert("tok".into());
        let http = arbesk(state.clone());
        (state, http)
    }

    fn urls(http: &FakeHttp, count: usize) -> Vec<String> {
        let answer = http
            .post_json(
                &format!("{ORIGIN}/api/v1/ipfs/upload-urls"),
                &json!({ "count": count }),
                Some("tok"),
            )
            .unwrap();
        let answer: Value = serde_json::from_slice(&answer).unwrap();
        answer["credentials"]
            .as_array()
            .unwrap()
            .iter()
            .map(|credential| credential["url"].as_str().unwrap().to_string())
            .collect()
    }

    fn relay(http: &FakeHttp, session: Option<&str>, token: u64) -> Result<Value, HttpError> {
        let body = json!({ "op": "updateUri", "tokenId": token.to_string(),
            "params": { "newUri": "ipfs://bafyNew" } });
        http.post_json_slow(&format!("{ORIGIN}/api/v1/wallet/relay"), &body, session)
            .map(|bytes| serde_json::from_slice(&bytes).unwrap())
    }

    #[test]
    fn an_upload_is_served_by_the_gateway_and_a_relay_moves_the_token() {
        let (_, http) = with_session();
        let urls = urls(&http, 2);
        assert_eq!(urls, ["https://up.test/0", "https://up.test/1"]);
        let fields = [("network", "public")];
        let answer = http
            .post_multipart(&urls[0], "a.bin", b"hello", &fields, None)
            .unwrap();
        let answer: Value = serde_json::from_slice(&answer).unwrap();
        assert_eq!(answer["data"]["cid"], "bafyUp0");
        let sent = http.requests().into_iter().last().unwrap();
        assert_eq!(sent.data.as_deref(), Some(&b"hello"[..]));
        assert_eq!(sent.fields, [("network".to_string(), "public".to_string())]);
        assert_eq!(http.get("https://gw.test/ipfs/bafyUp0").unwrap(), b"hello");
        let again = http
            .post_multipart(&urls[0], "a.bin", b"hello", &fields, None)
            .unwrap_err();
        assert_eq!(again.status, Some(409));

        let receipt = relay(&http, Some("tok"), 1).unwrap();
        assert_eq!(receipt["receipt"]["status"], true);
        let config = chain::config(&http, &site()).unwrap();
        assert_eq!(chain::token_uri(&http, &config, "1").unwrap(), "bafyNew");
        assert_eq!(
            relay(&http, Some("other"), 1).unwrap_err().status,
            Some(401)
        );
        assert_eq!(relay(&http, None, 1).unwrap_err().status, Some(401));
        assert_eq!(relay(&http, Some("tok"), 4).unwrap_err().status, Some(403));
    }

    #[test]
    fn the_relay_can_stall_fail_or_refuse() {
        let (state, http) = with_session();
        for (relay_mode, status) in [(Relay::Pending, None), (Relay::Failed, Some(false))] {
            state.lock().unwrap().relay = relay_mode;
            let receipt = relay(&http, Some("tok"), 1).unwrap();
            assert_eq!(receipt["receipt"]["status"].as_bool(), status);
        }
        assert_eq!(state.lock().unwrap().tokens["1"], "bafyC1");
        state.lock().unwrap().relay = Relay::Status(502);
        assert_eq!(relay(&http, Some("tok"), 1).unwrap_err().status, Some(502));
    }

    #[test]
    fn kubo_upload_urls_name_the_api_instead() {
        let (state, http) = with_session();
        state.lock().unwrap().kubo = true;
        let answer = http
            .post_json(
                &format!("{ORIGIN}/api/v1/ipfs/upload-urls"),
                &json!({ "count": 1 }),
                Some("tok"),
            )
            .unwrap();
        let answer: Value = serde_json::from_slice(&answer).unwrap();
        assert_eq!(answer["credentials"][0]["strategy"], "kubo-api");
        assert_eq!(answer["credentials"][0]["reusable"], true);
    }
}
