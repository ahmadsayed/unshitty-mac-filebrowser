//! Parsing and formatting of `gdrive://` locations.
//!
//! The file id is authoritative; the name after `~` is a display hint. This
//! keeps two same-named Drive files distinct and survives a rename made in
//! another client.
//!
//! ```text
//! gdrive://<account>/<file-id>~<percent-encoded-name>/<file-id>~<name>
//! ```

use std::fmt;
use std::path::PathBuf;

use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, percent_decode_str, utf8_percent_encode};

/// Every cloud path starts with this, and nothing else is cloud.
pub const DRIVE_SCHEME: &str = "gdrive://";

const ACCOUNT_MAX: usize = 64;
const ID_MAX: usize = 128;

/// Characters allowed literally in a display name. `~` is the id/name
/// separator, so it is deliberately encoded (`%7E`).
const NAME_SET: &AsciiSet = &NON_ALPHANUMERIC.remove(b'.').remove(b'-').remove(b'_');

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Location {
    Local(PathBuf),
    Drive(DriveLocation),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DriveLocation {
    pub account: String,
    /// Never empty for a location produced by [`DriveLocation::parse`].
    pub segments: Vec<Segment>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    /// Opaque Drive file or folder id.
    pub id: String,
    /// Decoded display name; may be empty when a pasted URI has no name.
    pub name: String,
}

impl DriveLocation {
    /// The root of an account, as `about.get` reports it.
    pub fn root(
        account: impl Into<String>,
        id: impl Into<String>,
        name: impl Into<String>,
    ) -> Self {
        Self {
            account: account.into(),
            segments: vec![Segment {
                id: id.into(),
                name: name.into(),
            }],
        }
    }

    /// The id of the item this location points at.
    pub fn file_id(&self) -> Option<&str> {
        self.segments.last().map(|s| s.id.as_str())
    }

    /// The display name of the item this location points at.
    pub fn name(&self) -> Option<&str> {
        self.segments.last().map(|s| s.name.as_str())
    }

    /// The canonical URI for this location.
    pub fn to_uri(&self) -> String {
        let mut out = String::from(DRIVE_SCHEME);
        out.push_str(&self.account);
        for segment in &self.segments {
            out.push('/');
            out.push_str(&segment.id);
            out.push('~');
            out.push_str(&encode_name(&segment.name));
        }
        out
    }

    pub fn parse(path: &str) -> Result<Self, String> {
        let rest = path
            .strip_prefix(DRIVE_SCHEME)
            .ok_or_else(|| "Not a Google Drive location".to_string())?;
        let rest = rest.trim_end_matches('/');
        if rest.is_empty() {
            return Err("Google Drive location is missing an account".into());
        }

        let mut parts = rest.split('/');
        let account = parts.next().unwrap_or_default();
        if !valid_account(account) {
            return Err(format!("Invalid Google Drive account “{account}”"));
        }

        let mut segments = Vec::new();
        for raw in parts {
            if raw.is_empty() {
                return Err("Google Drive location has an empty path segment".into());
            }
            segments.push(Segment::parse(raw)?);
        }
        if segments.is_empty() {
            return Err("Google Drive location is missing a file id".into());
        }

        Ok(Self {
            account: account.to_string(),
            segments,
        })
    }
}

impl Segment {
    fn parse(raw: &str) -> Result<Self, String> {
        let (id, encoded_name) = raw.split_once('~').unwrap_or((raw, ""));
        if !valid_id(id) {
            return Err(format!("Invalid Google Drive file id “{id}”"));
        }
        let name = decode_name(encoded_name)?;
        Ok(Self {
            id: id.to_string(),
            name,
        })
    }
}

impl fmt::Display for DriveLocation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_uri())
    }
}

impl fmt::Display for Location {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Location::Local(path) => write!(f, "{}", path.display()),
            Location::Drive(drive) => drive.fmt(f),
        }
    }
}

/// Whether `path` is a Drive URI. Local paths and `file://` are not.
pub fn is_cloud(path: &str) -> bool {
    path.starts_with(DRIVE_SCHEME)
}

/// Resolve a path into a local path or a Drive location. A malformed
/// `gdrive://` URI is an error; it must never fall through to `std::fs`.
pub fn parse(path: &str) -> Result<Location, String> {
    if is_cloud(path) {
        DriveLocation::parse(path).map(Location::Drive)
    } else {
        Ok(Location::Local(PathBuf::from(path)))
    }
}

fn valid_account(account: &str) -> bool {
    !account.is_empty()
        && account.len() <= ACCOUNT_MAX
        && account
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= ID_MAX
        && id != "."
        && id != ".."
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-' || b == b'.')
}

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

fn decode_name(encoded: &str) -> Result<String, String> {
    if !valid_percent(encoded) {
        return Err("Google Drive name has a malformed percent escape".into());
    }
    let name = percent_decode_str(encoded)
        .decode_utf8()
        .map_err(|_| "Google Drive name is not valid UTF-8".to_string())?
        .into_owned();
    if name.contains('/') || name.contains('\0') {
        return Err("Google Drive name contains an invalid character".into());
    }
    Ok(name)
}

fn encode_name(name: &str) -> String {
    utf8_percent_encode(name, NAME_SET).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(id: &str, name: &str) -> DriveLocation {
        DriveLocation {
            account: "acct".into(),
            segments: vec![Segment {
                id: id.into(),
                name: name.into(),
            }],
        }
    }

    #[test]
    fn round_trips_every_kind_of_name() {
        for name in [
            "report.docx",
            "My Drive",
            "50% off",
            "a~b",
            "a#b?c",
            "üñî çödé",
            "",
            "...",
            "a.b-c_d",
        ] {
            let loc = one("file-id", name);
            let uri = loc.to_uri();
            assert_eq!(
                DriveLocation::parse(&uri).unwrap(),
                loc,
                "round trip failed for {name:?} ({uri})"
            );
        }
    }

    #[test]
    fn the_separator_and_percent_are_encoded() {
        let uri = one("file-id", "a~b%c").to_uri();
        assert!(uri.contains('%'), "the name must be percent-encoded");
        assert!(uri.ends_with("file-id~a%7Eb%25c"), "got {uri}");
    }

    #[test]
    fn duplicate_names_stay_distinct() {
        let a = one("id-a", "same.txt");
        let b = one("id-b", "same.txt");
        assert_ne!(a.to_uri(), b.to_uri());
    }

    #[test]
    fn an_id_without_a_name_is_accepted() {
        let loc = DriveLocation::parse("gdrive://acct/file-id").unwrap();
        assert_eq!(loc.name(), Some(""));
    }

    #[test]
    fn a_trailing_slash_is_normalized() {
        let loc = DriveLocation::parse("gdrive://acct/file-id~name/").unwrap();
        assert_eq!(loc, one("file-id", "name"));
    }

    #[test]
    fn invalid_ids_are_rejected() {
        for uri in [
            "gdrive://acct/../x",
            "gdrive://acct/.",
            "gdrive://acct/a b~x",
            "gdrive://acct/",
            "gdrive://acct//x~y",
        ] {
            assert!(DriveLocation::parse(uri).is_err(), "accepted {uri}");
        }
    }

    #[test]
    fn invalid_names_are_rejected() {
        for uri in [
            "gdrive://acct/id~a%2Fb",
            "gdrive://acct/id~a%ZZb",
            "gdrive://acct/id~a%",
            "gdrive://acct/id~%FF",
        ] {
            assert!(DriveLocation::parse(uri).is_err(), "accepted {uri}");
        }
    }

    #[test]
    fn invalid_accounts_are_rejected() {
        assert!(DriveLocation::parse("gdrive:///id~x").is_err());
        assert!(DriveLocation::parse("gdrive://a b/id~x").is_err());
        assert!(DriveLocation::parse(&format!("gdrive://{}/id~x", "a".repeat(65))).is_err());
    }

    #[test]
    fn the_uri_has_at_least_one_segment() {
        assert!(
            DriveLocation::root("acct", "root", "My Drive")
                .to_uri()
                .ends_with("root~My%20Drive")
        );
    }

    #[test]
    fn display_and_parse_agree() {
        let loc = Location::Drive(one("id", "a b"));
        assert_eq!(loc.to_string(), "gdrive://acct/id~a%20b");
        assert_eq!(
            DriveLocation::parse(&loc.to_string()).unwrap(),
            one("id", "a b")
        );
    }
}
