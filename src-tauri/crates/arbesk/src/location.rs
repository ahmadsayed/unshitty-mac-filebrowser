//! Parsing and formatting of `arbesk://` locations. Ids decide; the name
//! after `~` is only for display, so two items with one name stay distinct.
//!
//! ```text
//! arbesk://<address>                                           an account
//! arbesk://<address>/<token-id>~<name>                         a collection
//! arbesk://<address>/<token-id>~<name>/<asset-id>~<name>.<ext> an asset
//! ```
//!
//! Ids and names are percent-encoded with Drive's set, which encodes `~` and
//! `/`, so the first `~` of a segment always ends its id.

use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, percent_decode_str, utf8_percent_encode};

/// Characters allowed literally in a display name. `~` is the id/name
/// separator, so it is deliberately encoded (`%7E`).
pub(crate) const NAME_SET: &AsciiSet = &NON_ALPHANUMERIC.remove(b'.').remove(b'-').remove(b'_');

/// Reject malformed `%XX` sequences instead of silently turning them into
/// literal text; a bad URI should be reported, not guessed at.
fn valid_percent(input: &str) -> bool {
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            if i + 2 >= bytes.len()
                || !bytes[i + 1].is_ascii_hexdigit()
                || !bytes[i + 2].is_ascii_hexdigit()
            {
                return false;
            }
            i += 3;
        } else {
            i += 1;
        }
    }
    true
}

/// Every Arbesk location starts with this, and nothing else is Arbesk.
pub const PREFIX: &str = "arbesk://";

/// What an `arbesk://` path names. Ids only; names are for display.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArbeskPath {
    Account {
        address: String,
    },
    Collection {
        address: String,
        token_id: String,
    },
    Asset {
        address: String,
        token_id: String,
        asset_id: String,
    },
}

impl ArbeskPath {
    /// The wallet address of the account the path belongs to.
    pub fn address(&self) -> &str {
        match self {
            Self::Account { address }
            | Self::Collection { address, .. }
            | Self::Asset { address, .. } => address,
        }
    }
}

/// Whether `path` names a folder (an account or collection) or a file (an
/// asset); `None` when it isn't an Arbesk location.
pub fn is_folder(path: &str) -> Option<bool> {
    parse(path)
        .ok()
        .map(|parsed| !matches!(parsed, ArbeskPath::Asset { .. }))
}

pub fn is_arbesk(path: &str) -> bool {
    path.starts_with(PREFIX)
}

pub fn account_path(address: &str) -> String {
    format!("{PREFIX}{}", address.to_ascii_lowercase())
}

pub fn collection_path(address: &str, token_id: &str, name: &str) -> String {
    format!("{}/{}", account_path(address), segment(token_id, name))
}

/// An asset in `collection_path`, named `<name>.<ext>` (the listing names
/// its own with `asset_path_named`).
#[cfg(test)]
pub fn asset_path(collection_path: &str, asset_id: &str, name: &str, ext: &str) -> String {
    asset_path_named(collection_path, asset_id, &format!("{name}.{ext}"))
}

/// An asset in `collection_path` shown as `file_name` as it is.
pub fn asset_path_named(collection_path: &str, asset_id: &str, file_name: &str) -> String {
    format!(
        "{}/{}",
        collection_path.trim_end_matches('/'),
        segment(asset_id, file_name)
    )
}

/// Where a copy into Arbesk goes: the collection and the listed name,
/// from `arbesk://<address>/<token>~<name>/~<encoded name>`.
pub fn parse_target(path: &str) -> Result<(String, String, String), String> {
    let pending = || "Paste into a collection.".to_string();
    let (parent, last) = path.trim_end_matches('/').rsplit_once('/').ok_or_else(pending)?;
    let name = last.strip_prefix('~').filter(|name| valid_percent(name)).ok_or_else(pending)?;
    let name = percent_decode_str(name).decode_utf8().map_err(|_| pending())?;
    if name.is_empty() {
        return Err(pending());
    }
    match parse(parent)? {
        ArbeskPath::Collection { address, token_id } => Ok((address, token_id, name.into_owned())),
        _ => Err(pending()),
    }
}

fn segment(id: &str, name: &str) -> String {
    format!("{}~{}", encode(id), encode(name))
}

fn encode(text: &str) -> String {
    utf8_percent_encode(text, NAME_SET).to_string()
}

/// What `path` names, or why it isn't an Arbesk location.
pub fn parse(path: &str) -> Result<ArbeskPath, String> {
    let refused = || format!("“{path}” is not an Arbesk location.");
    let rest = path.strip_prefix(PREFIX).ok_or_else(refused)?;
    let mut parts = rest.trim_end_matches('/').split('/');
    let address = parts
        .next()
        .filter(|a| is_address(a))
        .ok_or_else(refused)?
        .to_ascii_lowercase();
    let ids: Vec<String> = parts
        .map(segment_id)
        .collect::<Option<_>>()
        .ok_or_else(refused)?;
    match ids.as_slice() {
        [] => Ok(ArbeskPath::Account { address }),
        [token] if is_token_id(token) => Ok(ArbeskPath::Collection {
            address,
            token_id: token.clone(),
        }),
        [token, asset] if is_token_id(token) => Ok(ArbeskPath::Asset {
            address,
            token_id: token.clone(),
            asset_id: asset.clone(),
        }),
        _ => Err(refused()),
    }
}

/// A wallet address: `0x` and 40 hexadecimal digits.
fn is_address(text: &str) -> bool {
    text.len() == 42 && text.starts_with("0x") && text[2..].bytes().all(|b| b.is_ascii_hexdigit())
}

fn is_token_id(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit())
}

/// The decoded id before a segment's first `~`; `None` when it is empty or
/// either half is badly encoded.
fn segment_id(segment: &str) -> Option<String> {
    let (id, name) = segment.split_once('~').unwrap_or((segment, ""));
    if id.is_empty() || !valid_percent(id) || !valid_percent(name) {
        return None;
    }
    percent_decode_str(name).decode_utf8().ok()?;
    let id = percent_decode_str(id).decode_utf8().ok()?.into_owned();
    (!id.is_empty()).then_some(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ADDRESS: &str = "0x00000000000000000000000000000000000000ab";

    #[test]
    fn a_pending_name_in_a_collection_is_a_target() {
        let collection = collection_path(ADDRESS, "12", "My Set");
        assert_eq!(
            parse_target(&format!("{collection}/~Box%20%2F%20x.gltf")).unwrap(),
            (ADDRESS.into(), "12".into(), "Box / x.gltf".into())
        );
        let asset = asset_path(&collection, "a1", "Box", "gltf");
        let account = account_path(ADDRESS);
        let pending = "Paste into a collection.";
        for bad in [
            format!("{account}/~Box.gltf"),
            format!("{asset}/~Box.gltf"),
            format!("{collection}/~"),
            format!("{collection}/Box.gltf"),
            format!("{collection}/~%zz"),
            collection.clone(),
            account.clone(),
        ] {
            assert_eq!(parse_target(&bad).unwrap_err(), pending, "{bad}");
        }
        assert!(parse_target("arbesk://bad/~x").is_err());
    }

    #[test]
    fn an_account_round_trips() {
        let path = account_path(ADDRESS);
        assert_eq!(path, format!("arbesk://{ADDRESS}"));
        assert_eq!(
            parse(&path).unwrap(),
            ArbeskPath::Account {
                address: ADDRESS.into()
            }
        );
    }

    #[test]
    fn the_address_is_lower_cased() {
        let upper = "arbesk://0x00000000000000000000000000000000000000AB";
        assert_eq!(
            parse(upper).unwrap(),
            ArbeskPath::Account {
                address: ADDRESS.into()
            }
        );
    }

    #[test]
    fn a_collection_round_trips() {
        let path = collection_path(ADDRESS, "12", "My Set");
        assert_eq!(path, format!("arbesk://{ADDRESS}/12~My%20Set"));
        assert_eq!(
            parse(&path).unwrap(),
            ArbeskPath::Collection {
                address: ADDRESS.into(),
                token_id: "12".into()
            }
        );
    }

    #[test]
    fn an_asset_round_trips_whatever_its_name_and_id() {
        let collection = collection_path(ADDRESS, "12", "Set");
        for (id, name) in [
            ("asset_1730000000000", "Box"),
            ("a/b~c%d", "x/y~z%w é 🧊"),
            ("plain", ""),
        ] {
            let path = asset_path(&collection, id, name, "gltf");
            assert_eq!(
                parse(&path).unwrap(),
                ArbeskPath::Asset {
                    address: ADDRESS.into(),
                    token_id: "12".into(),
                    asset_id: id.into()
                },
                "{path}"
            );
            assert_eq!(path.matches('/').count(), 4, "{path}");
        }
    }

    #[test]
    fn an_asset_name_carries_its_extension() {
        let collection = collection_path(ADDRESS, "12", "Set");
        assert!(asset_path(&collection, "a1", "Box", "3mf").ends_with("/a1~Box.3mf"));
    }

    #[test]
    fn an_empty_name_parses() {
        let path = format!("arbesk://{ADDRESS}/12~");
        assert!(matches!(parse(&path), Ok(ArbeskPath::Collection { .. })));
    }

    #[test]
    fn malformed_paths_are_refused() {
        for path in [
            "arbesk://".to_string(),
            "arbesk://0xabc".to_string(),
            "arbesk://00000000000000000000000000000000000000000ab".to_string(),
            "arbesk://0x00000000000000000000000000000000000000ag".to_string(),
            format!("arbesk://{ADDRESS}/~Set"),
            format!("arbesk://{ADDRESS}/x1~Set"),
            format!("arbesk://{ADDRESS}/12~Set/~Box.gltf"),
            format!("arbesk://{ADDRESS}/12~Set/a1~Box.gltf/more"),
            format!("arbesk://{ADDRESS}/12~Set/a%2~Box.gltf"),
            format!("arbesk://{ADDRESS}/12~Se%zzt"),
            "gdrive://x".to_string(),
        ] {
            assert!(parse(&path).is_err(), "{path}");
        }
    }

    #[test]
    fn only_the_scheme_makes_a_path_arbesk() {
        assert!(is_arbesk("arbesk://anything"));
        assert!(!is_arbesk("/home/arbesk://x"));
        assert!(!is_arbesk("gdrive://x"));
    }
}
