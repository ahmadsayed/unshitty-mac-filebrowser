//! The page a phone opens from a QR code (see `index.html`): connect to the
//! sharer through its relay, download the shared files and check each one
//! against the hashes the link vouches for. The wire format is the app's
//! own module, compiled in here so both sides always agree.

use iroh::endpoint::{Connection, RecvStream, presets};
use iroh::{Endpoint, EndpointAddr, RelayMode};
use js_sys::{Array, Function, Uint8Array};
use wasm_bindgen::prelude::*;

#[path = "../../../src-tauri/src/share/file_link.rs"]
mod file_link;

use file_link::{Listed, Status, Ticket};

const CHUNK: usize = 64 * 1024;
const UNREACHABLE: &str =
    "Could not reach the computer sharing this file. It has to be on, with its QR code still showing.";
const DAMAGED: &str = "The file arrived damaged. Try again.";
const NEWER: &str = "The computer sharing this file needs a newer version of this page.";

fn fail(message: &str) -> JsError {
    JsError::new(message)
}

/// The total size a link announces, before connecting.
#[wasm_bindgen]
pub fn ticket_size(ticket: &str) -> Result<f64, JsError> {
    Ok(Ticket::decode(ticket).map_err(fail)?.size as f64)
}

/// How many files a link shares (a version 1 link shares one).
#[wasm_bindgen]
pub fn ticket_files(ticket: &str) -> Result<u32, JsError> {
    Ok(Ticket::decode(ticket).map_err(fail)?.files.unwrap_or(1))
}

/// Download every file a ticket shares. `on_list(names, sizes)` runs once
/// the sharer has named them, `on_chunk(index, bytes)` for every piece in
/// order; the promise resolves once every file has arrived whole and
/// matches its hash.
#[wasm_bindgen]
pub async fn download(ticket: String, on_list: Function, on_chunk: Function) -> Result<(), JsError> {
    let ticket = Ticket::decode(&ticket).map_err(fail)?;
    // Browsers only reach peers through a relay: use the sharer's own.
    let endpoint = Endpoint::builder(presets::Minimal)
        .relay_mode(RelayMode::Custom(ticket.relay.clone().into()))
        .bind()
        .await
        .map_err(|e| JsError::new(&e.to_string()))?;
    let result = match ticket.files {
        Some(_) => fetch_list(&endpoint, &ticket, &on_list, &on_chunk).await,
        None => fetch_one(&endpoint, &ticket, &on_list, &on_chunk).await,
    };
    endpoint.close().await;
    result
}

async fn connect(endpoint: &Endpoint, ticket: &Ticket, alpn: &[u8]) -> Result<Connection, JsError> {
    let addr = EndpointAddr::new(ticket.endpoint).with_relay_url(ticket.relay.clone());
    endpoint.connect(addr, alpn).await.map_err(|_| fail(UNREACHABLE))
}

/// Read the status byte that starts every answer.
async fn status(recv: &mut RecvStream) -> Result<(), JsError> {
    let mut status = [0u8; 1];
    recv.read_exact(&mut status).await.map_err(|_| fail(UNREACHABLE))?;
    match Status::from_byte(status[0]) {
        Some(Status::Ok) => Ok(()),
        Some(refused) => Err(fail(refused.message())),
        None => Err(fail(NEWER)),
    }
}

/// Send a request on a new stream and wait for its `OK`.
async fn ask(conn: &Connection, request: &[u8]) -> Result<RecvStream, JsError> {
    let (mut send, mut recv) = conn.open_bi().await.map_err(|_| fail(UNREACHABLE))?;
    send.write_all(request).await.map_err(|_| fail(UNREACHABLE))?;
    send.finish().ok();
    status(&mut recv).await?;
    Ok(recv)
}

fn announce(on_list: &Function, files: &[Listed]) -> Result<(), JsError> {
    let names: Array = files.iter().map(|f| JsValue::from_str(&file_link::safe_file_name(&f.name))).collect();
    let sizes: Array = files.iter().map(|f| JsValue::from_f64(f.size as f64)).collect();
    on_list
        .call2(&JsValue::NULL, &names, &sizes)
        .map(|_| ())
        .map_err(|_| fail("The page could not start the download."))
}

/// Stream one file to `on_chunk` and check it is exactly the listed one.
async fn receive(recv: &mut RecvStream, index: u32, file: &Listed, on_chunk: &Function) -> Result<(), JsError> {
    let mut hasher = blake3::Hasher::new();
    let mut received = 0u64;
    let mut buf = vec![0u8; CHUNK];
    let index = JsValue::from_f64(index as f64);
    loop {
        let read = recv.read(&mut buf).await.map_err(|_| fail(UNREACHABLE))?;
        let Some(n) = read else { break };
        received += n as u64;
        if received > file.size {
            return Err(fail(DAMAGED));
        }
        hasher.update(&buf[..n]);
        on_chunk
            .call2(&JsValue::NULL, &index, &Uint8Array::from(&buf[..n]))
            .map_err(|_| fail("The page could not keep the file. The phone may be out of memory."))?;
    }
    if received != file.size {
        return Err(fail(UNREACHABLE));
    }
    if hasher.finalize().as_bytes() != &file.hash {
        return Err(fail(DAMAGED));
    }
    Ok(())
}

/// The list the link vouches for: its hash, count and total size must match.
async fn list(conn: &Connection, ticket: &Ticket) -> Result<Vec<Listed>, JsError> {
    let mut recv = ask(conn, &file_link::encode_request(&ticket.token, file_link::LIST, 0)).await?;
    let mut len = [0u8; 4];
    recv.read_exact(&mut len).await.map_err(|_| fail(DAMAGED))?;
    let len = u32::from_le_bytes(len) as usize;
    if len > file_link::MAX_LIST {
        return Err(fail(DAMAGED));
    }
    let mut bytes = vec![0u8; len];
    recv.read_exact(&mut bytes).await.map_err(|_| fail(DAMAGED))?;
    if blake3::hash(&bytes).as_bytes() != &ticket.hash {
        return Err(fail(DAMAGED));
    }
    let files = file_link::decode_list(&bytes).map_err(fail)?;
    let total: u64 = files.iter().map(|f| f.size).sum();
    if Some(files.len() as u32) != ticket.files || total != ticket.size {
        return Err(fail(DAMAGED));
    }
    Ok(files)
}

async fn fetch_list(endpoint: &Endpoint, ticket: &Ticket, on_list: &Function, on_chunk: &Function) -> Result<(), JsError> {
    let conn = connect(endpoint, ticket, file_link::ALPN).await?;
    let files = list(&conn, ticket).await?;
    announce(on_list, &files)?;
    for (index, file) in files.iter().enumerate() {
        let index = index as u32;
        let mut recv = ask(&conn, &file_link::encode_request(&ticket.token, index, 0)).await?;
        receive(&mut recv, index, file, on_chunk).await?;
    }
    conn.close(0u32.into(), b"done");
    Ok(())
}

/// A version 1 link (0.7.0): one file, its name before its bytes.
async fn fetch_one(endpoint: &Endpoint, ticket: &Ticket, on_list: &Function, on_chunk: &Function) -> Result<(), JsError> {
    let conn = connect(endpoint, ticket, file_link::ALPN_V1).await?;
    let mut recv = ask(&conn, &file_link::encode_request_v1(&ticket.token, 0)).await?;
    let mut len = [0u8; 2];
    recv.read_exact(&mut len).await.map_err(|_| fail(DAMAGED))?;
    let len = u16::from_le_bytes(len) as usize;
    if len > file_link::MAX_NAME {
        return Err(fail(DAMAGED));
    }
    let mut name = vec![0u8; len];
    recv.read_exact(&mut name).await.map_err(|_| fail(DAMAGED))?;
    let file = Listed { name: String::from_utf8_lossy(&name).into_owned(), size: ticket.size, hash: ticket.hash };
    announce(on_list, std::slice::from_ref(&file))?;
    receive(&mut recv, 0, &file, on_chunk).await?;
    conn.close(0u32.into(), b"done");
    Ok(())
}
