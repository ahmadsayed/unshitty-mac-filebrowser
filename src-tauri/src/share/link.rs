//! Share links: a self-contained, copy-pasteable text that says where a
//! share lives and carries the secret needed to open it.

use data_encoding::{Encoding, Specification};
use serde::{Deserialize, Serialize};
use std::sync::LazyLock;

pub type ShareId = [u8; 16];
pub type Token = [u8; 32];

/// Links read `share://<ticket>`, like the `share://<id>/…` locations a
/// received share opens at; the frontend tells them apart by length, since a
/// ticket is always far longer than a 16-character id.
pub const LINK_PREFIX: &str = "share://";
const VERSION: u8 = 1;
const NOT_A_LINK: &str = "This isn't a share link.";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LinkKind {
    Anyone,
    Person,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShareLink {
    pub addr: iroh::EndpointAddr,
    pub share: ShareId,
    pub token: Token,
    pub kind: LinkKind,
    pub sharer: String,
}

/// On-the-wire shape; `version` stays first so it can be checked alone.
#[derive(Serialize, Deserialize)]
struct Wire {
    version: u8,
    addr: iroh::EndpointAddr,
    share: ShareId,
    token: Token,
    kind: LinkKind,
    sharer: String,
}

/// Lowercase base32 without padding.
static B32: LazyLock<Encoding> = LazyLock::new(|| {
    let mut spec = Specification::new();
    spec.symbols.push_str("abcdefghijklmnopqrstuvwxyz234567");
    spec.encoding().expect("valid base32 specification")
});

impl ShareLink {
    pub fn encode(&self) -> String {
        let wire = Wire {
            version: VERSION,
            addr: self.addr.clone(),
            share: self.share,
            token: self.token,
            kind: self.kind,
            sharer: self.sharer.clone(),
        };
        let bytes = postcard::to_stdvec(&wire).expect("a link always serializes");
        format!("{LINK_PREFIX}{}", B32.encode(&bytes))
    }

    pub fn decode(text: &str) -> Result<ShareLink, String> {
        let mut t = text.trim();
        if let Some(inner) = t.strip_prefix('<').and_then(|s| s.strip_suffix('>')) {
            t = inner.trim();
        }
        let t = t.to_ascii_lowercase();
        let body = t.strip_prefix(LINK_PREFIX).ok_or(NOT_A_LINK)?;
        let body = body.strip_suffix('/').unwrap_or(body);
        let bytes = B32.decode(body.as_bytes()).map_err(|_| NOT_A_LINK)?;
        match bytes.first() {
            None => return Err(NOT_A_LINK.into()),
            Some(&v) if v != VERSION => {
                return Err("This link needs a newer version of Files.".into());
            }
            _ => {}
        }
        let w: Wire = postcard::from_bytes(&bytes).map_err(|_| NOT_A_LINK)?;
        Ok(ShareLink {
            addr: w.addr,
            share: w.share,
            token: w.token,
            kind: w.kind,
            sharer: w.sharer,
        })
    }
}

fn random<const N: usize>() -> [u8; N] {
    let mut b = [0u8; N];
    getrandom::fill(&mut b).expect("system randomness");
    b
}

pub fn random_id() -> ShareId {
    random()
}

pub fn random_token() -> Token {
    random()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> ShareLink {
        let key = iroh::SecretKey::from_bytes(&[7u8; 32]);
        ShareLink {
            addr: iroh::EndpointAddr::new(key.public()),
            share: [1; 16],
            token: [2; 32],
            kind: LinkKind::Person,
            sharer: "Ahmed".into(),
        }
    }

    #[test]
    fn a_link_round_trips() {
        let l = sample();
        let text = l.encode();
        assert!(text.starts_with("share://"));
        assert_eq!(ShareLink::decode(&text).unwrap(), l);
    }

    /// The frontend takes `share://` + 64 or more base32 characters for a
    /// link and anything shorter for a received share's location.
    #[test]
    fn a_link_is_far_longer_than_a_received_id() {
        let text = sample().encode();
        let ticket = text.strip_prefix("share://").unwrap();
        assert!(
            ticket.len() >= 64,
            "ticket is only {} characters",
            ticket.len()
        );
        assert!(
            ticket
                .bytes()
                .all(|b| b"abcdefghijklmnopqrstuvwxyz234567".contains(&b))
        );
    }

    #[test]
    fn only_share_links_decode() {
        let text = sample().encode();
        let ticket = text.strip_prefix("share://").unwrap();
        assert_eq!(
            ShareLink::decode(&format!("fbshare1:{ticket}")),
            Err("This isn't a share link.".to_string())
        );
    }

    #[test]
    fn decode_accepts_a_trailing_slash_and_any_case() {
        let l = sample();
        let text = l.encode();
        assert_eq!(ShareLink::decode(&format!("{text}/")).unwrap(), l);
        assert_eq!(ShareLink::decode(&text.to_uppercase()).unwrap(), l);
    }

    #[test]
    fn decode_tolerates_chat_wrapping() {
        let l = sample();
        let text = format!("  <{}>\n", l.encode());
        assert_eq!(ShareLink::decode(&text).unwrap(), l);
    }

    #[test]
    fn decode_rejects_garbage() {
        for t in [
            "hello",
            "fbshare1:aeu73ziw",
            "share://!!!",
            "share://0123456789abcdef",
            "share://",
            "",
        ] {
            assert_eq!(
                ShareLink::decode(t),
                Err("This isn't a share link.".to_string())
            );
        }
    }

    #[test]
    fn decode_rejects_a_truncated_link() {
        let text = sample().encode();
        let cut = &text[..text.len() - 10];
        assert_eq!(
            ShareLink::decode(cut),
            Err("This isn't a share link.".to_string())
        );
    }

    #[test]
    fn decode_rejects_another_version() {
        let l = sample();
        let wire = Wire {
            version: 2,
            addr: l.addr,
            share: l.share,
            token: l.token,
            kind: l.kind,
            sharer: l.sharer,
        };
        let text = format!(
            "{LINK_PREFIX}{}",
            B32.encode(&postcard::to_stdvec(&wire).unwrap())
        );
        assert_eq!(
            ShareLink::decode(&text),
            Err("This link needs a newer version of Files.".to_string())
        );
    }
}
