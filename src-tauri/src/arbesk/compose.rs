//! Turning what Arbesk stores back into a file, as asset-core's `compose`
//! does: a composite glTF becomes a self-contained `.gltf` (every
//! `ipfs://` buffer and image as a `data:` URI), a composite 3MF becomes a
//! `.3mf` zip again. Anything else is refused rather than guessed at.

use std::io::{Cursor, Write};

use data_encoding::BASE64;
use serde_json::{Map, Value};
use zip::ZipWriter;
use zip::write::SimpleFileOptions;

use crate::arbesk::ipfs::Gateway;

pub const UNKNOWN_FORMAT: &str = "This asset uses an Arbesk format File Browser can't read yet.";

const IPFS: &str = "ipfs://";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Gltf,
    ThreeMf,
}

impl Format {
    pub fn ext(self) -> &'static str {
        match self {
            Format::Gltf => "gltf",
            Format::ThreeMf => "3mf",
        }
    }

    /// The extension of the file the preview loads (`compose_preview`).
    pub fn preview_ext(self) -> &'static str {
        match self {
            Format::Gltf => "glb",
            Format::ThreeMf => "3mf",
        }
    }
}

/// The file a stored source becomes: glTF without a marker, 3MF for
/// `composite-3mf`; any other marker is refused.
pub fn format_of(source: &Value) -> Result<Format, String> {
    match source.get("arbesk_format") {
        None => Ok(Format::Gltf),
        Some(marker) if marker == "composite-3mf" => Ok(Format::ThreeMf),
        Some(_) => Err(UNKNOWN_FORMAT.into()),
    }
}

/// The CID of an asset's stored source: the first node's source when the
/// asset manifest only wraps one (asset-core's `resolveCompositeSourceCid`),
/// else the manifest itself.
pub fn source_cid(manifest: &Value, cid: &str) -> String {
    let wraps = ["buffers", "meshes", "arbesk_format"]
        .iter()
        .all(|key| manifest.get(key).is_none());
    let source = manifest
        .pointer("/scene/nodes/0/source/cid")
        .and_then(Value::as_str);
    match source {
        Some(source) if wraps => source.to_string(),
        _ => cid.to_string(),
    }
}

/// Whether the asset is a live link to another collection's asset.
pub fn is_child_ref(manifest: &Value) -> bool {
    let nodes = manifest.pointer("/scene/nodes").and_then(Value::as_array);
    nodes
        .into_iter()
        .flatten()
        .any(|node| node.get("child_ref").is_some())
}

/// The file `source` stands for, its parts fetched through `gateway`: the
/// `.gltf` (or `.3mf`) an asset is, as Arbesk builds it. Kept for copying
/// assets out, which isn't offered yet; the preview uses `compose_preview`.
#[allow(dead_code)]
pub fn compose(gateway: &Gateway, source: &Value) -> Result<Vec<u8>, String> {
    compose_within(gateway, source, BUILD_CAP)
}

pub const TOO_LARGE_TO_BUILD: &str = "This asset is too large to open.";
/// The most an asset's parts may add up to.
pub const BUILD_CAP: u64 = 512 * 1024 * 1024;

/// `compose`, refusing once the parts fetched add up to more than `cap` bytes.
#[allow(dead_code)]
pub fn compose_within(gateway: &Gateway, source: &Value, cap: u64) -> Result<Vec<u8>, String> {
    let parts = Parts {
        gateway,
        left: std::cell::Cell::new(cap),
    };
    match format_of(source)? {
        Format::Gltf => compose_gltf(&parts, source),
        Format::ThreeMf => compose_3mf(&parts, source),
    }
}

/// What the Space preview loads: a glTF as one binary `.glb` (no base64
/// for the webview to decode, a small JSON to parse), a 3MF as itself. The
/// `.gltf` that `compose` builds stays the asset's file.
pub fn compose_preview(gateway: &Gateway, source: &Value) -> Result<Vec<u8>, String> {
    compose_preview_within(gateway, source, BUILD_CAP)
}

/// `compose_preview`, refusing once the parts fetched add up to more than `cap` bytes.
pub fn compose_preview_within(
    gateway: &Gateway,
    source: &Value,
    cap: u64,
) -> Result<Vec<u8>, String> {
    let parts = Parts {
        gateway,
        left: std::cell::Cell::new(cap),
    };
    match format_of(source)? {
        Format::Gltf => glb::compose(&parts, source),
        Format::ThreeMf => compose_3mf(&parts, source),
    }
}

/// The parts of one build, fetched within its budget.
struct Parts<'g> {
    gateway: &'g Gateway<'g>,
    left: std::cell::Cell<u64>,
}

impl Parts<'_> {
    fn fetch(&self, cid: &str) -> Result<Vec<u8>, String> {
        let bytes = self.gateway.bytes(cid)?;
        let left = self
            .left
            .get()
            .checked_sub(bytes.len() as u64)
            .ok_or(TOO_LARGE_TO_BUILD)?;
        self.left.set(left);
        Ok(bytes)
    }
}

// ---- glTF --------------------------------------------------------------------

/// A buffer's or image's bytes: an `ipfs://` part, or a base64 `data:` URI.
fn resource_bytes(parts: &Parts, uri: Option<&str>) -> Result<Vec<u8>, String> {
    let uri = uri.ok_or(UNKNOWN_FORMAT)?;
    if let Some(cid) = uri.strip_prefix(IPFS) {
        return parts.fetch(cid);
    }
    let data = uri.strip_prefix("data:").ok_or(UNKNOWN_FORMAT)?;
    let (header, payload) = data.split_once(',').ok_or(UNKNOWN_FORMAT)?;
    if !header.ends_with(";base64") {
        return Err(UNKNOWN_FORMAT.into());
    }
    BASE64
        .decode(payload.as_bytes())
        .map_err(|_| UNKNOWN_FORMAT.into())
}

/// The glTF as a GLB: every buffer and image placed in one BIN chunk.
mod glb {
    use super::{Parts, UNKNOWN_FORMAT, resource_bytes};
    use serde_json::{Map, Value, json};

    const MAGIC: &[u8; 4] = b"glTF";
    const JSON_CHUNK: &[u8; 4] = b"JSON";
    const BIN_CHUNK: &[u8; 4] = b"BIN\0";

    pub(super) fn compose(parts: &Parts, source: &Value) -> Result<Vec<u8>, String> {
        let mut gltf = source.clone();
        let mut bin = Vec::new();
        let starts = place_buffers(parts, &mut gltf, &mut bin)?;
        point_views(&mut gltf, &starts)?;
        place_images(parts, &mut gltf, &mut bin)?;
        let root = gltf.as_object_mut().ok_or(UNKNOWN_FORMAT)?;
        if bin.is_empty() {
            root.remove("buffers");
        } else {
            root.insert("buffers".into(), json!([{ "byteLength": bin.len() }]));
        }
        Ok(container(&gltf, &bin))
    }

    fn array<'a>(gltf: &'a mut Value, key: &str) -> Vec<&'a mut Map<String, Value>> {
        gltf.get_mut(key)
            .and_then(Value::as_array_mut)
            .map(|items| items.iter_mut().filter_map(Value::as_object_mut).collect())
            .unwrap_or_default()
    }

    /// Append `bytes` on a 4-byte boundary; where they start.
    fn append(bin: &mut Vec<u8>, bytes: &[u8]) -> usize {
        bin.resize(bin.len().next_multiple_of(4), 0);
        let start = bin.len();
        bin.extend_from_slice(bytes);
        start
    }

    /// Every buffer's bytes into `bin`; where each starts.
    fn place_buffers(
        parts: &Parts,
        gltf: &mut Value,
        bin: &mut Vec<u8>,
    ) -> Result<Vec<usize>, String> {
        array(gltf, "buffers")
            .into_iter()
            .map(|buffer| {
                Ok(append(
                    bin,
                    &resource_bytes(parts, buffer.get("uri").and_then(Value::as_str))?,
                ))
            })
            .collect()
    }

    /// Each buffer view, now into the one buffer at its old buffer's start.
    fn point_views(gltf: &mut Value, starts: &[usize]) -> Result<(), String> {
        for view in array(gltf, "bufferViews") {
            let buffer = view
                .get("buffer")
                .and_then(Value::as_u64)
                .ok_or(UNKNOWN_FORMAT)?;
            let start = *starts.get(buffer as usize).ok_or(UNKNOWN_FORMAT)?;
            let offset = view.get("byteOffset").and_then(Value::as_u64).unwrap_or(0) as usize;
            view.insert("buffer".into(), json!(0));
            view.insert("byteOffset".into(), json!(start + offset));
        }
        Ok(())
    }

    /// Each image with a URI into `bin`, as a buffer view of its own.
    fn place_images(parts: &Parts, gltf: &mut Value, bin: &mut Vec<u8>) -> Result<(), String> {
        let mut views = Vec::new();
        let first_view = gltf
            .get("bufferViews")
            .and_then(Value::as_array)
            .map_or(0, Vec::len);
        for image in array(gltf, "images") {
            image.remove("_arbesk");
            let Some(uri) = image.remove("uri") else {
                continue;
            };
            let bytes = resource_bytes(parts, uri.as_str())?;
            let mime = image
                .get("mimeType")
                .and_then(Value::as_str)
                .map(String::from);
            let mime =
                mime.unwrap_or_else(|| image_mime(uri.as_str().unwrap_or_default(), &bytes).into());
            image.insert("mimeType".into(), json!(mime));
            image.insert("bufferView".into(), json!(first_view + views.len()));
            views.push(json!({ "buffer": 0, "byteOffset": append(bin, &bytes), "byteLength": bytes.len() }));
        }
        if !views.is_empty() {
            let root = gltf.as_object_mut().ok_or(UNKNOWN_FORMAT)?;
            let all = root.entry("bufferViews").or_insert_with(|| json!([]));
            all.as_array_mut().ok_or(UNKNOWN_FORMAT)?.extend(views);
        }
        Ok(())
    }

    /// An image's type: its data: URI's, else what its first bytes say.
    fn image_mime(uri: &str, bytes: &[u8]) -> &'static str {
        if uri.starts_with("data:image/jpeg") || bytes.starts_with(&[0xff, 0xd8]) {
            "image/jpeg"
        } else if uri.starts_with("data:image/webp")
            || (bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP"))
        {
            "image/webp"
        } else {
            "image/png"
        }
    }

    /// The GLB container: header, JSON chunk (padded with spaces), BIN chunk.
    fn container(gltf: &Value, bin: &[u8]) -> Vec<u8> {
        let mut json = serde_json::to_vec(gltf).unwrap_or_default();
        json.resize(json.len().next_multiple_of(4), b' ');
        let bin_len = bin.len().next_multiple_of(4);
        let bin_chunk = if bin.is_empty() { 0 } else { 8 + bin_len };
        let total = 12 + 8 + json.len() + bin_chunk;
        let mut out = Vec::with_capacity(total);
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&2u32.to_le_bytes());
        out.extend_from_slice(&(total as u32).to_le_bytes());
        out.extend_from_slice(&(json.len() as u32).to_le_bytes());
        out.extend_from_slice(JSON_CHUNK);
        out.extend_from_slice(&json);
        if !bin.is_empty() {
            out.extend_from_slice(&(bin_len as u32).to_le_bytes());
            out.extend_from_slice(BIN_CHUNK);
            out.extend_from_slice(bin);
            out.resize(total, 0);
        }
        out
    }
}

fn compose_gltf(parts: &Parts, source: &Value) -> Result<Vec<u8>, String> {
    let mut gltf = source.clone();
    for (key, is_image) in [("buffers", false), ("images", true)] {
        if let Some(items) = gltf.get_mut(key).and_then(Value::as_array_mut) {
            for item in items.iter_mut().filter_map(Value::as_object_mut) {
                embed(parts, item, is_image)?;
            }
        }
    }
    serde_json::to_vec(&gltf).map_err(|error| error.to_string())
}

/// One buffer or image as a standard glTF holds it: Arbesk's `_arbesk`
/// removed, an `ipfs://` URI replaced by the data itself. An image with a
/// URI loses its `bufferView`, as asset-core's compose does.
fn embed(parts: &Parts, item: &mut Map<String, Value>, is_image: bool) -> Result<(), String> {
    item.remove("_arbesk");
    let Some(uri) = item.get("uri").and_then(Value::as_str).map(String::from) else {
        return Ok(());
    };
    if is_image {
        item.remove("bufferView");
    }
    if uri.starts_with("data:") {
        return Ok(());
    }
    let cid = uri.strip_prefix(IPFS).ok_or(UNKNOWN_FORMAT)?;
    let mime = match item.get("mimeType").and_then(Value::as_str) {
        Some(mime) if is_image => mime.to_string(),
        _ if is_image => "image/png".into(),
        _ => "application/octet-stream".into(),
    };
    let data = BASE64.encode(&parts.fetch(cid)?);
    item.insert(
        "uri".into(),
        Value::String(format!("data:{mime};base64,{data}")),
    );
    Ok(())
}

// ---- 3MF ---------------------------------------------------------------------

fn compose_3mf(parts: &Parts, source: &Value) -> Result<Vec<u8>, String> {
    let text = |key: &str| {
        source
            .get(key)
            .and_then(Value::as_str)
            .ok_or_else(|| format!("The stored 3MF has no {key}."))
    };
    let model_path = source
        .get("modelPath")
        .and_then(Value::as_str)
        .unwrap_or("3D/3dmodel.model");
    // Like every part's, the model's name may not leave the package.
    let model_path = &part_path(model_path)?;
    let mut files: Vec<(String, Vec<u8>)> = vec![
        (
            "[Content_Types].xml".into(),
            text("contentTypes")?.as_bytes().to_vec(),
        ),
        ("_rels/.rels".into(), text("rootRels")?.as_bytes().to_vec()),
        (model_path.into(), text("model")?.as_bytes().to_vec()),
    ];
    if let Some(rels) = source.get("modelRels").and_then(Value::as_str) {
        files.push((rels_path_for(model_path), rels.as_bytes().to_vec()));
    }
    for (path, part) in source
        .get("parts")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
    {
        let cid = part
            .get("cid")
            .and_then(Value::as_str)
            .ok_or(UNKNOWN_FORMAT)?;
        files.push((part_path(path)?, parts.fetch(cid)?));
    }
    zip_files(&files)
}

/// The `.rels` part that belongs to `part`, as asset-core's `relsPathFor`.
fn rels_path_for(part: &str) -> String {
    let (dir, base) = part.rsplit_once('/').unwrap_or(("", part));
    format!("{dir}/_rels/{base}.rels")
}

/// A part's path inside the zip, refused if it could leave the package.
fn part_path(path: &str) -> Result<String, String> {
    let climbs = path
        .split('/')
        .any(|segment| segment.is_empty() || segment == "." || segment == "..");
    if climbs || path.contains('\\') {
        return Err(UNKNOWN_FORMAT.into());
    }
    Ok(path.to_string())
}

fn zip_files(files: &[(String, Vec<u8>)]) -> Result<Vec<u8>, String> {
    let failed = |error: zip::result::ZipError| format!("The 3MF file could not be built: {error}");
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    for (name, bytes) in files {
        writer
            .start_file(name.as_str(), SimpleFileOptions::default())
            .map_err(failed)?;
        writer.write_all(bytes).map_err(|error| error.to_string())?;
    }
    Ok(writer.finish().map_err(failed)?.into_inner())
}

/// Reading a GLB back, for tests here and in `service.rs`.
#[cfg(test)]
pub(crate) mod testing {
    use data_encoding::BASE64;
    use serde_json::Value;

    /// A GLB's JSON and BIN chunks, after checking its header and chunks.
    pub fn read_glb(bytes: &[u8]) -> (Value, Vec<u8>) {
        let word = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap()) as usize;
        assert_eq!(&bytes[0..4], b"glTF");
        assert_eq!(word(4), 2, "version");
        assert_eq!(word(8), bytes.len(), "total length");
        let json_len = word(12);
        assert_eq!(&bytes[16..20], b"JSON");
        assert_eq!(json_len % 4, 0);
        let json: Value = serde_json::from_slice(&bytes[20..20 + json_len]).unwrap();
        let mut bin = Vec::new();
        let at = 20 + json_len;
        if at < bytes.len() {
            let bin_len = word(at);
            assert_eq!(&bytes[at + 4..at + 8], b"BIN\0");
            assert_eq!(bin_len % 4, 0);
            bin = bytes[at + 8..at + 8 + bin_len].to_vec();
            assert_eq!(at + 8 + bin_len, bytes.len());
        }
        (json, bin)
    }

    fn data_uri(uri: &str) -> Vec<u8> {
        BASE64
            .decode(uri.split_once(",").unwrap().1.as_bytes())
            .unwrap()
    }

    /// `glb` holds exactly the model `gltf` (a .gltf with data: URIs) holds:
    /// every buffer view's and image's bytes, and the same JSON otherwise.
    pub fn assert_same_model(glb: &[u8], gltf: &Value) {
        let (ours, bin) = read_glb(glb);
        let buffers: Vec<Vec<u8>> = gltf["buffers"]
            .as_array()
            .unwrap()
            .iter()
            .map(|b| data_uri(b["uri"].as_str().unwrap()))
            .collect();
        let num = |v: &Value, key: &str| v[key].as_u64().unwrap_or(0) as usize;
        for (i, view) in gltf["bufferViews"].as_array().unwrap().iter().enumerate() {
            let theirs =
                &buffers[num(view, "buffer")][num(view, "byteOffset")..][..num(view, "byteLength")];
            let mine = &ours["bufferViews"][i];
            assert_eq!(num(mine, "buffer"), 0);
            assert_eq!(
                &bin[num(mine, "byteOffset")..][..num(mine, "byteLength")],
                theirs,
                "view {i}"
            );
            for key in ["byteStride", "target"] {
                assert_eq!(mine.get(key), view.get(key), "view {i} {key}");
            }
        }
        for (i, image) in gltf["images"].as_array().into_iter().flatten().enumerate() {
            let mine = &ours["images"][i];
            let view = &ours["bufferViews"][num(mine, "bufferView")];
            assert_eq!(
                &bin[num(view, "byteOffset")..][..num(view, "byteLength")],
                data_uri(image["uri"].as_str().unwrap()),
                "image {i}"
            );
            assert!(mine.get("uri").is_none());
            assert_eq!(mine["mimeType"], image["mimeType"], "image {i}");
        }
        let rest = |v: &Value| {
            let mut v = v.clone();
            for key in ["buffers", "bufferViews", "images"] {
                v.as_object_mut().unwrap().remove(key);
            }
            v
        };
        assert_eq!(rest(&ours), rest(gltf));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arbesk::http::fake::{FakeHttp, not_found};
    use crate::arbesk::ipfs::{Gateway, ManifestCache};
    use serde_json::{Value, json};
    use std::collections::BTreeMap;
    use std::io::Read;
    use std::path::{Path, PathBuf};

    fn fixture(case: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/arbesk")
            .join(case)
    }

    /// A gateway serving the fixture's `cids/` folder.
    fn serving(case: &str) -> FakeHttp {
        let cids = fixture(case).join("cids");
        FakeHttp::new(move |request| {
            let cid = request.url.rsplit('/').next().unwrap();
            std::fs::read(cids.join(cid)).map_err(|_| not_found())
        })
    }

    fn compose_case(case: &str) -> Vec<u8> {
        let http = serving(case);
        let cache = tempfile::tempdir().unwrap();
        let gateway = Gateway::new(
            &http,
            "https://gw.test/ipfs/",
            ManifestCache::new(cache.path().into(), 1 << 20),
        );
        let source: Value =
            serde_json::from_slice(&std::fs::read(fixture(case).join("source.json")).unwrap())
                .unwrap();
        compose(&gateway, &source).unwrap()
    }

    fn entries(zip: &[u8]) -> BTreeMap<String, Vec<u8>> {
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(zip)).unwrap();
        (0..archive.len())
            .map(|i| {
                let mut file = archive.by_index(i).unwrap();
                let mut bytes = Vec::new();
                file.read_to_end(&mut bytes).unwrap();
                (file.name().unwrap().into_owned(), bytes)
            })
            .collect()
    }

    #[test]
    fn gltf_matches_arbesk() {
        for case in ["glb-box", "gltf-image"] {
            let ours: Value = serde_json::from_slice(&compose_case(case)).unwrap();
            let theirs: Value = serde_json::from_slice(
                &std::fs::read(fixture(case).join("expected.gltf")).unwrap(),
            )
            .unwrap();
            assert_eq!(ours, theirs, "{case}");
        }
    }

    fn preview_case(case: &str) -> Vec<u8> {
        let http = serving(case);
        let cache = tempfile::tempdir().unwrap();
        let gateway = Gateway::new(
            &http,
            "https://gw.test/ipfs/",
            ManifestCache::new(cache.path().into(), 1 << 20),
        );
        let source: Value =
            serde_json::from_slice(&std::fs::read(fixture(case).join("source.json")).unwrap())
                .unwrap();
        compose_preview(&gateway, &source).unwrap()
    }

    #[test]
    fn a_gltf_previews_as_a_glb_holding_what_arbesk_builds() {
        for case in ["glb-box", "gltf-image"] {
            let theirs: Value = serde_json::from_slice(
                &std::fs::read(fixture(case).join("expected.gltf")).unwrap(),
            )
            .unwrap();
            testing::assert_same_model(&preview_case(case), &theirs);
        }
    }

    #[test]
    fn a_3mf_previews_as_itself() {
        for case in ["3mf-box", "3mf-thumbnail"] {
            assert_eq!(
                entries(&preview_case(case)),
                entries(&compose_case(case)),
                "{case}"
            );
        }
    }

    /// A gateway answering each CID with its bytes from `parts`.
    fn parts_gateway(parts: &'static [(&'static str, &'static [u8])]) -> FakeHttp {
        FakeHttp::new(move |request| {
            let cid = request.url.rsplit('/').next().unwrap();
            parts
                .iter()
                .find(|(c, _)| *c == cid)
                .map(|(_, b)| b.to_vec())
                .ok_or_else(not_found)
        })
    }

    fn preview_of(http: &FakeHttp, source: Value) -> Result<Vec<u8>, String> {
        let cache = tempfile::tempdir().unwrap();
        let gateway = Gateway::new(
            http,
            "https://gw.test/ipfs",
            ManifestCache::new(cache.path().into(), 1 << 20),
        );
        compose_preview(&gateway, &source)
    }

    #[test]
    fn several_buffers_and_a_data_buffer_each_start_aligned() {
        let http = parts_gateway(&[("bafyA", b"AAAAA"), ("bafyB", b"BBBBBBB")]);
        let source = json!({
            "buffers": [
                { "uri": "ipfs://bafyA", "byteLength": 5, "_arbesk": { "hash": "x" } },
                { "uri": "data:application/octet-stream;base64,Q0ND", "byteLength": 3 },
                { "uri": "ipfs://bafyB", "byteLength": 7 }
            ],
            "bufferViews": [
                { "buffer": 0, "byteOffset": 1, "byteLength": 3 },
                { "buffer": 1, "byteLength": 3, "target": 34962 },
                { "buffer": 2, "byteOffset": 2, "byteLength": 4, "byteStride": 4 }
            ]
        });
        let (json, bin) = testing::read_glb(&preview_of(&http, source).unwrap());
        let view = |i: usize| {
            let v = &json["bufferViews"][i];
            let at = v["byteOffset"].as_u64().unwrap() as usize;
            (
                at,
                &bin[at..at + v["byteLength"].as_u64().unwrap() as usize],
            )
        };
        assert_eq!(view(0).1, b"AAA");
        assert_eq!(view(1).1, b"CCC");
        assert_eq!(view(2).1, b"BBBB");
        // Each buffer starts on a 4-byte boundary: views 1 and 2 are at their buffer's start + their offset.
        assert_eq!(view(1).0 % 4, 0);
        assert_eq!((view(2).0 - 2) % 4, 0);
        // The BIN chunk is padded to 4: up to 3 bytes longer than its buffer, as glTF allows.
        let declared = json["buffers"][0]["byteLength"].as_u64().unwrap() as usize;
        assert!(
            declared <= bin.len() && bin.len() - declared < 4,
            "{declared} vs {}",
            bin.len()
        );
        assert_eq!(json["buffers"].as_array().unwrap().len(), 1);
        assert_eq!(json["bufferViews"][1]["target"], 34962);
        assert_eq!(json["bufferViews"][2]["byteStride"], 4);
    }

    #[test]
    fn an_image_without_a_mime_type_gets_the_one_its_bytes_say() {
        let http = parts_gateway(&[
            ("bafyJ", &[0xff, 0xd8, 0xff, 0xe0, 1, 2]),
            ("bafyW", b"RIFF\x04\0\0\0WEBPxxxx"),
        ]);
        let source = json!({ "images": [{ "uri": "ipfs://bafyJ" }, { "uri": "ipfs://bafyW" }, { "uri": "data:image/png;base64,iVBORw0KGgo=" }] });
        let (json, _) = testing::read_glb(&preview_of(&http, source).unwrap());
        let mimes: Vec<&str> = json["images"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["mimeType"].as_str().unwrap())
            .collect();
        assert_eq!(mimes, ["image/jpeg", "image/webp", "image/png"]);
    }

    #[test]
    fn a_preview_refuses_what_it_can_not_place() {
        let http = parts_gateway(&[]);
        for source in [
            json!({ "buffers": [{ "uri": "https://elsewhere/x.bin" }] }),
            json!({ "buffers": [{ "byteLength": 4 }] }),
            json!({ "images": [{ "uri": "x.png" }] }),
            json!({ "buffers": [], "bufferViews": [{ "buffer": 3, "byteLength": 1 }] }),
            json!({ "buffers": [{ "uri": "data:application/octet-stream,plain" }] }),
            json!({ "arbesk_format": "composite-example" }),
        ] {
            assert_eq!(
                preview_of(&http, source.clone()).unwrap_err(),
                UNKNOWN_FORMAT,
                "{source}"
            );
        }
    }

    #[test]
    fn a_preview_with_nothing_to_place_is_a_glb_without_a_bin_chunk() {
        let (json, bin) = testing::read_glb(
            &preview_of(
                &parts_gateway(&[]),
                json!({ "asset": { "version": "2.0" } }),
            )
            .unwrap(),
        );
        assert!(bin.is_empty());
        assert_eq!(json, json!({ "asset": { "version": "2.0" } }));
    }

    #[test]
    fn a_preview_stops_once_its_parts_pass_the_cap() {
        let http = parts_gateway(&[("bafyA", &[7u8; 600]), ("bafyB", &[7u8; 600])]);
        let cache = tempfile::tempdir().unwrap();
        let gateway = Gateway::new(
            &http,
            "https://gw.test/ipfs",
            ManifestCache::new(cache.path().into(), 1 << 20),
        );
        let source = json!({ "buffers": [{ "uri": "ipfs://bafyA" }, { "uri": "ipfs://bafyB" }] });
        assert!(compose_preview_within(&gateway, &source, 1200).is_ok());
        assert_eq!(
            compose_preview_within(&gateway, &source, 1199).unwrap_err(),
            TOO_LARGE_TO_BUILD
        );
    }

    #[test]
    fn threemf_matches_arbesk() {
        for case in ["3mf-box", "3mf-thumbnail"] {
            let ours = entries(&compose_case(case));
            let theirs = entries(&std::fs::read(fixture(case).join("expected.3mf")).unwrap());
            assert_eq!(ours, theirs, "{case}");
        }
    }

    fn compose_offline(source: Value) -> Result<Vec<u8>, String> {
        let http = FakeHttp::new(|_| Ok(b"bytes".to_vec()));
        let cache = tempfile::tempdir().unwrap();
        let gateway = Gateway::new(
            &http,
            "https://gw.test/ipfs",
            ManifestCache::new(cache.path().into(), 1 << 20),
        );
        compose(&gateway, &source)
    }

    #[test]
    fn a_build_stops_once_its_parts_pass_the_cap() {
        let http = FakeHttp::new(|_| Ok(vec![7u8; 600]));
        let cache = tempfile::tempdir().unwrap();
        let gateway = Gateway::new(
            &http,
            "https://gw.test/ipfs",
            ManifestCache::new(cache.path().into(), 1 << 20),
        );
        let two_parts =
            json!({ "buffers": [{ "uri": "ipfs://bafyA" }, { "uri": "ipfs://bafyB" }] });
        assert!(compose_within(&gateway, &two_parts, 1200).is_ok());
        assert_eq!(
            compose_within(&gateway, &two_parts, 1199).unwrap_err(),
            TOO_LARGE_TO_BUILD
        );
        let threemf = json!({
            "arbesk_format": "composite-3mf", "contentTypes": "<x/>", "rootRels": "<x/>", "model": "<m/>",
            "parts": { "a.png": { "cid": "bafyA" }, "b.png": { "cid": "bafyB" } }
        });
        assert_eq!(
            compose_within(&gateway, &threemf, 1000).unwrap_err(),
            TOO_LARGE_TO_BUILD
        );
    }

    #[test]
    fn the_format_comes_from_the_marker() {
        assert_eq!(
            format_of(&json!({ "asset": { "version": "2.0" } })).unwrap(),
            Format::Gltf
        );
        assert_eq!(
            format_of(&json!({ "arbesk_format": "composite-3mf" })).unwrap(),
            Format::ThreeMf
        );
        assert_eq!(Format::Gltf.ext(), "gltf");
        assert_eq!(Format::ThreeMf.ext(), "3mf");
    }

    #[test]
    fn an_unknown_format_is_refused() {
        for source in [
            json!({ "arbesk_format": "composite-example" }),
            json!({ "arbesk_format": 3 }),
        ] {
            assert_eq!(format_of(&source).unwrap_err(), UNKNOWN_FORMAT);
            assert_eq!(compose_offline(source).unwrap_err(), UNKNOWN_FORMAT);
        }
    }

    #[test]
    fn a_link_the_composite_does_not_hold_is_refused() {
        for source in [
            json!({ "buffers": [{ "uri": "https://elsewhere/x.bin" }] }),
            json!({ "images": [{ "uri": "file:///etc/passwd" }] }),
            json!({ "buffers": [{ "uri": "x.bin" }] }),
        ] {
            assert_eq!(compose_offline(source).unwrap_err(), UNKNOWN_FORMAT);
        }
    }

    #[test]
    fn data_uris_and_items_without_one_stay() {
        let source = json!({
            "buffers": [{ "uri": "data:application/octet-stream;base64,AAAA", "byteLength": 3 }],
            "images": [{ "bufferView": 0, "mimeType": "image/png" }]
        });
        let out: Value = serde_json::from_slice(&compose_offline(source.clone()).unwrap()).unwrap();
        assert_eq!(out, source);
    }

    #[test]
    fn a_3mf_without_its_model_is_refused() {
        let source =
            json!({ "arbesk_format": "composite-3mf", "contentTypes": "<x/>", "rootRels": "<x/>" });
        assert!(compose_offline(source).is_err());
    }

    #[test]
    fn a_3mf_model_path_that_climbs_is_refused() {
        for model_path in ["../evil.model", "/etc/evil.model", "3D//x.model"] {
            let source = json!({
                "arbesk_format": "composite-3mf", "contentTypes": "<x/>", "rootRels": "<x/>", "model": "<m/>",
                "modelPath": model_path
            });
            assert_eq!(
                compose_offline(source).unwrap_err(),
                UNKNOWN_FORMAT,
                "{model_path}"
            );
        }
    }

    #[test]
    fn a_3mf_part_path_that_climbs_is_refused() {
        let source = json!({
            "arbesk_format": "composite-3mf", "contentTypes": "<x/>", "rootRels": "<x/>", "model": "<m/>",
            "parts": { "../evil.png": { "cid": "bafyP" } }
        });
        assert!(compose_offline(source).is_err());
    }

    #[test]
    fn the_source_of_a_wrapper_manifest_is_its_first_nodes() {
        let wrapper =
            json!({ "name": "Box", "scene": { "nodes": [{ "source": { "cid": "bafySrc" } }] } });
        assert_eq!(source_cid(&wrapper, "bafyM"), "bafySrc");
        let gltf =
            json!({ "buffers": [], "scene": { "nodes": [{ "source": { "cid": "bafySrc" } }] } });
        assert_eq!(source_cid(&gltf, "bafyM"), "bafyM");
        assert_eq!(source_cid(&json!({ "name": "Box" }), "bafyM"), "bafyM");
    }

    #[test]
    fn a_child_ref_is_noticed() {
        assert!(is_child_ref(
            &json!({ "scene": { "nodes": [{ "child_ref": { "tokenId": "4" } }] } })
        ));
        assert!(!is_child_ref(
            &json!({ "scene": { "nodes": [{ "source": { "cid": "x" } }] } })
        ));
        assert!(!is_child_ref(&json!({})));
    }
}
