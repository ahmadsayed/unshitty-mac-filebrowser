//! Turning a model file into what Arbesk stores, as asset-core's
//! `decompose` does (`packages/asset-core/src/formats/gltf/glb-parser.ts`,
//! `gltf/decomposer.ts`, `gltf/dedup.ts`, `3mf/decomposer.ts`): a composite
//! JSON and the parts it points at, each part's `_arbesk` hash taken over
//! its raw bytes, glTF parts brotli-framed and 3MF parts raw. Nothing here
//! talks to the network: part `i` is `ipfs://part:{i}` (a glTF `uri`) or
//! `part:{i}` (a 3MF part's `cid`) until `fill` puts the CIDs in.
//!
//! Where asset-core leaves a file broken this reads it as it was meant: a
//! relative URI (in a `.gltf` or a `.glb`) is read from the model's folder
//! or below, and a GLB's `data:` URIs are read in place (gltf-transform's
//! `binaryToJSON` renames them before asset-core sees them).

use std::collections::{HashMap, HashSet};
use std::io::{Cursor, Read};
use std::path::{Component, Path, PathBuf};
use std::sync::LazyLock;

use data_encoding::{BASE64_NOPAD, Encoding};
use percent_encoding::percent_decode_str;
use serde_json::{Map, Value, json};

use crate::compose::Format;

/// The largest file (a `.gltf` with the files it reads) copied to Arbesk.
pub const MAX_UPLOAD: u64 = 100 * 1024 * 1024;

pub const NOT_A_MODEL: &str =
    "Only .gltf, .glb and .3mf files on this computer can be copied to Arbesk.";
pub const TOO_LARGE: &str = "This file is too large for Arbesk (100 MB at most).";
pub const NOT_3MF: &str = "This isn't a 3MF file.";
pub const NOT_GLTF: &str = "This isn't a glTF file.";

fn outside(uri: &str) -> String {
    format!("“{uri}” is outside the model's folder.")
}

fn missing(uri: &str) -> String {
    format!("“{uri}” can't be found.")
}

const IPFS: &str = "ipfs://";
const HASH_ALGO: &str = "murmur3-128";
const BROTLI_MAGIC: &[u8] = b"ARB\x01";
/// The brotli quality asset-core's `compressAuto` picks for JSON and for
/// anything else.
const JSON_QUALITY: u32 = 11;
const BINARY_QUALITY: u32 = 5;

/// One part to upload: `stored` goes up as is, named `file_name`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Part {
    pub file_name: String,
    pub stored: Vec<u8>,
    pub raw_len: u64,
}

/// A model file split as Arbesk stores it; the composite is uploaded last
/// (`stored_json(&fill(..), composite_compressed)`) as `composite_name`.
/// As the Studio writes it: a glTF composite brotli-framed (its decomposer
/// asks for compression), a 3MF composite raw (its decomposer doesn't).
#[derive(Debug, Clone)]
pub struct Decomposed {
    pub format: Format,
    pub composite: Value,
    pub parts: Vec<Part>,
    pub composite_name: String,
    pub composite_compressed: bool,
}

// ---- Dispatch -------------------------------------------------------------

#[derive(Clone, Copy)]
enum Kind {
    Gltf,
    Glb,
    ThreeMf,
}

fn kind_of(file: &Path) -> Option<Kind> {
    let ext = file.extension()?.to_str()?.to_ascii_lowercase();
    match ext.as_str() {
        "gltf" => Some(Kind::Gltf),
        "glb" => Some(Kind::Glb),
        "3mf" => Some(Kind::ThreeMf),
        _ => None,
    }
}

/// A local `.gltf`, `.glb` or `.3mf` split as asset-core's `decompose`
/// (`formats/index.ts`) splits it, its files named after `asset_name`.
/// Refused before anything is read: another extension, not a regular
/// file, more than `MAX_UPLOAD` bytes.
pub fn decompose(file: &Path, asset_name: &str) -> Result<Decomposed, String> {
    let kind = kind_of(file).ok_or(NOT_A_MODEL)?;
    let meta = std::fs::metadata(file).map_err(|_| NOT_A_MODEL)?;
    if !meta.is_file() {
        return Err(NOT_A_MODEL.into());
    }
    if meta.len() > MAX_UPLOAD {
        return Err(TOO_LARGE.into());
    }
    let bytes = std::fs::read(file).map_err(|e| e.to_string())?;
    let dir = file.parent().filter(|p| !p.as_os_str().is_empty());
    let mut folder = Folder {
        dir: dir.unwrap_or(Path::new(".")).to_path_buf(),
        budget: MAX_UPLOAD - meta.len(),
    };
    let mut uploads = Uploads::default();
    let base = sanitize_file_name(asset_name);
    let (format, composite, composite_name) = match kind {
        Kind::Gltf => {
            let gltf = parse_json_text(&bytes)?;
            let composite = decompose_gltf(gltf, &mut folder, &base, &mut uploads)?;
            (Format::Gltf, composite, format!("{base}_composite.gltf.br"))
        }
        Kind::Glb => {
            let composite = decompose_glb(&bytes, &mut folder, &base, &mut uploads)?;
            (Format::Gltf, composite, format!("{base}_composite.gltf.br"))
        }
        Kind::ThreeMf => {
            let composite = decompose_3mf(&bytes, asset_name, &mut uploads)?;
            let base = sanitize_file_name(or(asset_name, "composite"));
            (
                Format::ThreeMf,
                composite,
                format!("{base}_composite.3mf.json"),
            )
        }
    };
    Ok(Decomposed {
        format,
        composite,
        parts: uploads.parts,
        composite_name,
        composite_compressed: format == Format::Gltf,
    })
}

/// `name`, or `fallback` when it is empty (JavaScript's `name || fallback`).
fn or<'a>(name: &'a str, fallback: &'a str) -> &'a str {
    if name.is_empty() { fallback } else { name }
}

/// asset-core's `sanitizeFileName` (`utils/uri.ts`): lower case, every run
/// of other characters than `a-z0-9_-` one `_`, at most 40 characters,
/// `asset` when that leaves nothing.
pub(crate) fn sanitize_file_name(name: &str) -> String {
    let mut out = String::new();
    let mut in_run = false;
    for c in or(name, "asset").to_lowercase().chars() {
        let kept = c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-';
        if kept || !in_run {
            out.push(if kept { c } else { '_' });
        }
        in_run = !kept;
    }
    out.truncate(40);
    if out.is_empty() { "asset".into() } else { out }
}

// ---- Placeholders ----------------------------------------------------------

/// The composite with each part's CID where its placeholder was: glTF
/// buffers' and images' `ipfs://part:{i}`, 3MF parts' `part:{i}`; which of
/// the two is the file's `format`, never a key its own content may carry.
pub fn fill(composite: &Value, format: Format, cids: &[String]) -> Value {
    let mut out = composite.clone();
    if format == Format::ThreeMf {
        if let Some(parts) = out.get_mut("parts").and_then(Value::as_object_mut) {
            for part in parts.values_mut() {
                put_cid(part, "cid", "part:", "", cids);
            }
        }
        return out;
    }
    for key in ["buffers", "images"] {
        for item in array_mut(&mut out, key) {
            put_cid(item, "uri", "ipfs://part:", IPFS, cids);
        }
    }
    out
}

/// Replace `item[key]` when it is `{prefix}{i}` with `{to}{cids[i]}`.
fn put_cid(item: &mut Value, key: &str, prefix: &str, to: &str, cids: &[String]) {
    let Some(slot) = item.get_mut(key) else {
        return;
    };
    let index = slot
        .as_str()
        .and_then(|s| s.strip_prefix(prefix)?.parse::<usize>().ok());
    if let Some(cid) = index.and_then(|i| cids.get(i)) {
        *slot = Value::String(format!("{to}{cid}"));
    }
}

fn array_mut<'a>(value: &'a mut Value, key: &str) -> impl Iterator<Item = &'a mut Value> {
    value
        .get_mut(key)
        .and_then(Value::as_array_mut)
        .into_iter()
        .flatten()
}

fn array<'a>(value: &'a Value, key: &str) -> impl Iterator<Item = &'a Value> {
    value
        .get(key)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
}

// ---- Hash and compression --------------------------------------------------

const C: [u32; 4] = [0x239b961b, 0xab0e9789, 0x38b34ae5, 0xa1e38b93];
const LANE_ROTATE: [u32; 4] = [15, 16, 17, 18];
const H_ROTATE: [u32; 4] = [19, 17, 15, 13];
const H_ADD: [u32; 4] = [0x561ccd1b, 0x0bcaa747, 0x96cd1c35, 0x32ac3b17];

/// One 32-bit lane's key mix of MurmurHash3 x86_128.
fn mix_k(k: u32, lane: usize) -> u32 {
    k.wrapping_mul(C[lane])
        .rotate_left(LANE_ROTATE[lane])
        .wrapping_mul(C[(lane + 1) % 4])
}

fn fmix32(mut h: u32) -> u32 {
    h ^= h >> 16;
    h = h.wrapping_mul(0x85ebca6b);
    h ^= h >> 13;
    h = h.wrapping_mul(0xc2b2ae35);
    h ^ (h >> 16)
}

/// Add the other lanes into the first, then the first into the others.
fn cross_add(h: &mut [u32; 4]) {
    h[0] = h[0]
        .wrapping_add(h[1])
        .wrapping_add(h[2])
        .wrapping_add(h[3]);
    for i in 1..4 {
        h[i] = h[i].wrapping_add(h[0]);
    }
}

/// MurmurHash3 x86 128-bit with seed 0, as 32 hex digits: asset-core's
/// `murmur3_128` (`utils/hash.ts`), which `hashBytes` uses for `_arbesk`.
pub fn murmur3_128_hex(bytes: &[u8]) -> String {
    let mut h = [0u32; 4];
    let (blocks, tail) = bytes.as_chunks::<16>();
    for block in blocks {
        for lane in 0..4 {
            let k = u32::from_le_bytes(block[lane * 4..lane * 4 + 4].try_into().unwrap());
            h[lane] = (h[lane] ^ mix_k(k, lane))
                .rotate_left(H_ROTATE[lane])
                .wrapping_add(h[(lane + 1) % 4])
                .wrapping_mul(5)
                .wrapping_add(H_ADD[lane]);
        }
    }
    for (lane, chunk) in tail.chunks(4).enumerate() {
        let k = chunk
            .iter()
            .enumerate()
            .fold(0u32, |k, (i, b)| k | (u32::from(*b) << (8 * i)));
        h[lane] ^= mix_k(k, lane);
    }
    for lane in &mut h {
        *lane ^= bytes.len() as u32;
    }
    cross_add(&mut h);
    h = h.map(fmix32);
    cross_add(&mut h);
    h.iter().map(|lane| format!("{lane:08x}")).collect()
}

/// Brotli behind asset-core's `ARB\x01` frame (`utils/brotli-frame.ts`),
/// as `compressAuto` writes it (brotli-wasm: the `brotli` crate, window 22).
pub fn brotli_framed(bytes: &[u8], quality: u32) -> Vec<u8> {
    let params = brotli::enc::BrotliEncoderParams {
        quality: quality as i32,
        lgwin: 22,
        ..Default::default()
    };
    let mut out = BROTLI_MAGIC.to_vec();
    brotli::BrotliCompress(&mut &bytes[..], &mut out, &params).expect("brotli into memory");
    out
}

/// A JSON document as the Studio's `writeJSONToIPFS` stores it: compact,
/// brotli-framed at quality 11 when asked to compress, else raw.
pub fn stored_json(value: &Value, compress: bool) -> Vec<u8> {
    let text = serde_json::to_vec(value).expect("JSON of a Value");
    match compress {
        true => brotli_framed(&text, JSON_QUALITY),
        false => text,
    }
}

// ---- Parts -----------------------------------------------------------------

/// The parts one file splits into, identical raw bytes once: asset-core's
/// `uploadWithDedup` (`gltf/dedup.ts`), whose in-flight map makes equal
/// payloads share one upload.
#[derive(Default)]
struct Uploads {
    parts: Vec<Part>,
    seen: HashMap<(String, bool), usize>,
}

impl Uploads {
    /// `uploadWithDedup`: the part holding `bytes` (named `file_name`, plus
    /// `.br` when compressed) and its `_arbesk` metadata.
    fn add(&mut self, bytes: &[u8], file_name: &str, compress: bool) -> (usize, Value) {
        let hash = murmur3_128_hex(bytes);
        let meta = json!({
            "hash": hash,
            "hashAlgo": HASH_ALGO,
            "compressed": compress,
            "bytes": bytes.len(),
        });
        let parts = &mut self.parts;
        let index = *self.seen.entry((hash, compress)).or_insert_with(|| {
            let (stored, file_name) = if compress {
                (
                    brotli_framed(bytes, BINARY_QUALITY),
                    format!("{file_name}.br"),
                )
            } else {
                (bytes.to_vec(), file_name.to_string())
            };
            parts.push(Part {
                file_name,
                stored,
                raw_len: bytes.len() as u64,
            });
            parts.len() - 1
        });
        (index, meta)
    }

    /// A glTF buffer or image stored as part of the composite:
    /// `{...item, uri: ipfsUriFromCid(cid)}` plus `attachDedupMeta`.
    fn store(&mut self, item: &Value, bytes: &[u8], file_name: &str) -> Map<String, Value> {
        let (index, meta) = self.add(bytes, file_name, true);
        let mut item = item.as_object().cloned().unwrap_or_default();
        item.insert("uri".into(), json!(format!("ipfs://part:{index}")));
        item.insert("_arbesk".into(), meta);
        item
    }
}

// ---- Resources -------------------------------------------------------------

/// A buffer's or image's bytes, and the MIME type its `data:` URI names.
struct Extracted {
    bytes: Vec<u8>,
    mime: Option<String>,
}

/// `atob` as browsers decode: ASCII whitespace ignored, padding optional,
/// stray bits at the end dropped.
static FORGIVING_BASE64: LazyLock<Encoding> = LazyLock::new(|| {
    let mut spec = BASE64_NOPAD.specification();
    spec.check_trailing_bits = false;
    spec.encoding().expect("base64 specification")
});

fn atob(text: &str) -> Result<Vec<u8>, String> {
    let mut text: String = text
        .chars()
        .filter(|c| !matches!(c, '\t' | '\n' | '\x0c' | '\r' | ' '))
        .collect();
    if text.len().is_multiple_of(4) {
        for _ in 0..2 {
            if text.ends_with('=') {
                text.pop();
            }
        }
    }
    FORGIVING_BASE64
        .decode(text.as_bytes())
        .map_err(|_| NOT_GLTF.to_string())
}

/// asset-core's `extractDataURI` (`utils/uri.ts`): the bytes and MIME type
/// of a `data:` URI, None for another URI or one without a comma.
fn data_uri(uri: &str) -> Result<Option<Extracted>, String> {
    let Some((header, payload)) = uri.split_once(',').filter(|_| uri.starts_with("data:")) else {
        return Ok(None);
    };
    let mime = header["data:".len()..]
        .split(';')
        .next()
        .filter(|m| !m.is_empty());
    let bytes = if header.contains(";base64") {
        atob(payload)?
    } else {
        payload.as_bytes().to_vec()
    };
    Ok(Some(Extracted {
        bytes,
        mime: Some(mime.unwrap_or("application/octet-stream").to_string()),
    }))
}

/// The folder a model's relative URIs are read from, and how many bytes
/// those files may still add before the model is too large for Arbesk.
struct Folder {
    dir: PathBuf,
    budget: u64,
}

impl Folder {
    /// The bytes behind a relative URI, from the model's folder or below.
    fn read(&mut self, uri: &str) -> Result<Vec<u8>, String> {
        let path = self.resolve(uri)?;
        let len = std::fs::metadata(&path).map_err(|_| missing(uri))?.len();
        self.budget = self.budget.checked_sub(len).ok_or(TOO_LARGE)?;
        std::fs::read(&path).map_err(|_| missing(uri))
    }

    fn resolve(&self, uri: &str) -> Result<PathBuf, String> {
        let relative = relative_path(uri).ok_or_else(|| outside(uri))?;
        let dir = self.dir.canonicalize().map_err(|_| missing(uri))?;
        let path = dir
            .join(relative)
            .canonicalize()
            .map_err(|_| missing(uri))?;
        if !path.starts_with(&dir) {
            return Err(outside(uri));
        }
        if !path.is_file() {
            return Err(missing(uri));
        }
        Ok(path)
    }
}

/// A URI as a path below the model's folder (percent decoded), or None
/// for one with a scheme, an absolute one or one climbing out with `..`.
fn relative_path(uri: &str) -> Option<PathBuf> {
    let scheme = uri.split_once(':').is_some_and(|(scheme, _)| {
        scheme.starts_with(|c: char| c.is_ascii_alphabetic())
            && scheme
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "+.-".contains(c))
    });
    if scheme {
        return None;
    }
    let path = PathBuf::from(percent_decode_str(uri).decode_utf8_lossy().as_ref());
    path.components()
        .all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
        .then_some(path)
}

/// A non-empty string field: what JavaScript's `item.key &&` lets through.
fn text<'a>(item: &'a Value, key: &str) -> Option<&'a str> {
    item.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
}

/// What a buffer's or image's `uri` holds: None without one or for an
/// `ipfs://` one, a `data:` URI's bytes (None if asset-core can't read
/// it), else the file it names beside the model.
fn uri_bytes(item: &Value, folder: &mut Folder) -> Result<Option<Extracted>, String> {
    match text(item, "uri") {
        None => Ok(None),
        Some(uri) if uri.starts_with(IPFS) => Ok(None),
        Some(uri) if uri.starts_with("data:") => data_uri(uri),
        Some(uri) => Ok(Some(Extracted {
            bytes: folder.read(uri)?,
            mime: None,
        })),
    }
}

/// asset-core's `detectImageMimeType` (`gltf/image-mime.ts`).
fn detect_image_mime_type(bytes: &[u8]) -> Option<String> {
    const ANY: i16 = -1;
    const SIGNATURES: [(&str, &[i16]); 5] = [
        ("image/png", &[0x89, 0x50, 0x4e, 0x47]),
        ("image/jpeg", &[0xff, 0xd8, 0xff]),
        (
            "image/webp",
            &[
                0x52, 0x49, 0x46, 0x46, ANY, ANY, ANY, ANY, 0x57, 0x45, 0x42, 0x50,
            ],
        ),
        (
            "image/ktx2",
            &[
                0xab, 0x4b, 0x54, 0x58, 0x20, 0x31, 0x31, 0xbb, 0x0d, 0x0a, 0x1a, 0x0a,
            ],
        ),
        ("image/gif", &[0x47, 0x49, 0x46]),
    ];
    if bytes.len() < 4 {
        return None;
    }
    let matches = |magic: &[i16]| {
        bytes.len() >= magic.len()
            && magic
                .iter()
                .zip(bytes)
                .all(|(m, b)| *m == ANY || *m == i16::from(*b))
    };
    SIGNATURES
        .iter()
        .find(|(_, magic)| matches(magic))
        .map(|(mime, _)| mime.to_string())
}

/// asset-core's `extFromMimeType` (`gltf/image-mime.ts`).
fn ext_from_mime_type(mime: Option<&str>) -> String {
    let Some(mime) = mime.filter(|m| !m.is_empty()) else {
        return "bin".into();
    };
    let known = [
        ("image/png", "png"),
        ("image/jpeg", "jpg"),
        ("image/webp", "webp"),
        ("image/ktx2", "ktx2"),
        ("image/gif", "gif"),
        ("application/octet-stream", "bin"),
    ];
    match known.iter().find(|(m, _)| *m == mime) {
        Some((_, ext)) => ext.to_string(),
        None => or(mime.rsplit('/').next().unwrap_or(""), "bin").to_string(),
    }
}

/// The JSON of a `.gltf` or of a GLB's JSON chunk, read as `TextDecoder`
/// and `JSON.parse` read it: a leading BOM dropped, bad UTF-8 replaced.
fn parse_json_text(bytes: &[u8]) -> Result<Value, String> {
    let bytes = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes);
    serde_json::from_str::<Value>(&String::from_utf8_lossy(bytes))
        .ok()
        .filter(Value::is_object)
        .ok_or_else(|| NOT_GLTF.to_string())
}

// ---- glTF ------------------------------------------------------------------

/// asset-core's `isComposite` (`gltf/gltf-core.ts`): a buffer or image
/// already on IPFS.
fn is_composite(gltf: &Value) -> bool {
    ["buffers", "images"]
        .iter()
        .flat_map(|key| array(gltf, key))
        .any(|item| text(item, "uri").is_some_and(|uri| uri.starts_with(IPFS)))
}

/// asset-core's glTF `decompose` (`gltf/decomposer.ts`) through
/// `decomposeGltfJson` (`gltf/gltf-core.ts`): every buffer then every
/// image a URI holds becomes a part; a composite is kept as it is.
fn decompose_gltf(
    gltf: Value,
    folder: &mut Folder,
    base: &str,
    uploads: &mut Uploads,
) -> Result<Value, String> {
    if is_composite(&gltf) {
        return Ok(gltf);
    }
    let mut composite = gltf;
    for (i, buffer) in array_mut(&mut composite, "buffers").enumerate() {
        if let Some(found) = uri_bytes(buffer, folder)? {
            let name = format!("{base}_buffer_{i}.bin");
            *buffer = Value::Object(uploads.store(buffer, &found.bytes, &name));
        }
    }
    for (i, image) in array_mut(&mut composite, "images").enumerate() {
        if let Some(found) = uri_bytes(image, folder)? {
            let mime = found
                .mime
                .or_else(|| text(image, "mimeType").map(String::from))
                .or_else(|| detect_image_mime_type(&found.bytes));
            let ext = mime.as_deref().and_then(|m| m.split('/').nth(1));
            let name = format!("{base}_texture_{i}.{}", or(ext.unwrap_or(""), "bin"));
            *image = Value::Object(uploads.store(image, &found.bytes, &name));
        }
    }
    Ok(composite)
}

// ---- GLB -------------------------------------------------------------------

const GLB_MAGIC: u32 = 0x46546c67;
const CHUNK_JSON: u32 = 0x4e4f534a;
const CHUNK_BIN: u32 = 0x004e4942;

/// asset-core's `parseGLB` through gltf-transform's `binaryToJSON`: the
/// JSON chunk, and the BIN chunk when the next chunk is one.
fn parse_glb(bytes: &[u8]) -> Result<(Value, Option<Vec<u8>>), String> {
    let word = |at: usize| {
        let word = bytes.get(at..at.checked_add(4)?)?;
        Some(u32::from_le_bytes(word.try_into().unwrap()) as usize)
    };
    let not_gltf = || NOT_GLTF.to_string();
    if word(0) != Some(GLB_MAGIC as usize)
        || word(4) != Some(2)
        || word(16) != Some(CHUNK_JSON as usize)
    {
        return Err(not_gltf());
    }
    let json_end = 20 + word(12).ok_or_else(not_gltf)?;
    let json = parse_json_text(bytes.get(20..json_end).ok_or_else(not_gltf)?)?;
    check_images(&json)?;
    if bytes.len() <= json_end {
        return Ok((json, None));
    }
    // gltf-transform reads the chunk header as a Uint32Array: aligned or nothing.
    let bin_len = word(json_end)
        .filter(|_| json_end.is_multiple_of(4))
        .ok_or_else(not_gltf)?;
    if word(json_end + 4) != Some(CHUNK_BIN as usize) {
        return Ok((json, None));
    }
    let bin = bytes
        .get(json_end + 8..json_end + 8 + bin_len)
        .ok_or_else(not_gltf)?;
    Ok((json, Some(bin.to_vec())))
}

/// gltf-transform's `_readResourcesInternal` refuses an image with neither
/// a `uri` nor a `bufferView`.
fn check_images(json: &Value) -> Result<(), String> {
    let bare = |image: &Value| image.get("uri").is_none() && image.get("bufferView").is_none();
    match array(json, "images").any(bare) {
        true => Err(NOT_GLTF.into()),
        false => Ok(()),
    }
}

/// A JSON number as an index or byte count; JavaScript's `|| 0` for none.
fn int(value: Option<&Value>) -> Option<u64> {
    let value = value?;
    value.as_u64().or_else(|| {
        value
            .as_f64()
            .filter(|f| f.fract() == 0.0 && *f >= 0.0)
            .map(|f| f as u64)
    })
}

fn int_or_zero(item: &Value, key: &str) -> u64 {
    int(item.get(key)).unwrap_or(0)
}

/// asset-core's GLB `decompose` (`gltf/glb-parser.ts`): images become
/// parts and leave the BIN buffer, then each buffer becomes a part.
fn decompose_glb(
    bytes: &[u8],
    folder: &mut Folder,
    base: &str,
    uploads: &mut Uploads,
) -> Result<Value, String> {
    let (mut composite, bin) = parse_glb(bytes)?;
    let mut buffer_bytes = resolve_buffers(&composite, bin.as_deref(), folder)?;
    let tasks = collect_image_tasks(&composite, &buffer_bytes, folder)?;
    let removals = upload_images(&mut composite, tasks, base, uploads);
    if !removals.is_empty() {
        prune_buffer_image_data(&mut composite, &mut buffer_bytes, &removals);
    }
    upload_buffers(&mut composite, &buffer_bytes, base, uploads);
    Ok(composite)
}

/// Each buffer's bytes as `decompose` resolves them before any upload
/// (asset-core's `resolveBufferBytes`): the BIN chunk for a buffer without
/// a URI, a `data:` URI's bytes, a file beside the model; None for IPFS.
fn resolve_buffers(
    composite: &Value,
    bin: Option<&[u8]>,
    folder: &mut Folder,
) -> Result<Vec<Option<Vec<u8>>>, String> {
    array(composite, "buffers")
        .map(|buffer| match text(buffer, "uri") {
            None => bin
                .map(|bin| Some(bin.to_vec()))
                .ok_or(NOT_GLTF.to_string()),
            Some(uri) if uri.starts_with("data:") => match data_uri(uri)? {
                Some(found) => Ok(Some(found.bytes)),
                None => Err(NOT_GLTF.to_string()),
            },
            Some(_) => Ok(uri_bytes(buffer, folder)?.map(|found| found.bytes)),
        })
        .collect()
}

/// A bufferView's bytes, cut out of its buffer once its image is a part.
#[derive(Clone, Copy)]
struct Range {
    start: u64,
    end: u64,
}

#[derive(Clone, Copy)]
struct Removal {
    buffer: u64,
    old_view: u64,
    range: Range,
}

/// An image to make a part of (asset-core's `ImageUploadTask`).
struct ImageTask {
    index: usize,
    bytes: Vec<u8>,
    mime: Option<String>,
    removal: Option<Removal>,
}

/// asset-core's `collectImageUploadTasks`: each image's bytes and MIME
/// type, and the range of the buffer its bufferView held.
fn collect_image_tasks(
    composite: &Value,
    buffer_bytes: &[Option<Vec<u8>>],
    folder: &mut Folder,
) -> Result<Vec<ImageTask>, String> {
    let mut tasks = Vec::new();
    for (index, image) in array(composite, "images").enumerate() {
        let Some(Extracted { bytes, mime }) =
            resolve_glb_image_bytes(composite, buffer_bytes, image, folder)?
        else {
            continue;
        };
        let view = int(image.get("bufferView"));
        let removal = view.and_then(|view| {
            let view_def = composite.get("bufferViews")?.get(view as usize)?;
            let start = int_or_zero(view_def, "byteOffset");
            Some(Removal {
                buffer: int_or_zero(view_def, "buffer"),
                old_view: view,
                range: Range {
                    start,
                    end: start.saturating_add(int_or_zero(view_def, "byteLength")),
                },
            })
        });
        tasks.push(ImageTask {
            index,
            bytes,
            mime,
            removal,
        });
    }
    Ok(tasks)
}

/// asset-core's `resolveGlbImageBytes` (`gltf/glb-image-resolve.ts`): an
/// image's bytes from its URI or its bufferView, its MIME type from the
/// entry, the `data:` URI or the bytes; None when there are none.
fn resolve_glb_image_bytes(
    composite: &Value,
    buffer_bytes: &[Option<Vec<u8>>],
    image: &Value,
    folder: &mut Folder,
) -> Result<Option<Extracted>, String> {
    let declared = text(image, "mimeType").map(String::from);
    let found = match uri_bytes(image, folder)? {
        Some(found) => {
            let mime = declared
                .or(found.mime)
                .or_else(|| detect_image_mime_type(&found.bytes));
            Some(Extracted {
                bytes: found.bytes,
                mime,
            })
        }
        None if text(image, "uri").is_none() => {
            from_buffer_view(composite, buffer_bytes, image, declared)
        }
        None => None,
    };
    Ok(found.filter(|found| !found.bytes.is_empty()))
}

/// asset-core's `resolveFromBufferView`: the slice of the buffer the
/// image's bufferView names, its MIME type sniffed when the entry has none.
fn from_buffer_view(
    composite: &Value,
    buffer_bytes: &[Option<Vec<u8>>],
    image: &Value,
    declared: Option<String>,
) -> Option<Extracted> {
    let view = composite
        .get("bufferViews")?
        .get(int(image.get("bufferView"))? as usize)?;
    let source = buffer_bytes
        .get(int(view.get("buffer"))? as usize)?
        .as_ref()?;
    let start = (int_or_zero(view, "byteOffset") as usize).min(source.len());
    let end = start
        .saturating_add(int_or_zero(view, "byteLength") as usize)
        .min(source.len());
    let bytes = source[start..end].to_vec();
    let mime = declared.or_else(|| detect_image_mime_type(&bytes));
    Some(Extracted { bytes, mime })
}

/// asset-core's `uploadImageTask` for each image: the entry points at its
/// part, loses its bufferView and gains the MIME type it lacked. Returns
/// the buffer ranges to prune, grouped by buffer in the order first seen.
fn upload_images(
    composite: &mut Value,
    tasks: Vec<ImageTask>,
    base: &str,
    uploads: &mut Uploads,
) -> Vec<(u64, Vec<Removal>)> {
    let mut by_buffer: Vec<(u64, Vec<Removal>)> = Vec::new();
    for task in tasks {
        let ext = ext_from_mime_type(task.mime.as_deref());
        let name = format!("{base}_texture_{}.{ext}", task.index);
        let Some(image) = composite
            .get_mut("images")
            .and_then(|images| images.get_mut(task.index))
        else {
            continue;
        };
        let mut entry = uploads.store(image, &task.bytes, &name);
        entry.remove("bufferView");
        let has_mime = entry.get("mimeType").is_some_and(truthy);
        if let (Some(mime), false) = (task.mime, has_mime) {
            entry.insert("mimeType".into(), Value::String(mime));
        }
        *image = Value::Object(entry);
        if let Some(removal) = task.removal {
            match by_buffer
                .iter_mut()
                .find(|(buffer, _)| *buffer == removal.buffer)
            {
                Some((_, list)) => list.push(removal),
                None => by_buffer.push((removal.buffer, vec![removal])),
            }
        }
    }
    by_buffer
}

/// JavaScript truthiness of a JSON value.
fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64() != Some(0.0),
        Value::String(s) => !s.is_empty(),
        _ => true,
    }
}

/// asset-core's `collectBufferUploadTasks` and `uploadBufferTask`: every
/// buffer with bytes becomes a part its entry points at.
fn upload_buffers(
    composite: &mut Value,
    buffer_bytes: &[Option<Vec<u8>>],
    base: &str,
    uploads: &mut Uploads,
) {
    for (i, buffer) in array_mut(composite, "buffers").enumerate() {
        if let Some(Some(bytes)) = buffer_bytes.get(i) {
            let name = format!("{base}_buffer_{i}.bin");
            *buffer = Value::Object(uploads.store(buffer, bytes, &name));
        }
    }
}

// ---- GLB: cutting images out of buffers -------------------------------------

/// asset-core's `forEachBufferViewRef`: every bufferView reference of the
/// accessors (with their sparse parts) and of Draco-compressed primitives.
/// Images' references are not among them.
fn for_each_buffer_view_ref(composite: &mut Value, f: &mut dyn FnMut(&mut Value)) {
    for accessor in array_mut(composite, "accessors") {
        for holder in ["", "/sparse/indices", "/sparse/values"] {
            if let Some(slot) = accessor
                .pointer_mut(holder)
                .and_then(|h| h.get_mut("bufferView"))
            {
                f(slot);
            }
        }
    }
    for mesh in array_mut(composite, "meshes") {
        for primitive in array_mut(mesh, "primitives") {
            if let Some(slot) =
                primitive.pointer_mut("/extensions/KHR_draco_mesh_compression/bufferView")
            {
                f(slot);
            }
        }
    }
}

/// asset-core's `collectReferencedBufferViews`.
fn collect_referenced_buffer_views(composite: &mut Value) -> HashSet<u64> {
    let mut referenced = HashSet::new();
    for_each_buffer_view_ref(composite, &mut |slot| {
        referenced.extend(int(Some(slot)));
    });
    referenced
}

/// asset-core's `pruneBufferImageData`: the bufferViews of images that
/// became parts go (unless geometry uses them too), the others are
/// renumbered and moved down, and their bytes leave the buffer.
fn prune_buffer_image_data(
    composite: &mut Value,
    buffer_bytes: &mut [Option<Vec<u8>>],
    removals: &[(u64, Vec<Removal>)],
) {
    let referenced = collect_referenced_buffer_views(composite);
    let to_remove: HashSet<u64> = removals
        .iter()
        .flat_map(|(_, list)| list.iter().map(|r| r.old_view))
        .filter(|view| !referenced.contains(view))
        .collect();
    if to_remove.is_empty() {
        return;
    }
    compact_buffer_views(composite, &to_remove);
    for (buffer, list) in removals {
        let relevant: Vec<Range> = list
            .iter()
            .filter(|r| to_remove.contains(&r.old_view))
            .map(|r| r.range)
            .collect();
        prune_buffer(composite, buffer_bytes, *buffer, &relevant);
    }
}

/// Drop the removed bufferViews and renumber every reference to the rest
/// (asset-core's compaction and `renumberBufferViewRefs`).
fn compact_buffer_views(composite: &mut Value, to_remove: &HashSet<u64>) {
    let old = match composite.get_mut("bufferViews").map(Value::take) {
        Some(Value::Array(views)) => views,
        _ => Vec::new(),
    };
    let mut mapping = HashMap::new();
    let mut kept = Vec::new();
    for (i, view) in old.into_iter().enumerate() {
        if !to_remove.contains(&(i as u64)) {
            mapping.insert(i as u64, kept.len() as u64);
            kept.push(view);
        }
    }
    composite["bufferViews"] = Value::Array(kept);
    for_each_buffer_view_ref(composite, &mut |slot| {
        if let Some(new) = int(Some(slot)).and_then(|old| mapping.get(&old)) {
            *slot = json!(new);
        }
    });
}

/// One buffer's part of `pruneBufferImageData`: cut the ranges out unless
/// a remaining view overlaps them.
fn prune_buffer(
    composite: &mut Value,
    buffer_bytes: &mut [Option<Vec<u8>>],
    buffer: u64,
    relevant: &[Range],
) {
    let Some(Some(bytes)) = buffer_bytes.get(buffer as usize) else {
        return;
    };
    if relevant.is_empty() {
        return;
    }
    let merged = merge_ranges(relevant);
    if any_remaining_view_overlaps(composite, buffer, &merged) {
        return;
    }
    let pruned = splice_out_ranges(bytes, &merged);
    adjust_buffer_view_offsets(composite, buffer, &merged);
    let entry = composite
        .get_mut("buffers")
        .and_then(|buffers| buffers.get_mut(buffer as usize))
        .and_then(Value::as_object_mut);
    if let Some(entry) = entry {
        entry.insert("byteLength".into(), json!(pruned.len()));
    }
    buffer_bytes[buffer as usize] = Some(pruned);
}

/// asset-core's `mergeRanges`: sorted by start, overlapping or adjacent
/// ones merged.
fn merge_ranges(ranges: &[Range]) -> Vec<Range> {
    let mut sorted = ranges.to_vec();
    sorted.sort_by_key(|r| r.start);
    let mut merged: Vec<Range> = Vec::new();
    for r in sorted {
        match merged.last_mut() {
            Some(last) if r.start <= last.end => last.end = last.end.max(r.end),
            _ => merged.push(r),
        }
    }
    merged
}

/// The views of `buffer` with their byte ranges.
fn views_of(composite: &Value, buffer: u64) -> impl Iterator<Item = &Value> {
    array(composite, "bufferViews").filter(move |view| int(view.get("buffer")) == Some(buffer))
}

/// asset-core's `anyRemainingViewOverlaps`.
fn any_remaining_view_overlaps(composite: &Value, buffer: u64, ranges: &[Range]) -> bool {
    views_of(composite, buffer).any(|view| {
        let start = int_or_zero(view, "byteOffset");
        let end = start.saturating_add(int_or_zero(view, "byteLength"));
        ranges.iter().any(|r| !(end <= r.start || start >= r.end))
    })
}

/// asset-core's `spliceOutRanges` (with `subarray`'s clamping).
fn splice_out_ranges(bytes: &[u8], ranges: &[Range]) -> Vec<u8> {
    let clamp = |at: u64| (at as usize).min(bytes.len());
    let mut out = Vec::new();
    let mut last = 0;
    for r in ranges {
        if r.start > last {
            out.extend_from_slice(&bytes[clamp(last)..clamp(r.start)]);
        }
        last = r.end;
    }
    out.extend_from_slice(&bytes[clamp(last)..]);
    out
}

/// asset-core's `adjustBufferViewOffsets`: every view of `buffer` gets a
/// `byteOffset`, less the bytes cut out before it.
fn adjust_buffer_view_offsets(composite: &mut Value, buffer: u64, ranges: &[Range]) {
    for view in array_mut(composite, "bufferViews") {
        if int(view.get("buffer")) != Some(buffer) {
            continue;
        }
        let old = int_or_zero(view, "byteOffset");
        let cut: u64 = ranges.iter().filter(|r| r.end <= old).fold(0u64, |cut, r| {
            cut.saturating_add(r.end.saturating_sub(r.start))
        });
        view["byteOffset"] = json!(old.saturating_sub(cut));
    }
}

// ---- 3MF -------------------------------------------------------------------

const CONTENT_TYPES_PATH: &str = "[Content_Types].xml";
const ROOT_RELS_PATH: &str = "_rels/.rels";

/// asset-core's `relsPathFor` (`3mf/decomposer.ts`).
fn rels_path_for(part: &str) -> String {
    let (dir, base) = part.rsplit_once('/').unwrap_or(("", part));
    format!("{dir}/_rels/{base}.rels")
}

/// A zip entry's name as fflate reads it: UTF-8 when the entry says so,
/// else one character per byte. The zip crate reads unflagged names as
/// CP437, which never matches a non-ASCII name's UTF-8 reading.
fn entry_name(raw: &[u8], named: Option<&str>) -> String {
    let utf8 = String::from_utf8_lossy(raw);
    if raw.is_ascii() || named == Some(utf8.as_ref()) {
        return utf8.into_owned();
    }
    raw.iter().map(|b| char::from(*b)).collect()
}

/// fflate's `unzipSync` as asset-core's `unzipBytes` (`3mf/zip.ts`) calls
/// it: every entry, folders included, in the archive's order.
fn unzip(bytes: &[u8]) -> Result<Vec<(String, Vec<u8>)>, String> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|_| NOT_3MF)?;
    // What goes up is the unzipped entries, so they count against
    // MAX_UPLOAD (a few kilobytes of zip can stand for gigabytes).
    let mut budget = MAX_UPLOAD;
    let mut entries = Vec::new();
    for i in 0..archive.len() {
        let mut file = archive.by_index(i).map_err(|_| NOT_3MF)?;
        let name = entry_name(file.name_raw(), file.name().ok().as_deref());
        let mut data = Vec::new();
        (&mut file)
            .take(budget + 1)
            .read_to_end(&mut data)
            .map_err(|_| NOT_3MF)?;
        budget = budget.checked_sub(data.len() as u64).ok_or(TOO_LARGE)?;
        entries.push((name, data));
    }
    Ok(entries)
}

/// The text of a package part, as fflate's `strFromU8` (a `TextDecoder`)
/// reads it.
fn str_from_u8(bytes: &[u8]) -> String {
    let bytes = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes);
    String::from_utf8_lossy(bytes).into_owned()
}

/// asset-core's 3MF `decompose` (`3mf/decomposer.ts`): the package's XML
/// in the composite, every other entry a raw part.
fn decompose_3mf(bytes: &[u8], asset_name: &str, uploads: &mut Uploads) -> Result<Value, String> {
    let entries = unzip(bytes)?;
    let find = |path: &str| entries.iter().find(|(p, _)| p == path).map(|(_, b)| b);
    let model_path = entries
        .iter()
        .map(|(p, _)| p.clone())
        .find(|p| p.ends_with(".model"))
        .ok_or(NOT_3MF)?;
    let content_types = find(CONTENT_TYPES_PATH).ok_or(NOT_3MF)?;
    let root_rels = find(ROOT_RELS_PATH).ok_or(NOT_3MF)?;
    let model_rels_path = rels_path_for(&model_path);
    let core = [
        CONTENT_TYPES_PATH,
        ROOT_RELS_PATH,
        &model_path,
        &model_rels_path,
    ];
    let base = sanitize_file_name(or(asset_name, "3mf"));
    let mut parts = Map::new();
    for (path, bytes) in entries.iter().filter(|(p, _)| !core.contains(&p.as_str())) {
        let file_name = path.rsplit('/').next().unwrap_or("");
        let (index, meta) = uploads.add(bytes, &format!("{base}_{file_name}"), false);
        parts.insert(
            path.clone(),
            json!({"cid": format!("part:{index}"), "_arbesk": meta}),
        );
    }
    Ok(json!({
        "arbesk_format": "composite-3mf",
        "modelPath": model_path,
        "contentTypes": str_from_u8(content_types),
        "rootRels": str_from_u8(root_rels),
        "modelRels": find(&model_rels_path).map(|b| str_from_u8(b)),
        "model": str_from_u8(find(&model_path).expect("the model found above")),
        "parts": parts,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ipfs::unpack;
    use serde_json::json;
    use std::collections::HashMap;
    use std::io::Write;
    use std::path::PathBuf;

    fn fixture(case: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/arbesk")
            .join(case)
    }

    fn read_json(path: PathBuf) -> Value {
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
    }

    /// A fixture part as asset-core stored it.
    fn stored_part(case: &str, cid: &str) -> Vec<u8> {
        std::fs::read(fixture(case).join("cids").join(cid)).unwrap()
    }

    fn raw(stored: &[u8]) -> Vec<u8> {
        unpack(stored.to_vec(), 1 << 40).unwrap()
    }

    /// The fixture's 168-byte box buffer and 70-byte red PNG, unpacked.
    fn box_bin() -> Vec<u8> {
        raw(&stored_part("glb-box", "bafymem00000000"))
    }

    fn red_png() -> Vec<u8> {
        raw(&stored_part("gltf-image", "bafymem00000003"))
    }

    /// What every stored part's reference is (a placeholder or a CID) and
    /// the hash its `_arbesk` gives.
    fn refs(composite: &Value) -> Vec<(String, String)> {
        let hash = |item: &Value| item["_arbesk"]["hash"].as_str().unwrap().to_string();
        if composite.get("arbesk_format").is_some() {
            let parts = composite["parts"].as_object().unwrap();
            return parts
                .values()
                .map(|part| (part["cid"].as_str().unwrap().to_string(), hash(part)))
                .collect();
        }
        ["buffers", "images"]
            .iter()
            .flat_map(|key| composite[key].as_array().into_iter().flatten())
            .filter(|item| item.get("_arbesk").is_some())
            .map(|item| (item["uri"].as_str().unwrap().to_string(), hash(item)))
            .collect()
    }

    fn placeholder(reference: &str) -> usize {
        reference.rsplit(':').next().unwrap().parse().unwrap()
    }

    // MurmurHash3 x86_128 of [1, 2, …, n] for n = 0..=17, as asset-core's
    // `murmur3_128` (packages/asset-core/src/utils/hash.ts) printed them
    // when run once with Bun.
    const VECTORS: [&str; 18] = [
        "00000000000000000000000000000000",
        "da3dd3f084f1d1dd84f1d1dd84f1d1dd",
        "22873b80fc710253fc710253fc710253",
        "f60164e1b5134233b5134233b5134233",
        "92f751ff0702dd330702dd330702dd33",
        "7ec1805b6d165469a553f215a553f215",
        "f99d9ed24e96163de9445520e9445520",
        "643d8c4702fc63bef4d7322af4d7322a",
        "2ba830ec90744e228f8ec2518f8ec251",
        "5dae59556ee32fec2eaa1c35bb09ec8b",
        "bc513223c10e2ca7ef5f8798935e13c4",
        "acdd472b0c27b7e23c732b4cee094c8d",
        "9ac744913203dd346e54fd6dd3100b57",
        "ff257bed03e042742248ebf614bfd9b0",
        "9c37f65e93bb5ea7c5619c3c4241a488",
        "4b54782fd463959a779b8d59b78c1a7b",
        "121b035e3ff275b78b6939617769eae2",
        "eb490bba07412f90d56f0e6fed78fd0e",
    ];
    // The same, of 1000 bytes (i * 7 + 3) & 255 and of 33 bytes 0xff.
    const VECTOR_1000: &str = "b16407cd3fc1b57f1516d8205e445560";
    const VECTOR_FF_33: &str = "efef5bd1964d9e1e1d1a6195cfc9f7bb";

    #[test]
    fn hashes_match_asset_core() {
        for (n, expected) in VECTORS.iter().enumerate() {
            let bytes: Vec<u8> = (1..=n as u8).collect();
            assert_eq!(murmur3_128_hex(&bytes), *expected, "length {n}");
        }
        let pattern: Vec<u8> = (0..1000u32).map(|i| ((i * 7 + 3) & 255) as u8).collect();
        assert_eq!(murmur3_128_hex(&pattern), VECTOR_1000);
        assert_eq!(murmur3_128_hex(&[0xff; 33]), VECTOR_FF_33);
        assert_eq!(
            murmur3_128_hex(&box_bin()),
            "5c237da9173219dc09253c3ae6c65ecc"
        );
        assert_eq!(
            murmur3_128_hex(&red_png()),
            "be047e57fb51936fab370a6fab2e1c8e"
        );
        for case in ["glb-box", "gltf-image", "glb-texture", "3mf-thumbnail"] {
            for (cid, hash) in refs(&read_json(fixture(case).join("source.json"))) {
                let cid = cid.trim_start_matches("ipfs://");
                assert_eq!(
                    murmur3_128_hex(&raw(&stored_part(case, cid))),
                    hash,
                    "{case} {cid}"
                );
            }
        }
    }

    const CASES: [(&str, &str, Format, &str); 5] = [
        ("glb-box", "glb", Format::Gltf, "asset_composite.gltf.br"),
        (
            "gltf-image",
            "gltf",
            Format::Gltf,
            "asset_composite.gltf.br",
        ),
        (
            "glb-texture",
            "glb",
            Format::Gltf,
            "asset_composite.gltf.br",
        ),
        (
            "3mf-box",
            "3mf",
            Format::ThreeMf,
            "composite_composite.3mf.json",
        ),
        (
            "3mf-thumbnail",
            "3mf",
            Format::ThreeMf,
            "composite_composite.3mf.json",
        ),
    ];

    /// Each sample asset-core decomposed (`scripts/make-arbesk-fixtures.ts`,
    /// without an asset name) comes out the same here: the composite once
    /// the parts' CIDs are in, every part's bytes, flags and file name.
    #[test]
    fn decompose_matches_asset_core() {
        for (case, ext, format, composite_name) in CASES {
            let ours = decompose(&fixture(case).join(format!("input.{ext}")), "").unwrap();
            let source = read_json(fixture(case).join("source.json"));
            let names = read_json(fixture(case).join("names.json"));
            let by_hash: HashMap<String, String> = refs(&source)
                .into_iter()
                .map(|(cid, hash)| (hash, cid.trim_start_matches("ipfs://").to_string()))
                .collect();
            assert_eq!(ours.parts.len(), by_hash.len(), "{case}");
            let mut cids = vec![String::new(); ours.parts.len()];
            for (reference, hash) in refs(&ours.composite) {
                cids[placeholder(&reference)] = by_hash[&hash].clone();
            }
            assert_eq!(fill(&ours.composite, ours.format, &cids), source, "{case}");
            for (part, cid) in ours.parts.iter().zip(&cids) {
                let theirs = stored_part(case, cid);
                assert_eq!(raw(&part.stored), raw(&theirs), "{case} {cid}");
                assert_eq!(part.stored, theirs, "{case} {cid}: stored bytes");
                assert_eq!(part.raw_len, raw(&theirs).len() as u64, "{case} {cid}");
                assert_eq!(part.file_name, names[cid.as_str()], "{case} {cid}");
            }
            assert_eq!(ours.format, format, "{case}");
            assert_eq!(ours.composite_name, composite_name, "{case}");
            // The Studio compresses a glTF composite only.
            assert_eq!(ours.composite_compressed, format == Format::Gltf, "{case}");
        }
    }

    /// asset-core's `sanitizeFileName` gave these, run once with Bun.
    #[test]
    fn names_are_made_safe_as_asset_core_does() {
        let cases = [
            ("Red Box!", "red_box_"),
            ("a_!b", "a__b"),
            ("", "asset"),
            ("!!!", "_"),
            ("Ünïcödé Böx", "_n_c_d_b_x"),
            ("\u{212A}elvin", "kelvin"),
            (&"A".repeat(50), &"a".repeat(40)),
            ("x--y__z", "x--y__z"),
            ("a!!_!!b", "a___b"),
        ];
        for (name, safe) in cases {
            assert_eq!(sanitize_file_name(name), safe, "{name}");
        }
    }

    #[test]
    fn the_asset_name_names_the_files() {
        let ours = decompose(&fixture("glb-texture").join("input.glb"), "Red Box!").unwrap();
        let names: Vec<&str> = ours.parts.iter().map(|p| p.file_name.as_str()).collect();
        assert_eq!(
            names,
            ["red_box__texture_0.png.br", "red_box__buffer_0.bin.br"]
        );
        assert_eq!(ours.composite_name, "red_box__composite.gltf.br");
        let ours = decompose(&fixture("3mf-thumbnail").join("input.3mf"), "Red Box!").unwrap();
        assert_eq!(ours.parts[0].file_name, "red_box__thumbnail.png");
        assert_eq!(ours.composite_name, "red_box__composite.3mf.json");
        assert!(!ours.composite_compressed);
    }

    fn write(path: &Path, bytes: &[u8]) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }

    /// A `.gltf` in `dir` whose buffer and image point at these URIs.
    fn gltf_with(dir: &Path, buffer: &str, image: &str) -> PathBuf {
        let gltf = json!({
            "asset": {"version": "2.0"},
            "buffers": [{"byteLength": 168, "uri": buffer}],
            "images": [{"uri": image, "name": "red"}],
        });
        let file = dir.join("scene.gltf");
        write(&file, gltf.to_string().as_bytes());
        file
    }

    fn arbesk(bytes: &[u8], compressed: bool) -> Value {
        json!({
            "hash": murmur3_128_hex(bytes),
            "hashAlgo": "murmur3-128",
            "compressed": compressed,
            "bytes": bytes.len(),
        })
    }

    #[test]
    fn a_gltf_uploads_files_beside_it() {
        let dir = tempfile::tempdir().unwrap();
        write(&dir.path().join("scene.bin"), &box_bin());
        write(&dir.path().join("tex/a b.png"), &red_png());
        let file = gltf_with(dir.path(), "scene.bin", "tex/a%20b.png");
        let ours = decompose(&file, "Box").unwrap();
        assert_eq!(
            ours.composite,
            json!({
                "asset": {"version": "2.0"},
                "buffers": [{"byteLength": 168, "uri": "ipfs://part:0", "_arbesk": arbesk(&box_bin(), true)}],
                "images": [{"uri": "ipfs://part:1", "name": "red", "_arbesk": arbesk(&red_png(), true)}],
            })
        );
        let names: Vec<&str> = ours.parts.iter().map(|p| p.file_name.as_str()).collect();
        assert_eq!(names, ["box_buffer_0.bin.br", "box_texture_0.png.br"]);
        assert_eq!(raw(&ours.parts[0].stored), box_bin());
        assert_eq!(raw(&ours.parts[1].stored), red_png());
        assert_eq!(ours.composite_name, "box_composite.gltf.br");
        assert!(ours.composite_compressed);
    }

    #[test]
    fn gltf_parts_stay_in_its_folder() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("model");
        write(&root.path().join("x.bin"), &box_bin());
        write(&dir.join("red.png"), &red_png());
        std::os::unix::fs::symlink(root.path().join("x.bin"), dir.join("link.bin")).unwrap();
        for uri in [
            "../x.bin",
            "%2e%2e/x.bin",
            "/etc/x",
            "http://example.com/x.bin",
            "link.bin",
        ] {
            let file = gltf_with(&dir, uri, "red.png");
            assert_eq!(decompose(&file, "Box").unwrap_err(), outside(uri), "{uri}");
        }
        let file = gltf_with(&dir, "scene.bin", "red.png");
        assert_eq!(
            decompose(&file, "Box").unwrap_err(),
            "“scene.bin” can't be found."
        );
        assert_eq!(missing("scene.bin"), "“scene.bin” can't be found.");
        assert_eq!(outside("/etc/x"), "“/etc/x” is outside the model's folder.");
    }

    fn data(mime: &str, bytes: &[u8]) -> String {
        format!("data:{mime};base64,{}", data_encoding::BASE64.encode(bytes))
    }

    #[test]
    fn identical_bytes_are_one_part() {
        let dir = tempfile::tempdir().unwrap();
        let png = data("image/png", &red_png());
        let gltf = json!({
            "buffers": [{"byteLength": 168, "uri": data("application/octet-stream", &box_bin())}],
            "images": [{"uri": png}, {"uri": png}],
        });
        let file = dir.path().join("two.gltf");
        write(&file, gltf.to_string().as_bytes());
        let ours = decompose(&file, "Two").unwrap();
        assert_eq!(ours.parts.len(), 2);
        assert_eq!(ours.composite["images"][0]["uri"], "ipfs://part:1");
        assert_eq!(ours.composite["images"][1]["uri"], "ipfs://part:1");
        assert_eq!(ours.parts[1].file_name, "two_texture_0.png.br");
    }

    fn zip_of(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for (name, bytes) in entries {
            writer
                .start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            writer.write_all(bytes).unwrap();
        }
        writer.finish().unwrap().into_inner()
    }

    #[test]
    fn a_zip_without_a_model_is_not_a_3mf() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("box.3mf");
        let types: (&str, &[u8]) = ("[Content_Types].xml", b"<Types/>");
        let rels: (&str, &[u8]) = ("_rels/.rels", b"<Relationships/>");
        let model: (&str, &[u8]) = ("3D/3dmodel.model", b"<model/>");
        for package in [
            zip_of(&[types, rels]),
            zip_of(&[rels, model]),
            zip_of(&[types, model]),
            b"not a zip".to_vec(),
        ] {
            write(&file, &package);
            assert_eq!(decompose(&file, "Box").unwrap_err(), NOT_3MF);
        }
        write(&file, &zip_of(&[types, rels, model]));
        let ours = decompose(&file, "Box").unwrap();
        assert_eq!(ours.composite["modelPath"], "3D/3dmodel.model");
        assert_eq!(ours.composite["modelRels"], Value::Null);
    }

    /// A GLB holding `json` and a BIN chunk of `bin`.
    fn glb(json: &Value, bin: &[u8]) -> Vec<u8> {
        let mut text = json.to_string().into_bytes();
        text.resize(text.len().div_ceil(4) * 4, b' ');
        let mut bin = bin.to_vec();
        bin.resize(bin.len().div_ceil(4) * 4, 0);
        let mut out = Vec::new();
        let total = 12 + 8 + text.len() + 8 + bin.len();
        for word in [0x46546c67, 2, total, text.len(), 0x4e4f534a] {
            out.extend_from_slice(&(word as u32).to_le_bytes());
        }
        out.extend_from_slice(&text);
        out.extend_from_slice(&(bin.len() as u32).to_le_bytes());
        out.extend_from_slice(&0x004e4942u32.to_le_bytes());
        out.extend_from_slice(&bin);
        out
    }

    /// Numbers and entries a GLB makes up never crash the app (the
    /// release build aborts on a panic): they clamp as `subarray` does or
    /// are refused.
    #[test]
    fn a_malformed_glb_is_never_a_crash() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("bad.glb");
        let png = red_png();
        let huge = json!({
            "images": [{"bufferView": 0}],
            "accessors": [{"bufferView": 1, "count": 1}],
            "bufferViews": [
                {"buffer": 0, "byteOffset": 1, "byteLength": 1e20},
                {"buffer": 0, "byteOffset": 1e20, "byteLength": 1e20},
            ],
            "buffers": [{"byteLength": png.len()}],
        });
        let not_an_object = json!({
            "images": [{"bufferView": 0}],
            "bufferViews": [{"buffer": 0, "byteLength": png.len()}],
            "buffers": ["x"],
        });
        for json in [huge, not_an_object] {
            write(&file, &glb(&json, &png));
            match decompose(&file, "Bad") {
                Ok(_) => {}
                Err(error) => assert_eq!(error, NOT_GLTF, "{json}"),
            }
        }
    }

    /// What a 3MF uploads is its unzipped entries: more than MAX_UPLOAD of
    /// them is too large, however small the zip.
    #[test]
    fn a_3mf_counts_what_it_unzips_to() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("bomb.3mf");
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default();
        for (name, bytes) in [
            ("[Content_Types].xml", &b"<Types/>"[..]),
            ("_rels/.rels", b"<Relationships/>"),
            ("3D/3dmodel.model", b"<model/>"),
        ] {
            writer.start_file(name, options).unwrap();
            writer.write_all(bytes).unwrap();
        }
        writer.start_file("Metadata/zeros.bin", options).unwrap();
        let chunk = vec![0u8; 1 << 20];
        for _ in 0..MAX_UPLOAD >> 20 {
            writer.write_all(&chunk).unwrap();
        }
        let package = writer.finish().unwrap().into_inner();
        assert!((package.len() as u64) < MAX_UPLOAD / 100);
        write(&file, &package);
        assert_eq!(decompose(&file, "Bomb").err().as_deref(), Some(TOO_LARGE));
    }

    #[test]
    fn other_extensions_and_large_files_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let stl = dir.path().join("box.stl");
        write(&stl, b"solid box\nendsolid box\n");
        assert_eq!(decompose(&stl, "Box").unwrap_err(), NOT_A_MODEL);
        std::fs::create_dir(dir.path().join("folder.glb")).unwrap();
        assert_eq!(
            decompose(&dir.path().join("folder.glb"), "Box").unwrap_err(),
            NOT_A_MODEL
        );
        assert_eq!(
            decompose(&dir.path().join("none.glb"), "Box").unwrap_err(),
            NOT_A_MODEL
        );

        let upper = dir.path().join("BOX.GLB");
        std::fs::copy(fixture("glb-box").join("input.glb"), &upper).unwrap();
        assert!(decompose(&upper, "Box").is_ok());

        let big = std::fs::File::create(dir.path().join("big.glb")).unwrap();
        big.set_len(MAX_UPLOAD + 1).unwrap();
        assert_eq!(
            decompose(&dir.path().join("big.glb"), "Big").unwrap_err(),
            TOO_LARGE
        );

        let bin = std::fs::File::create(dir.path().join("scene.bin")).unwrap();
        bin.set_len(MAX_UPLOAD - 10).unwrap();
        write(&dir.path().join("red.png"), &red_png());
        let file = gltf_with(dir.path(), "scene.bin", "red.png");
        assert_eq!(decompose(&file, "Big").unwrap_err(), TOO_LARGE);
        assert_eq!(
            TOO_LARGE,
            "This file is too large for Arbesk (100 MB at most)."
        );
    }

    /// A user's glTF that happens to carry an `arbesk_format` key is still a
    /// glTF: its buffers are filled, decided by the format and not the key.
    #[test]
    fn fill_goes_by_the_format_not_a_key() {
        let cids = ["bafyA".to_string()];
        let gltf = json!({
            "arbesk_format": "mine",
            "buffers": [{"uri": "ipfs://part:0"}],
        });
        assert_eq!(
            fill(&gltf, Format::Gltf, &cids),
            json!({"arbesk_format": "mine", "buffers": [{"uri": "ipfs://bafyA"}]})
        );
    }

    #[test]
    fn fill_puts_cids_in_place() {
        let cids = ["bafyA".to_string(), "bafyB".to_string()];
        let gltf = json!({
            "buffers": [{"uri": "ipfs://part:1", "byteLength": 4}],
            "images": [{"uri": "ipfs://part:0", "name": "part:0"}],
            "nodes": [{"name": "ipfs://part:0"}],
        });
        assert_eq!(
            fill(&gltf, Format::Gltf, &cids),
            json!({
                "buffers": [{"uri": "ipfs://bafyB", "byteLength": 4}],
                "images": [{"uri": "ipfs://bafyA", "name": "part:0"}],
                "nodes": [{"name": "ipfs://part:0"}],
            })
        );
        let three_mf = json!({
            "arbesk_format": "composite-3mf",
            "model": "part:0",
            "parts": {"Metadata/a.png": {"cid": "part:1", "_arbesk": {}}},
        });
        assert_eq!(
            fill(&three_mf, Format::ThreeMf, &cids),
            json!({
                "arbesk_format": "composite-3mf",
                "model": "part:0",
                "parts": {"Metadata/a.png": {"cid": "bafyB", "_arbesk": {}}},
            })
        );
    }

    /// A GLB image keeps a `mimeType` JavaScript finds truthy and gets the
    /// sniffed one otherwise.
    #[test]
    fn truthiness_is_javascripts() {
        for value in [json!(null), json!(false), json!(0), json!(0.0), json!("")] {
            assert!(!truthy(&value), "{value}");
        }
        for value in [
            json!(true),
            json!(2),
            json!("image/png"),
            json!([]),
            json!({}),
        ] {
            assert!(truthy(&value), "{value}");
        }
    }

    #[test]
    fn json_is_stored_raw_or_brotli_framed() {
        let value = json!({"a": [1, 2, 3], "b": "text"});
        let stored = stored_json(&value, true);
        assert!(stored.starts_with(b"ARB\x01"));
        assert_eq!(raw(&stored), serde_json::to_vec(&value).unwrap());
        assert_eq!(
            stored_json(&value, false),
            serde_json::to_vec(&value).unwrap()
        );
        assert_eq!(raw(&brotli_framed(b"bytes", 5)), b"bytes");
    }
}
