//! Sharing files with a QR code: the sharer's side (the wire format is
//! `file_link`). A share lives while its dialog is open: `prepare` hashes
//! the files and makes the link, `status` reports progress and downloads,
//! `stop` ends it, and a phone that asks afterwards is told so.
//!
//! The shares run on an iroh endpoint of their own, started on the first
//! one, so folder sharing is untouched. A link is only handed out once the
//! endpoint is connected to its home relay: a phone's browser can reach the
//! computer through that relay only.

use std::collections::{HashMap, HashSet};
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

use super::file_link::{self, Listed, Status, TOKEN_LEN, Ticket};

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
const NOTHING: &str = "Choose files to share.";
const TOO_MANY: &str = "Up to 1,000 files can be shared at once.";
const CHANGED_WHILE_PREPARING: &str = "The file changed while it was being prepared. Try again.";
const OFFLINE: &str = "Could not reach the relay server. Check the internet connection and try again.";

/// What the dialog shows.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "lowercase")]
pub enum View {
    /// Hashing the files (`progress` 0–1), then going online.
    Preparing { progress: f64 },
    Ready {
        link: String,
        /// The link as an SVG QR code.
        qr: String,
        /// Phones that downloaded every file.
        downloads: u32,
        /// How far the furthest running download is (0–1), if any.
        sending: Option<f64>,
    },
    Failed { error: String },
}

/// A shared file as it was hashed.
#[derive(Debug, Clone)]
struct SharedFile {
    path: PathBuf,
    size: u64,
    modified: Option<SystemTime>,
}

enum Phase {
    Preparing { hashed: u64, size: u64 },
    /// `list` is the encoded list the link's hash covers.
    Ready { link: String, qr: String, list: Vec<u8>, files: Vec<SharedFile> },
    Failed(String),
}

/// One phone's download: the files it finished, their bytes, and how far
/// the running one is.
#[derive(Debug, Default)]
struct Progress {
    done: HashSet<u32>,
    finished: u64,
    current: u64,
    total: u64,
}

impl Progress {
    fn fraction(&self) -> f64 {
        if self.total == 0 { 1.0 } else { ((self.finished + self.current) as f64 / self.total as f64).min(1.0) }
    }
}

struct Entry {
    /// The window whose dialog shows the share: closing it stops the share.
    window: String,
    token: [u8; TOKEN_LEN],
    cancel: Arc<AtomicBool>,
    phase: Phase,
    downloads: u32,
    /// Running downloads, by connection.
    sending: HashMap<u64, Progress>,
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
                sending: self.sending.values().map(Progress::fraction).reduce(f64::max),
            },
            Phase::Failed(error) => View::Failed { error: error.clone() },
        }
    }
}

#[derive(Default)]
struct Shares {
    entries: HashMap<String, Entry>,
    next_connection: u64,
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

    /// Start sharing `paths` from `window`; the returned id names the share
    /// for `status` and `stop`.
    pub fn prepare(&self, paths: &[PathBuf], window: &str) -> Result<String, String> {
        if paths.is_empty() {
            return Err(NOTHING.into());
        }
        if paths.len() > file_link::MAX_FILES {
            return Err(TOO_MANY.into());
        }
        let mut size = 0u64;
        for path in paths {
            let meta = std::fs::metadata(path).map_err(|e| e.to_string())?;
            if !meta.is_file() {
                return Err(NOT_A_FILE.into());
            }
            size += meta.len();
        }
        let id = data_encoding::HEXLOWER.encode(&random::<8>());
        let token = random::<TOKEN_LEN>();
        let cancel = Arc::new(AtomicBool::new(false));
        lock(&self.inner).entries.insert(
            id.clone(),
            Entry {
                window: window.to_string(),
                token,
                cancel: cancel.clone(),
                phase: Phase::Preparing { hashed: 0, size },
                downloads: 0,
                sending: HashMap::new(),
            },
        );
        let (inner, node, config, paths, job) =
            (self.inner.clone(), self.node.clone(), self.config.clone(), paths.to_vec(), id.clone());
        tokio::spawn(async move {
            let phase = match make_ready(&inner, &node, &config, &job, &paths, token, cancel).await {
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

fn file_name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "download".into())
}

/// Hash every file and list them, the list's hash going in the link.
fn hash_files(
    paths: &[PathBuf],
    cancel: &AtomicBool,
    progress: &dyn Fn(u64),
) -> Result<(Vec<u8>, Vec<SharedFile>, u64), String> {
    let mut listed = Vec::with_capacity(paths.len());
    let mut files = Vec::with_capacity(paths.len());
    let mut before = 0u64;
    for path in paths {
        let (hash, size, modified) = hash_file(path, cancel, &|hashed| progress(before + hashed))?;
        before += size;
        listed.push(Listed { name: file_name(path), size, hash });
        files.push(SharedFile { path: path.clone(), size, modified });
    }
    Ok((file_link::encode_list(&listed), files, before))
}

/// Hash the files, bring the endpoint online and make the link.
async fn make_ready(
    inner: &Inner,
    node: &OnceCell<Router>,
    config: &Config,
    id: &str,
    paths: &[PathBuf],
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
    let paths_owned = paths.to_vec();
    let (list, files, size) =
        tokio::task::spawn_blocking(move || hash_files(&paths_owned, &cancel, &progress))
            .await
            .map_err(|e| e.to_string())??;
    let router = node
        .get_or_try_init(|| start_node(inner.clone(), config.relay.clone()))
        .await?;
    let relay = home_relay(router.endpoint()).await?;
    let ticket = Ticket {
        endpoint: router.endpoint().id(),
        relay,
        token,
        hash: *blake3::hash(&list).as_bytes(),
        size,
        files: Some(files.len() as u32),
    };
    let link = format!("{}#{}", config.page, ticket.encode());
    let qr = qr_svg(&link)?;
    Ok(Phase::Ready { link, qr, list, files })
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
        // One phone's download is one connection: its streams share a progress.
        let connection = {
            let mut shares = lock(&self.0);
            shares.next_connection += 1;
            shares.next_connection
        };
        while let Ok((send, recv)) = conn.accept_bi().await {
            tokio::spawn(serve(self.0.clone(), connection, send, recv));
        }
        for entry in lock(&self.0).entries.values_mut() {
            entry.sending.remove(&connection);
        }
        Ok(())
    }
}

/// What a request asks for, if its share is still running and ready.
enum Found {
    List(Vec<u8>),
    File { id: String, index: u32, count: usize, total: u64, file: SharedFile },
}

/// Compare tokens without stopping at the first difference, so timing
/// says nothing about how much of a guess was right.
fn same_token(a: &[u8; TOKEN_LEN], b: &[u8; TOKEN_LEN]) -> bool {
    a.iter().zip(b).fold(0u8, |diff, (x, y)| diff | (x ^ y)) == 0
}

fn find(inner: &Inner, token: &[u8; TOKEN_LEN], index: u32) -> Result<Found, Status> {
    let shares = lock(inner);
    let (id, list, files) = shares
        .entries
        .iter()
        .find_map(|(id, entry)| match &entry.phase {
            Phase::Ready { list, files, .. } if same_token(&entry.token, token) => Some((id, list, files)),
            _ => None,
        })
        .ok_or(Status::NotShared)?;
    if index == file_link::LIST {
        return Ok(Found::List(list.clone()));
    }
    let file = files.get(index as usize).ok_or(Status::NoSuchFile)?.clone();
    let total = files.iter().map(|f| f.size).sum();
    Ok(Found::File { id: id.clone(), index, count: files.len(), total, file })
}

async fn refuse(mut send: SendStream, status: Status) {
    if send.write_all(&[status as u8]).await.is_ok() {
        send.finish().ok();
    }
}

async fn serve(inner: Inner, connection: u64, mut send: SendStream, mut recv: RecvStream) {
    let mut request = [0u8; file_link::REQUEST_LEN];
    match tokio::time::timeout(REQUEST_WITHIN, recv.read_exact(&mut request)).await {
        Ok(Ok(())) => {}
        _ => return,
    }
    let (token, index, offset) = file_link::decode_request(&request);
    match find(&inner, &token, index) {
        Err(status) => refuse(send, status).await,
        Ok(Found::List(list)) => {
            let mut answer = vec![Status::Ok as u8];
            answer.extend_from_slice(&(list.len() as u32).to_le_bytes());
            answer.extend(list);
            if send.write_all(&answer).await.is_ok() {
                send.finish().ok();
            }
        }
        Ok(Found::File { id, index, count, total, file }) => {
            let opened = match open_unchanged(&file) {
                Ok(_) if offset > file.size => Err(Status::BadOffset),
                opened => opened,
            };
            match opened {
                Err(status) => refuse(send, status).await,
                Ok(handle) => {
                    let sending = Sending { inner: &inner, id: &id, connection, index, count, total };
                    send_file(sending, &file, handle, offset, send).await;
                }
            }
        }
    }
}

/// Who a file is being sent to, for the share's progress and downloads.
struct Sending<'a> {
    inner: &'a Inner,
    id: &'a str,
    connection: u64,
    index: u32,
    count: usize,
    total: u64,
}

impl Sending<'_> {
    /// Record how far this file is; false once the share was stopped.
    fn at(&self, sent: u64) -> bool {
        let mut shares = lock(self.inner);
        let Some(entry) = shares.entries.get_mut(self.id) else { return false };
        let progress = entry.sending.entry(self.connection).or_insert_with(|| Progress { total: self.total, ..Progress::default() });
        progress.current = sent;
        true
    }

    /// The file arrived whole: once every file has, it counts as a download.
    fn finished(&self, size: u64) {
        let mut shares = lock(self.inner);
        let Some(entry) = shares.entries.get_mut(self.id) else { return };
        let progress = entry.sending.entry(self.connection).or_insert_with(|| Progress { total: self.total, ..Progress::default() });
        if progress.done.insert(self.index) {
            progress.finished += size;
        }
        progress.current = 0;
        if progress.done.len() == self.count {
            entry.sending.remove(&self.connection);
            entry.downloads += 1;
        }
    }
}

async fn send_file(sending: Sending<'_>, file: &SharedFile, handle: std::fs::File, offset: u64, mut send: SendStream) {
    if send.write_all(&[Status::Ok as u8]).await.is_err() {
        return;
    }
    let complete = send_from(&sending, file.size, handle, offset, &mut send).await;
    if complete {
        sending.finished(file.size);
        send.finish().ok();
    } else {
        sending.at(0);
        send.reset(0u32.into()).ok();
    }
}

/// The shared file, if it is still the one that was hashed.
fn open_unchanged(file: &SharedFile) -> Result<std::fs::File, Status> {
    let handle = std::fs::File::open(&file.path).map_err(|_| Status::Changed)?;
    let meta = handle.metadata().map_err(|_| Status::Changed)?;
    if meta.len() != file.size || meta.modified().ok() != file.modified {
        return Err(Status::Changed);
    }
    Ok(handle)
}

/// Stream the file from `offset`; false when the share was stopped or the
/// phone went away first. Reading happens on a blocking thread, a few
/// chunks ahead.
async fn send_from(sending: &Sending<'_>, size: u64, mut file: std::fs::File, offset: u64, send: &mut SendStream) -> bool {
    use std::io::{Seek, SeekFrom};
    let (tx, mut rx) = tokio::sync::mpsc::channel::<std::io::Result<Vec<u8>>>(4);
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
    if !sending.at(sent) {
        return false;
    }
    while sent < size {
        let Some(Ok(chunk)) = rx.recv().await else { return false };
        if chunk.is_empty() || send.write_all(&chunk).await.is_err() {
            return false;
        }
        sent += chunk.len() as u64;
        if !sending.at(sent) {
            return false;
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
        files: Vec<PathBuf>,
        data: Vec<Vec<u8>>,
    }

    fn bytes(len: usize, seed: u32) -> Vec<u8> {
        (0..len as u32).map(|i| (i.wrapping_add(seed).wrapping_mul(2654435761) >> 11) as u8).collect()
    }

    fn fixture(files: &[(&str, usize)]) -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let mut paths = Vec::new();
        let mut data = Vec::new();
        for (i, (name, len)) in files.iter().enumerate() {
            let path = dir.path().join(name);
            let content = bytes(*len, i as u32);
            std::fs::write(&path, &content).unwrap();
            paths.push(path);
            data.push(content);
        }
        Fixture { _dir: dir, files: paths, data }
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

    /// A phone: its endpoint (kept alive) and a connection to the sharer.
    struct Phone {
        _endpoint: Endpoint,
        conn: Connection,
    }

    impl Phone {
        fn close(&self) {
            self.conn.close(0u32.into(), b"done");
        }
    }

    /// Dial the sharer's own addresses.
    async fn phone(shares: &FileShares) -> Phone {
        let router = shares.node.get().expect("node started");
        let addr = router.endpoint().addr();
        let endpoint = crate::share::test_endpoint().await;
        let lookup = MemoryLookup::new();
        lookup.add_endpoint_info(addr.clone());
        endpoint.address_lookup().unwrap().add(lookup);
        let conn = endpoint.connect(addr.id, file_link::ALPN).await.unwrap();
        Phone { _endpoint: endpoint, conn }
    }

    /// Ask for the list or a file; the list comes without its length.
    async fn ask(phone: &Phone, token: &[u8; TOKEN_LEN], index: u32, offset: u64) -> (Status, Vec<u8>) {
        let (mut send, mut recv) = phone.conn.open_bi().await.unwrap();
        send.write_all(&file_link::encode_request(token, index, offset)).await.unwrap();
        send.finish().unwrap();
        let mut status = [0u8; 1];
        recv.read_exact(&mut status).await.unwrap();
        let status = Status::from_byte(status[0]).unwrap();
        if status != Status::Ok {
            return (status, Vec::new());
        }
        if index == file_link::LIST {
            let mut len = [0u8; 4];
            recv.read_exact(&mut len).await.unwrap();
            let mut list = vec![0u8; u32::from_le_bytes(len) as usize];
            recv.read_exact(&mut list).await.unwrap();
            return (status, list);
        }
        (status, recv.read_to_end(usize::MAX).await.unwrap())
    }

    async fn settle() {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    #[tokio::test]
    async fn a_phone_downloads_every_file_and_each_matches_the_list() {
        tokio::time::timeout(Duration::from_secs(30), async {
            let fx = fixture(&[("report.pdf", 700_000), ("photo.jpg", 3_000)]);
            let shares = FileShares::new(None, None);
            let id = shares.prepare(&fx.files, "main").unwrap();
            let view = ready(&shares, &id).await;
            let View::Ready { qr, downloads: 0, sending: None, .. } = &view else { panic!("{view:?}") };
            assert!(qr.starts_with("<?xml") && qr.contains("<svg"), "{}", &qr[..60]);
            let ticket = ticket_of(&view);
            assert_eq!(ticket.files, Some(2));
            assert_eq!(ticket.size, 703_000);

            let conn = phone(&shares).await;
            let (status, list) = ask(&conn, &ticket.token, file_link::LIST, 0).await;
            assert_eq!(status, Status::Ok);
            assert_eq!(*blake3::hash(&list).as_bytes(), ticket.hash, "the list matches the link");
            let listed = file_link::decode_list(&list).unwrap();
            let names: Vec<_> = listed.iter().map(|f| f.name.as_str()).collect();
            assert_eq!(names, ["report.pdf", "photo.jpg"]);
            for (i, file) in listed.iter().enumerate() {
                assert_eq!(file.size, fx.data[i].len() as u64);
                assert_eq!(file.hash, *blake3::hash(&fx.data[i]).as_bytes());
            }

            assert_eq!(ask(&conn, &ticket.token, 0, 0).await, (Status::Ok, fx.data[0].clone()));
            settle().await;
            assert!(matches!(shares.status(&id), Some(View::Ready { downloads: 0, .. })), "one of two files");
            assert_eq!(ask(&conn, &ticket.token, 1, 0).await, (Status::Ok, fx.data[1].clone()));
            settle().await;
            assert!(matches!(shares.status(&id), Some(View::Ready { downloads: 1, .. })), "every file: one download");

            // Resuming from an offset, on another connection.
            let again = phone(&shares).await;
            assert_eq!(ask(&again, &ticket.token, 0, 1000).await.1, fx.data[0][1000..]);
            conn.close();
            again.close();
            settle().await;
            assert!(matches!(shares.status(&id), Some(View::Ready { downloads: 1, sending: None, .. })));
        })
        .await
        .expect("timed out");
    }

    #[tokio::test]
    async fn one_file_is_a_list_of_one() {
        tokio::time::timeout(Duration::from_secs(30), async {
            let fx = fixture(&[("notes.txt", 10)]);
            let shares = FileShares::new(None, None);
            let id = shares.prepare(&fx.files, "main").unwrap();
            let ticket = ticket_of(&ready(&shares, &id).await);
            assert_eq!((ticket.files, ticket.size), (Some(1), 10));
            let conn = phone(&shares).await;
            let list = file_link::decode_list(&ask(&conn, &ticket.token, file_link::LIST, 0).await.1).unwrap();
            assert_eq!(list[0].name, "notes.txt");
            assert_eq!(ask(&conn, &ticket.token, 0, 0).await.1, fx.data[0]);
            settle().await;
            assert!(matches!(shares.status(&id), Some(View::Ready { downloads: 1, .. })));
        })
        .await
        .expect("timed out");
    }

    #[tokio::test]
    async fn stopped_changed_unknown_missing_and_overlong_requests_are_refused() {
        tokio::time::timeout(Duration::from_secs(30), async {
            let fx = fixture(&[("a.bin", 5_000), ("b.bin", 7)]);
            let shares = FileShares::new(Some("http://localhost:9/get/".into()), None);
            let id = shares.prepare(&fx.files, "main").unwrap();
            let View::Ready { link, .. } = ready(&shares, &id).await else { panic!() };
            assert!(link.starts_with("http://localhost:9/get/#"));
            let ticket = Ticket::decode(link.split_once('#').unwrap().1).unwrap();
            let conn = phone(&shares).await;

            assert_eq!(ask(&conn, &[0u8; TOKEN_LEN], file_link::LIST, 0).await.0, Status::NotShared);
            assert_eq!(ask(&conn, &[0u8; TOKEN_LEN], 0, 0).await.0, Status::NotShared);
            assert_eq!(ask(&conn, &ticket.token, 2, 0).await.0, Status::NoSuchFile);
            assert_eq!(ask(&conn, &ticket.token, 0, 5_001).await.0, Status::BadOffset);
            assert_eq!(ask(&conn, &ticket.token, 0, 5_000).await.1, Vec::<u8>::new());

            // Same size, new content and time: refused, never served stale.
            std::thread::sleep(Duration::from_millis(20));
            std::fs::write(&fx.files[0], vec![1u8; 5_000]).unwrap();
            assert_eq!(ask(&conn, &ticket.token, 0, 0).await.0, Status::Changed);
            assert_eq!(ask(&conn, &ticket.token, 1, 0).await.1, fx.data[1], "the others still download");

            shares.stop(&id);
            assert_eq!(shares.status(&id), None);
            assert_eq!(ask(&conn, &ticket.token, file_link::LIST, 0).await.0, Status::NotShared);
            assert_eq!(ask(&conn, &ticket.token, 1, 0).await.0, Status::NotShared);
        })
        .await
        .expect("timed out");
    }

    #[tokio::test]
    async fn folders_missing_files_and_too_many_files_cannot_be_shared() {
        let fx = fixture(&[("a.txt", 1)]);
        let dir = tempfile::tempdir().unwrap();
        let shares = FileShares::new(None, None);
        assert_eq!(shares.prepare(&[dir.path().to_path_buf()], "main").unwrap_err(), NOT_A_FILE);
        assert_eq!(shares.prepare(&[fx.files[0].clone(), dir.path().to_path_buf()], "main").unwrap_err(), NOT_A_FILE);
        assert!(shares.prepare(&[dir.path().join("gone")], "main").is_err());
        assert_eq!(shares.prepare(&[], "main").unwrap_err(), NOTHING);
        let many = vec![fx.files[0].clone(); file_link::MAX_FILES + 1];
        assert_eq!(shares.prepare(&many, "main").unwrap_err(), TOO_MANY);
        assert_eq!(shares.status("nope"), None);
        shares.stop("nope");
    }

    #[tokio::test]
    async fn closing_a_window_stops_only_its_shares() {
        let fx = fixture(&[("a.txt", 100)]);
        let shares = FileShares::new(None, None);
        let mine = shares.prepare(&fx.files, "main").unwrap();
        let other = shares.prepare(&fx.files, "window-2").unwrap();
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
        let fx = fixture(&[("big.bin", 3 * HASH_CHUNK + 10)]);
        let seen = Mutex::new(Vec::new());
        let file = &fx.files[0];
        let (hash, size, _) = hash_file(file, &AtomicBool::new(false), &|n| seen.lock().unwrap().push(n)).unwrap();
        assert_eq!(hash, *blake3::hash(&fx.data[0]).as_bytes());
        assert_eq!(size, fx.data[0].len() as u64);
        assert_eq!(seen.into_inner().unwrap().last(), Some(&size));
        assert_eq!(hash_file(file, &AtomicBool::new(true), &|_| {}).unwrap_err(), "Stopped.");
    }

    #[test]
    fn views_report_progress_across_files() {
        let entry = |phase| Entry {
            window: String::new(),
            token: [0; TOKEN_LEN],
            cancel: Arc::default(),
            phase,
            downloads: 1,
            sending: HashMap::from([
                (1, Progress { done: HashSet::new(), finished: 0, current: 25, total: 100 }),
                (2, Progress { done: HashSet::from([0]), finished: 40, current: 10, total: 100 }),
            ]),
        };
        assert_eq!(entry(Phase::Preparing { hashed: 5, size: 10 }).view(), View::Preparing { progress: 0.5 });
        assert_eq!(entry(Phase::Preparing { hashed: 0, size: 0 }).view(), View::Preparing { progress: 1.0 });
        let ready = entry(Phase::Ready { link: "l".into(), qr: "q".into(), list: Vec::new(), files: Vec::new() }).view();
        assert_eq!(ready, View::Ready { link: "l".into(), qr: "q".into(), downloads: 1, sending: Some(0.5) });
        assert_eq!(entry(Phase::Failed("no".into())).view(), View::Failed { error: "no".into() });
        assert!(qr_svg("https://example.com/get/#abc").unwrap().contains("<svg"));
    }
}
