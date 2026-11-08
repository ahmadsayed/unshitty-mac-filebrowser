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

/// The file `source` stands for, its parts fetched through `gateway`.
pub fn compose(gateway: &Gateway, source: &Value) -> Result<Vec<u8>, String> {
    compose_within(gateway, source, BUILD_CAP)
}

pub const TOO_LARGE_TO_BUILD: &str = "This asset is too large to open.";
/// The most an asset's parts may add up to.
pub const BUILD_CAP: u64 = 512 * 1024 * 1024;

/// `compose`, refusing once the parts fetched add up to more than `cap` bytes.
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
