//! What the extensions answer (D4): a save of an item, what sharing did, a
//! sign-in waiting for the browser, and a provider's own record (an
//! account, a preview's scene) handed on to the frontend as its provider
//! wrote it.

use serde::Serialize;
use serde_json::value::RawValue;

use crate::{Error, Result};

/// One save of an item (`ext::Versions`): its id (an Arbesk manifest's CID)
/// and when it was saved, in ms, when the place knows.
#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct Version {
    pub id: String,
    pub timestamp: Option<i64>,
}

/// What sharing did (`ext::Sharing`): the member's address and role, and
/// whether it was added (`false` when an existing member's role changed).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Shared {
    pub address: String,
    pub role: u8,
    pub added: bool,
}

/// A sign-in started (`ext::Accounts`): its id, for finishing or cancelling
/// it, and the page the frontend opens in the browser.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignIn {
    pub flow_id: String,
    pub url: String,
}

/// A provider's own record, already serialised as it serialises it (its
/// fields, in its order), so a command hands the frontend exactly the JSON
/// it got before the provider answered through an extension: each place's
/// accounts have their own shape, and so does a preview's scene.
#[derive(Debug, Clone, Serialize)]
#[serde(transparent)]
pub struct Json(Box<RawValue>);

impl Json {
    /// `value` as its JSON.
    pub fn of<T: Serialize + ?Sized>(value: &T) -> Result<Json> {
        serde_json::value::to_raw_value(value)
            .map(Json)
            .map_err(|error| Error::Other(error.to_string()))
    }

    /// The JSON text.
    pub fn as_str(&self) -> &str {
        self.0.get()
    }
}

impl PartialEq for Json {
    fn eq(&self, other: &Self) -> bool {
        self.as_str() == other.as_str()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Serialize)]
    struct Account {
        id: String,
        email: String,
        chain_id: u64,
    }

    /// A record keeps its provider's fields in its provider's order, byte
    /// for byte, alone or in a list.
    #[test]
    fn a_record_serialises_as_its_provider_wrote_it() {
        let account = Account { id: "a1".into(), email: "me@example.com".into(), chain_id: 84532 };
        let direct = serde_json::to_string(&account).unwrap();
        let json = Json::of(&account).unwrap();
        assert_eq!(json.as_str(), direct);
        assert_eq!(serde_json::to_string(&json).unwrap(), direct);
        let list = vec![Json::of(&account).unwrap(), json.clone()];
        assert_eq!(serde_json::to_string(&list).unwrap(), format!("[{direct},{direct}]"));
        assert_eq!(json, Json::of(&account).unwrap());
    }

    #[test]
    fn a_save_and_a_share_serialise_as_arbesk_s_did() {
        let version = Version { id: "bafyH1".into(), timestamp: Some(1000) };
        assert_eq!(serde_json::to_string(&version).unwrap(), r#"{"id":"bafyH1","timestamp":1000}"#);
        let unknown = Version { id: "bafyH2".into(), timestamp: None };
        assert_eq!(serde_json::to_string(&unknown).unwrap(), r#"{"id":"bafyH2","timestamp":null}"#);
        let shared = Shared { address: "0x11".into(), role: 2, added: true };
        assert_eq!(serde_json::to_string(&shared).unwrap(), r#"{"address":"0x11","role":2,"added":true}"#);
    }
}
