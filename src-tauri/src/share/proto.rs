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
const SLOW: &str = "The other side took too long.";

pub async fn write_msg<T: Serialize>(send: &mut SendStream, msg: &T) -> Result<(), String> {
    let body = postcard::to_stdvec(msg).map_err(|e| e.to_string())?;
    if body.len() > MAX_MESSAGE {
        return Err(TOO_LARGE.into());
    }
    let len = (body.len() as u32).to_be_bytes();
    send.write_all(&len).await.map_err(|e| e.to_string())?;
    send.write_all(&body).await.map_err(|e| e.to_string())
}

/// How long one message may take to arrive.
pub const READ_DEADLINE: std::time::Duration = std::time::Duration::from_secs(10);

pub async fn read_msg<T: DeserializeOwned>(recv: &mut RecvStream) -> Result<T, String> {
    read_within(recv, READ_DEADLINE).await
}

pub(crate) async fn read_within<T: DeserializeOwned>(
    recv: &mut RecvStream,
    deadline: std::time::Duration,
) -> Result<T, String> {
    match tokio::time::timeout(deadline, read_now(recv)).await {
        Ok(result) => result,
        Err(_) => Err(SLOW.into()),
    }
}

async fn read_now<T: DeserializeOwned>(recv: &mut RecvStream) -> Result<T, String> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::share::test_endpoint;
    use iroh::address_lookup::memory::MemoryLookup;
    use std::time::Duration;

    #[tokio::test]
    async fn a_partial_length_prefix_times_out() {
        tokio::time::timeout(Duration::from_secs(20), async {
            let server = test_endpoint().await;
            let client = test_endpoint().await;
            let lookup = MemoryLookup::new();
            let accepting_addr = server.addr();
            lookup.add_endpoint_info(accepting_addr.clone());
            client.address_lookup().unwrap().add(lookup);
            let accepting = tokio::spawn(async move {
                let conn = server.accept().await.unwrap().await.unwrap();
                let (_send, recv) = conn.accept_bi().await.unwrap();
                (server, conn, recv)
            });
            let conn = client
                .connect(accepting_addr, iroh_blobs::ALPN)
                .await
                .unwrap();
            let (mut send, _recv) = conn.open_bi().await.unwrap();
            send.write_all(&[0, 0, 0]).await.unwrap();
            let (_server, _theirs, mut recv) = accepting.await.unwrap();
            let started = std::time::Instant::now();
            let got: Result<Request, String> =
                read_within(&mut recv, Duration::from_millis(300)).await;
            assert_eq!(got, Err("The other side took too long.".to_string()));
            assert!(started.elapsed() < Duration::from_secs(5));
            assert_eq!(READ_DEADLINE, Duration::from_secs(10));
        })
        .await
        .expect("timed out")
    }
}
