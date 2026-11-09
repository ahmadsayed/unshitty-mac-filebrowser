//! What Arbesk's backend and chain say: the deployment's settings
//! (`/api/v1/config`), which collection tokens an address owns or edits
//! (`/api/v1/indexer/{owned,shared}`), and each token's manifest CID
//! (`tokenURI` over JSON-RPC `eth_call`, encoded by hand: one call, no
//! Ethereum crate).

use serde_json::{Value, json};

use crate::http::Http;

/// Which Arbesk deployment and chain an account lives on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Site {
    /// The backend, e.g. `https://promptscad.com`, without a trailing slash.
    pub origin: String,
    pub chain_id: u64,
}

/// What `/api/v1/config` says about the account's chain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChainConfig {
    /// The collection contract whose `tokenURI` names each manifest.
    pub contract: String,
    pub rpc_url: String,
    /// The IPFS gateway, without a trailing slash.
    pub gateway: String,
}

/// How an error starts when the chain didn't answer: a listing fails with it
/// rather than leaving the collection out.
pub const CHAIN_UNREACHABLE: &str = "The chain could not be reached";
/// How an error starts when `tokenURI` reverted: the token doesn't exist.
pub const TOKEN_REFUSED: &str = "The chain refused collection";

/// `tokenURI(uint256)`, `editorListURI(uint256)` and
/// `editorSetVersion(uint256)` selectors.
const TOKEN_URI: &str = "0xc87b56dd";
const EDITOR_LIST_URI: &str = "0x51d7d6e1";
const EDITOR_SET_VERSION: &str = "0xb0f762fa";

fn get_json(http: &dyn Http, url: &str) -> Result<Value, String> {
    let bytes = http
        .get(url)
        .map_err(|error| format!("Arbesk could not be reached: {}", error.message))?;
    serde_json::from_slice(&bytes)
        .map_err(|_| "Arbesk sent an answer File Browser can't read.".to_string())
}

pub fn config(http: &dyn Http, site: &Site) -> Result<ChainConfig, String> {
    let body = get_json(http, &format!("{}/api/v1/config", site.origin))?;
    let network = &body["networkConfigs"][site.chain_id.to_string()];
    let text = |value: &Value| value.as_str().filter(|s| !s.is_empty()).map(String::from);
    let missing = || format!("Arbesk has no settings for chain {}.", site.chain_id);
    Ok(ChainConfig {
        contract: text(&network["contractAddress"]).ok_or_else(missing)?,
        rpc_url: text(&network["rpcUrl"]).ok_or_else(missing)?,
        gateway: text(&body["ipfsGatewayUrl"])
            .ok_or_else(missing)?
            .trim_end_matches('/')
            .to_string(),
    })
}

/// The collection tokens `address` owns, then those it may edit, each once.
pub fn collection_ids(http: &dyn Http, site: &Site, address: &str) -> Result<Vec<String>, String> {
    let mut ids: Vec<String> = Vec::new();
    for scope in ["owned", "shared"] {
        let url = format!(
            "{}/api/v1/indexer/{scope}?address={address}&chainId={}",
            site.origin, site.chain_id
        );
        let body = get_json(http, &url)?;
        for id in body[scope]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            if !ids.iter().any(|known| known == id) {
                ids.push(id.to_string());
            }
        }
    }
    Ok(ids)
}

/// The manifest CID a collection token points at (`ipfs://` removed).
pub fn token_uri(http: &dyn Http, chain: &ChainConfig, token_id: &str) -> Result<String, String> {
    let result = eth_call(http, chain, TOKEN_URI, token_id, token_id)?;
    let uri = decode_abi_string(&result)?;
    Ok(uri.strip_prefix("ipfs://").unwrap_or(&uri).to_string())
}

/// The editor list CID a collection token points at (`ipfs://` removed).
pub fn editor_list_uri(
    http: &dyn Http,
    chain: &ChainConfig,
    token_id: &str,
) -> Result<String, String> {
    let result = eth_call(http, chain, EDITOR_LIST_URI, token_id, token_id)?;
    let uri = decode_abi_string(&result)?;
    Ok(uri.strip_prefix("ipfs://").unwrap_or(&uri).to_string())
}

/// The current editor-set version of a collection token (1 for an untouched
/// new collection).
pub fn editor_set_version(
    http: &dyn Http,
    chain: &ChainConfig,
    token_id: &str,
) -> Result<u64, String> {
    let result = eth_call(http, chain, EDITOR_SET_VERSION, token_id, token_id)?;
    decode_abi_uint(&result)
}

/// One `eth_call` of `selector(token_id)`; `what` names the token in a
/// refusal.
fn eth_call(
    http: &dyn Http,
    chain: &ChainConfig,
    selector: &str,
    token_id: &str,
    what: &str,
) -> Result<String, String> {
    let data = call_data(selector, token_id)?;
    let call = json!({
        "jsonrpc": "2.0", "id": 1, "method": "eth_call",
        "params": [{ "to": chain.contract, "data": data }, "latest"]
    });
    let bytes = http
        .post_json(&chain.rpc_url, &call, None)
        .map_err(|error| format!("{CHAIN_UNREACHABLE}: {}", error.message))?;
    let body: Value = serde_json::from_slice(&bytes)
        .map_err(|_| "The chain sent an answer File Browser can't read.".to_string())?;
    if let Some(message) = body["error"]["message"].as_str() {
        return Err(format!("{TOKEN_REFUSED} {what}: {message}"));
    }
    Ok(body["result"].as_str().unwrap_or_default().to_string())
}

/// A call's data: `selector` and the token id as a uint256.
fn call_data(selector: &str, token_id: &str) -> Result<String, String> {
    Ok(uint256_word(token_id)?
        .iter()
        .fold(selector.to_string(), |hex, byte| {
            hex + &format!("{byte:02x}")
        }))
}

/// `tokenURI(token_id)` as call data.
#[cfg(test)]
fn encode_token_uri_call(token_id: &str) -> Result<String, String> {
    call_data(TOKEN_URI, token_id)
}

/// A decimal token id as a 32-byte big-endian uint256 word.
pub(crate) fn uint256_word(token_id: &str) -> Result<[u8; 32], String> {
    let refused = || format!("“{token_id}” is not a collection token.");
    if token_id.is_empty() || !token_id.bytes().all(|b| b.is_ascii_digit()) {
        return Err(refused());
    }
    let mut word = [0u8; 32];
    for digit in token_id.bytes().map(|b| u32::from(b - b'0')) {
        let mut carry = digit;
        for byte in word.iter_mut().rev() {
            let value = u32::from(*byte) * 10 + carry;
            *byte = (value & 0xff) as u8;
            carry = value >> 8;
        }
        if carry != 0 {
            return Err(refused());
        }
    }
    Ok(word)
}

/// Big-endian bytes as a decimal number: the token id a hash names.
pub(crate) fn decimal(bytes: &[u8]) -> String {
    let mut digits = vec![0u8];
    for &byte in bytes {
        let mut carry = u32::from(byte);
        for digit in digits.iter_mut() {
            let value = u32::from(*digit) * 256 + carry;
            *digit = (value % 10) as u8;
            carry = value / 10;
        }
        while carry > 0 {
            digits.push((carry % 10) as u8);
            carry /= 10;
        }
    }
    let decimal: String = digits
        .iter()
        .rev()
        .skip_while(|digit| **digit == 0)
        .map(|digit| char::from(b'0' + digit))
        .collect();
    if decimal.is_empty() {
        "0".into()
    } else {
        decimal
    }
}

/// One ABI-encoded `uint256` return value, for the values that fit a u64.
fn decode_abi_uint(hex: &str) -> Result<u64, String> {
    let refused = || "The chain's answer is not an editor-set version.".to_string();
    let digits = hex.strip_prefix("0x").unwrap_or(hex);
    if digits.len() != 64 {
        return Err(refused());
    }
    if digits[..48].bytes().any(|byte| byte != b'0') {
        return Err(refused());
    }
    u64::from_str_radix(&digits[48..], 16).map_err(|_| refused())
}

/// One ABI-encoded `string` return value: offset 32, length, bytes.
fn decode_abi_string(hex: &str) -> Result<String, String> {
    let refused = || "The chain's answer is not a manifest address.".to_string();
    let digits = hex.strip_prefix("0x").unwrap_or(hex);
    if digits.len() % 2 != 0 {
        return Err(refused());
    }
    let bytes: Vec<u8> = (0..digits.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&digits[i..i + 2], 16))
        .collect::<Result<_, _>>()
        .map_err(|_| refused())?;
    let word = |at: usize| -> Option<usize> {
        let word = bytes.get(at..at + 32)?;
        word[..24].iter().all(|b| *b == 0).then(|| {
            word[24..]
                .iter()
                .fold(0usize, |n, b| (n << 8) | usize::from(*b))
        })
    };
    if word(0) != Some(32) {
        return Err(refused());
    }
    let length = word(32).ok_or_else(refused)?;
    let text = bytes
        .get(64..64usize.checked_add(length).ok_or_else(refused)?)
        .ok_or_else(refused)?;
    String::from_utf8(text.to_vec()).map_err(|_| refused())
}

/// Test helpers other modules' tests share.
#[cfg(test)]
pub(crate) mod testing {
    /// The ABI encoding of one string return value, as hex with `0x`.
    pub fn abi_string(text: &str) -> String {
        let bytes = text.as_bytes();
        let mut out = format!("0x{:064x}{:064x}", 32, bytes.len());
        for byte in bytes {
            out.push_str(&format!("{byte:02x}"));
        }
        while (out.len() - 2) % 64 != 0 {
            out.push('0');
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::testing::abi_string;
    use super::*;
    use crate::http::HttpError;
    use crate::http::fake::{FakeHttp, Request, not_found};
    use serde_json::json;

    const ORIGIN: &str = "https://arbesk.test";

    fn site() -> Site {
        Site {
            origin: ORIGIN.into(),
            chain_id: 84532,
        }
    }

    fn chain() -> ChainConfig {
        ChainConfig {
            contract: "0xC0".into(),
            rpc_url: "https://rpc.test".into(),
            gateway: "https://gw.test/ipfs".into(),
        }
    }

    #[test]
    fn the_config_names_the_chains_contract_rpc_and_gateway() {
        let http = FakeHttp::new(|request| {
            assert_eq!(request.url, format!("{ORIGIN}/api/v1/config"));
            Ok(json!({
                "contractAddress": "0xDEFAULT",
                "ipfsGatewayUrl": "https://gw.test/ipfs/",
                "networkConfigs": { "84532": { "contractAddress": "0xC0", "rpcUrl": "https://rpc.test" } }
            })
            .to_string()
            .into_bytes())
        });
        assert_eq!(config(&http, &site()).unwrap(), chain());
    }

    #[test]
    fn a_config_without_the_chain_is_refused() {
        let http = FakeHttp::new(|_| {
            Ok(
                json!({ "ipfsGatewayUrl": "https://gw/", "networkConfigs": {} })
                    .to_string()
                    .into_bytes(),
            )
        });
        assert!(config(&http, &site()).is_err());
    }

    #[test]
    fn collection_ids_merge_owned_and_shared_in_order() {
        let http = FakeHttp::new(|request| {
            let body = if request.url.contains("/owned?") {
                json!({ "owned": ["1", "2"] })
            } else {
                json!({ "shared": ["2", "3"] })
            };
            Ok(body.to_string().into_bytes())
        });
        assert_eq!(
            collection_ids(&http, &site(), "0xab").unwrap(),
            ["1", "2", "3"]
        );
        let urls: Vec<String> = http.requests().into_iter().map(|r| r.url).collect();
        assert_eq!(
            urls,
            [
                format!("{ORIGIN}/api/v1/indexer/owned?address=0xab&chainId=84532"),
                format!("{ORIGIN}/api/v1/indexer/shared?address=0xab&chainId=84532"),
            ]
        );
    }

    #[test]
    fn a_failing_index_is_an_error() {
        let http = FakeHttp::new(|_| Err(not_found()));
        assert!(collection_ids(&http, &site(), "0xab").is_err());
    }

    #[test]
    fn the_token_uri_call_is_its_selector_and_a_uint256() {
        assert_eq!(
            encode_token_uri_call("7").unwrap(),
            format!("0xc87b56dd{}7", "0".repeat(63))
        );
        assert_eq!(
            encode_token_uri_call("256").unwrap(),
            format!("0xc87b56dd{}100", "0".repeat(61))
        );
        let max = "115792089237316195423570985008687907853269984665640564039457584007913129639935";
        assert_eq!(
            encode_token_uri_call(max).unwrap(),
            format!("0xc87b56dd{}", "f".repeat(64))
        );
    }

    #[test]
    fn a_token_id_that_is_not_a_uint256_is_refused() {
        for id in [
            "",
            "x7",
            "-1",
            "1.5",
            "115792089237316195423570985008687907853269984665640564039457584007913129639936",
        ] {
            assert!(encode_token_uri_call(id).is_err(), "{id}");
        }
    }

    #[test]
    fn an_abi_string_decodes() {
        assert_eq!(
            decode_abi_string(&abi_string("ipfs://bafyX")).unwrap(),
            "ipfs://bafyX"
        );
        assert_eq!(decode_abi_string(&abi_string("")).unwrap(), "");
        let long = "x".repeat(70);
        assert_eq!(decode_abi_string(&abi_string(&long)).unwrap(), long);
    }

    #[test]
    fn a_truncated_or_odd_abi_answer_is_refused() {
        let good = abi_string("ipfs://bafyX");
        for bad in [
            &good[..good.len() - 64],
            "0x",
            "0x12",
            &good.replacen("0x", "", 1).replace('0', "z"),
        ] {
            assert!(decode_abi_string(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn token_uri_posts_an_eth_call_and_returns_the_cid() {
        let http = FakeHttp::new(|_| {
            Ok(
                json!({ "jsonrpc": "2.0", "id": 1, "result": abi_string("ipfs://bafyCID") })
                    .to_string()
                    .into_bytes(),
            )
        });
        assert_eq!(token_uri(&http, &chain(), "7").unwrap(), "bafyCID");
        let requests = http.requests();
        assert_eq!(
            requests,
            [Request {
                method: "POST",
                url: "https://rpc.test".into(),
                body: Some(json!({
                    "jsonrpc": "2.0", "id": 1, "method": "eth_call",
                    "params": [{ "to": "0xC0", "data": encode_token_uri_call("7").unwrap() }, "latest"]
                })),
                session: None,
                data: None,
                fields: Vec::new(),
            }]
        );
    }

    #[test]
    fn the_editor_list_and_version_reads_use_their_selectors() {
        let http = FakeHttp::new(|request| {
            let data = request.body.as_ref().unwrap()["params"][0]["data"]
                .as_str()
                .unwrap()
                .to_string();
            let answer = if data.starts_with("0x51d7d6e1") {
                json!({ "result": abi_string("ipfs://bafyEditors") })
            } else {
                assert!(data.starts_with("0xb0f762fa"), "{data}");
                json!({ "result": format!("0x{:064x}", 7) })
            };
            Ok(answer.to_string().into_bytes())
        });
        assert_eq!(
            editor_list_uri(&http, &chain(), "256").unwrap(),
            "bafyEditors"
        );
        assert_eq!(editor_set_version(&http, &chain(), "256").unwrap(), 7);
        let requests = http.requests();
        assert_eq!(
            requests[0].body.as_ref().unwrap()["params"][0]["data"],
            format!("0x51d7d6e1{}100", "0".repeat(61))
        );
        assert_eq!(
            requests[1].body.as_ref().unwrap()["params"][0]["data"],
            format!("0xb0f762fa{}100", "0".repeat(61))
        );
    }

    #[test]
    fn a_version_that_is_not_a_uint256_is_refused() {
        for bad in [
            "0x",
            "0x01",
            "0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
        ] {
            assert!(decode_abi_uint(bad).is_err(), "{bad}");
        }
        assert_eq!(
            decode_abi_uint(&format!("0x{:064x}", 1)).unwrap(),
            1
        );
    }

    #[test]
    fn a_json_rpc_error_is_an_error() {
        let http = FakeHttp::new(|_| {
            Ok(json!({ "jsonrpc": "2.0", "id": 1, "error": { "code": 3, "message": "execution reverted" } }).to_string().into_bytes())
        });
        let error = token_uri(&http, &chain(), "7").unwrap_err();
        assert!(error.contains("execution reverted"), "{error}");
    }

    #[test]
    fn an_unreachable_rpc_is_an_error() {
        let http = FakeHttp::new(|_| Err(HttpError::new(None, "connection refused")));
        assert!(token_uri(&http, &chain(), "7").is_err());
    }
}
