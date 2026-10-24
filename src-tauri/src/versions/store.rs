//! The content-addressed object store inside `.versions/objects/`.
//!
//! Full objects and deltas are plain zstd frames named by the BLAKE3 hash of
//! their *decoded* bytes. Objects are immutable. This module knows nothing
//! about versions, paths or SQLite: `index` says which objects form a chain,
//! `store` writes and rebuilds them.

use std::fs::{self, File};
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::versions::{
    Error, MAX_DELTA_SIZE, MAX_FILE_SIZE, ObjectKind, ObjectMeta, Result, compression_level,
    is_valid_hash, window_log_for,
};

/// Largest window a delta may need (`base + new`, both ≤ 128 MiB). The
/// decoder must allow the same value.
const MAX_WINDOW_LOG: u32 = 28;

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredObject {
    pub hash: String,
    pub size: u64,
    pub stored_size: u64,
}

/// The two directories an object store uses. `tmp` is on the same filesystem.
#[derive(Debug, Clone)]
pub struct Store {
    pub objects: PathBuf,
    pub tmp: PathBuf,
}

impl Store {
    pub fn new(objects: PathBuf, tmp: PathBuf) -> Self {
        Self { objects, tmp }
    }

    /// `objects/<first two hex>/<hash>`; validates the hash before joining (D15).
    pub fn object_path(&self, hash: &str) -> Result<PathBuf> {
        if !is_valid_hash(hash) {
            return Err(Error::IndexDamaged(
                "object hash is not 64 hex characters".into(),
            ));
        }
        Ok(self.objects.join(&hash[..2]).join(hash))
    }

    fn create_temp(&self) -> Result<(PathBuf, File)> {
        fs::create_dir_all(&self.tmp)?;
        for _ in 0..100 {
            let n = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
            let path = self.tmp.join(format!("obj-{}-{n}", std::process::id()));
            match fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(file) => return Ok((path, file)),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(Error::Io(e.to_string())),
            }
        }
        Err(Error::Io("could not create a temporary object".into()))
    }

    /// Stream `reader` through BLAKE3 and zstd into `objects/xx/<hash>`.
    /// Never holds the whole file in memory (D5).
    ///
    /// This always installs the object. Capture should use
    /// `write_full_unless` with a check against the `objects` table: a full
    /// frame and a delta frame of the same bytes share a name (D1), so the
    /// row decides which encoding is authoritative.
    pub fn write_full<R: Read>(&self, reader: R, size: u64) -> Result<StoredObject> {
        self.write_full_unless(reader, size, |_| Ok(true))?
            .ok_or(Error::Damaged)
    }

    /// Like `write_full`, but asks `accept` before installing the object.
    /// Capture passes a check against the `objects` table: if the hash is
    /// already known, the temp frame is discarded and `None` is returned, so
    /// a referenced full or delta object is never replaced.
    pub fn write_full_unless<R: Read, F>(
        &self,
        mut reader: R,
        size: u64,
        accept: F,
    ) -> Result<Option<StoredObject>>
    where
        F: FnOnce(&str) -> Result<bool>,
    {
        if size > MAX_FILE_SIZE {
            return Err(Error::TooLarge);
        }
        fs::create_dir_all(&self.objects)?;
        let (temp_path, file) = self.create_temp()?;
        let _guard = TempGuard {
            path: temp_path.clone(),
        };

        let level = compression_level(size);
        let mut encoder = zstd::stream::write::Encoder::new(BufWriter::new(file), level)
            .map_err(|e| Error::Io(e.to_string()))?;
        if size > 8 * 1024 * 1024 {
            encoder
                .long_distance_matching(true)
                .map_err(|e| Error::Io(e.to_string()))?;
        }
        encoder
            .set_pledged_src_size(Some(size))
            .map_err(|e| Error::Io(e.to_string()))?;

        let mut hasher = blake3::Hasher::new();
        let mut buf = vec![0u8; 64 * 1024];
        let mut total = 0u64;
        loop {
            let n = reader
                .read(&mut buf)
                .map_err(|e| Error::Io(e.to_string()))?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
            encoder
                .write_all(&buf[..n])
                .map_err(|e| Error::Io(e.to_string()))?;
            total += n as u64;
        }
        if total != size {
            return Err(Error::Changed);
        }
        let buffered = encoder.finish().map_err(|e| Error::Io(e.to_string()))?;
        let mut buffered = buffered;
        buffered.flush().map_err(|e| Error::Io(e.to_string()))?;
        let file = buffered
            .into_inner()
            .map_err(|e| Error::Io(e.to_string()))?;
        file.sync_all().map_err(|e| Error::Io(e.to_string()))?;

        let hash = hasher.finalize().to_hex().to_string();
        if !accept(&hash)? {
            return Ok(None);
        }
        self.finish_object(&temp_path, hash, size).map(Some)
    }

    /// Compress `new` with `base` as a zstd reference prefix (D6). Callers
    /// should check `delta_is_worthwhile` before choosing a delta. This
    /// installs unconditionally; capture should use `write_delta_unless`.
    pub fn write_delta(&self, new: &[u8], base: &[u8]) -> Result<StoredObject> {
        self.write_delta_unless(new, base, |_| Ok(true))?
            .ok_or(Error::Damaged)
    }

    /// Like `write_delta`, with the same `accept` check as `write_full_unless`.
    /// The hash is known before compression, so an existing object skips the
    /// compress/write work entirely.
    pub fn write_delta_unless<F>(
        &self,
        new: &[u8],
        base: &[u8],
        accept: F,
    ) -> Result<Option<StoredObject>>
    where
        F: FnOnce(&str) -> Result<bool>,
    {
        if new.len() as u64 > MAX_DELTA_SIZE || base.len() as u64 > MAX_DELTA_SIZE {
            return Err(Error::TooLarge);
        }
        let hash = blake3::hash(new).to_hex().to_string();
        if !accept(&hash)? {
            return Ok(None);
        }

        fs::create_dir_all(&self.objects)?;
        let (temp_path, file) = self.create_temp()?;
        let _guard = TempGuard {
            path: temp_path.clone(),
        };

        let level = compression_level(new.len() as u64);
        let mut encoder =
            zstd::stream::write::Encoder::with_ref_prefix(BufWriter::new(file), level, base)
                .map_err(|e| Error::Io(e.to_string()))?;
        encoder
            .window_log(window_log_for((base.len() + new.len()) as u64))
            .map_err(|e| Error::Io(e.to_string()))?;
        encoder
            .set_pledged_src_size(Some(new.len() as u64))
            .map_err(|e| Error::Io(e.to_string()))?;
        encoder
            .write_all(new)
            .map_err(|e| Error::Io(e.to_string()))?;
        let buffered = encoder.finish().map_err(|e| Error::Io(e.to_string()))?;
        let mut buffered = buffered;
        buffered.flush().map_err(|e| Error::Io(e.to_string()))?;
        let file = buffered
            .into_inner()
            .map_err(|e| Error::Io(e.to_string()))?;
        file.sync_all().map_err(|e| Error::Io(e.to_string()))?;

        self.finish_object(&temp_path, hash, new.len() as u64)
            .map(Some)
    }

    fn finish_object(&self, temp_path: &Path, hash: String, size: u64) -> Result<StoredObject> {
        let final_path = self.object_path(&hash)?;
        // Always install the encoding the caller asked for. Content hashing
        // means a full frame and a delta frame of the same bytes share a name,
        // so the *index* is what decides which encoding is authoritative:
        // callers check `objects` first and reuse an existing row instead of
        // asking the store to rewrite it. Overwriting here is atomic and only
        // happens for a new row or an unreferenced orphan.
        if let Some(parent) = final_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::rename(temp_path, &final_path)?;
        let stored_size = fs::metadata(&final_path)?.len();
        Ok(StoredObject {
            hash,
            size,
            stored_size,
        })
    }

    /// Rebuild `chain` (oldest first: a full object then deltas) and verify the
    /// final BLAKE3. Every stage is capped at its recorded size.
    pub fn read(&self, chain: &[ObjectMeta], expected_hash: &str) -> Result<Vec<u8>> {
        if chain.is_empty() {
            return Err(Error::Damaged);
        }
        let mut previous: Option<Vec<u8>> = None;
        for (i, object) in chain.iter().enumerate() {
            object.validate()?;
            match (i, object.kind) {
                (0, ObjectKind::Full) => {}
                (0, ObjectKind::Delta) => return Err(Error::Damaged),
                (_, ObjectKind::Full) => return Err(Error::Damaged),
                (_, ObjectKind::Delta) => {
                    let prev_meta = &chain[i - 1];
                    if object.base.as_deref() != Some(prev_meta.hash.as_str())
                        || object.depth != prev_meta.depth + 1
                    {
                        return Err(Error::Damaged);
                    }
                }
            }
            let path = self.object_path(&object.hash)?;
            let compressed = match fs::read(&path) {
                Ok(bytes) => bytes,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(Error::Damaged),
                Err(e) => return Err(Error::Io(e.to_string())),
            };
            let decoded = decode_frame(&compressed, previous.as_deref(), object.size)?;
            previous = Some(decoded);
        }
        let bytes = previous.ok_or(Error::Damaged)?;
        if blake3::hash(&bytes).to_hex().as_str() != expected_hash {
            return Err(Error::Damaged);
        }
        Ok(bytes)
    }

    /// Rebuild into a writer. A single full object is streamed, so restoring a
    /// 1 GiB version does not allocate 1 GiB; delta chains still buffer their
    /// bounded stages (D5).
    pub fn read_to<W: Write>(
        &self,
        chain: &[ObjectMeta],
        expected_hash: &str,
        writer: &mut W,
    ) -> Result<u64> {
        if chain.len() == 1 && chain[0].kind == ObjectKind::Full {
            let object = &chain[0];
            object.validate()?;
            let path = self.object_path(&object.hash)?;
            let file = File::open(&path).map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    Error::Damaged
                } else {
                    Error::Io(e.to_string())
                }
            })?;
            let mut decoder = zstd::stream::read::Decoder::with_buffer(BufReader::new(file))
                .map_err(|_| Error::Damaged)?;
            decoder
                .window_log_max(MAX_WINDOW_LOG)
                .map_err(|_| Error::Damaged)?;
            let mut hasher = blake3::Hasher::new();
            let mut buf = [0u8; 64 * 1024];
            let mut total = 0u64;
            loop {
                let n = decoder.read(&mut buf).map_err(|_| Error::Damaged)?;
                if n == 0 {
                    break;
                }
                total += n as u64;
                if total > object.size {
                    return Err(Error::Damaged);
                }
                hasher.update(&buf[..n]);
                writer
                    .write_all(&buf[..n])
                    .map_err(|e| Error::Io(e.to_string()))?;
            }
            if total != object.size || hasher.finalize().to_hex().as_str() != expected_hash {
                return Err(Error::Damaged);
            }
            return Ok(total);
        }
        let bytes = self.read(chain, expected_hash)?;
        writer
            .write_all(&bytes)
            .map_err(|e| Error::Io(e.to_string()))?;
        Ok(bytes.len() as u64)
    }
}

struct TempGuard {
    path: PathBuf,
}

impl Drop for TempGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn decode_frame(compressed: &[u8], prefix: Option<&[u8]>, expected: u64) -> Result<Vec<u8>> {
    let mut decoder = match prefix {
        Some(prefix) => zstd::stream::read::Decoder::with_ref_prefix(compressed, prefix)
            .map_err(|_| Error::Damaged)?,
        None => zstd::stream::read::Decoder::with_buffer(compressed).map_err(|_| Error::Damaged)?,
    };
    decoder
        .window_log_max(MAX_WINDOW_LOG)
        .map_err(|_| Error::Damaged)?;
    let mut out = Vec::with_capacity(expected.min(8u64 << 20) as usize);
    decoder
        .take(expected.saturating_add(1))
        .read_to_end(&mut out)
        .map_err(|_| Error::Damaged)?;
    if out.len() as u64 != expected {
        return Err(Error::Damaged);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::versions::{MAX_DELTA_DEPTH, delta_is_worthwhile};
    use tempfile::tempdir;

    fn store() -> (tempfile::TempDir, Store) {
        let dir = tempdir().unwrap();
        let store = Store::new(dir.path().join("objects"), dir.path().join("tmp"));
        (dir, store)
    }

    fn full(store: &Store, bytes: &[u8]) -> ObjectMeta {
        let stored = store
            .write_full(std::io::Cursor::new(bytes), bytes.len() as u64)
            .unwrap();
        ObjectMeta {
            hash: stored.hash,
            kind: ObjectKind::Full,
            base: None,
            depth: 0,
            size: stored.size,
            stored_size: stored.stored_size,
        }
    }

    fn delta(store: &Store, new: &[u8], base: &ObjectMeta, base_bytes: &[u8]) -> ObjectMeta {
        let stored = store.write_delta(new, base_bytes).unwrap();
        ObjectMeta {
            hash: stored.hash,
            kind: ObjectKind::Delta,
            base: Some(base.hash.clone()),
            depth: base.depth + 1,
            size: stored.size,
            stored_size: stored.stored_size,
        }
    }

    #[test]
    fn full_object_round_trip() {
        let (_dir, store) = store();
        let bytes = b"hello, versioned world\n".repeat(100);
        let object = full(&store, &bytes);
        assert_eq!(object.size, bytes.len() as u64);
        assert!(object.stored_size > 0);
        assert_eq!(
            store
                .read(std::slice::from_ref(&object), &object.hash)
                .unwrap(),
            bytes
        );
        let mut streamed = Vec::new();
        store
            .read_to(std::slice::from_ref(&object), &object.hash, &mut streamed)
            .unwrap();
        assert_eq!(streamed, bytes);
    }

    #[test]
    fn empty_file_round_trip() {
        let (_dir, store) = store();
        let object = full(&store, b"");
        assert_eq!(
            store
                .read(std::slice::from_ref(&object), &object.hash)
                .unwrap(),
            b""
        );
    }

    #[test]
    fn delta_round_trip_and_smaller_than_full() {
        let (dir, store) = store();
        let mut base_bytes = Vec::with_capacity(64 * 1024);
        for i in 0..2048u32 {
            base_bytes.extend_from_slice(blake3::hash(&i.to_le_bytes()).as_bytes());
        }
        let mut new_bytes = base_bytes.clone();
        new_bytes[30_000] ^= 0xff;
        let base = full(&store, &base_bytes);
        let full_copy = store
            .write_full(std::io::Cursor::new(&new_bytes), new_bytes.len() as u64)
            .unwrap();
        let full_size = full_copy.stored_size;
        let delta_obj = delta(&store, &new_bytes, &base, &base_bytes);

        assert!(delta_obj.depth == 1);
        assert!(
            delta_is_worthwhile(delta_obj.stored_size, new_bytes.len() as u64),
            "delta {} vs raw {}",
            delta_obj.stored_size,
            new_bytes.len()
        );
        let rebuilt = store
            .read(
                &[base.clone(), delta_obj.clone()],
                &blake3::hash(&new_bytes).to_hex().to_string(),
            )
            .unwrap();
        assert_eq!(rebuilt, new_bytes);
        let mut streamed = Vec::new();
        store
            .read_to(
                &[base, delta_obj.clone()],
                &blake3::hash(&new_bytes).to_hex().to_string(),
                &mut streamed,
            )
            .unwrap();
        assert_eq!(streamed, new_bytes);
        assert!(
            delta_obj.stored_size < full_size,
            "delta {} vs full {full_size}",
            delta_obj.stored_size
        );
        let _ = dir;
    }

    #[test]
    fn a_nineteen_delta_chain_rebuilds() {
        let (_dir, store) = store();
        let mut content = b"seed\n".repeat(200);
        let full_obj = full(&store, &content);
        let mut chain = vec![full_obj];
        for i in 0..MAX_DELTA_DEPTH {
            let mut next = content.clone();
            next.extend_from_slice(format!("edit {i}\n").as_bytes());
            let base = chain.last().unwrap().clone();
            let delta_obj = delta(&store, &next, &base, &content);
            chain.push(delta_obj);
            content = next;
        }
        assert_eq!(chain.len(), 20, "one full + 19 deltas");
        let expected = blake3::hash(&content).to_hex().to_string();
        assert_eq!(store.read(&chain, &expected).unwrap(), content);
    }

    #[test]
    fn a_tampered_object_is_damaged() {
        let (_dir, store) = store();
        let bytes = b"important bytes".repeat(100);
        let object = full(&store, &bytes);
        let path = store.object_path(&object.hash).unwrap();
        let mut data = fs::read(&path).unwrap();
        let middle = data.len() / 2;
        data[middle] ^= 0xff;
        fs::write(&path, data).unwrap();

        let err = store
            .read(std::slice::from_ref(&object), &object.hash)
            .unwrap_err();
        assert_eq!(err, Error::Damaged);
    }

    #[test]
    fn a_frame_that_expands_past_its_size_is_damaged() {
        let (_dir, store) = store();
        let bytes = b"ten bytes!".repeat(10);
        let mut object = full(&store, &bytes);
        object.size = 1;
        let err = store
            .read(std::slice::from_ref(&object), &object.hash)
            .unwrap_err();
        assert_eq!(err, Error::Damaged);
    }

    #[test]
    fn a_missing_object_is_damaged() {
        let (_dir, store) = store();
        let object = ObjectMeta {
            hash: "a".repeat(64),
            kind: ObjectKind::Full,
            base: None,
            depth: 0,
            size: 3,
            stored_size: 3,
        };
        assert_eq!(
            store
                .read(std::slice::from_ref(&object), &object.hash)
                .unwrap_err(),
            Error::Damaged
        );
    }

    #[test]
    fn a_broken_chain_is_damaged() {
        let (_dir, store) = store();
        let base = full(&store, b"base");
        let other = full(&store, b"other");
        let delta_obj = delta(&store, b"new", &base, b"base");
        // The delta points at base, but the chain gives it another full base.
        let err = store
            .read(&[other, delta_obj], &"0".repeat(64))
            .unwrap_err();
        assert_eq!(err, Error::Damaged);
    }

    #[test]
    fn an_existing_object_is_not_replaced() {
        let (_dir, store) = store();
        let bytes = b"same bytes".repeat(100);
        let object = full(&store, &bytes);
        let path = store.object_path(&object.hash).unwrap();
        let before = fs::read(&path).unwrap();

        // A delta of the same content would use the same object name. The
        // accept check must refuse it, leaving the referenced full frame alone.
        let base_bytes = b"another base".repeat(100);
        let _base = full(&store, &base_bytes);
        let attempt = store
            .write_delta_unless(&bytes, &base_bytes, |hash| Ok(hash != object.hash))
            .unwrap();
        assert!(attempt.is_none());
        assert_eq!(fs::read(&path).unwrap(), before);
        assert_eq!(
            store
                .read(std::slice::from_ref(&object), &object.hash)
                .unwrap(),
            bytes
        );

        // A new hash is installed normally.
        let fresh = b"fresh bytes".repeat(100);
        let installed = store
            .write_full_unless(std::io::Cursor::new(&fresh), fresh.len() as u64, |hash| {
                Ok(hash != object.hash)
            })
            .unwrap();
        assert!(installed.is_some());
    }

    #[test]
    fn identical_content_stores_one_object() {
        let (dir, store) = store();
        let bytes = b"same content".repeat(300);
        let a = store
            .write_full(std::io::Cursor::new(&bytes), bytes.len() as u64)
            .unwrap();
        let b = store
            .write_full(std::io::Cursor::new(&bytes), bytes.len() as u64)
            .unwrap();
        assert_eq!(a.hash, b.hash);
        let files: Vec<_> = walk_objects(&dir.path().join("objects"));
        assert_eq!(files.len(), 1, "{files:?}");
    }

    fn walk_objects(dir: &Path) -> Vec<PathBuf> {
        let mut out = Vec::new();
        let Ok(entries) = fs::read_dir(dir) else {
            return out;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                out.extend(walk_objects(&path));
            } else {
                out.push(path);
            }
        }
        out
    }

    #[test]
    fn a_short_read_is_reported_as_changed() {
        let (_dir, store) = store();
        let err = store
            .write_full(std::io::Cursor::new(b"abc"), 5)
            .unwrap_err();
        assert_eq!(err, Error::Changed);
    }

    #[test]
    fn oversized_files_are_refused_before_reading() {
        let (_dir, store) = store();
        let err = store
            .write_full(std::io::empty(), MAX_FILE_SIZE + 1)
            .unwrap_err();
        assert_eq!(err, Error::TooLarge);
    }

    #[test]
    fn invalid_hashes_never_escape_the_objects_directory() {
        let (_dir, store) = store();
        assert!(store.object_path("../x").is_err());
        assert!(store.object_path(&"g".repeat(64)).is_err());
        assert!(store.object_path(&"A".repeat(64)).is_err());
    }
}
