//! Reading Arbesk's content from its IPFS gateway. Payloads are stored
//! brotli-framed (`ARB\x01` + a brotli stream, asset-core's
//! `utils/brotli-frame.ts`), gzipped, or as they are; [`unpack`] undoes
//! either. Manifests are content-addressed and never change, so their JSON
//! is kept on disk by CID ([`ManifestCache`]).

use std::io::Read;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::Value;

use crate::arbesk::http::Http;

const BROTLI_MAGIC: &[u8] = b"ARB\x01";
const GZIP_MAGIC: &[u8] = &[0x1f, 0x8b];

/// A stored payload as it was before Arbesk packed it.
pub fn unpack(bytes: Vec<u8>) -> Result<Vec<u8>, String> {
    let damaged = |_| "Arbesk sent a damaged file.".to_string();
    let mut out = Vec::new();
    if let Some(packed) = bytes.strip_prefix(BROTLI_MAGIC) {
        brotli::Decompressor::new(packed, 4096)
            .read_to_end(&mut out)
            .map_err(damaged)?;
    } else if bytes.starts_with(GZIP_MAGIC) {
        flate2::read::GzDecoder::new(bytes.as_slice())
            .read_to_end(&mut out)
            .map_err(damaged)?;
    } else {
        return Ok(bytes);
    }
    Ok(out)
}

/// A CID as the gateway's path takes it: letters and digits only, so
/// nothing in it can change which URL is asked for or which file is read.
fn checked_cid(cid: &str) -> Result<&str, String> {
    if !cid.is_empty() && cid.bytes().all(|b| b.is_ascii_alphanumeric()) {
        Ok(cid)
    } else {
        Err(format!("“{cid}” is not an IPFS address."))
    }
}

/// Manifests by CID in the app cache, at most `cap` bytes; the oldest go
/// first. A write that a crash cut short fails to parse and is fetched again.
pub struct ManifestCache {
    dir: PathBuf,
    cap: u64,
}

impl ManifestCache {
    pub fn new(dir: PathBuf, cap: u64) -> Self {
        Self { dir, cap }
    }

    pub fn get(&self, cid: &str) -> Option<Vec<u8>> {
        std::fs::read(self.dir.join(checked_cid(cid).ok()?)).ok()
    }

    /// Keep `bytes` for `cid`, best effort: the cache is only a shortcut.
    pub fn put(&self, cid: &str, bytes: &[u8]) {
        let Ok(cid) = checked_cid(cid) else { return };
        if std::fs::create_dir_all(&self.dir).is_ok()
            && std::fs::write(self.dir.join(cid), bytes).is_ok()
        {
            self.trim(cid);
        }
    }

    /// Remove the oldest entries, never `kept`, until the rest fit the cap.
    fn trim(&self, kept: &str) {
        let Ok(read) = std::fs::read_dir(&self.dir) else {
            return;
        };
        let mut entries: Vec<(std::time::SystemTime, u64, PathBuf)> = read
            .flatten()
            .filter_map(|entry| {
                let meta = entry.metadata().ok().filter(|meta| meta.is_file())?;
                Some((meta.modified().ok()?, meta.len(), entry.path()))
            })
            .collect();
        let mut total: u64 = entries.iter().map(|(_, len, _)| len).sum();
        entries.sort();
        for (_, len, path) in entries {
            if total <= self.cap {
                break;
            }
            if path.file_name().is_some_and(|name| name != kept)
                && std::fs::remove_file(&path).is_ok()
            {
                total -= len;
            }
        }
    }
}

/// The IPFS gateway of an account's chain.
pub struct Gateway<'a> {
    http: &'a dyn Http,
    base: String,
    cache: ManifestCache,
}

impl<'a> Gateway<'a> {
    pub fn new(http: &'a dyn Http, base: &str, cache: ManifestCache) -> Self {
        Self {
            http,
            base: base.trim_end_matches('/').to_string(),
            cache,
        }
    }

    /// The content at `cid`, unpacked.
    pub fn bytes(&self, cid: &str) -> Result<Vec<u8>, String> {
        let url = format!("{}/{}", self.base, checked_cid(cid)?);
        let packed = self
            .http
            .get(&url)
            .map_err(|error| format!("Arbesk's storage could not be reached: {}", error.message))?;
        unpack(packed)
    }

    /// The JSON document at `cid`, from the cache when it is there.
    pub fn json(&self, cid: &str) -> Result<Value, String> {
        checked_cid(cid)?;
        if let Some(value) = self
            .cache
            .get(cid)
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        {
            return Ok(value);
        }
        let bytes = self.bytes(cid)?;
        let value = serde_json::from_slice(&bytes)
            .map_err(|_| "Arbesk sent a manifest File Browser can't read.".to_string())?;
        self.cache.put(cid, &bytes);
        Ok(value)
    }
}

/// `work` for each item on at most `limit` threads, answers in order.
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
    use crate::arbesk::http::fake::{FakeHttp, not_found};
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
        assert_eq!(unpack(brotli_framed(&text)).unwrap(), text);
        assert_eq!(unpack(gzipped(&text)).unwrap(), text);
        assert_eq!(unpack(text.clone()).unwrap(), text);
        assert_eq!(unpack(Vec::new()).unwrap(), Vec::<u8>::new());
    }

    #[test]
    fn a_broken_brotli_frame_is_an_error() {
        assert!(unpack(b"ARB\x01not brotli at all".to_vec()).is_err());
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
    fn a_damaged_cache_entry_is_fetched_again() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("bafyM"), b"{ half").unwrap();
        let http = FakeHttp::new(|_| Ok(b"{\"name\":\"Box\"}".to_vec()));
        assert_eq!(
            gateway_over(&http, dir.path()).json("bafyM").unwrap()["name"],
            "Box"
        );
        assert_eq!(http.requests().len(), 1);
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
    fn the_cache_drops_its_oldest_entries_past_its_cap() {
        let dir = tempfile::tempdir().unwrap();
        let cache = ManifestCache::new(dir.path().to_path_buf(), 25);
        for (cid, byte) in [("a1", b'a'), ("b2", b'b'), ("c3", b'c')] {
            cache.put(cid, &[byte; 10]);
            // mtimes must differ for "oldest" to mean something.
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert_eq!(cache.get("a1"), None);
        assert_eq!(cache.get("b2"), Some(vec![b'b'; 10]));
        assert_eq!(cache.get("c3"), Some(vec![b'c'; 10]));
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
