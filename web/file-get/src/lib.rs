//! The page a phone opens from a QR code (see `index.html`): connect to the
//! sharer through its relay, download the one shared file and check it
//! against the hash in the link. The wire format is the app's own module,
//! compiled in here so both sides always agree.

use iroh::endpoint::presets;
use iroh::{Endpoint, EndpointAddr, RelayMode};
use js_sys::{Function, Uint8Array};
use wasm_bindgen::prelude::*;

#[path = "../../../src-tauri/src/share/file_link.rs"]
mod file_link;

use file_link::{Status, Ticket};

const CHUNK: usize = 64 * 1024;
const UNREACHABLE: &str =
    "Could not reach the computer sharing this file. It has to be on, with its QR code still showing.";
const DAMAGED: &str = "The file arrived damaged. Try again.";

fn fail(message: &str) -> JsError {
    JsError::new(message)
}

/// The size a link announces, before connecting.
#[wasm_bindgen]
pub fn ticket_size(ticket: &str) -> Result<f64, JsError> {
    Ok(Ticket::decode(ticket).map_err(fail)?.size as f64)
}

/// Download the file a ticket names. `on_start(name, size)` runs once the
/// sharer answers, `on_chunk(bytes)` for every piece in order; the result is
/// the file name to save under, once every byte has arrived and the whole
/// file matches the link's hash.
#[wasm_bindgen]
pub async fn download(ticket: String, on_start: Function, on_chunk: Function) -> Result<String, JsError> {
    let ticket = Ticket::decode(&ticket).map_err(fail)?;
    // Browsers only reach peers through a relay: use the sharer's own.
    let endpoint = Endpoint::builder(presets::Minimal)
        .relay_mode(RelayMode::Custom(ticket.relay.clone().into()))
        .bind()
        .await
        .map_err(|e| JsError::new(&e.to_string()))?;
    let result = fetch(&endpoint, &ticket, &on_start, &on_chunk).await;
    endpoint.close().await;
    result
}

async fn fetch(endpoint: &Endpoint, ticket: &Ticket, on_start: &Function, on_chunk: &Function) -> Result<String, JsError> {
    let addr = EndpointAddr::new(ticket.endpoint).with_relay_url(ticket.relay.clone());
    let conn = endpoint.connect(addr, file_link::ALPN).await.map_err(|_| fail(UNREACHABLE))?;
    let (mut send, mut recv) = conn.open_bi().await.map_err(|_| fail(UNREACHABLE))?;
    send.write_all(&file_link::encode_request(&ticket.token, 0))
        .await
        .map_err(|_| fail(UNREACHABLE))?;
    send.finish().ok();

    let mut status = [0u8; 1];
    recv.read_exact(&mut status).await.map_err(|_| fail(UNREACHABLE))?;
    match Status::from_byte(status[0]) {
        Some(Status::Ok) => {}
        Some(refused) => return Err(fail(refused.message())),
        None => return Err(fail("The computer sharing this file needs a newer version of this page.")),
    }
    let mut len = [0u8; 2];
    recv.read_exact(&mut len).await.map_err(|_| fail(DAMAGED))?;
    let len = u16::from_le_bytes(len) as usize;
    if len > file_link::MAX_NAME {
        return Err(fail(DAMAGED));
    }
    let mut name = vec![0u8; len];
    recv.read_exact(&mut name).await.map_err(|_| fail(DAMAGED))?;
    let name = file_link::safe_file_name(&String::from_utf8_lossy(&name));
    on_start
        .call2(&JsValue::NULL, &JsValue::from_str(&name), &JsValue::from_f64(ticket.size as f64))
        .map_err(|_| fail("The page could not start the download."))?;

    let mut hasher = blake3::Hasher::new();
    let mut received = 0u64;
    let mut buf = vec![0u8; CHUNK];
    loop {
        let read = recv.read(&mut buf).await.map_err(|_| fail(UNREACHABLE))?;
        let Some(n) = read else { break };
        received += n as u64;
        if received > ticket.size {
            return Err(fail(DAMAGED));
        }
        hasher.update(&buf[..n]);
        on_chunk
            .call1(&JsValue::NULL, &Uint8Array::from(&buf[..n]))
            .map_err(|_| fail("The page could not keep the file. The phone may be out of memory."))?;
    }
    conn.close(0u32.into(), b"done");
    if received != ticket.size {
        return Err(fail(UNREACHABLE));
    }
    if hasher.finalize().as_bytes() != &ticket.hash {
        return Err(fail(DAMAGED));
    }
    Ok(name)
}
