//! Share links: a self-contained, copy-pasteable text that says where a
//! share lives and carries the secret needed to open it.
//! Version 2 encrypts the entire version 1 ticket with an optional passphrase.

use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::{
    XChaCha20Poly1305, XNonce,
    aead::{Aead, KeyInit, Payload},
};
use data_encoding::{Encoding, Specification};
use serde::{Deserialize, Serialize};
use std::sync::LazyLock;
use tokio::sync::Semaphore;
use zeroize::Zeroizing;

pub type ShareId = [u8; 16];
pub type Token = [u8; 32];

/// Links read `share://<ticket>`, like the `share://<id>/…` locations a
/// received share opens at; the frontend tells them apart by length, since a
/// ticket is always far longer than a 16-character id.
pub const LINK_PREFIX: &str = "share://";
const VERSION: u8 = 1;
const PROTECTED_VERSION: u8 = 2;
const NOT_A_LINK: &str = "This isn't a share link.";
const NEWER_VERSION: &str = "This link needs a newer version of Files.";
pub const PASSPHRASE_REQUIRED: &str = "This share link requires a passphrase.";
const CANNOT_UNLOCK: &str = "Incorrect passphrase or damaged share link.";
const TOO_LARGE: &str = "This share link is too large.";
const MAX_BYTES: usize = 64 * 1024;
const MAX_TEXT: usize = (MAX_BYTES * 8).div_ceil(5);
const HEADER_LEN: usize = 1 + 16 + 24;
const TAG_LEN: usize = 16;
const CONTEXT: &[u8] = b"filebrowser/share-link/2";

/// One 64 MiB key derivation at a time, even if an IPC caller is cancelled.
static CRYPTO_SLOT: Semaphore = Semaphore::const_new(1);

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

enum Ticket {
    Plain(ShareLink),
    Protected(Vec<u8>),
}

/// Lowercase base32 without padding.
static B32: LazyLock<Encoding> = LazyLock::new(|| {
    let mut spec = Specification::new();
    spec.symbols.push_str("abcdefghijklmnopqrstuvwxyz234567");
    spec.encoding().expect("valid base32 specification")
});

impl ShareLink {
    pub fn encode(&self) -> String {
        encode_bytes(&self.payload())
    }

    fn payload(&self) -> Vec<u8> {
        let wire = Wire {
            version: VERSION,
            addr: self.addr.clone(),
            share: self.share,
            token: self.token,
            kind: self.kind,
            sharer: self.sharer.clone(),
        };
        postcard::to_stdvec(&wire).expect("a link always serializes")
    }

    /// Plain tickets in private settings need no passphrase. A protected
    /// invitation can only be opened through `decode_pasted`.
    pub fn decode(text: &str) -> Result<ShareLink, String> {
        match parse(text)? {
            Ticket::Plain(link) => Ok(link),
            Ticket::Protected(_) => Err(PASSPHRASE_REQUIRED.into()),
        }
    }

    /// Detect protection locally; malformed and future-version links fail
    /// here, before showing a prompt or starting a network endpoint.
    pub fn requires_passphrase(text: &str) -> Result<bool, String> {
        Ok(matches!(parse(text)?, Ticket::Protected(_)))
    }

    /// Unlock only when needed. Key derivation never runs on an async worker.
    pub async fn decode_pasted(
        text: &str,
        passphrase: Option<String>,
    ) -> Result<ShareLink, String> {
        let passphrase = passphrase.map(Zeroizing::new);
        let bytes = match parse(text)? {
            Ticket::Plain(link) => return Ok(link),
            Ticket::Protected(bytes) => bytes,
        };
        let passphrase = passphrase.ok_or(PASSPHRASE_REQUIRED)?;
        check_passphrase(&passphrase)?;
        crypto(move || {
            let key = derive_key(&passphrase, &bytes[1..17])?;
            let cipher = XChaCha20Poly1305::new((&*key).into());
            let aad = authenticated_header(&bytes[..HEADER_LEN]);
            let plain = cipher
                .decrypt(
                    XNonce::from_slice(&bytes[17..HEADER_LEN]),
                    Payload { msg: &bytes[HEADER_LEN..], aad: &aad },
                )
                .map(Zeroizing::new)
                .map_err(|_| CANNOT_UNLOCK)?;
            from_payload(&plain).map_err(|_| CANNOT_UNLOCK.to_string())
        })
        .await
    }

    /// Make a fresh encrypted invitation. Only ciphertext is returned; the
    /// passphrase and derived key are transient and zeroized on drop.
    pub async fn protect(self, passphrase: Zeroizing<String>) -> Result<String, String> {
        check_passphrase(&passphrase)?;
        let plain = Zeroizing::new(self.payload());
        if plain.len() + HEADER_LEN + TAG_LEN > MAX_BYTES {
            return Err(TOO_LARGE.into());
        }
        crypto(move || {
            let mut header = [0u8; HEADER_LEN];
            header[0] = PROTECTED_VERSION;
            getrandom::fill(&mut header[1..]).map_err(|_| "Could not protect the share link.")?;
            let key = derive_key(&passphrase, &header[1..17])?;
            let cipher = XChaCha20Poly1305::new((&*key).into());
            let aad = authenticated_header(&header);
            let encrypted = cipher
                .encrypt(
                    XNonce::from_slice(&header[17..]),
                    Payload { msg: &plain, aad: &aad },
                )
                .map_err(|_| "Could not protect the share link.")?;
            let mut bytes = header.to_vec();
            bytes.extend_from_slice(&encrypted);
            Ok(encode_bytes(&bytes))
        })
        .await
    }
}

fn encode_bytes(bytes: &[u8]) -> String {
    format!("{LINK_PREFIX}{}", B32.encode(bytes))
}

fn ticket_bytes(text: &str) -> Result<Vec<u8>, String> {
    let mut t = text.trim();
    if let Some(inner) = t.strip_prefix('<').and_then(|s| s.strip_suffix('>')) {
        t = inner.trim();
    }
    if t.len() > LINK_PREFIX.len() + MAX_TEXT + 1 {
        return Err(TOO_LARGE.into());
    }
    let t = t.to_ascii_lowercase();
    let body = t.strip_prefix(LINK_PREFIX).ok_or(NOT_A_LINK)?;
    let body = body.strip_suffix('/').unwrap_or(body);
    let bytes = B32.decode(body.as_bytes()).map_err(|_| NOT_A_LINK)?;
    if bytes.len() > MAX_BYTES {
        return Err(TOO_LARGE.into());
    }
    Ok(bytes)
}

fn parse(text: &str) -> Result<Ticket, String> {
    let bytes = ticket_bytes(text)?;
    match bytes.first() {
        Some(&VERSION) => from_payload(&bytes).map(Ticket::Plain),
        Some(&PROTECTED_VERSION) if bytes.len() > HEADER_LEN + TAG_LEN => {
            Ok(Ticket::Protected(bytes))
        }
        Some(&PROTECTED_VERSION) | None => Err(NOT_A_LINK.into()),
        Some(_) => Err(NEWER_VERSION.into()),
    }
}

fn from_payload(bytes: &[u8]) -> Result<ShareLink, String> {
    let (w, rest): (Wire, _) = postcard::take_from_bytes(bytes).map_err(|_| NOT_A_LINK)?;
    if w.version != VERSION || !rest.is_empty() {
        return Err(NOT_A_LINK.into());
    }
    Ok(ShareLink {
        addr: w.addr,
        share: w.share,
        token: w.token,
        kind: w.kind,
        sharer: w.sharer,
    })
}

fn check_passphrase(passphrase: &str) -> Result<(), String> {
    if passphrase.is_empty() {
        return Err("Enter a passphrase.".into());
    }
    if passphrase.len() > 1024 {
        return Err("Use a passphrase of at most 1,024 bytes.".into());
    }
    Ok(())
}

fn derive_key(passphrase: &str, salt: &[u8]) -> Result<Zeroizing<[u8; 32]>, String> {
    // Fixed by link version: untrusted input never selects the work factor.
    let params = Params::new(64 * 1024, 3, 1, Some(32)).expect("valid Argon2 parameters");
    let mut memory = Zeroizing::new(vec![argon2::Block::default(); params.block_count()]);
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut key = Zeroizing::new([0u8; 32]);
    argon
        .hash_password_into_with_memory(passphrase.as_bytes(), salt, &mut *key, &mut *memory)
        .map_err(|_| "Could not process the passphrase.".to_string())?;
    Ok(key)
}

fn authenticated_header(header: &[u8]) -> Vec<u8> {
    let mut aad = CONTEXT.to_vec();
    aad.extend_from_slice(header);
    aad
}

async fn crypto<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    let permit = CRYPTO_SLOT.acquire().await.expect("crypto semaphore stays open");
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        work()
    })
    .await
    .map_err(|_| "Could not process the share link.".to_string())?
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
            version: 3,
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

    const PHRASE: &str = "  Violet river — 灯台  ";

    #[tokio::test]
    async fn a_reference_argon2_and_libsodium_ticket_unlocks() {
        // Independent fixture: argon2-cffi (ID, v19, m=65536, t=3, p=1)
        // and libsodium's crypto_aead_xchacha20poly1305_ietf_encrypt.
        // Salt = 00..0f, nonce = 10..27, plaintext = the v1 sample below.
        const PLAIN: &str = "share://ahveu3dd4kofecv66vihwezoyx4zkr3wv27l464siipou2iui3jcyaabaeaqcaibaeaqcaibaeaqcaibaibaeaqcaibaeaqcaibaeaqcaibaeaqcaibaeaqcaibaeaqcaibacbkbnbwwkza";
        const PROTECTED: &str = "share://aiaacaqdaqcqmbyibefawdanbyhraeiscmkbkfqxdamrugy4dupb6ibbeirsijjge7x7z2z73cfiolgb3txuecci2puzi3w5ji7bwx4jmgl4klsddo4pe7xrccpphn2ygsdxmlydnkgj6gmd4vovgeclrqttv5n42pvzlgvwbrdxgwa2auiurpvv3p4ssymcsykj4s6qzga7veflqmxoeb6g2kep7k2pz5cih3tfme";
        assert_eq!(sample().encode(), PLAIN);
        let decoded = ShareLink::decode_pasted(PROTECTED, Some("violet river lantern".into())).await.unwrap();
        assert_eq!(decoded, sample());
    }

    #[tokio::test]
    async fn protected_links_round_trip_both_kinds_without_exposing_the_ticket() {
        for kind in [LinkKind::Anyone, LinkKind::Person] {
            let mut link = sample();
            link.kind = kind;
            link.addr = link.addr.with_ip_addr("127.0.0.1:12345".parse().unwrap());
            let encoded = link.clone().protect(Zeroizing::new(PHRASE.into())).await.unwrap();
            assert!(ShareLink::requires_passphrase(&encoded).unwrap());
            assert_eq!(ShareLink::decode(&encoded), Err(PASSPHRASE_REQUIRED.into()));
            assert_eq!(ShareLink::decode_pasted(&encoded, None).await, Err(PASSPHRASE_REQUIRED.into()));
            let bytes = ticket_bytes(&encoded).unwrap();
            assert!(!bytes.windows(link.token.len()).any(|w| w == link.token));
            assert!(!bytes.windows(link.sharer.len()).any(|w| w == link.sharer.as_bytes()));
            assert!(!bytes.windows(PHRASE.len()).any(|w| w == PHRASE.as_bytes()));
            let pasted = format!("  <{}/>", encoded.to_uppercase());
            assert_eq!(ShareLink::decode_pasted(&pasted, Some(PHRASE.into())).await.unwrap(), link);
        }
    }

    #[tokio::test]
    async fn repeated_encryption_randomizes_salt_nonce_and_ciphertext() {
        let a = sample().protect(Zeroizing::new(PHRASE.into())).await.unwrap();
        let b = sample().protect(Zeroizing::new(PHRASE.into())).await.unwrap();
        let a = ticket_bytes(&a).unwrap();
        let b = ticket_bytes(&b).unwrap();
        assert_ne!(&a[1..17], &b[1..17]);
        assert_ne!(&a[17..HEADER_LEN], &b[17..HEADER_LEN]);
        assert_ne!(&a[HEADER_LEN..], &b[HEADER_LEN..]);
    }

    #[tokio::test]
    async fn a_passphrase_is_exact_and_cannot_be_replaced_by_a_name() {
        let text = sample().protect(Zeroizing::new(PHRASE.into())).await.unwrap();
        for wrong in ["Ahmed", PHRASE.trim(), &PHRASE.to_lowercase()] {
            assert_eq!(
                ShareLink::decode_pasted(&text, Some(wrong.into())).await,
                Err(CANNOT_UNLOCK.into()),
            );
        }
    }

    #[tokio::test]
    async fn changes_to_the_header_ciphertext_or_tag_are_rejected() {
        let text = sample().protect(Zeroizing::new(PHRASE.into())).await.unwrap();
        let original = ticket_bytes(&text).unwrap();
        for at in [1, 17, HEADER_LEN, original.len() - 1] {
            let mut bytes = original.clone();
            bytes[at] ^= 1;
            assert_eq!(
                ShareLink::decode_pasted(&encode_bytes(&bytes), Some(PHRASE.into())).await,
                Err(CANNOT_UNLOCK.into()),
            );
        }
        assert_eq!(
            ShareLink::decode_pasted(&encode_bytes(&original[..original.len() - 1]), Some(PHRASE.into())).await,
            Err(CANNOT_UNLOCK.into()),
        );
        let mut downgraded = original;
        downgraded[0] = VERSION;
        assert!(ShareLink::decode(&encode_bytes(&downgraded)).is_err());
    }

    #[tokio::test]
    async fn invalid_envelopes_and_passphrases_are_refused() {
        let mut truncated = vec![0; HEADER_LEN + TAG_LEN];
        truncated[0] = PROTECTED_VERSION;
        for end in [1, 17, HEADER_LEN, truncated.len()] {
            let text = encode_bytes(&truncated[..end]);
            assert_eq!(ShareLink::requires_passphrase(&text), Err(NOT_A_LINK.into()));
            assert_eq!(ShareLink::decode_pasted(&text, Some(PHRASE.into())).await, Err(NOT_A_LINK.into()));
        }
        assert_eq!(ShareLink::requires_passphrase(&encode_bytes(&[3])), Err(NEWER_VERSION.into()));
        let oversized = format!("{LINK_PREFIX}{}", "a".repeat(MAX_TEXT + 2));
        assert_eq!(ShareLink::requires_passphrase(&oversized), Err(TOO_LARGE.into()));
        let mut trailing = sample().payload();
        trailing.push(0);
        assert_eq!(ShareLink::decode(&encode_bytes(&trailing)), Err(NOT_A_LINK.into()));

        for (passphrase, error) in [
            (String::new(), "Enter a passphrase."),
            ("灯".repeat(342), "Use a passphrase of at most 1,024 bytes."),
        ] {
            assert_eq!(sample().protect(Zeroizing::new(passphrase)).await, Err(error.into()));
        }
        let mut too_big = sample();
        too_big.sharer = "x".repeat(MAX_BYTES);
        assert_eq!(too_big.protect(Zeroizing::new(PHRASE.into())).await, Err(TOO_LARGE.into()));
        let plain = sample().encode();
        assert!(!ShareLink::requires_passphrase(&plain).unwrap());
        assert_eq!(ShareLink::decode_pasted(&plain, None).await.unwrap(), sample());
    }
}
