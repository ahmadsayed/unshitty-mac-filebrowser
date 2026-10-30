//! Sharing one file with a QR code: the sharer's side (the wire format is
//! `file_link`). A share lives while its dialog is open: `prepare` hashes
//! the file and makes the link, `status` reports progress and downloads,
//! `stop` ends it, and a phone that asks afterwards is told so.
//!
//! The shares run on an iroh endpoint of their own, started on the first
//! one, so folder sharing is untouched. A link is only handed out once the
//! endpoint is connected to its home relay: a phone's browser can reach the
//! computer through that relay only.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, SystemTime};

use iroh::endpoint::{Connection, RecvStream, SendStream};
use iroh::protocol::{AcceptError, ProtocolHandler, Router};
use iroh::{Endpoint, RelayUrl};
use serde::Serialize;
use tokio::sync::OnceCell;

use super::file_link::{self, Status, TOKEN_LEN, Ticket};

/// Where the page a phone opens lives (`web/file-get`, published to GitHub
/// Pages by `.github/workflows/file-page.yml`). `FILEBROWSER_FILE_PAGE`
/// overrides it, for a fork or a test.
pub const PAGE: &str = "https://ahmadsayed.github.io/unshitty-mac-filebrowser/get/";
/// How long to wait for the endpoint to reach its relay.
const ONLINE_WITHIN: Duration = Duration::from_secs(20);
/// How long a phone may take to send its request.
const REQUEST_WITHIN: Duration = Duration::from_secs(10);
const HASH_CHUNK: usize = 1 << 20;
const SEND_CHUNK: usize = 64 * 1024;

const NOT_A_FILE: &str = "Only files can be shared with a QR code.";
const CHANGED_WHILE_PREPARING: &str = "The file changed while it was being prepared. Try again.";
const OFFLINE: &str = "Could not reach the relay server. Check the internet connection and try again.";

/// What the dialog shows.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "lowercase")]
pub enum View {
    /// Hashing the file (`progress` 0–1), then going online.
    Preparing { progress: f64 },
    Ready {
        link: String,
        /// The link as an SVG QR code.
        qr: String,
        /// Downloads that reached the end of the file.
        downloads: u32,
        /// How far the furthest running download is (0–1), if any.
        sending: Option<f64>,
    },
    Failed { error: String },
}

enum Phase {
    Preparing { hashed: u64, size: u64 },
    Ready { link: String, qr: String, size: u64, modified: Option<SystemTime> },
    Failed(String),
}

struct Entry {
    /// The window whose dialog shows the share: closing it stops the share.
    window: String,
    path: PathBuf,
    name: String,
    token: [u8; TOKEN_LEN],
    cancel: Arc<AtomicBool>,
    phase: Phase,
    downloads: u32,
    /// Running downloads: bytes sent so far and the file's size.
    sending: HashMap<u64, (u64, u64)>,
}

impl Entry {
    fn view(&self) -> View {
        match &self.phase {
            Phase::Preparing { hashed, size } => View::Preparing {
                progress: if *size == 0 { 1.0 } else { *hashed as f64 / *size as f64 },
            },
            Phase::Ready { link, qr, .. } => View::Ready {
                link: link.clone(),
                qr: qr.clone(),
                downloads: self.downloads,
                sending: self
                    .sending
                    .values()
                    .map(|(sent, size)| if *size == 0 { 1.0 } else { *sent as f64 / *size as f64 })
                    .reduce(f64::max),
            },
            Phase::Failed(error) => View::Failed { error: error.clone() },
        }
    }
}

#[derive(Default)]
struct Shares {
    entries: HashMap<String, Entry>,
    next_transfer: u64,
}

type Inner = Arc<Mutex<Shares>>;

fn lock(inner: &Inner) -> MutexGuard<'_, Shares> {
    inner.lock().unwrap_or_else(|e| e.into_inner())
}

/// Where links point and which relay to use.
#[derive(Clone)]
struct Config {
    page: String,
    relay: Option<RelayUrl>,
}

pub struct FileShares {
    inner: Inner,
    node: Arc<OnceCell<Router>>,
    config: Config,
}

impl FileShares {
    /// `page` and `relay` come from the environment (`FILEBROWSER_FILE_PAGE`,
    /// `FILEBROWSER_RELAY`); unset means this repo's page and n0's relays.
    pub fn new(page: Option<String>, relay: Option<String>) -> Self {
        FileShares {
            inner: Inner::default(),
            node: Arc::new(OnceCell::new()),
            config: Config {
                page: page.filter(|p| !p.is_empty()).unwrap_or_else(|| PAGE.into()),
                relay: relay.and_then(|r| r.parse().ok()),
            },
        }
    }

    /// Start sharing `path` from `window`; the returned id names it for
    /// `status` and `stop`.
    pub fn prepare(&self, path: &Path, window: &str) -> Result<String, String> {
        let meta = std::fs::metadata(path).map_err(|e| e.to_string())?;
        if !meta.is_file() {
            return Err(NOT_A_FILE.into());
        }
        let id = data_encoding::HEXLOWER.encode(&random::<8>());
        let token = random::<TOKEN_LEN>();
        let cancel = Arc::new(AtomicBool::new(false));
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "download".into());
        lock(&self.inner).entries.insert(
            id.clone(),
            Entry {
                window: window.to_string(),
                path: path.to_path_buf(),
                name,
                token,
                cancel: cancel.clone(),
                phase: Phase::Preparing { hashed: 0, size: meta.len() },
                downloads: 0,
                sending: HashMap::new(),
            },
        );
        let (inner, node, config, path, job) =
            (self.inner.clone(), self.node.clone(), self.config.clone(), path.to_path_buf(), id.clone());
        tokio::spawn(async move {
            let phase = match make_ready(&inner, &node, &config, &job, &path, token, cancel).await {
                Ok(phase) => phase,
                Err(error) => Phase::Failed(error),
            };
            // Stopped meanwhile: nothing to update.
            if let Some(entry) = lock(&inner).entries.get_mut(&job) {
                entry.phase = phase;
            }
        });
        Ok(id)
    }

    pub fn status(&self, id: &str) -> Option<View> {
        lock(&self.inner).entries.get(id).map(Entry::view)
    }

    /// Stop sharing: preparation is cancelled and running downloads end.
    pub fn stop(&self, id: &str) {
        if let Some(entry) = lock(&self.inner).entries.remove(id) {
            entry.cancel.store(true, Ordering::SeqCst);
        }
    }

    /// A window closed: its dialogs went with it, so do its shares.
    pub fn stop_window(&self, window: &str) {
        lock(&self.inner).entries.retain(|_, entry| {
            let keep = entry.window != window;
            if !keep {
                entry.cancel.store(true, Ordering::SeqCst);
            }
            keep
        });
    }
}

fn random<const N: usize>() -> [u8; N] {
    let mut bytes = [0u8; N];
    getrandom::fill(&mut bytes).expect("system randomness");
    bytes
}

/// Hash the file, bring the endpoint online and make the link.
async fn make_ready(
    inner: &Inner,
    node: &OnceCell<Router>,
    config: &Config,
    id: &str,
    path: &Path,
    token: [u8; TOKEN_LEN],
    cancel: Arc<AtomicBool>,
) -> Result<Phase, String> {
    let progress = {
        let (inner, id) = (inner.clone(), id.to_string());
        move |hashed: u64| {
            if let Some(Entry { phase: Phase::Preparing { hashed: done, .. }, .. }) =
                lock(&inner).entries.get_mut(&id)
            {
                *done = hashed;
            }
        }
    };
    let path_owned = path.to_path_buf();
    let (hash, size, modified) =
        tokio::task::spawn_blocking(move || hash_file(&path_owned, &cancel, &progress))
            .await
            .map_err(|e| e.to_string())??;
    let router = node
        .get_or_try_init(|| start_node(inner.clone(), config.relay.clone()))
        .await?;
    let relay = home_relay(router.endpoint()).await?;
    let ticket = Ticket { endpoint: router.endpoint().id(), relay, token, hash, size };
    let link = format!("{}#{}", config.page, ticket.encode());
    let qr = qr_svg(&link)?;
    Ok(Phase::Ready { link, qr, size, modified })
}

/// BLAKE3 of the whole file, its size and modification time. The file must
/// not change while it is read.
fn hash_file(
    path: &Path,
    cancel: &AtomicBool,
    progress: &dyn Fn(u64),
) -> Result<([u8; 32], u64, Option<SystemTime>), String> {
    let before = std::fs::metadata(path).map_err(|e| e.to_string())?;
    let mut file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut hasher = blake3::Hasher::new();
    let mut buf = vec![0u8; HASH_CHUNK];
    let mut hashed = 0u64;
    loop {
        if cancel.load(Ordering::SeqCst) {
            return Err("Stopped.".into());
        }
        let n = file.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        hashed += n as u64;
        progress(hashed);
    }
    let after = std::fs::metadata(path).map_err(|e| e.to_string())?;
    if after.len() != hashed || before.len() != hashed || after.modified().ok() != before.modified().ok() {
        return Err(CHANGED_WHILE_PREPARING.into());
    }
    Ok((*hasher.finalize().as_bytes(), hashed, after.modified().ok()))
}

pub fn qr_svg(link: &str) -> Result<String, String> {
    use qrcode::render::svg;
    let code = qrcode::QrCode::with_error_correction_level(link.as_bytes(), qrcode::EcLevel::M)
        .map_err(|e| e.to_string())?;
    // Black on white whatever the theme: scanners need the contrast.
    Ok(code
        .render::<svg::Color>()
        .min_dimensions(240, 240)
        .dark_color(svg::Color("#000000"))
        .light_color(svg::Color("#ffffff"))
        .quiet_zone(true)
        .build())
}

#[cfg(not(test))]
async fn start_node(inner: Inner, relay: Option<RelayUrl>) -> Result<Router, String> {
    let mut builder = Endpoint::builder(iroh::endpoint::presets::N0);
    if let Some(relay) = relay {
        builder = builder.relay_mode(iroh::RelayMode::Custom(relay.into()));
    }
    let endpoint = builder.bind().await.map_err(|e| e.to_string())?;
    Ok(Router::builder(endpoint).accept(file_link::ALPN, FileProtocol(inner)).spawn())
}

#[cfg(not(test))]
async fn home_relay(endpoint: &Endpoint) -> Result<RelayUrl, String> {
    tokio::time::timeout(ONLINE_WITHIN, endpoint.online())
        .await
        .map_err(|_| OFFLINE.to_string())?;
    endpoint.addr().relay_urls().next().cloned().ok_or_else(|| OFFLINE.into())
}

/// Tests stay on loopback, with no relay: phones are played by a client
/// that dials the endpoint's own addresses.
#[cfg(test)]
async fn start_node(inner: Inner, _relay: Option<RelayUrl>) -> Result<Router, String> {
    let endpoint = super::test_endpoint().await;
    Ok(Router::builder(endpoint).accept(file_link::ALPN, FileProtocol(inner)).spawn())
}

#[cfg(test)]
async fn home_relay(_endpoint: &Endpoint) -> Result<RelayUrl, String> {
    let _ = ONLINE_WITHIN;
    let _ = OFFLINE;
    Ok("http://relay.invalid./".parse().unwrap())
}

// ---- serving ------------------------------------------------------------

#[derive(Clone)]
struct FileProtocol(Inner);

impl std::fmt::Debug for FileProtocol {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("FileProtocol")
    }
}

impl ProtocolHandler for FileProtocol {
    async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
        while let Ok((send, recv)) = conn.accept_bi().await {
            tokio::spawn(serve(self.0.clone(), send, recv));
        }
        Ok(())
    }
}

/// The file a token stands for, if it is still shared and ready.
struct Found {
    id: String,
    path: PathBuf,
    name: String,
    size: u64,
    modified: Option<SystemTime>,
}

/// Compare tokens without stopping at the first difference, so timing
/// says nothing about how much of a guess was right.
fn same_token(a: &[u8; TOKEN_LEN], b: &[u8; TOKEN_LEN]) -> bool {
    a.iter().zip(b).fold(0u8, |diff, (x, y)| diff | (x ^ y)) == 0
}

fn find(inner: &Inner, token: &[u8; TOKEN_LEN]) -> Option<Found> {
    let shares = lock(inner);
    shares.entries.iter().find_map(|(id, entry)| match &entry.phase {
        Phase::Ready { size, modified, .. } if same_token(&entry.token, token) => Some(Found {
            id: id.clone(),
            path: entry.path.clone(),
            name: entry.name.clone(),
            size: *size,
            modified: *modified,
        }),
        _ => None,
    })
}

async fn serve(inner: Inner, mut send: SendStream, mut recv: RecvStream) {
    let mut request = [0u8; file_link::REQUEST_LEN];
    match tokio::time::timeout(REQUEST_WITHIN, recv.read_exact(&mut request)).await {
        Ok(Ok(())) => {}
        _ => return,
    }
    let (token, offset) = file_link::decode_request(&request);
    let status = match find(&inner, &token) {
        None => Err(Status::NotShared),
        Some(found) => match open_unchanged(&found) {
            Err(status) => Err(status),
            Ok(_) if offset > found.size => Err(Status::BadOffset),
            Ok(file) => Ok((found, file)),
        },
    };
    let (found, file) = match status {
        Err(status) => {
            if send.write_all(&[status as u8]).await.is_ok() {
                send.finish().ok();
            }
            return;
        }
        Ok(ready) => ready,
    };
    let mut header = vec![Status::Ok as u8];
    header.extend(file_link::encode_name(&found.name));
    if send.write_all(&header).await.is_err() {
        return;
    }
    let transfer = {
        let mut shares = lock(&inner);
        shares.next_transfer += 1;
        let transfer = shares.next_transfer;
        if let Some(entry) = shares.entries.get_mut(&found.id) {
            entry.sending.insert(transfer, (offset, found.size));
        }
        transfer
    };
    let complete = send_from(&inner, &found, transfer, file, offset, &mut send).await;
    let mut shares = lock(&inner);
    if let Some(entry) = shares.entries.get_mut(&found.id) {
        entry.sending.remove(&transfer);
        if complete {
            entry.downloads += 1;
        }
    }
    drop(shares);
    if complete {
        send.finish().ok();
    } else {
        send.reset(0u32.into()).ok();
    }
}

/// The shared file, if it is still the one that was hashed.
fn open_unchanged(found: &Found) -> Result<std::fs::File, Status> {
    let file = std::fs::File::open(&found.path).map_err(|_| Status::Changed)?;
    let meta = file.metadata().map_err(|_| Status::Changed)?;
    if meta.len() != found.size || meta.modified().ok() != found.modified {
        return Err(Status::Changed);
    }
    Ok(file)
}

/// Stream the file from `offset`; false when the share was stopped or the
/// phone went away first. Reading happens on a blocking thread, a few
/// chunks ahead.
async fn send_from(
    inner: &Inner,
    found: &Found,
    transfer: u64,
    mut file: std::fs::File,
    offset: u64,
    send: &mut SendStream,
) -> bool {
    use std::io::{Seek, SeekFrom};
    let (tx, mut rx) = tokio::sync::mpsc::channel::<std::io::Result<Vec<u8>>>(4);
    let size = found.size;
    tokio::task::spawn_blocking(move || {
        if let Err(e) = file.seek(SeekFrom::Start(offset)) {
            let _ = tx.blocking_send(Err(e));
            return;
        }
        let mut left = size - offset;
        while left > 0 {
            let mut buf = vec![0u8; SEND_CHUNK.min(left as usize)];
            let read = file.read(&mut buf);
            let done = matches!(read, Ok(0) | Err(_));
            let chunk = read.map(|n| {
                buf.truncate(n);
                buf
            });
            if let Ok(chunk) = &chunk {
                left -= chunk.len() as u64;
            }
            // The receiver is gone: the download ended early.
            if tx.blocking_send(chunk).is_err() || done {
                return;
            }
        }
    });
    let mut sent = offset;
    while sent < size {
        let Some(Ok(chunk)) = rx.recv().await else { return false };
        if chunk.is_empty() || send.write_all(&chunk).await.is_err() {
            return false;
        }
        sent += chunk.len() as u64;
        let mut shares = lock(inner);
        let Some(entry) = shares.entries.get_mut(&found.id) else {
            return false;
        };
        if let Some(progress) = entry.sending.get_mut(&transfer) {
            progress.0 = sent;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use iroh::address_lookup::memory::MemoryLookup;

    struct Fixture {
        _dir: tempfile::TempDir,
        file: PathBuf,
        data: Vec<u8>,
    }

    fn fixture(len: usize) -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("report.pdf");
        let data: Vec<u8> = (0..len as u32).map(|i| (i.wrapping_mul(2654435761) >> 11) as u8).collect();
        std::fs::write(&file, &data).unwrap();
        Fixture { _dir: dir, file, data }
    }

    async fn ready(shares: &FileShares, id: &str) -> View {
        for _ in 0..500 {
            match shares.status(id) {
                Some(View::Preparing { .. }) => tokio::time::sleep(Duration::from_millis(10)).await,
                Some(view) => return view,
                None => panic!("share {id} is gone"),
            }
        }
        panic!("share {id} never got ready");
    }

    fn ticket_of(view: &View) -> Ticket {
        let View::Ready { link, .. } = view else { panic!("not ready: {view:?}") };
        let (page, ticket) = link.split_once('#').unwrap();
        assert_eq!(page, PAGE);
        Ticket::decode(ticket).unwrap()
    }

    /// A phone: dial the sharer's own addresses, ask, read everything.
    async fn download(shares: &FileShares, token: &[u8; TOKEN_LEN], offset: u64) -> (Status, String, Vec<u8>) {
        let router = shares.node.get().expect("node started");
        let addr = router.endpoint().addr();
        let phone = crate::share::test_endpoint().await;
        let lookup = MemoryLookup::new();
        lookup.add_endpoint_info(addr.clone());
        phone.address_lookup().unwrap().add(lookup);
        let conn = phone.connect(addr.id, file_link::ALPN).await.unwrap();
        let (mut send, mut recv) = conn.open_bi().await.unwrap();
        send.write_all(&file_link::encode_request(token, offset)).await.unwrap();
        send.finish().unwrap();
        let mut status = [0u8; 1];
        recv.read_exact(&mut status).await.unwrap();
        let status = Status::from_byte(status[0]).unwrap();
        if status != Status::Ok {
            return (status, String::new(), Vec::new());
        }
        let mut len = [0u8; 2];
        recv.read_exact(&mut len).await.unwrap();
        let mut name = vec![0u8; u16::from_le_bytes(len) as usize];
        recv.read_exact(&mut name).await.unwrap();
        let body = recv.read_to_end(usize::MAX).await.unwrap();
        conn.close(0u32.into(), b"done");
        (status, String::from_utf8(name).unwrap(), body)
    }

    #[tokio::test]
    async fn a_phone_downloads_the_file_and_it_matches_the_ticket() {
        tokio::time::timeout(Duration::from_secs(30), async {
            let fx = fixture(700_000);
            let shares = FileShares::new(None, None);
            let id = shares.prepare(&fx.file, "main").unwrap();
            let view = ready(&shares, &id).await;
            let View::Ready { qr, downloads: 0, sending: None, .. } = &view else { panic!("{view:?}") };
            assert!(qr.starts_with("<?xml") && qr.contains("<svg"), "{}", &qr[..60]);
            let ticket = ticket_of(&view);
            assert_eq!(ticket.size, fx.data.len() as u64);
            assert_eq!(ticket.hash, *blake3::hash(&fx.data).as_bytes());

            let (status, name, body) = download(&shares, &ticket.token, 0).await;
            assert_eq!((status, name.as_str()), (Status::Ok, "report.pdf"));
            assert_eq!(body, fx.data);
            let (_, _, rest) = download(&shares, &ticket.token, 1000).await;
            assert_eq!(rest, fx.data[1000..]);
            tokio::time::sleep(Duration::from_millis(50)).await;
            assert!(matches!(shares.status(&id), Some(View::Ready { downloads: 2, sending: None, .. })));
        })
        .await
        .expect("timed out");
    }

    #[tokio::test]
    async fn stopped_changed_unknown_and_overlong_requests_are_refused() {
        tokio::time::timeout(Duration::from_secs(30), async {
            let fx = fixture(5_000);
            let shares = FileShares::new(Some("http://localhost:9/get/".into()), None);
            let id = shares.prepare(&fx.file, "main").unwrap();
            let view = ready(&shares, &id).await;
            let View::Ready { link, .. } = &view else { panic!() };
            assert!(link.starts_with("http://localhost:9/get/#"));
            let ticket = Ticket::decode(link.split_once('#').unwrap().1).unwrap();

            assert_eq!(download(&shares, &[0u8; TOKEN_LEN], 0).await.0, Status::NotShared);
            assert_eq!(download(&shares, &ticket.token, 5_001).await.0, Status::BadOffset);
            assert_eq!(download(&shares, &ticket.token, 5_000).await.2, Vec::<u8>::new());

            // Same size, new content and time: refused, never served stale.
            std::thread::sleep(Duration::from_millis(20));
            std::fs::write(&fx.file, vec![1u8; 5_000]).unwrap();
            assert_eq!(download(&shares, &ticket.token, 0).await.0, Status::Changed);

            shares.stop(&id);
            assert_eq!(shares.status(&id), None);
            assert_eq!(download(&shares, &ticket.token, 0).await.0, Status::NotShared);
        })
        .await
        .expect("timed out");
    }

    #[tokio::test]
    async fn folders_and_missing_files_cannot_be_shared() {
        let dir = tempfile::tempdir().unwrap();
        let shares = FileShares::new(None, None);
        assert_eq!(shares.prepare(dir.path(), "main").unwrap_err(), NOT_A_FILE);
        assert!(shares.prepare(&dir.path().join("gone"), "main").is_err());
        assert_eq!(shares.status("nope"), None);
        shares.stop("nope");
    }

    #[tokio::test]
    async fn closing_a_window_stops_only_its_shares() {
        let fx = fixture(100);
        let shares = FileShares::new(None, None);
        let mine = shares.prepare(&fx.file, "main").unwrap();
        let other = shares.prepare(&fx.file, "window-2").unwrap();
        shares.stop_window("main");
        assert_eq!(shares.status(&mine), None);
        assert!(shares.status(&other).is_some());
    }

    #[test]
    fn tokens_match_only_when_every_byte_does() {
        let token = [7u8; TOKEN_LEN];
        assert!(same_token(&token, &[7u8; TOKEN_LEN]));
        let mut last = token;
        last[TOKEN_LEN - 1] = 8;
        assert!(!same_token(&token, &last));
        assert!(!same_token(&token, &[0u8; TOKEN_LEN]));
    }

    #[test]
    fn hashing_reports_progress_and_stops_when_cancelled() {
        let fx = fixture(3 * HASH_CHUNK + 10);
        let seen = Mutex::new(Vec::new());
        let (hash, size, _) = hash_file(&fx.file, &AtomicBool::new(false), &|n| seen.lock().unwrap().push(n)).unwrap();
        assert_eq!(hash, *blake3::hash(&fx.data).as_bytes());
        assert_eq!(size, fx.data.len() as u64);
        assert_eq!(seen.into_inner().unwrap().last(), Some(&size));
        assert_eq!(hash_file(&fx.file, &AtomicBool::new(true), &|_| {}).unwrap_err(), "Stopped.");
    }

    #[test]
    fn views_report_progress() {
        let entry = |phase| Entry {
            window: String::new(),
            path: PathBuf::new(),
            name: String::new(),
            token: [0; TOKEN_LEN],
            cancel: Arc::default(),
            phase,
            downloads: 1,
            sending: HashMap::from([(1, (25, 100)), (2, (50, 100))]),
        };
        assert_eq!(entry(Phase::Preparing { hashed: 5, size: 10 }).view(), View::Preparing { progress: 0.5 });
        assert_eq!(entry(Phase::Preparing { hashed: 0, size: 0 }).view(), View::Preparing { progress: 1.0 });
        let ready = entry(Phase::Ready { link: "l".into(), qr: "q".into(), size: 100, modified: None }).view();
        assert_eq!(ready, View::Ready { link: "l".into(), qr: "q".into(), downloads: 1, sending: Some(0.5) });
        assert_eq!(entry(Phase::Failed("no".into())).view(), View::Failed { error: "no".into() });
        assert!(qr_svg("https://example.com/get/#abc").unwrap().contains("<svg"));
    }
}
