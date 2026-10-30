//! Sharing one file with a QR code (spec: 2026-10-30-qr-file-share-design):
//! the ticket a link carries and the `filebrowser/file/1` wire format. Pure
//! code, compiled into the app (the sharer) and into the page a phone opens
//! (`web/file-get`, through `#[path]`), so both sides always agree.
//!
//! A link is `<page>#<ticket>`: browsers never send what follows `#` to the
//! page's server. The ticket names the sharer's endpoint and home relay (a
//! browser can only connect through a relay), a random token that is the
//! permission to download, and the file's size and BLAKE3 hash, which the
//! phone checks the bytes against.
//!
//! On the wire, one bidirectional stream per download: the phone sends the
//! token and the offset to start from; the sharer answers one status byte,
//! then (when the status is `OK`) the file name and the bytes from the
//! offset to the end.

// The phone's half (decoding tickets, making requests, naming the saved
// file) is only used by the page.
#![allow(dead_code)]

use data_encoding::BASE64URL_NOPAD;
use iroh::{EndpointId, RelayUrl};

pub const ALPN: &[u8] = b"filebrowser/file/1";
const VERSION: u8 = 1;
pub const TOKEN_LEN: usize = 16;
/// Token and offset.
pub const REQUEST_LEN: usize = TOKEN_LEN + 8;
/// The longest file name the sharer sends.
pub const MAX_NAME: usize = 1024;

/// What the sharer answers first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Ok = 0,
    /// Unknown token: the share was stopped, or never existed.
    NotShared = 1,
    /// The file changed (or went away) since its link was made.
    Changed = 2,
    /// The offset is past the end of the file.
    BadOffset = 3,
}

impl Status {
    pub fn from_byte(byte: u8) -> Option<Status> {
        [Status::Ok, Status::NotShared, Status::Changed, Status::BadOffset]
            .into_iter()
            .find(|status| *status as u8 == byte)
    }

    /// What the phone shows for a refusal.
    pub fn message(self) -> &'static str {
        match self {
            Status::Ok => "",
            Status::NotShared => "This file is no longer shared. Ask for a new QR code.",
            Status::Changed => "The file changed since it was shared. Ask for a new QR code.",
            Status::BadOffset => "The download could not be resumed.",
        }
    }
}

/// Everything a phone needs to fetch and check one file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ticket {
    pub endpoint: EndpointId,
    pub relay: RelayUrl,
    pub token: [u8; TOKEN_LEN],
    pub hash: [u8; 32],
    pub size: u64,
}

const FIXED: usize = 1 + 32 + TOKEN_LEN + 32 + 8;

impl Ticket {
    /// URL-safe text for after the `#`.
    pub fn encode(&self) -> String {
        let mut bytes = Vec::with_capacity(FIXED + 64);
        bytes.push(VERSION);
        bytes.extend_from_slice(self.endpoint.as_bytes());
        bytes.extend_from_slice(&self.token);
        bytes.extend_from_slice(&self.hash);
        bytes.extend_from_slice(&self.size.to_le_bytes());
        bytes.extend_from_slice(self.relay.as_str().as_bytes());
        BASE64URL_NOPAD.encode(&bytes)
    }

    pub fn decode(text: &str) -> Result<Ticket, &'static str> {
        const BAD: &str = "This link is damaged. Scan the QR code again.";
        let bytes = BASE64URL_NOPAD.decode(text.trim().as_bytes()).map_err(|_| BAD)?;
        if bytes.len() <= FIXED {
            return Err(BAD);
        }
        if bytes[0] != VERSION {
            return Err("This link needs a newer page. Reload it and try again.");
        }
        let take = |from: usize, len: usize| &bytes[from..from + len];
        let endpoint = EndpointId::from_bytes(take(1, 32).try_into().map_err(|_| BAD)?).map_err(|_| BAD)?;
        let token = take(33, TOKEN_LEN).try_into().map_err(|_| BAD)?;
        let hash = take(33 + TOKEN_LEN, 32).try_into().map_err(|_| BAD)?;
        let size = u64::from_le_bytes(take(65 + TOKEN_LEN, 8).try_into().map_err(|_| BAD)?);
        let relay = std::str::from_utf8(&bytes[FIXED..])
            .ok()
            .and_then(|url| url.parse::<RelayUrl>().ok())
            .ok_or(BAD)?;
        Ok(Ticket { endpoint, relay, token, hash, size })
    }
}

pub fn encode_request(token: &[u8; TOKEN_LEN], offset: u64) -> [u8; REQUEST_LEN] {
    let mut bytes = [0u8; REQUEST_LEN];
    bytes[..TOKEN_LEN].copy_from_slice(token);
    bytes[TOKEN_LEN..].copy_from_slice(&offset.to_le_bytes());
    bytes
}

pub fn decode_request(bytes: &[u8; REQUEST_LEN]) -> ([u8; TOKEN_LEN], u64) {
    let mut token = [0u8; TOKEN_LEN];
    token.copy_from_slice(&bytes[..TOKEN_LEN]);
    let mut offset = [0u8; 8];
    offset.copy_from_slice(&bytes[TOKEN_LEN..]);
    (token, u64::from_le_bytes(offset))
}

/// The name that follows an `OK`: its length (u16, little-endian) and UTF-8.
pub fn encode_name(name: &str) -> Vec<u8> {
    let mut end = name.len().min(MAX_NAME);
    while !name.is_char_boundary(end) {
        end -= 1;
    }
    let name = &name[..end];
    let mut bytes = (name.len() as u16).to_le_bytes().to_vec();
    bytes.extend_from_slice(name.as_bytes());
    bytes
}

/// A name from the sharer, made safe to save under: no folders, no control
/// characters, never empty.
pub fn safe_file_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| if c == '/' || c == '\\' || c.is_control() { '_' } else { c })
        .collect();
    let cleaned = cleaned.trim().trim_start_matches('.').to_string();
    if cleaned.is_empty() { "download".into() } else { cleaned }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ticket() -> Ticket {
        Ticket {
            endpoint: iroh::SecretKey::from_bytes(&[7u8; 32]).public(),
            relay: "https://euw1-1.relay.n0.iroh-canary.iroh.link./".parse().unwrap(),
            token: [9u8; TOKEN_LEN],
            hash: [3u8; 32],
            size: 123_456_789,
        }
    }

    #[test]
    fn a_ticket_round_trips_through_url_safe_text() {
        let text = ticket().encode();
        assert!(text.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'), "{text}");
        assert_eq!(Ticket::decode(&text).unwrap(), ticket());
        assert_eq!(Ticket::decode(&format!(" {text}\n")).unwrap(), ticket());
    }

    #[test]
    fn damaged_or_newer_tickets_are_refused() {
        let text = ticket().encode();
        assert!(Ticket::decode(&text[..40]).is_err());
        assert!(Ticket::decode("not base64 !").is_err());
        assert!(Ticket::decode("").is_err());
        let mut bytes = BASE64URL_NOPAD.decode(text.as_bytes()).unwrap();
        bytes[0] = 2;
        assert!(Ticket::decode(&BASE64URL_NOPAD.encode(&bytes)).unwrap_err().contains("newer"));
        let mut bad_relay = BASE64URL_NOPAD.decode(text.as_bytes()).unwrap();
        bad_relay.truncate(FIXED);
        bad_relay.extend_from_slice(b"not a url");
        assert!(Ticket::decode(&BASE64URL_NOPAD.encode(&bad_relay)).is_err());
    }

    #[test]
    fn requests_and_names_encode_both_ways() {
        let token = [5u8; TOKEN_LEN];
        assert_eq!(decode_request(&encode_request(&token, 42)), (token, 42));
        assert_eq!(encode_name("a.pdf"), [5, 0, b'a', b'.', b'p', b'd', b'f']);
        let long = "é".repeat(MAX_NAME);
        let encoded = encode_name(&long);
        let len = u16::from_le_bytes([encoded[0], encoded[1]]) as usize;
        assert!(len <= MAX_NAME);
        assert!(std::str::from_utf8(&encoded[2..]).is_ok());
    }

    #[test]
    fn statuses_have_bytes_and_messages() {
        for status in [Status::Ok, Status::NotShared, Status::Changed, Status::BadOffset] {
            assert_eq!(Status::from_byte(status as u8), Some(status));
        }
        assert_eq!(Status::from_byte(9), None);
        assert!(Status::NotShared.message().contains("QR code"));
    }

    #[test]
    fn saved_names_stay_in_the_download_folder() {
        assert_eq!(safe_file_name("report.pdf"), "report.pdf");
        assert_eq!(safe_file_name("../../etc/passwd"), "_.._etc_passwd");
        assert_eq!(safe_file_name("a\\b\nc"), "a_b_c");
        assert_eq!(safe_file_name("..."), "download");
        assert_eq!(safe_file_name(""), "download");
    }
}
