//! Turning what Arbesk stores back into a file, as asset-core's `compose`
//! does: a composite glTF becomes a self-contained `.gltf` (every
//! `ipfs://` buffer and image as a `data:` URI), a composite 3MF becomes a
//! `.3mf` zip again. Anything else is refused rather than guessed at.

use std::collections::{HashMap, VecDeque};
use std::io::{Cursor, Write};
use std::sync::{Condvar, Mutex, MutexGuard};

use data_encoding::BASE64;
use serde_json::{Map, Value};
use zip::ZipWriter;
use zip::write::SimpleFileOptions;

use crate::http::TICK;
use crate::ipfs::Gateway;
use crate::limiter::READS;
use crate::progress::CANCELLED;

pub const UNKNOWN_FORMAT: &str = "This asset uses an Arbesk format File Browser can't read yet.";

const IPFS: &str = "ipfs://";
/// How many parts of unknown size are fetched at once: in pairs, charged
/// once both land, as every part was before the sliding window ([`Window`]).
/// Two keeps the latency win while bounding the bytes held before the
/// budget is checked to two parts.
const PARALLEL_PARTS: usize = 2;

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
/// `.gltf` (or `.3mf`) an asset is, as Arbesk builds it: what a copy out of
/// Arbesk writes (`service::export`). The preview uses `compose_preview`.
pub fn compose(gateway: &Gateway, source: &Value) -> Result<Vec<u8>, String> {
    compose_within(gateway, source, BUILD_CAP)
}

pub const TOO_LARGE_TO_BUILD: &str = "This asset is too large to open.";
/// The most an asset's parts may add up to.
pub const BUILD_CAP: u64 = 512 * 1024 * 1024;

/// `compose`, refusing once the parts fetched add up to more than `cap` bytes.
pub fn compose_within(gateway: &Gateway, source: &Value, cap: u64) -> Result<Vec<u8>, String> {
    let format = validate_source(source, false)?;
    Parts::build(gateway, source, format, cap, |parts| match format {
        Format::Gltf => compose_gltf(parts, source),
        Format::ThreeMf => compose_3mf(parts, source),
    })
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
    let format = validate_source(source, true)?;
    Parts::build(gateway, source, format, cap, |parts| match format {
        Format::Gltf => glb::compose(parts, source),
        Format::ThreeMf => compose_3mf(parts, source),
    })
}

/// Refuse everything that can be known before a part is fetched, in the order
/// the assemblers used to check it: the format, then every URI/path and the
/// 3MF fields the package needs. `preview` also checks the glTF URIs the
/// preview reads; the export assembler leaves a buffer without a URI and a
/// non-base64 `data:` URI alone, as it always has.
fn validate_source(source: &Value, preview: bool) -> Result<Format, String> {
    let format = format_of(source)?;
    match format {
        Format::Gltf if preview => validate_gltf(source)?,
        Format::Gltf => {}
        Format::ThreeMf => validate_3mf(source)?,
    }
    Ok(format)
}

/// The glTF URIs the preview's `resource_bytes`/`place_images` read.
fn validate_gltf(source: &Value) -> Result<(), String> {
    for buffer in array(source, "buffers") {
        let uri = buffer.get("uri").and_then(Value::as_str);
        if !is_resource_uri(uri) {
            return Err(UNKNOWN_FORMAT.into());
        }
    }
    for image in array(source, "images") {
        if let Some(uri) = image.get("uri").and_then(Value::as_str)
            && !is_resource_uri(Some(uri))
        {
            return Err(UNKNOWN_FORMAT.into());
        }
    }
    Ok(())
}

fn array<'a>(value: &'a Value, key: &str) -> impl Iterator<Item = &'a Map<String, Value>> {
    value
        .get(key)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_object)
}

/// Whether a URI is one [`resource_bytes`] can fetch or decode.
fn is_resource_uri(uri: Option<&str>) -> bool {
    let Some(uri) = uri else {
        return false;
    };
    if uri.starts_with(IPFS) {
        return true;
    }
    uri.strip_prefix("data:")
        .and_then(|data| data.split_once(','))
        .is_some_and(|(header, _)| header.ends_with(";base64"))
}

/// The 3MF package fields and paths `compose_3mf` needs, checked before the
/// parts are fetched.
fn validate_3mf(source: &Value) -> Result<(), String> {
    for key in ["contentTypes", "rootRels", "model"] {
        source
            .get(key)
            .and_then(Value::as_str)
            .ok_or_else(|| format!("The stored 3MF has no {key}."))?;
    }
    let model_path = source
        .get("modelPath")
        .and_then(Value::as_str)
        .unwrap_or("3D/3dmodel.model");
    part_path(model_path)?;
    for (path, part) in source
        .get("parts")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
    {
        part_path(path)?;
        part.get("cid")
            .and_then(Value::as_str)
            .ok_or(UNKNOWN_FORMAT)?;
    }
    Ok(())
}

/// One part a build fetches: its CID, how many references the source makes
/// to it, and its unpacked size when the source says (a buffer's
/// `byteLength`, else the `_arbesk.bytes` asset-core records on a part).
#[derive(Debug, Clone, PartialEq)]
struct Plan {
    cid: String,
    refs: usize,
    size: Option<u64>,
}

impl Plan {
    /// What the part will be charged, when its size is known.
    fn need(&self) -> Option<u64> {
        self.size.map(|size| size.saturating_mul(self.refs as u64))
    }
}

/// The parts of one build as they are fetched: the queue, the budget and
/// what has landed. Workers fetch through a sliding window (performance
/// plan, phase 2): a part whose size is known starts as soon as a worker is
/// free and its bytes, reserved before the fetch, fit the budget left after
/// the others in flight (alone, it starts anyway and is charged when it
/// lands, as before); parts of unknown size are fetched as before, in
/// pairs charged when they land, and never beside a known one. So the
/// bytes held never pass the cap by more than one part's, or two unknown
/// ones'. The assembler takes the parts in its own order.
struct Window {
    queue: VecDeque<Plan>,
    /// The budget left once what landed is charged.
    left: u64,
    /// What the known parts in flight will be charged.
    reserved: u64,
    known: usize,
    unknown: usize,
    /// Whether another unknown-size part may join the pair in flight.
    pair_open: bool,
    /// How many times the source still references each part.
    remaining: HashMap<String, usize>,
    /// Bytes fetched and not yet consumed by the assembler.
    fetched: HashMap<String, Vec<u8>>,
    /// The first failure: a read, the budget, a cancel. It fails the build.
    failed: Option<String>,
    /// The assembler is done: workers start nothing more.
    done: bool,
    /// Workers still running: once none is, nothing more lands.
    workers: usize,
}

impl Window {
    fn may_start(&self) -> bool {
        let Some(next) = self.queue.front() else {
            return false;
        };
        match next.need() {
            Some(need) => {
                self.unknown == 0
                    && (self.known == 0 || self.reserved.saturating_add(need) <= self.left)
            }
            None => {
                self.known == 0
                    && (self.unknown == 0 || (self.pair_open && self.unknown < PARALLEL_PARTS))
            }
        }
    }

    fn start(&mut self) -> Option<Plan> {
        let plan = self.queue.pop_front()?;
        match plan.need() {
            Some(need) => {
                self.known += 1;
                self.reserved = self.reserved.saturating_add(need);
            }
            None => {
                if self.unknown == 0 {
                    self.pair_open = true;
                }
                self.unknown += 1;
            }
        }
        Some(plan)
    }

    fn land(&mut self, plan: &Plan, answer: Result<Vec<u8>, String>) {
        match plan.need() {
            Some(need) => {
                self.known -= 1;
                self.reserved = self.reserved.saturating_sub(need);
            }
            None => {
                self.unknown -= 1;
                self.pair_open = false;
            }
        }
        if self.failed.is_some() {
            return;
        }
        match answer.and_then(|bytes| self.charge(bytes, plan.refs)) {
            Ok(bytes) => {
                self.fetched.insert(plan.cid.clone(), bytes);
            }
            Err(error) => self.failed = Some(error),
        }
    }

    /// Charge `bytes` once per reference, refusing once past the budget.
    fn charge(&mut self, bytes: Vec<u8>, refs: usize) -> Result<Vec<u8>, String> {
        let charged = (bytes.len() as u64).saturating_mul(refs as u64);
        self.left = self.left.checked_sub(charged).ok_or(TOO_LARGE_TO_BUILD)?;
        Ok(bytes)
    }

    /// One reference's bytes, released at the last one.
    fn take(&mut self, cid: &str) -> Result<Vec<u8>, String> {
        let refs = self.remaining.get_mut(cid).ok_or(UNKNOWN_FORMAT)?;
        *refs -= 1;
        let last = *refs == 0;
        if last {
            self.remaining.remove(cid);
            self.fetched.remove(cid).ok_or_else(|| UNKNOWN_FORMAT.to_string())
        } else {
            self.fetched
                .get(cid)
                .cloned()
                .ok_or_else(|| UNKNOWN_FORMAT.to_string())
        }
    }
}

/// The parts of one build, fetched within its budget (see [`Window`]).
struct Parts<'g> {
    gateway: &'g Gateway<'g>,
    window: Mutex<Window>,
    changed: Condvar,
}

impl<'g> Parts<'g> {
    /// `assemble`'s answer, the parts `source` references fetched for it
    /// meanwhile by at most `limiter::READS` workers, whose reads take
    /// their permits from the client's limiter.
    fn build<T>(
        gateway: &'g Gateway<'g>,
        source: &Value,
        format: Format,
        cap: u64,
        assemble: impl FnOnce(&Parts) -> Result<T, String>,
    ) -> Result<T, String> {
        let plans = part_plans(source, format);
        let workers = plans.len().min(READS);
        let parts = Parts {
            gateway,
            window: Mutex::new(Window {
                remaining: plans.iter().map(|plan| (plan.cid.clone(), plan.refs)).collect(),
                queue: plans.into(),
                left: cap,
                reserved: 0,
                known: 0,
                unknown: 0,
                pair_open: false,
                fetched: HashMap::new(),
                failed: None,
                done: false,
                workers,
            }),
            changed: Condvar::new(),
        };
        std::thread::scope(|scope| {
            for _ in 0..workers {
                scope.spawn(|| parts.work());
            }
            let built = assemble(&parts);
            parts.lock().done = true;
            parts.changed.notify_all();
            built
        })
    }

    fn lock(&self) -> MutexGuard<'_, Window> {
        self.window.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn wait<'a>(&self, window: MutexGuard<'a, Window>) -> MutexGuard<'a, Window> {
        match self.changed.wait_timeout(window, TICK) {
            Ok((window, _)) => window,
            Err(poisoned) => poisoned.into_inner().0,
        }
    }

    /// A worker: fetch the next part whenever the window lets it, until the
    /// queue is empty, the build failed or the assembler is done.
    fn work(&self) {
        /// Counts the worker out however it ends.
        struct Out<'a, 'g>(&'a Parts<'g>);
        impl Drop for Out<'_, '_> {
            fn drop(&mut self) {
                self.0.lock().workers -= 1;
                self.0.changed.notify_all();
            }
        }
        let _out = Out(self);
        loop {
            let plan = {
                let mut window = self.lock();
                loop {
                    if window.done || window.failed.is_some() || window.queue.is_empty() {
                        return;
                    }
                    if self.gateway.stopped() {
                        window.failed = Some(CANCELLED.into());
                        self.changed.notify_all();
                        return;
                    }
                    if window.may_start() {
                        break window.start();
                    }
                    window = self.wait(window);
                }
            };
            let Some(plan) = plan else { return };
            let answer = self.gateway.bytes(&plan.cid);
            self.lock().land(&plan, answer);
            self.changed.notify_all();
        }
    }

    /// The bytes of `cid`, once a worker has them. A part is charged for
    /// every reference, and its bytes are released after the last one. A
    /// part the source did not declare (or asked for past its references)
    /// still gets one reference and is fetched here, so a caller can never
    /// skip it silently. Any failure of the build fails it.
    fn fetch(&self, cid: &str) -> Result<Vec<u8>, String> {
        let mut window = self.lock();
        if !window.remaining.contains_key(cid) {
            window.remaining.insert(cid.to_string(), 1);
            drop(window);
            let answer = self.gateway.bytes(cid);
            window = self.lock();
            let bytes = answer.and_then(|bytes| window.charge(bytes, 1))?;
            window.fetched.insert(cid.to_string(), bytes);
        }
        loop {
            if window.fetched.contains_key(cid) {
                return window.take(cid);
            }
            if let Some(error) = &window.failed {
                return Err(error.clone());
            }
            if window.workers == 0 {
                // A worker ended without landing its part (it panicked).
                return Err("The download stopped unexpectedly.".into());
            }
            window = self.wait(window);
        }
    }
}

/// Every part the source references, in first-seen order, with how many
/// references and its size where the source gives one: glTF buffers and
/// images, or 3MF package parts. Data URIs are in the source itself. A part
/// referenced with two sizes, or once without one, counts as unknown.
fn part_plans(source: &Value, format: Format) -> Vec<Plan> {
    let mut index: HashMap<String, usize> = HashMap::new();
    let mut order: Vec<Plan> = Vec::new();
    let mut add = |cid: &str, item: &Map<String, Value>| {
        let size = item
            .get("byteLength")
            .and_then(Value::as_u64)
            .or_else(|| item.get("_arbesk")?.get("bytes")?.as_u64());
        if let Some(at) = index.get(cid).copied() {
            let plan = &mut order[at];
            plan.refs += 1;
            if plan.size != size {
                plan.size = None;
            }
        } else {
            index.insert(cid.to_string(), order.len());
            order.push(Plan {
                cid: cid.to_string(),
                refs: 1,
                size,
            });
        }
    };
    match format {
        Format::Gltf => {
            for key in ["buffers", "images"] {
                for item in array(source, key) {
                    if let Some(cid) = item
                        .get("uri")
                        .and_then(Value::as_str)
                        .and_then(|uri| uri.strip_prefix(IPFS))
                    {
                        add(cid, item);
                    }
                }
            }
        }
        Format::ThreeMf => {
            for part in source
                .get("parts")
                .and_then(Value::as_object)
                .into_iter()
                .flatten()
                .filter_map(|(_, part)| part.as_object())
            {
                if let Some(cid) = part.get("cid").and_then(Value::as_str) {
                    add(cid, part);
                }
            }
        }
    }
    order
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
    use crate::http::fake::{FakeHttp, not_found};
    use crate::ipfs::{Gateway, ManifestCache};
    use serde_json::{Value, json};
    use std::collections::BTreeMap;
    use std::io::Read;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Condvar, Mutex};
    use std::time::{Duration, Instant};

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

    /// A gateway that answers every CID with `bytes` and counts the requests.
    fn counting_gateway(bytes: &'static [u8]) -> (FakeHttp, Arc<AtomicUsize>) {
        let count = Arc::new(AtomicUsize::new(0));
        let counted = count.clone();
        let http = FakeHttp::new(move |_| {
            counted.fetch_add(1, Ordering::SeqCst);
            Ok(bytes.to_vec())
        });
        (http, count)
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

    /// A gateway serving `parts` that waits up to a second for a second
    /// request and records the most requests in flight at once.
    fn parallel_parts_gateway(
        parts: &'static [(&'static str, &'static [u8])],
    ) -> (FakeHttp, Arc<AtomicUsize>) {
        let running = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let waiting = Arc::new((Mutex::new(0usize), Condvar::new()));
        let (r, p, w) = (running.clone(), peak.clone(), waiting.clone());
        let http = FakeHttp::new(move |request| {
            let now = r.fetch_add(1, Ordering::SeqCst) + 1;
            p.fetch_max(now, Ordering::SeqCst);
            let (lock, ready) = &*w;
            let mut started = lock.lock().unwrap();
            *started += 1;
            ready.notify_all();
            let deadline = Instant::now() + Duration::from_secs(1);
            while *started < 2 {
                let left = deadline.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    break;
                }
                started = ready.wait_timeout(started, left).unwrap().0;
            }
            drop(started);
            let cid = request.url.rsplit('/').next().unwrap();
            let answer = parts
                .iter()
                .find(|(c, _)| *c == cid)
                .map(|(_, b)| b.to_vec())
                .ok_or_else(not_found);
            r.fetch_sub(1, Ordering::SeqCst);
            answer
        });
        (http, peak)
    }

    #[test]
    fn parts_are_fetched_in_parallel() {
        let (http, peak) =
            parallel_parts_gateway(&[("bafyA", b"AAAAA"), ("bafyB", b"BBBBBBB")]);
        let source = json!({
            "buffers": [
                { "uri": "ipfs://bafyA", "byteLength": 5 },
                { "uri": "ipfs://bafyB", "byteLength": 7 }
            ]
        });
        let (json, bin) = testing::read_glb(&preview_of(&http, source).unwrap());
        assert_eq!(json["buffers"].as_array().unwrap().len(), 1);
        assert!(bin.windows(5).any(|bytes| bytes == b"AAAAA"));
        assert!(bin.windows(7).any(|bytes| bytes == b"BBBBBBB"));
        assert_eq!(peak.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn an_unknown_format_is_refused_without_fetching_parts() {
        let http = FakeHttp::new(|_| panic!("no part should be fetched"));
        let source = json!({ "arbesk_format": "composite-example", "parts": { "a": { "cid": "bafyP" } } });
        assert_eq!(preview_of(&http, source).unwrap_err(), UNKNOWN_FORMAT);
        assert!(http.requests().is_empty());
    }

    #[test]
    fn a_bad_3mf_path_is_refused_without_fetching_parts() {
        let http = FakeHttp::new(|_| panic!("no part should be fetched"));
        let source = json!({
            "arbesk_format": "composite-3mf",
            "contentTypes": "x",
            "rootRels": "x",
            "model": "x",
            "modelPath": "../model",
            "parts": { "a": { "cid": "bafyP" } }
        });
        assert_eq!(preview_of(&http, source).unwrap_err(), UNKNOWN_FORMAT);
        assert!(http.requests().is_empty());
    }

    #[test]
    fn a_missing_part_is_still_the_storage_error() {
        let http = parts_gateway(&[]);
        let source = json!({ "buffers": [{ "uri": "ipfs://bafyGone" }] });
        let error = preview_of(&http, source).unwrap_err();
        assert!(error.starts_with("Arbesk's storage has nothing at bafyGone."), "{error}");
    }

    #[test]
    fn a_build_over_its_cap_is_refused_after_one_batch() {
        let (http, requests) = counting_gateway(b"0123456789");
        let buffers: Vec<Value> = (0..8)
            .map(|i| json!({ "uri": format!("ipfs://bafyP{i}") }))
            .collect();
        let source = json!({ "buffers": buffers });
        let cache = tempfile::tempdir().unwrap();
        let gateway = Gateway::new(
            &http,
            "https://gw.test/ipfs",
            ManifestCache::new(cache.path().into(), 1 << 20),
        );
        assert_eq!(
            compose_preview_within(&gateway, &source, 15).unwrap_err(),
            TOO_LARGE_TO_BUILD
        );
        assert!(requests.load(Ordering::SeqCst) <= PARALLEL_PARTS);
    }

    /// A source whose buffers are `cids`, each `size` bytes by its
    /// `byteLength`.
    fn sized_buffers(cids: &[&str], size: u64) -> Value {
        let buffers: Vec<Value> = cids
            .iter()
            .map(|cid| json!({ "uri": format!("ipfs://{cid}"), "byteLength": size }))
            .collect();
        json!({ "buffers": buffers })
    }

    fn gateway_for<'a>(http: &'a FakeHttp, cache: &Path) -> Gateway<'a> {
        Gateway::new(
            http,
            "https://gw.test/ipfs",
            ManifestCache::new(cache.into(), 1 << 20),
        )
    }

    /// Performance plan, phase 2: parts go through a sliding window, not
    /// fixed pairs. A slow part holds only its own slot: every other part
    /// is asked for while it is still on its way.
    #[test]
    fn a_slow_part_does_not_idle_the_others() {
        let asked = Arc::new((Mutex::new(Vec::<String>::new()), Condvar::new()));
        let others_asked_meanwhile = Arc::new(AtomicUsize::new(0));
        let (seen, meanwhile) = (asked.clone(), others_asked_meanwhile.clone());
        let http = FakeHttp::new(move |request| {
            let cid = request.url.rsplit('/').next().unwrap().to_string();
            let (lock, changed) = &*seen;
            let mut asked = lock.lock().unwrap();
            asked.push(cid.clone());
            changed.notify_all();
            if cid == "bafySlow" {
                // Waits (up to two seconds) for the five others.
                let deadline = Instant::now() + Duration::from_secs(2);
                while asked.len() < 6 && Instant::now() < deadline {
                    let left = deadline.saturating_duration_since(Instant::now());
                    asked = changed.wait_timeout(asked, left).unwrap().0;
                }
                meanwhile.store(asked.len() - 1, Ordering::SeqCst);
            }
            // Each part its own byte: the slow one 0, bafyF<n> n.
            let byte = cid.strip_prefix("bafyF").map_or(0, |n| n.parse().unwrap());
            Ok(vec![byte; 10])
        });
        let cache = tempfile::tempdir().unwrap();
        let source = sized_buffers(&["bafySlow", "bafyF1", "bafyF2", "bafyF3", "bafyF4", "bafyF5"], 10);
        let (json, bin) = testing::read_glb(&compose_preview(&gateway_for(&http, cache.path()), &source).unwrap());
        assert_eq!(json["buffers"].as_array().unwrap().len(), 1);
        // Assembled in order: six buffers of ten bytes, each started on a
        // multiple of four.
        let starts: Vec<u8> = (0..6).map(|i| bin[i * 12]).collect();
        assert_eq!(starts, [0, 1, 2, 3, 4, 5]);
        assert_eq!(others_asked_meanwhile.load(Ordering::SeqCst), 5);
        assert_eq!(http.requests().len(), 6);
    }

    /// The build's budget is reserved before a part of known size is
    /// fetched: with several parts in flight, what landed and what is on
    /// its way never pass the cap. Fixed pairs asked for two more parts
    /// with only 50 bytes left.
    #[test]
    fn parts_in_flight_never_overshoot_the_budget() {
        let state = Arc::new(Mutex::new((0u64, 0u64, 0usize, false)));
        let seen = state.clone();
        let http = FakeHttp::new(move |_| {
            {
                let mut s = seen.lock().unwrap();
                s.1 += 100;
                s.2 += 1;
                // Landed plus on its way, while several are in flight.
                if s.2 >= 2 && s.0 + s.1 > 250 {
                    s.3 = true;
                }
            }
            std::thread::sleep(Duration::from_millis(30));
            let mut s = seen.lock().unwrap();
            s.1 -= 100;
            s.2 -= 1;
            s.0 += 100;
            Ok(vec![1u8; 100])
        });
        let cache = tempfile::tempdir().unwrap();
        let source = sized_buffers(&["bafyP0", "bafyP1", "bafyP2", "bafyP3"], 100);
        let gateway = gateway_for(&http, cache.path());
        assert_eq!(
            compose_preview_within(&gateway, &source, 250).unwrap_err(),
            TOO_LARGE_TO_BUILD
        );
        assert!(!state.lock().unwrap().3, "the budget was overshot");
        // What fits is built as before.
        assert!(compose_preview_within(&gateway, &source, 400).is_ok());
    }

    /// One failed read among parts fetched together fails the build with
    /// its own words, as a failed read in a pair did.
    #[test]
    fn one_failed_part_among_many_fails_the_build() {
        let http = FakeHttp::new(|request| {
            if request.url.ends_with("/bafyP3") {
                return Err(not_found());
            }
            Ok(vec![1u8; 10])
        });
        let cache = tempfile::tempdir().unwrap();
        let cids = ["bafyP0", "bafyP1", "bafyP2", "bafyP3", "bafyP4", "bafyP5"];
        let gateway = gateway_for(&http, cache.path());
        for source in [sized_buffers(&cids, 10), {
            let buffers: Vec<Value> = cids.iter().map(|cid| json!({ "uri": format!("ipfs://{cid}") })).collect();
            json!({ "buffers": buffers })
        }] {
            let error = compose_preview(&gateway, &source).unwrap_err();
            assert!(error.starts_with("Arbesk's storage has nothing at bafyP3."), "{error}");
            assert_eq!(compose(&gateway, &source).unwrap_err(), error);
        }
    }

    #[test]
    fn a_part_size_comes_from_byte_length_or_its_recorded_bytes() {
        let source = json!({
            "buffers": [
                { "uri": "ipfs://bafyB", "byteLength": 12, "_arbesk": { "bytes": 99 } },
                { "uri": "ipfs://bafyB", "byteLength": 12 },
                { "uri": "data:application/octet-stream;base64,AA==", "byteLength": 1 }
            ],
            "images": [
                { "uri": "ipfs://bafyI", "_arbesk": { "bytes": 70 } },
                { "uri": "ipfs://bafyN" },
                { "uri": "ipfs://bafyMixed", "_arbesk": { "bytes": 5 } },
                { "uri": "ipfs://bafyMixed" }
            ]
        });
        let plan = |cid: &str, refs: usize, size: Option<u64>| Plan { cid: cid.into(), refs, size };
        assert_eq!(
            part_plans(&source, Format::Gltf),
            [
                plan("bafyB", 2, Some(12)),
                plan("bafyI", 1, Some(70)),
                plan("bafyN", 1, None),
                plan("bafyMixed", 2, None)
            ]
        );
        assert_eq!(plan("bafyB", 2, Some(12)).need(), Some(24));
        let threemf = json!({ "parts": { "a.png": { "cid": "bafyA", "_arbesk": { "bytes": 70 } }, "b.png": { "cid": "bafyB" } } });
        assert_eq!(
            part_plans(&threemf, Format::ThreeMf),
            [plan("bafyA", 1, Some(70)), plan("bafyB", 1, None)]
        );
    }

    /// A build through the app's limiter: its parts take read permits,
    /// and a cancel while they wait for one stops the build at once.
    #[test]
    fn a_build_waiting_for_a_permit_stops_on_a_cancel() {
        use crate::limiter::{Limited, Limiter, never};
        let http = FakeHttp::new(|_| Ok(vec![1u8; 10]));
        let limiter = Limiter::new(2, 1);
        let held: Vec<_> = (0..2).map(|_| limiter.read(&never).unwrap()).collect();
        let started = Instant::now();
        let stop = move || started.elapsed() > Duration::from_millis(50);
        let limited = Limited::new(&http, &limiter, &stop);
        let cache = tempfile::tempdir().unwrap();
        let gateway = Gateway::new(
            &limited,
            "https://gw.test/ipfs",
            ManifestCache::new(cache.path().into(), 1 << 20),
        );
        let source = sized_buffers(&["bafyP0", "bafyP1", "bafyP2"], 10);
        assert_eq!(compose_preview(&gateway, &source).unwrap_err(), crate::progress::CANCELLED);
        assert!(started.elapsed() < Duration::from_secs(1), "{:?}", started.elapsed());
        assert!(http.requests().is_empty());
        drop(held);
        assert_eq!(limiter.reading(), 0);
        // With permits free, the same build is made through them.
        let limited = Limited::new(&http, &limiter, &never);
        let gateway = Gateway::new(
            &limited,
            "https://gw.test/ipfs",
            ManifestCache::new(cache.path().into(), 1 << 20),
        );
        assert!(compose_preview(&gateway, &source).is_ok());
        assert_eq!(limiter.reading(), 0);
    }

    #[test]
    fn a_duplicate_part_is_charged_for_every_reference() {
        let http = parts_gateway(&[("bafySame", b"0123456789")]);
        let source = json!({
            "buffers": [
                { "uri": "ipfs://bafySame" },
                { "uri": "ipfs://bafySame" }
            ]
        });
        let cache = tempfile::tempdir().unwrap();
        let gateway = Gateway::new(
            &http,
            "https://gw.test/ipfs",
            ManifestCache::new(cache.path().into(), 1 << 20),
        );
        assert_eq!(
            compose_preview_within(&gateway, &source, 15).unwrap_err(),
            TOO_LARGE_TO_BUILD
        );
        assert_eq!(http.requests().len(), 1);
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
    fn an_export_leaves_what_embed_leaves() {
        let http = FakeHttp::new(|_| panic!("nothing is fetched"));
        let source = json!({
            "buffers": [{ "byteLength": 4 }],
            "images": [{ "uri": "data:image/png,plain" }]
        });
        let cache = tempfile::tempdir().unwrap();
        let gateway = Gateway::new(
            &http,
            "https://gw.test/ipfs",
            ManifestCache::new(cache.path().into(), 1 << 20),
        );
        let out: Value = serde_json::from_slice(&compose(&gateway, &source).unwrap()).unwrap();
        assert!(out["buffers"][0].get("uri").is_none());
        assert_eq!(out["images"][0]["uri"], "data:image/png,plain");
        assert!(http.requests().is_empty());
    }

    #[test]
    fn a_preview_refuses_the_same_missing_uri() {
        let http = FakeHttp::new(|_| panic!("nothing is fetched before validation"));
        let source = json!({ "buffers": [{ "byteLength": 4 }] });
        assert_eq!(preview_of(&http, source).unwrap_err(), UNKNOWN_FORMAT);
        assert!(http.requests().is_empty());
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
