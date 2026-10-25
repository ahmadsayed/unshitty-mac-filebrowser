//! The share control protocol (spec D5): one postcard request per
//! bidirectional stream, each message behind a u32 length prefix and capped
//! at `MAX_MESSAGE`.

use super::link::{ShareId, Token};
use iroh::endpoint::{RecvStream, SendStream};
use iroh_blobs::Hash;
use serde::{Deserialize, Serialize, de::DeserializeOwned};

pub const ALPN: &[u8] = b"filebrowser/share/1";
pub const MAX_MESSAGE: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    Claim {
        share: ShareId,
        token: Token,
    },
    Root {
        share: ShareId,
    },
    Content {
        share: ShareId,
        root: Hash,
        names: Vec<String>,
    },
    Watch {
        share: ShareId,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Reply {
    Ok {
        folder: String,
        sharer: String,
    },
    Root {
        root: Hash,
        folder: String,
        sharer: String,
    },
    Content {
        hash: Hash,
        size: u64,
    },
    Changed,
    /// The shared folder is gone (answers `Root` and `Content`).
    Missing,
    Denied(String),
    Busy,
}

const TOO_LARGE: &str = "The message is too large.";

pub async fn write_msg<T: Serialize>(send: &mut SendStream, msg: &T) -> Result<(), String> {
    let body = postcard::to_stdvec(msg).map_err(|e| e.to_string())?;
    if body.len() > MAX_MESSAGE {
        return Err(TOO_LARGE.into());
    }
    let len = (body.len() as u32).to_be_bytes();
    send.write_all(&len).await.map_err(|e| e.to_string())?;
    send.write_all(&body).await.map_err(|e| e.to_string())
}

pub async fn read_msg<T: DeserializeOwned>(recv: &mut RecvStream) -> Result<T, String> {
    let mut len = [0u8; 4];
    recv.read_exact(&mut len).await.map_err(|e| e.to_string())?;
    let len = u32::from_be_bytes(len) as usize;
    if len > MAX_MESSAGE {
        return Err(TOO_LARGE.into());
    }
    let mut body = vec![0u8; len];
    recv.read_exact(&mut body)
        .await
        .map_err(|e| e.to_string())?;
    postcard::from_bytes(&body).map_err(|e| e.to_string())
}
