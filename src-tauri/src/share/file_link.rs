//! Sharing files with a QR code (spec: 2026-10-30-qr-file-share-design):
//! the ticket a link carries and the `filebrowser/file/2` wire format. Pure
//! code, compiled into the app (the sharer) and into the page a phone opens
//! (`web/file-get`, through `#[path]`), so both sides always agree.
//!
//! A link is `<page>#<ticket>`: browsers never send what follows `#` to the
//! page's server. The ticket names the sharer's endpoint and home relay (a
//! browser can only connect through a relay), a random token that is the
//! permission to download, the number of files, their total size and the
//! BLAKE3 hash of their list. The list names each file with its size and
//! hash, so the phone checks every byte it saves, and the QR code stays the
//! same size however many files there are.
//!
//! On the wire, one bidirectional stream per request: the phone sends the
//! token, which file it wants (or `LIST`) and the offset to start from; the
//! sharer answers one status byte, then (when the status is `OK`) the list
//! (u32 length, little-endian, then the list) or the file's bytes from the
//! offset to the end.
//!
//! Version 1 links (0.7.0) named one file; the page still downloads them
//! (`ALPN_V1`, `encode_request_v1`; the sharer answers the file's name
//! before its bytes).

// The phone's half (decoding tickets and lists, making requests, naming the
// saved file) is only used by the page.
#![allow(dead_code)]

use data_encoding::BASE64URL_NOPAD;
use iroh::{EndpointId, RelayUrl};

pub const ALPN: &[u8] = b"filebrowser/file/2";
pub const ALPN_V1: &[u8] = b"filebrowser/file/1";
const VERSION: u8 = 2;
const VERSION_1: u8 = 1;
pub const TOKEN_LEN: usize = 16;
/// Token, file index and offset.
pub const REQUEST_LEN: usize = TOKEN_LEN + 4 + 8;
/// A version 1 request: token and offset.
pub const REQUEST_LEN_V1: usize = TOKEN_LEN + 8;
/// The file index that asks for the list.
pub const LIST: u32 = u32::MAX;
/// The longest file name the sharer sends.
pub const MAX_NAME: usize = 1024;
/// The most files one share holds.
pub const MAX_FILES: usize = 1000;
/// The longest list: every file at its longest name.
pub const MAX_LIST: usize = 4 + MAX_FILES * (8 + 32 + 2 + MAX_NAME);

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
    /// No file has that index.
    NoSuchFile = 4,
}

const STATUSES: [Status; 5] = [Status::Ok, Status::NotShared, Status::Changed, Status::BadOffset, Status::NoSuchFile];

impl Status {
    pub fn from_byte(byte: u8) -> Option<Status> {
        STATUSES.into_iter().find(|status| *status as u8 == byte)
    }

    /// What the phone shows for a refusal.
    pub fn message(self) -> &'static str {
        match self {
            Status::Ok => "",
            Status::NotShared => "Sharing has stopped. Ask for a new QR code.",
            Status::Changed => "The file changed since it was shared. Ask for a new QR code.",
            Status::BadOffset => "The download could not be resumed.",
            Status::NoSuchFile => "This file is not in the share. Ask for a new QR code.",
        }
    }
}

/// Everything a phone needs to fetch and check the shared files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ticket {
    pub endpoint: EndpointId,
    pub relay: RelayUrl,
    pub token: [u8; TOKEN_LEN],
    /// The list's hash; for a version 1 link, the one file's.
    pub hash: [u8; 32],
    /// The total size of the files.
    pub size: u64,
    /// How many files; `None` for a version 1 link (one file, no list).
    pub files: Option<u32>,
}

const FIXED: usize = 1 + 32 + TOKEN_LEN + 32 + 8;

impl Ticket {
    /// URL-safe text for after the `#`.
    pub fn encode(&self) -> String {
        let mut bytes = Vec::with_capacity(FIXED + 68);
        bytes.push(if self.files.is_some() { VERSION } else { VERSION_1 });
        bytes.extend_from_slice(self.endpoint.as_bytes());
        bytes.extend_from_slice(&self.token);
        bytes.extend_from_slice(&self.hash);
        bytes.extend_from_slice(&self.size.to_le_bytes());
        if let Some(files) = self.files {
            bytes.extend_from_slice(&files.to_le_bytes());
        }
        bytes.extend_from_slice(self.relay.as_str().as_bytes());
        BASE64URL_NOPAD.encode(&bytes)
    }

    pub fn decode(text: &str) -> Result<Ticket, &'static str> {
        const BAD: &str = "This link is damaged. Scan the QR code again.";
        let bytes = BASE64URL_NOPAD.decode(text.trim().as_bytes()).map_err(|_| BAD)?;
        let version = *bytes.first().ok_or(BAD)?;
        if version != VERSION && version != VERSION_1 {
            return Err("This link needs a newer page. Reload it and try again.");
        }
        let fixed = if version == VERSION { FIXED + 4 } else { FIXED };
        if bytes.len() <= fixed {
            return Err(BAD);
        }
        let take = |from: usize, len: usize| &bytes[from..from + len];
        let endpoint = EndpointId::from_bytes(take(1, 32).try_into().map_err(|_| BAD)?).map_err(|_| BAD)?;
        let token = take(33, TOKEN_LEN).try_into().map_err(|_| BAD)?;
        let hash = take(33 + TOKEN_LEN, 32).try_into().map_err(|_| BAD)?;
        let size = u64::from_le_bytes(take(65 + TOKEN_LEN, 8).try_into().map_err(|_| BAD)?);
        let files = match version {
            VERSION => Some(u32::from_le_bytes(take(FIXED, 4).try_into().map_err(|_| BAD)?)),
            _ => None,
        };
        if files.is_some_and(|n| n == 0 || n as usize > MAX_FILES) {
            return Err(BAD);
        }
        let relay = std::str::from_utf8(&bytes[fixed..])
            .ok()
            .and_then(|url| url.parse::<RelayUrl>().ok())
            .ok_or(BAD)?;
        Ok(Ticket { endpoint, relay, token, hash, size, files })
    }
}

pub fn encode_request(token: &[u8; TOKEN_LEN], index: u32, offset: u64) -> [u8; REQUEST_LEN] {
    let mut bytes = [0u8; REQUEST_LEN];
    bytes[..TOKEN_LEN].copy_from_slice(token);
    bytes[TOKEN_LEN..TOKEN_LEN + 4].copy_from_slice(&index.to_le_bytes());
    bytes[TOKEN_LEN + 4..].copy_from_slice(&offset.to_le_bytes());
    bytes
}

pub fn decode_request(bytes: &[u8; REQUEST_LEN]) -> ([u8; TOKEN_LEN], u32, u64) {
    let mut token = [0u8; TOKEN_LEN];
    token.copy_from_slice(&bytes[..TOKEN_LEN]);
    let index = u32::from_le_bytes(bytes[TOKEN_LEN..TOKEN_LEN + 4].try_into().unwrap());
    let offset = u64::from_le_bytes(bytes[TOKEN_LEN + 4..].try_into().unwrap());
    (token, index, offset)
}

pub fn encode_request_v1(token: &[u8; TOKEN_LEN], offset: u64) -> [u8; REQUEST_LEN_V1] {
    let mut bytes = [0u8; REQUEST_LEN_V1];
    bytes[..TOKEN_LEN].copy_from_slice(token);
    bytes[TOKEN_LEN..].copy_from_slice(&offset.to_le_bytes());
    bytes
}

/// One file in a share's list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listed {
    pub name: String,
    pub size: u64,
    pub hash: [u8; 32],
}

/// The list: the count (u32), then for each file its size (u64), hash and
/// name (as `encode_name`). All little-endian.
pub fn encode_list(files: &[Listed]) -> Vec<u8> {
    let mut bytes = (files.len() as u32).to_le_bytes().to_vec();
    for file in files {
        bytes.extend_from_slice(&file.size.to_le_bytes());
        bytes.extend_from_slice(&file.hash);
        bytes.extend(encode_name(&file.name));
    }
    bytes
}

pub fn decode_list(bytes: &[u8]) -> Result<Vec<Listed>, &'static str> {
    const DAMAGED: &str = "The list of files arrived damaged. Try again.";
    let mut at = 0usize;
    let mut take = |len: usize| -> Result<&[u8], &'static str> {
        let part = bytes.get(at..at + len).ok_or(DAMAGED)?;
        at += len;
        Ok(part)
    };
    let count = u32::from_le_bytes(take(4)?.try_into().unwrap()) as usize;
    if count == 0 || count > MAX_FILES {
        return Err(DAMAGED);
    }
    let mut files = Vec::with_capacity(count);
    for _ in 0..count {
        let size = u64::from_le_bytes(take(8)?.try_into().unwrap());
        let hash = take(32)?.try_into().unwrap();
        let len = u16::from_le_bytes(take(2)?.try_into().unwrap()) as usize;
        if len > MAX_NAME {
            return Err(DAMAGED);
        }
        let name = std::str::from_utf8(take(len)?).map_err(|_| DAMAGED)?.to_string();
        files.push(Listed { name, size, hash });
    }
    if at != bytes.len() {
        return Err(DAMAGED);
    }
    Ok(files)
}

/// A name on the wire: its length (u16, little-endian) and UTF-8, cut to
/// `MAX_NAME` bytes.
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
            files: None,
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
        bytes[0] = 3;
        assert!(Ticket::decode(&BASE64URL_NOPAD.encode(&bytes)).unwrap_err().contains("newer"));
        let mut bad_relay = BASE64URL_NOPAD.decode(text.as_bytes()).unwrap();
        bad_relay.truncate(FIXED);
        bad_relay.extend_from_slice(b"not a url");
        assert!(Ticket::decode(&BASE64URL_NOPAD.encode(&bad_relay)).is_err());
    }

    #[test]
    fn requests_and_names_encode_both_ways() {
        assert_eq!(encode_name("a.pdf"), [5, 0, b'a', b'.', b'p', b'd', b'f']);
        let long = "é".repeat(MAX_NAME);
        let encoded = encode_name(&long);
        let len = u16::from_le_bytes([encoded[0], encoded[1]]) as usize;
        assert!(len <= MAX_NAME);
        assert!(std::str::from_utf8(&encoded[2..]).is_ok());
    }

    fn listed(name: &str, size: u64) -> Listed {
        Listed { name: name.into(), size, hash: *blake3::hash(name.as_bytes()).as_bytes() }
    }

    fn list_ticket() -> Ticket {
        Ticket { files: Some(3), ..ticket() }
    }

    #[test]
    fn a_list_ticket_round_trips_and_keeps_its_count() {
        let text = list_ticket().encode();
        assert_eq!(Ticket::decode(&text).unwrap(), list_ticket());
        assert_eq!(BASE64URL_NOPAD.decode(text.as_bytes()).unwrap()[0], 2);
        // A version 1 link (0.7.0) still decodes, as one file.
        assert_eq!(BASE64URL_NOPAD.decode(ticket().encode().as_bytes()).unwrap()[0], 1);
        assert_eq!(Ticket::decode(&ticket().encode()).unwrap().files, None);
    }

    #[test]
    fn a_damaged_list_ticket_is_refused() {
        let mut bytes = BASE64URL_NOPAD.decode(list_ticket().encode().as_bytes()).unwrap();
        bytes.truncate(FIXED + 4);
        assert!(Ticket::decode(&BASE64URL_NOPAD.encode(&bytes)).is_err());
        let mut newer = BASE64URL_NOPAD.decode(list_ticket().encode().as_bytes()).unwrap();
        newer[0] = 3;
        assert!(Ticket::decode(&BASE64URL_NOPAD.encode(&newer)).unwrap_err().contains("newer"));
        let mut none = BASE64URL_NOPAD.decode(list_ticket().encode().as_bytes()).unwrap();
        none[FIXED..FIXED + 4].copy_from_slice(&0u32.to_le_bytes());
        assert!(Ticket::decode(&BASE64URL_NOPAD.encode(&none)).is_err());
    }

    #[test]
    fn list_requests_name_the_file_and_the_offset() {
        let token = [5u8; TOKEN_LEN];
        assert_eq!(decode_request(&encode_request(&token, 2, 42)), (token, 2, 42));
        assert_eq!(decode_request(&encode_request(&token, LIST, 0)), (token, LIST, 0));
        assert_eq!(encode_request_v1(&token, 42).len(), TOKEN_LEN + 8);
    }

    #[test]
    fn a_file_list_round_trips() {
        let files = vec![listed("a.jpg", 10), listed("Été 2026.png", 0), listed("notes.txt", 1 << 40)];
        let bytes = encode_list(&files);
        assert_eq!(decode_list(&bytes).unwrap(), files);
    }

    #[test]
    fn damaged_or_oversized_lists_are_refused() {
        let bytes = encode_list(&[listed("a.jpg", 10), listed("b.jpg", 20)]);
        assert!(decode_list(&bytes[..bytes.len() - 1]).is_err(), "cut short");
        assert!(decode_list(&[bytes.as_slice(), &[0]].concat()).is_err(), "trailing bytes");
        assert!(decode_list(&0u32.to_le_bytes()).is_err(), "no files");
        assert!(decode_list(&((MAX_FILES as u32) + 1).to_le_bytes()).is_err(), "too many files");
        let mut bad_name = encode_list(&[listed("ab", 1)]);
        let at = bad_name.len() - 2;
        bad_name[at] = 0xff;
        assert!(decode_list(&bad_name).is_err(), "not UTF-8");
        let long = "x".repeat(MAX_NAME + 10);
        let names = decode_list(&encode_list(&[listed(&long, 1)])).unwrap();
        assert_eq!(names[0].name.len(), MAX_NAME, "names are cut, as on the wire");
    }

    #[test]
    fn statuses_have_bytes_and_messages() {
        assert!(Status::NoSuchFile.message().contains("QR code"));
        for status in [Status::Ok, Status::NotShared, Status::Changed, Status::BadOffset, Status::NoSuchFile] {
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
