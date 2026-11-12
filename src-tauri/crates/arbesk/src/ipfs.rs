//! Reading Arbesk's content from its IPFS gateway. Payloads are stored
//! brotli-framed (`ARB\x01` + a brotli stream, asset-core's
//! `utils/brotli-frame.ts`), gzipped, or as they are; [`unpack`] undoes
//! either. Manifests are content-addressed and never change, so their JSON
//! is kept on disk by CID ([`ManifestCache`]).

use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use serde_json::Value;

use crate::http::{Http, TOO_LONG};
use crate::progress::CANCELLED;

const BROTLI_MAGIC: &[u8] = b"ARB\x01";
const GZIP_MAGIC: &[u8] = &[0x1f, 0x8b];

/// How an error starts when the gateway didn't answer: a listing fails with
/// it rather than leaving the row out.
pub const STORAGE_UNREACHABLE: &str = "Arbesk's storage could not be reached";

pub const TOO_LARGE: &str = "Arbesk sent a file larger than File Browser reads.";
/// The most a manifest may hold once unpacked.
pub const JSON_CAP: u64 = 16 * 1024 * 1024;
/// The most one part of an asset may hold once unpacked.
pub const PART_CAP: u64 = 512 * 1024 * 1024;
/// A capped read found more than its cap (see [`Gateway::bytes_within`]).
const OVER: &str = "The part is larger than its source says.";

/// How far a part whose source declares `size` bytes may be read: a
/// little more, for a packed body's framing and a padded buffer.
pub fn within(size: u64) -> u64 {
    size.saturating_add(size / 64).saturating_add(4096)
}

/// The most a thumbnail may hold once unpacked; the Studio's are a few
/// hundred kilobytes.
pub const IMAGE_CAP: u64 = 16 * 1024 * 1024;

/// A stored payload as it was before Arbesk packed it, at most `cap` bytes:
/// a few packed bytes can stand for gigabytes, so unpacking stops there.
pub fn unpack(bytes: Vec<u8>, cap: u64) -> Result<Vec<u8>, String> {
    let damaged = |_| "Arbesk sent a damaged file.".to_string();
    let mut out = Vec::new();
    if let Some(packed) = bytes.strip_prefix(BROTLI_MAGIC) {
        brotli::Decompressor::new(packed, 4096)
            .take(cap + 1)
            .read_to_end(&mut out)
            .map_err(damaged)?;
    } else if bytes.starts_with(GZIP_MAGIC) {
        flate2::read::GzDecoder::new(bytes.as_slice())
            .take(cap + 1)
            .read_to_end(&mut out)
            .map_err(damaged)?;
    } else {
        out = bytes;
    }
    if out.len() as u64 > cap {
        return Err(TOO_LARGE.into());
    }
    Ok(out)
}

/// A failed gateway request in words: nothing there, or no answer.
fn storage_error(cid: &str, error: crate::http::HttpError) -> String {
    match error.status {
        // The gateway answered: it has nothing there. Not worth failing a listing for.
        Some(404 | 410) => format!("Arbesk's storage has nothing at {cid}."),
        _ => format!("{STORAGE_UNREACHABLE}: {}", error.message),
    }
}

/// A failed read of `cid` in words: a capped read's body past its cap is
/// [`OVER`], anything else [`storage_error`].
fn read_error(cid: &str, cap: Option<u64>, error: crate::http::HttpError) -> String {
    match cap {
        Some(_) if error.message == TOO_LONG => OVER.to_string(),
        _ => storage_error(cid, error),
    }
}

/// A CID as the gateway's path takes it: letters and digits only, so
/// nothing in it can change which URL is asked for or which file is read.
pub(crate) fn checked_cid(cid: &str) -> Result<&str, String> {
    if !cid.is_empty() && cid.bytes().all(|b| b.is_ascii_alphanumeric()) {
        Ok(cid)
    } else {
        Err(format!("“{cid}” is not an IPFS address."))
    }
}

/// How long a `.part` file may sit in a cache before a trim removes it: one
/// another writer is still making is younger than this.
const PART_AGE: Duration = Duration::from_secs(600);

/// The next `.part` name. A clock tick can repeat, a counter cannot.
static NEXT_PART: AtomicU64 = AtomicU64::new(0);

/// Every cache directory's size, so several instances on one folder share one
/// count instead of each growing it to `cap`. Each directory has its own lock,
/// so a trim of one never blocks another.
type Totals = Mutex<HashMap<PathBuf, Arc<Mutex<Option<u64>>>>>;

fn totals() -> &'static Totals {
    static TOTALS: OnceLock<Totals> = OnceLock::new();
    TOTALS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn total_for(dir: &Path) -> Arc<Mutex<Option<u64>>> {
    totals()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .entry(dir.to_path_buf())
        .or_insert_with(|| Arc::new(Mutex::new(None)))
        .clone()
}

fn forget_total(dir: &Path) {
    totals()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .remove(dir);
}

/// A cache of whole files by key, at most `cap` bytes. Writes are staged under
/// a `.part` name and renamed in, so a reader never sees half a file. The
/// cache is only a shortcut: a missing or damaged entry is fetched again.
pub struct FileCache {
    dir: PathBuf,
    cap: u64,
}

impl FileCache {
    pub fn new(dir: PathBuf, cap: u64) -> Self {
        Self { dir, cap }
    }

    /// A key safe as one file name in the cache.
    fn valid_key(key: &str) -> bool {
        !key.is_empty()
            && key != "."
            && key != ".."
            && key
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    }

    pub fn get(&self, key: &str) -> Option<Vec<u8>> {
        if !Self::valid_key(key) {
            return None;
        }
        let path = self.dir.join(key);
        let metadata = std::fs::metadata(&path).ok()?;
        // A corrupt entry can claim any size; don't read it into memory.
        if !metadata.is_file() {
            forget_total(&self.dir);
            return None;
        }
        if metadata.len() > self.cap {
            let _ = std::fs::remove_file(&path);
            forget_total(&self.dir);
            return None;
        }
        match std::fs::read(&path) {
            Ok(bytes) => Some(bytes),
            Err(_) => {
                // A damaged entry is removed so the next fetch can replace it.
                let _ = std::fs::remove_file(&path);
                forget_total(&self.dir);
                None
            }
        }
    }

    /// Keep `bytes` for `key`, best effort. An entry already there wins; the
    /// caller removes a damaged one before asking for it again.
    pub fn put(&self, key: &str, bytes: &[u8]) {
        if !Self::valid_key(key) || bytes.len() as u64 > self.cap {
            return;
        }
        if std::fs::create_dir_all(&self.dir).is_err() {
            return;
        }
        self.sweep_parts_once();
        let part = self.dir.join(format!(
            ".{key}.{}.{}.part",
            std::process::id(),
            NEXT_PART.fetch_add(1, Ordering::Relaxed)
        ));
        if std::fs::write(&part, bytes).is_err() {
            let _ = std::fs::remove_file(&part);
            return;
        }
        if crate::cache::rename_new(&part, &self.dir.join(key)).is_err() {
            // Someone stored the same (content-addressed) entry first.
            let _ = std::fs::remove_file(&part);
            return;
        }
        self.charge(key, bytes.len() as u64);
    }

    pub fn remove(&self, key: &str) {
        if !Self::valid_key(key) {
            return;
        }
        let _ = std::fs::remove_file(self.dir.join(key));
        // The next write scans the folder once and starts from the truth.
        forget_total(&self.dir);
    }

    /// Remove a crashed writer's `.part` once per directory per process.
    fn sweep_parts_once(&self) {
        static SWEPT: OnceLock<Mutex<HashSet<PathBuf>>> = OnceLock::new();
        let first = {
            let mut swept = SWEPT
                .get_or_init(|| Mutex::new(HashSet::new()))
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            swept.insert(self.dir.clone())
        };
        if first {
            let _ = trim(&self.dir, u64::MAX, "");
        }
    }

    /// Add `bytes` to the folder's count, trimming when it no longer fits.
    fn charge(&self, kept: &str, bytes: u64) {
        let slot = total_for(&self.dir);
        let mut total = slot
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        // A first scan already counts the file just renamed in.
        let current = match *total {
            Some(total) => total.saturating_add(bytes),
            None => dir_total(&self.dir),
        };
        *total = Some(if current > self.cap {
            trim(&self.dir, self.cap, kept)
        } else {
            current
        });
    }
}

/// The bytes of every file in `dir`, ignoring folders and unfinished parts.
fn dir_total(dir: &Path) -> u64 {
    let Ok(read) = std::fs::read_dir(dir) else {
        return 0;
    };
    read.flatten()
        .filter_map(|entry| {
            let metadata = entry.metadata().ok()?;
            (metadata.is_file() && !entry.file_name().to_string_lossy().ends_with(".part"))
                .then_some(metadata.len())
        })
        .sum()
}

/// Remove the oldest files in `dir`, never `kept`, until the rest fit `cap`;
/// the total left. `.part` files older than [`PART_AGE`] are dropped too.
fn trim(dir: &Path, cap: u64, kept: &str) -> u64 {
    let Ok(read) = std::fs::read_dir(dir) else {
        return 0;
    };
    let mut entries: Vec<(std::time::SystemTime, u64, PathBuf, String)> = Vec::new();
    for entry in read.flatten() {
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if !metadata.is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.ends_with(".part") {
            if metadata
                .modified()
                .ok()
                .and_then(|modified| modified.elapsed().ok())
                .is_some_and(|age| age >= PART_AGE)
            {
                let _ = std::fs::remove_file(entry.path());
            }
            continue;
        }
        let Ok(modified) = metadata.modified() else {
            continue;
        };
        entries.push((modified, metadata.len(), entry.path(), name));
    }
    entries.sort();
    let mut total: u64 = entries.iter().map(|(_, len, _, _)| len).sum();
    for (_, len, path, name) in entries {
        if total <= cap {
            break;
        }
        if name != kept && std::fs::remove_file(&path).is_ok() {
            total -= len;
        }
    }
    total
}

/// Manifests by CID in the app cache, at most `cap` bytes; the oldest go
/// first. A file damaged outside the app fails to parse, is removed, and is
/// fetched again.
pub struct ManifestCache {
    files: FileCache,
}

impl ManifestCache {
    pub fn new(dir: PathBuf, cap: u64) -> Self {
        Self {
            files: FileCache::new(dir, cap),
        }
    }

    pub fn get(&self, cid: &str) -> Option<Vec<u8>> {
        self.files.get(checked_cid(cid).ok()?)
    }

    /// Keep `bytes` for `cid`, best effort: the cache is only a shortcut.
    pub fn put(&self, cid: &str, bytes: &[u8]) {
        if let Ok(cid) = checked_cid(cid) {
            self.files.put(cid, bytes);
        }
    }

    /// Forget a damaged entry so the next fetch replaces it.
    pub fn remove(&self, cid: &str) {
        if let Ok(cid) = checked_cid(cid) {
            self.files.remove(cid);
        }
    }
}

/// The IPFS gateway of an account's chain.
pub struct Gateway<'a> {
    http: &'a dyn Http,
    base: String,
    cache: ManifestCache,
    json_cap: u64,
    part_cap: u64,
}

impl<'a> Gateway<'a> {
    pub fn new(http: &'a dyn Http, base: &str, cache: ManifestCache) -> Self {
        Self {
            http,
            base: base.trim_end_matches('/').to_string(),
            cache,
            json_cap: JSON_CAP,
            part_cap: PART_CAP,
        }
    }

    /// The same gateway with other caps (tests use small ones).
    #[cfg(test)]
    pub fn with_caps(self, json_cap: u64, part_cap: u64) -> Self {
        Self {
            json_cap,
            part_cap,
            ..self
        }
    }

    fn fetch(&self, cid: &str, cap: u64) -> Result<Vec<u8>, String> {
        unpack(self.get(cid)?, cap)
    }

    /// The stored bytes at `cid`, as the gateway sends them. Through the
    /// client's limits when it has some: a read permit, and a read of the
    /// same CID already under way shared rather than asked again.
    fn get(&self, cid: &str) -> Result<Vec<u8>, String> {
        self.get_within(cid, None)
    }

    /// [`Gateway::get`], reading at most `cap` bytes when given one: past
    /// it the answer is [`OVER`]. A capped read is only shared with reads
    /// of the same cap.
    fn get_within(&self, cid: &str, cap: Option<u64>) -> Result<Vec<u8>, String> {
        let url = format!("{}/{}", self.base, checked_cid(cid)?);
        self.limited(&url, cap, || {
            let answer = match cap {
                Some(cap) => self.http.get_within(&url, cap),
                None => self.http.get(&url),
            };
            answer.map_err(|error| read_error(cid, cap, error))
        })
    }

    /// `read` of `url` (at most `cap` bytes, when given) through the
    /// client's limits when it has some: a read permit, and a read of the
    /// same URL and cap already under way shared rather than asked again.
    fn limited(
        &self,
        url: &str,
        cap: Option<u64>,
        read: impl Fn() -> Result<Vec<u8>, String>,
    ) -> Result<Vec<u8>, String> {
        let Some(limits) = self.http.limits() else {
            return read();
        };
        let key = match cap {
            Some(cap) => format!("{url} within {cap}"),
            None => url.to_string(),
        };
        limits.limiter.shared(&key, limits.stop, || {
            let _permit = limits.limiter.read(limits.stop)?;
            read()
        })
    }

    /// A part's stored bytes at `cid` (performance plan, phase 6): shared
    /// and limited as [`Gateway::get_within`] is, and read on the same
    /// timeouts (`Http::get_chunked`: a stall, never a limit on the whole
    /// body), but handed over chunk by chunk, so between chunks, and every
    /// `TICK` while none arrives, the read hears its operation's stop: a
    /// Cancel or a closed preview ends it with [`CANCELLED`], its permit
    /// given back at once. Past `cap` bytes, when given, it stops with
    /// [`OVER`].
    fn download(&self, cid: &str, cap: Option<u64>) -> Result<Vec<u8>, String> {
        let url = format!("{}/{}", self.base, checked_cid(cid)?);
        self.limited(&url, cap, || self.stream(&url, cid, cap))
    }

    /// One read of [`Gateway::download`], unshared.
    fn stream(&self, url: &str, cid: &str, cap: Option<u64>) -> Result<Vec<u8>, String> {
        let mut bytes = Vec::new();
        let mut stopped = None;
        let read = self.http.get_chunked(url, cap, &mut |chunk| {
            stopped = if self.stopped() {
                Some(CANCELLED.to_string())
            } else if cap.is_some_and(|cap| (bytes.len() + chunk.len()) as u64 > cap) {
                Some(OVER.to_string())
            } else {
                bytes.extend_from_slice(chunk);
                None
            };
            stopped.is_none()
        });
        if let Some(error) = stopped {
            return Err(error);
        }
        read.map_err(|error| read_error(cid, cap, error))?;
        Ok(bytes)
    }

    /// What the gateway's client is limited by, if anything (a build's
    /// shared byte budget is its limiter's).
    pub fn limits(&self) -> Option<crate::limiter::Limits<'_>> {
        self.http.limits()
    }

    /// Whether the operation these reads are for was stopped (a cancel, a
    /// closed preview); never without limits.
    pub fn stopped(&self) -> bool {
        self.http.limits().is_some_and(|limits| (limits.stop)())
    }

    /// The stored bytes at `cid`, never unpacked: an asset's other file,
    /// which its entry's `encoding` decodes (a raw file that starts like a
    /// brotli frame or gzip stays as it is). Parts keep [`Gateway::bytes`].
    ///
    /// Streamed, given time for about `expected` bytes
    /// (`http::transfer_timeout`): `on_chunk` hears each chunk's length
    /// before it is kept and stops the read with its error (a cancel);
    /// more than `cap` bytes stops it with [`TOO_LARGE`].
    pub fn raw(
        &self,
        cid: &str,
        cap: u64,
        expected: u64,
        on_chunk: &mut dyn FnMut(usize) -> Result<(), String>,
    ) -> Result<Vec<u8>, String> {
        let url = format!("{}/{}", self.base, checked_cid(cid)?);
        // Its own read, not shared: its progress and cancel are its caller's.
        let limits = self.http.limits();
        let _permit = match limits {
            Some(limits) => Some(limits.limiter.read(limits.stop)?),
            None => None,
        };
        let mut bytes = Vec::new();
        let mut stopped = None;
        let read = self.http.get_streamed(&url, expected, &mut |chunk| {
            let kept = (bytes.len() + chunk.len()) as u64;
            let refused = if kept > cap { Err(TOO_LARGE.to_string()) } else { on_chunk(chunk.len()) };
            match refused {
                Ok(()) => bytes.extend_from_slice(chunk),
                Err(error) => stopped = Some(error),
            }
            stopped.is_none()
        });
        if let Some(error) = stopped {
            return Err(error);
        }
        read.map_err(|error| storage_error(cid, error))?;
        Ok(bytes)
    }

    /// The content at `cid`, unpacked: a part of unknown size (or an older
    /// save's model), downloaded as [`Gateway::download`] does.
    pub fn bytes(&self, cid: &str) -> Result<Vec<u8>, String> {
        self.part(cid, None)
    }

    /// A part's content at `cid`, unpacked to at most the part cap (or
    /// `cap`): the body read no further than `cap` when given.
    fn part(&self, cid: &str, cap: Option<u64>) -> Result<Vec<u8>, String> {
        unpack(self.download(cid, cap)?, cap.unwrap_or(self.part_cap))
    }

    /// The content at `cid`, unpacked, when it is about the `size` its
    /// source declares: neither the body nor what it unpacks to is read
    /// past [`within`]`(size)`, and `None` says the part is larger than
    /// declared (the caller fetches it again as one of unknown size). A
    /// size at the part cap or past it reads as [`Gateway::bytes`].
    pub fn bytes_within(&self, cid: &str, size: u64) -> Result<Option<Vec<u8>>, String> {
        let cap = within(size);
        if cap >= self.part_cap {
            return self.bytes(cid).map(Some);
        }
        match self.part(cid, Some(cap)) {
            Err(error) if error == OVER || error == TOO_LARGE => Ok(None),
            unpacked => unpacked.map(Some),
        }
    }

    /// The picture at `cid` (an asset's thumbnail), unpacked.
    pub fn image(&self, cid: &str) -> Result<Vec<u8>, String> {
        self.fetch(cid, IMAGE_CAP)
    }

    /// The JSON document at `cid`, from the cache when it is there.
    pub fn json(&self, cid: &str) -> Result<Value, String> {
        checked_cid(cid)?;
        if let Some(bytes) = self.cache.get(cid) {
            match serde_json::from_slice(&bytes) {
                Ok(value) => return Ok(value),
                // A damaged entry is replaced by this fetch, not fetched
                // forever.
                Err(_) => self.cache.remove(cid),
            }
        }
        let bytes = self.fetch(cid, self.json_cap)?;
        let value = match serde_json::from_slice(&bytes) {
            Ok(value) => value,
            Err(_) => {
                return Err("Arbesk sent a manifest File Browser can't read.".to_string());
            }
        };
        self.cache.put(cid, &bytes);
        Ok(value)
    }
}

/// `work` for each item on at most `limit` threads, answers in order. The
/// reads `work` makes take their permits from the client's limiter one by
/// one (a worker holding one while its next read waits for another could
/// leave every permit waiting), so `limit` only bounds the threads:
/// `limiter::READS`, the most that can read at once anyway.
pub fn in_parallel<T: Sync, R: Send>(
    items: &[T],
    limit: usize,
    work: impl Fn(&T) -> R + Sync,
) -> Vec<R> {
    let next = AtomicUsize::new(0);
    let answers: Mutex<Vec<Option<R>>> = Mutex::new(items.iter().map(|_| None).collect());
    std::thread::scope(|scope| {
        for _ in 0..limit.clamp(1, items.len().max(1)) {
            scope.spawn(|| {
                loop {
                    let index = next.fetch_add(1, Ordering::SeqCst);
                    let Some(item) = items.get(index) else { break };
                    let answer = work(item);
                    answers.lock().unwrap_or_else(|e| e.into_inner())[index] = Some(answer);
                }
            });
        }
    });
    answers
        .into_inner()
        .unwrap_or_else(|e| e.into_inner())
        .into_iter()
        .flatten()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::fake::{FakeHttp, not_found};
    use std::io::Write;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn brotli_framed(bytes: &[u8]) -> Vec<u8> {
        let mut packed = Vec::new();
        {
            let mut writer = brotli::CompressorWriter::new(&mut packed, 4096, 5, 22);
            writer.write_all(bytes).unwrap();
        }
        [b"ARB\x01".as_slice(), &packed].concat()
    }

    fn gzipped(bytes: &[u8]) -> Vec<u8> {
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(bytes).unwrap();
        encoder.finish().unwrap()
    }

    #[test]
    fn unpack_undoes_a_brotli_frame_gzip_or_nothing() {
        let text = b"{\"name\":\"Box\"}".to_vec();
        assert_eq!(unpack(brotli_framed(&text), 1 << 20).unwrap(), text);
        assert_eq!(unpack(gzipped(&text), 1 << 20).unwrap(), text);
        assert_eq!(unpack(text.clone(), 1 << 20).unwrap(), text);
        assert_eq!(unpack(Vec::new(), 1 << 20).unwrap(), Vec::<u8>::new());
    }

    #[test]
    fn unpacking_stops_at_its_cap() {
        let zeros = vec![0u8; 1 << 20];
        for packed in [brotli_framed(&zeros), gzipped(&zeros)] {
            assert!(packed.len() < 64 * 1024, "a small payload that inflates");
            assert_eq!(unpack(packed.clone(), 1 << 20).unwrap().len(), 1 << 20);
            assert_eq!(unpack(packed, (1 << 20) - 1).unwrap_err(), TOO_LARGE);
        }
        assert_eq!(unpack(vec![1u8; 11], 10).unwrap_err(), TOO_LARGE);
        assert_eq!(unpack(vec![1u8; 10], 10).unwrap().len(), 10);
    }

    #[test]
    fn a_manifest_over_its_cap_is_refused_and_not_cached() {
        let dir = tempfile::tempdir().unwrap();
        let big = format!("{{\"name\":\"{}\"}}", "x".repeat(200));
        let http = FakeHttp::new(move |_| Ok(brotli_framed(big.as_bytes())));
        let gateway = gateway_over(&http, dir.path()).with_caps(100, 1 << 20);
        assert_eq!(gateway.json("bafyM").unwrap_err(), TOO_LARGE);
        assert!(!dir.path().join("bafyM").exists());
        assert_eq!(gateway.bytes("bafyP").unwrap().len(), 211);
    }

    #[test]
    fn a_broken_brotli_frame_is_an_error() {
        assert!(unpack(b"ARB\x01not brotli at all".to_vec(), 1 << 20).is_err());
    }

    fn gateway_over<'a>(http: &'a FakeHttp, dir: &std::path::Path) -> Gateway<'a> {
        Gateway::new(
            http,
            "https://gw.test/ipfs/",
            ManifestCache::new(dir.to_path_buf(), 64 * 1024 * 1024),
        )
    }

    #[test]
    fn bytes_are_fetched_from_the_gateway_and_unpacked() {
        let dir = tempfile::tempdir().unwrap();
        let http = FakeHttp::new(|request| {
            assert_eq!(request.url, "https://gw.test/ipfs/bafyPart");
            Ok(gzipped(b"part"))
        });
        assert_eq!(
            gateway_over(&http, dir.path()).bytes("bafyPart").unwrap(),
            b"part"
        );
    }

    #[test]
    fn raw_bytes_are_never_unpacked() {
        let dir = tempfile::tempdir().unwrap();
        for stored in [brotli_framed(b"G1 X1"), gzipped(b"G1 X1"), b"plain".to_vec()] {
            let sent = stored.clone();
            let http = FakeHttp::new(move |request| {
                assert_eq!(request.url, "https://gw.test/ipfs/bafyRep");
                Ok(sent.clone())
            });
            let gateway = gateway_over(&http, dir.path());
            let raw = |cid: &str, cap: u64| gateway.raw(cid, cap, 0, &mut |_| Ok(()));
            assert_eq!(raw("bafyRep", 1 << 20).unwrap(), stored);
            assert_eq!(raw("bafyRep", stored.len() as u64).unwrap(), stored);
            assert_eq!(raw("bafyRep", stored.len() as u64 - 1).unwrap_err(), TOO_LARGE);
            assert!(raw("../x", 1 << 20).is_err());
        }
    }

    /// A raw file streams in chunks: each is heard before it is kept, a
    /// cancel stops the read there, and the cap stops it at the chunk
    /// that passes it, before the rest is read.
    #[test]
    fn raw_bytes_stream_and_stop_on_a_cancel_or_the_cap() {
        let dir = tempfile::tempdir().unwrap();
        let body: Vec<u8> = (0..20).collect();
        let sent = body.clone();
        let http = FakeHttp::new(move |_| Ok(sent.clone())).in_chunks(4, |_| {});
        let gateway = gateway_over(&http, dir.path());
        let mut heard = Vec::new();
        let whole = gateway.raw("bafyRep", 20, 20, &mut |n| {
            // An empty piece is a tick while nothing arrives.
            if n > 0 {
                heard.push(n);
            }
            Ok(())
        });
        assert_eq!((whole.unwrap(), heard), (body, vec![4; 5]));
        let read = |http: &FakeHttp| http.chunks.swap(0, std::sync::atomic::Ordering::SeqCst);
        read(&http);
        let mut calls = 0;
        let cancelled = gateway.raw("bafyRep", 20, 20, &mut |n| {
            calls += usize::from(n > 0);
            if calls == 2 { Err(crate::progress::CANCELLED.into()) } else { Ok(()) }
        });
        assert_eq!(cancelled.unwrap_err(), crate::progress::CANCELLED);
        assert_eq!(read(&http), 2, "nothing read past the cancel");
        assert_eq!(gateway.raw("bafyRep", 9, 20, &mut |_| Ok(())).unwrap_err(), TOO_LARGE);
        assert_eq!(read(&http), 3, "nothing read past the cap");
        let missing = FakeHttp::new(|_| Err(crate::http::fake::not_found()));
        let error = gateway_over(&missing, dir.path()).raw("bafyGone", 20, 0, &mut |_| Ok(()));
        assert_eq!(error.unwrap_err(), "Arbesk's storage has nothing at bafyGone.");
    }

    #[test]
    fn json_is_read_once_then_from_the_cache() {
        let dir = tempfile::tempdir().unwrap();
        let http = FakeHttp::new(|_| Ok(brotli_framed(b"{\"name\":\"Box\"}")));
        let first = gateway_over(&http, dir.path()).json("bafyM").unwrap();
        let second = gateway_over(&http, dir.path()).json("bafyM").unwrap();
        assert_eq!(first["name"], "Box");
        assert_eq!(second, first);
        assert_eq!(http.requests().len(), 1);
    }

    #[test]
    fn a_damaged_cache_entry_is_fetched_again_and_repaired() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("bafyM"), b"{ half").unwrap();
        let http = FakeHttp::new(|_| Ok(b"{\"name\":\"Box\"}".to_vec()));
        assert_eq!(
            gateway_over(&http, dir.path()).json("bafyM").unwrap()["name"],
            "Box"
        );
        assert_eq!(http.requests().len(), 1);
        // The damaged entry was replaced, not fetched on every visit.
        let repaired: serde_json::Value =
            serde_json::from_slice(&std::fs::read(dir.path().join("bafyM")).unwrap()).unwrap();
        assert_eq!(repaired["name"], "Box");
    }

    #[test]
    fn a_cid_that_is_not_one_is_refused_before_any_request() {
        let dir = tempfile::tempdir().unwrap();
        let http = FakeHttp::new(|_| Ok(Vec::new()));
        let gateway = gateway_over(&http, dir.path());
        for cid in ["", "../x", "a/b", "bafy?x=1", "bafy#x", "bafy x"] {
            assert!(gateway.json(cid).is_err(), "{cid}");
            assert!(gateway.bytes(cid).is_err(), "{cid}");
        }
        assert!(http.requests().is_empty());
    }

    #[test]
    fn a_failing_gateway_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let http = FakeHttp::new(|_| Err(not_found()));
        assert!(gateway_over(&http, dir.path()).json("bafyM").is_err());
        assert!(gateway_over(&http, dir.path()).bytes("bafyP").is_err());
    }

    #[test]
    fn a_file_cache_keeps_a_file_and_refuses_a_bad_key() {
        let dir = tempfile::tempdir().unwrap();
        let cache = FileCache::new(dir.path().to_path_buf(), 1 << 20);
        cache.put("bafyBox-128.png", b"whole");
        assert_eq!(cache.get("bafyBox-128.png"), Some(b"whole".to_vec()));
        for key in ["", ".", "..", "../x", "a/b", "a b", "bafy?x"] {
            cache.put(key, b"nope");
            assert_eq!(cache.get(key), None, "{key}");
            assert!(!dir.path().join(key).is_file(), "{key}");
        }
    }

    #[test]
    fn an_oversized_cache_entry_is_removed_before_reading_it() {
        let dir = tempfile::tempdir().unwrap();
        let cache = FileCache::new(dir.path().to_path_buf(), 4);
        let path = dir.path().join("huge");
        std::fs::write(&path, b"too many bytes").unwrap();
        assert_eq!(cache.get("huge"), None);
        assert!(!path.exists());
    }

    #[test]
    fn two_caches_on_one_directory_keep_the_cap() {
        let dir = tempfile::tempdir().unwrap();
        let one = FileCache::new(dir.path().to_path_buf(), 25);
        let two = FileCache::new(dir.path().to_path_buf(), 25);
        for i in 0..5 {
            one.put(&format!("one{i}"), &[b'a'; 10]);
            two.put(&format!("two{i}"), &[b'b'; 10]);
            let total: u64 = std::fs::read_dir(dir.path())
                .unwrap()
                .flatten()
                .filter_map(|entry| {
                    let name = entry.file_name().to_string_lossy().into_owned();
                    if name.ends_with(".part") {
                        return None;
                    }
                    entry.metadata().ok().map(|metadata| metadata.len())
                })
                .sum();
            assert!(total <= 25, "{total} bytes after put {i}");
        }
    }

    #[test]
    fn a_cached_file_is_never_half_written() {
        let dir = tempfile::tempdir().unwrap();
        let writer = FileCache::new(dir.path().to_path_buf(), 1 << 20);
        let reader = FileCache::new(dir.path().to_path_buf(), 1 << 20);
        let done = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let stop = done.clone();
        let read = std::thread::spawn(move || {
            while !stop.load(Ordering::SeqCst) {
                if let Some(bytes) = reader.get("whole") {
                    assert_eq!(bytes.len(), 64 * 1024);
                    assert!(bytes.iter().all(|byte| *byte == bytes[0]));
                }
                std::thread::yield_now();
            }
        });
        for i in 0..40u8 {
            writer.put("whole", &vec![i; 64 * 1024]);
        }
        done.store(true, Ordering::SeqCst);
        read.join().unwrap();
    }

    #[test]
    fn a_stale_part_file_is_removed_but_a_live_one_is_kept() {
        let dir = tempfile::tempdir().unwrap();
        let stale = dir.path().join(".old.1.1.part");
        let live = dir.path().join(".new.1.2.part");
        std::fs::write(&stale, b"old").unwrap();
        std::fs::write(&live, b"live").unwrap();
        let old = std::time::SystemTime::now() - PART_AGE - Duration::from_secs(1);
        std::fs::File::options()
            .write(true)
            .open(&stale)
            .unwrap()
            .set_modified(old)
            .unwrap();
        trim(dir.path(), 1 << 20, "kept");
        assert!(!stale.exists());
        assert!(live.exists());
    }

    #[test]
    fn the_cache_drops_its_oldest_entries_past_its_cap() {
        let dir = tempfile::tempdir().unwrap();
        let cache = ManifestCache::new(dir.path().to_path_buf(), 25);
        let now = std::time::SystemTime::now();
        for (n, (cid, byte)) in [("a1", b'a'), ("b2", b'b'), ("c3", b'c')]
            .into_iter()
            .enumerate()
        {
            cache.put(cid, &[byte; 10]);
            let file = std::fs::File::options()
                .write(true)
                .open(dir.path().join(cid))
                .unwrap();
            // Distinct times, without a sleep: a1 oldest, c3 newest.
            file.set_modified(now + std::time::Duration::from_secs(n as u64))
                .unwrap();
        }
        assert_eq!(cache.get("a1"), None);
        assert_eq!(cache.get("b2"), Some(vec![b'b'; 10]));
        assert_eq!(cache.get("c3"), Some(vec![b'c'; 10]));
    }

    /// Performance plan, phase 2: callers asking for one CID at once
    /// through the app's limiter share one read, each with its own bytes;
    /// a later caller reads again.
    #[test]
    fn callers_of_one_cid_at_once_share_one_read() {
        use crate::limiter::{Limited, Limiter, never};
        let dir = tempfile::tempdir().unwrap();
        let http = FakeHttp::new(|_| {
            std::thread::sleep(Duration::from_millis(100));
            Ok(gzipped(b"part"))
        });
        let limiter = Limiter::default();
        let limited = Limited::new(&http, &limiter, &never);
        let gateway = Gateway::new(
            &limited,
            "https://gw.test/ipfs/",
            ManifestCache::new(dir.path().to_path_buf(), 1 << 20),
        );
        let together = std::sync::Barrier::new(3);
        let answers: Vec<_> = std::thread::scope(|scope| {
            let callers: Vec<_> = (0..3)
                .map(|_| {
                    scope.spawn(|| {
                        together.wait();
                        gateway.bytes("bafyPart")
                    })
                })
                .collect();
            callers.into_iter().map(|caller| caller.join().unwrap()).collect()
        });
        assert!(answers.iter().all(|answer| answer.as_deref() == Ok(b"part".as_slice())));
        assert_eq!(http.requests().len(), 1);
        assert_eq!(gateway.bytes("bafyPart").unwrap(), b"part");
        assert_eq!(http.requests().len(), 2);
        assert_eq!(limiter.reading(), 0);
    }

    /// Serves `requests` GETs, each answered with `body` in `pieces`
    /// pieces `gap` apart: a part that keeps flowing, slowly. With
    /// `silent`, it goes silent after that many pieces until told to end.
    fn serve_slowly(
        body: Vec<u8>,
        pieces: usize,
        gap: Duration,
        requests: usize,
        silent: Option<(usize, std::sync::mpsc::Receiver<()>)>,
    ) -> (String, std::thread::JoinHandle<()>) {
        use std::io::Read;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            for _ in 0..requests {
                let (mut stream, _) = listener.accept().unwrap();
                let mut seen = Vec::new();
                let mut buffer = [0u8; 4096];
                while !seen.windows(4).any(|w| w == b"\r\n\r\n") {
                    let read = stream.read(&mut buffer).unwrap();
                    if read == 0 {
                        break;
                    }
                    seen.extend_from_slice(&buffer[..read]);
                }
                let head = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                if stream.write_all(head.as_bytes()).is_err() {
                    continue;
                }
                for (n, piece) in body.chunks(body.len().div_ceil(pieces)).enumerate() {
                    if let Some((after, end)) = &silent
                        && n == *after
                    {
                        let _ = end.recv_timeout(Duration::from_secs(10));
                        break;
                    }
                    std::thread::sleep(gap);
                    if stream.write_all(piece).and_then(|()| stream.flush()).is_err() {
                        break;
                    }
                }
            }
        });
        (url, server)
    }

    /// Performance plan, phase 6: a part that takes longer than the
    /// client's timeout (30 s; 1 s here) but keeps flowing is read whole,
    /// as it was before it was read in chunks: the timeout bounds each
    /// wait, never the whole body.
    #[test]
    fn a_part_slower_than_the_timeout_but_flowing_completes() {
        let dir = tempfile::tempdir().unwrap();
        let body = gzipped(&vec![7u8; 64 * 1024]);
        let (url, server) = serve_slowly(body, 8, Duration::from_millis(250), 1, None);
        let http = crate::http::ReqwestHttp::with_timeout(crate::http::MAX_BODY, Duration::from_secs(1)).unwrap();
        let gateway = Gateway::new(&http, &url, ManifestCache::new(dir.path().to_path_buf(), 1 << 20));
        let started = std::time::Instant::now();
        assert_eq!(gateway.bytes("bafyPart").unwrap(), vec![7u8; 64 * 1024]);
        assert!(started.elapsed() > Duration::from_secs(1), "{:?}", started.elapsed());
        server.join().unwrap();
    }

    /// A part whose gateway goes silent mid-read fails once nothing has
    /// arrived for the client's timeout (30 s; 1 s here), not later.
    #[test]
    fn a_stalled_part_fails_after_the_timeout() {
        let dir = tempfile::tempdir().unwrap();
        let (silent, end) = std::sync::mpsc::channel::<()>();
        let (url, server) = serve_slowly(vec![1u8; 1024], 4, Duration::ZERO, 1, Some((1, end)));
        let http = crate::http::ReqwestHttp::with_timeout(crate::http::MAX_BODY, Duration::from_secs(1)).unwrap();
        let gateway = Gateway::new(&http, &url, ManifestCache::new(dir.path().to_path_buf(), 1 << 20));
        let started = std::time::Instant::now();
        let error = gateway.bytes_within("bafyPart", 1024).unwrap_err();
        let took = started.elapsed();
        silent.send(()).unwrap();
        server.join().unwrap();
        assert!(error.starts_with(STORAGE_UNREACHABLE), "{error}");
        assert!(took >= Duration::from_secs(1) && took < Duration::from_secs(3), "{took:?}");
    }

    /// A shared read cancelled by its leader's stop mid-download does not
    /// fail the caller that joined it: that one reads again for itself and
    /// gets the part; the leader hears its cancel within a tick.
    #[test]
    fn a_cancelled_leader_does_not_fail_its_follower() {
        use crate::limiter::{Limited, Limiter};
        use std::sync::atomic::AtomicBool;
        use std::sync::mpsc;
        let dir = tempfile::tempdir().unwrap();
        let (reached, at_chunk) = mpsc::channel::<()>();
        let reached = Mutex::new(reached);
        let opened = Arc::new(AtomicBool::new(false));
        let gate = opened.clone();
        let http = FakeHttp::new(|_| Ok(gzipped(b"part"))).in_chunks(4, move |n| {
            if n == 1 && !gate.load(Ordering::SeqCst) {
                let _ = reached.lock().unwrap().send(());
                // Silent until the leader's cancel has been heard.
                while !gate.load(Ordering::SeqCst) {
                    std::thread::sleep(Duration::from_millis(5));
                }
            }
        });
        let limiter = Limiter::default();
        let (leader_stopped, follower_stopped) = (AtomicBool::new(false), AtomicBool::new(false));
        let leader_stop = || leader_stopped.load(Ordering::SeqCst);
        let follower_stop = || follower_stopped.load(Ordering::SeqCst);
        let (leading, following) = (Limited::new(&http, &limiter, &leader_stop), Limited::new(&http, &limiter, &follower_stop));
        let cache = || ManifestCache::new(dir.path().to_path_buf(), 1 << 20);
        let leader = Gateway::new(&leading, "https://gw.test/ipfs/", cache());
        let follower = Gateway::new(&following, "https://gw.test/ipfs/", cache());
        std::thread::scope(|scope| {
            let led = scope.spawn(|| leader.bytes("bafyPart"));
            at_chunk.recv_timeout(Duration::from_secs(5)).expect("the part is streamed");
            let followed = scope.spawn(|| follower.bytes("bafyPart"));
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            while limiter.followers("https://gw.test/ipfs/bafyPart") == 0 {
                assert!(std::time::Instant::now() < deadline, "the follower never joined");
                std::thread::sleep(Duration::from_millis(5));
            }
            let cancelled = std::time::Instant::now();
            leader_stopped.store(true, Ordering::SeqCst);
            let led = led.join().unwrap();
            let took = cancelled.elapsed();
            opened.store(true, Ordering::SeqCst);
            assert_eq!(led.unwrap_err(), CANCELLED);
            assert!(took < 2 * crate::http::TICK, "{took:?}");
            assert_eq!(followed.join().unwrap().unwrap(), b"part");
        });
        assert_eq!(http.requests().len(), 2, "the follower read again");
        assert_eq!(limiter.reading(), 0);
    }

    #[test]
    fn requests_never_exceed_the_limit_at_once() {
        let running = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let ids: Vec<usize> = (0..40).collect();
        let (r, p) = (running.clone(), peak.clone());
        let out = in_parallel(&ids, 8, move |id| {
            let now = r.fetch_add(1, Ordering::SeqCst) + 1;
            p.fetch_max(now, Ordering::SeqCst);
            std::thread::sleep(std::time::Duration::from_millis(5));
            r.fetch_sub(1, Ordering::SeqCst);
            id * 2
        });
        assert_eq!(out, ids.iter().map(|id| id * 2).collect::<Vec<_>>());
        assert!(peak.load(Ordering::SeqCst) <= 8);
        assert!(peak.load(Ordering::SeqCst) > 1);
    }
}
