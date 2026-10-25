//! Recipient paths (spec D10): `share://<received-id>/<name>/<name>…`, with
//! names percent-encoded like `gdrive://` names.

use crate::cloud::location::{NAME_SET, valid_percent};
use percent_encoding::{percent_decode_str, utf8_percent_encode};

pub const PREFIX: &str = "share://";

pub fn is_share(path: &str) -> bool {
    path.starts_with(PREFIX)
}

/// A place inside a received share: the entry names leading to it from the
/// share's root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShareLocation {
    pub received: String,
    pub names: Vec<String>,
}

impl ShareLocation {
    /// `None` for anything that isn't a well-formed share path, including
    /// a name that isn't safe (`safe_name`) once decoded.
    pub fn parse(path: &str) -> Option<ShareLocation> {
        let rest = path.strip_prefix(PREFIX)?;
        let rest = rest.strip_suffix('/').unwrap_or(rest);
        let mut parts = rest.split('/');
        let received = parts.next().filter(|r| !r.is_empty())?.to_string();
        let names = parts.map(decode).collect::<Option<Vec<_>>>()?;
        Some(ShareLocation { received, names })
    }

    pub fn to_path(&self) -> String {
        let mut path = format!("{PREFIX}{}", self.received);
        for name in &self.names {
            path.push('/');
            path.extend(utf8_percent_encode(name, NAME_SET));
        }
        path
    }

    pub fn child(&self, name: &str) -> ShareLocation {
        let mut names = self.names.clone();
        names.push(name.to_string());
        ShareLocation {
            received: self.received.clone(),
            names,
        }
    }
}

fn decode(segment: &str) -> Option<String> {
    if !valid_percent(segment) {
        return None;
    }
    let name = percent_decode_str(segment).decode_utf8().ok()?;
    safe_name(&name).map(str::to_string)
}

/// The name if it is safe to use as one path component (D8).
pub fn safe_name(name: &str) -> Option<&str> {
    let bad = name.is_empty() || name == "." || name == ".." || name.contains(['/', '\0']);
    (!bad).then_some(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_share_path_round_trips() {
        let p = "share://r1/Photos/2024%20Trip";
        let loc = ShareLocation::parse(p).unwrap();
        assert_eq!(loc.received, "r1");
        assert_eq!(loc.names, vec!["Photos", "2024 Trip"]);
        assert_eq!(loc.to_path(), p);
        let top = ShareLocation::parse("share://r1").unwrap();
        assert!(top.names.is_empty());
        assert_eq!(top.to_path(), "share://r1");
        assert_eq!(ShareLocation::parse("share://r1/").unwrap(), top);
        let odd = top.child("a b~%#");
        assert_eq!(odd.names, vec!["a b~%#"]);
        assert_eq!(odd.to_path(), "share://r1/a%20b%7E%25%23");
        assert_eq!(ShareLocation::parse(&odd.to_path()).unwrap(), odd);
        assert!(is_share(p));
        assert!(!is_share("/Users/me"));
    }

    #[test]
    fn malformed_share_paths_are_rejected() {
        for p in [
            "/Users/me",
            "share://",
            "share:///Photos",
            "share://r1//x",
            "share://r1/%2E%2E",
            "share://r1/a%2Fb",
            "share://r1/bad%zz",
            "share://r1/%FF",
        ] {
            assert_eq!(ShareLocation::parse(p), None, "{p}");
        }
    }

    #[test]
    fn unsafe_names_are_rejected() {
        for n in ["", ".", "..", "a/b", "a\0b"] {
            assert_eq!(safe_name(n), None, "{n:?}");
        }
        for n in ["a", ".hidden", "..x", "a b", "a\\b"] {
            assert_eq!(safe_name(n), Some(n));
        }
    }
}
