//! An asset's other files (its `representations` list), read and carried
//! forward as Arbesk's asset-core does (`manifest/representations.ts`,
//! `manifest/schema.ts`, `manifest/representation-codec.ts`) and as `besk
//! attach` writes them (spec F1). Everything works on the stored
//! `serde_json::Value`, so a manifest is never re-serialised through a
//! struct that would drop a key this app doesn't know: [`Rep`] only reads.
//!
//! Checked against asset-core's own answers:
//! `tests/fixtures/representations-stale.json`, made by
//! `scripts/make-representations-fixture.ts` from an Arbesk checkout.

use std::collections::{HashMap, HashSet};
use std::io::Read;

use serde::Serialize;
use serde_json::{Map, Value, json};

/// The reserved id of the asset's own model (`scene.nodes[0].source`).
pub(crate) const PRIMARY: &str = "primary";
/// Most entries one asset manifest may hold (writers check it), and most
/// formats a listing's summary carries.
pub(crate) const MAX_REPRESENTATIONS: usize = 32;
/// Largest file (decoded bytes) one entry may record: 256 MiB.
pub(crate) const MAX_BYTES: u64 = 256 * 1024 * 1024;
/// Most keys a `tool`, `profile` or `stats` map may hold.
const MAX_DETAIL_KEYS: usize = 32;
/// The frame asset-core puts before a brotli stream (`utils/brotli-frame.ts`).
const BROTLI_MAGIC: &[u8] = b"ARB\x01";
/// The quality `compressAuto` uses with the `binary` hint.
const BROTLI_QUALITY: u32 = 5;

pub(crate) const DAMAGED: &str = "Arbesk sent a damaged file.";
pub(crate) const TOO_LARGE: &str = "This file is too large to open.";

// ---- One entry ------------------------------------------------------------------

/// How an entry's bytes are stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Encoding {
    Raw,
    Brotli,
}

/// What an entry was made from.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct DerivedFrom {
    #[serde(rename = "ref")]
    pub(crate) reference: String,
    pub(crate) cid: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) transform_matrix: Option<Vec<f64>>,
}

/// A valid entry, as `representationSchema` parses it. Read only: writes
/// change the stored JSON, never a `Rep` turned back into JSON.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Rep {
    pub(crate) id: String,
    pub(crate) role: String,
    pub(crate) format: String,
    pub(crate) name: String,
    pub(crate) cid: String,
    pub(crate) bytes: u64,
    pub(crate) encoding: Encoding,
    pub(crate) derived_from: Option<DerivedFrom>,
    pub(crate) tool: Option<Map<String, Value>>,
    pub(crate) profile: Option<Map<String, Value>>,
    pub(crate) stats: Option<Map<String, Value>>,
    pub(crate) created_at: f64,
}

const ROLES: [&str; 5] = ["source", "mesh", "view", "machine", "other"];

/// `[a-z0-9-]{1,64}`, never `primary`: an entry's id.
pub(crate) fn valid_id(id: &str) -> bool {
    id != PRIMARY && is_slug(id, 64, true)
}

/// 1 to `max` ASCII lower-case letters and digits (and `-` when allowed).
fn is_slug(text: &str, max: usize, dash: bool) -> bool {
    (1..=max).contains(&text.len())
        && text
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || (dash && b == b'-'))
}

/// A string Zod's `min(1)` accepts.
fn non_empty(value: &Value) -> Option<String> {
    value.as_str().filter(|text| !text.is_empty()).map(String::from)
}

/// An optional key: absent is fine (`Ok(None)`), anything `check` refuses
/// (JSON `null` included) makes the entry invalid.
fn optional<T>(value: Option<&Value>, check: impl FnOnce(&Value) -> Option<T>) -> Result<Option<T>, ()> {
    match value {
        None => Ok(None),
        Some(value) => check(value).map(Some).ok_or(()),
    }
}

/// `tool`, `profile`, `stats`: at most 32 keys, each a string or a number.
fn detail(value: &Value) -> Option<Map<String, Value>> {
    let map = value.as_object()?;
    let plain = map.values().all(|v| v.is_string() || v.is_number());
    (plain && map.len() <= MAX_DETAIL_KEYS).then(|| map.clone())
}

fn derived_from(value: &Value) -> Option<DerivedFrom> {
    let object = value.as_object()?;
    let reference = object.get("ref")?.as_str()?;
    if reference != PRIMARY && !valid_id(reference) {
        return None;
    }
    let cid = non_empty(object.get("cid")?)?;
    let transform_matrix = optional(object.get("transform_matrix"), |matrix| {
        let values = matrix.as_array().filter(|values| values.len() == 16)?;
        values.iter().map(Value::as_f64).collect::<Option<Vec<f64>>>()
    })
    .ok()?;
    Some(DerivedFrom {
        reference: reference.into(),
        cid,
        transform_matrix,
    })
}

/// `bytes`: an integer-valued number from 0 to 256 MiB.
fn size(value: &Value) -> Option<u64> {
    let bytes = value.as_f64()?;
    let whole = bytes.fract() == 0.0 && (0.0..=MAX_BYTES as f64).contains(&bytes);
    whole.then_some(bytes as u64)
}

/// `entry` checked exactly as `representationSchema` checks it; `None`
/// when it doesn't parse.
pub(crate) fn parse_entry(entry: &Value) -> Option<Rep> {
    let object = entry.as_object()?;
    let field = |key: &str| object.get(key).unwrap_or(&Value::Null);
    let id = field("id").as_str().filter(|id| valid_id(id))?;
    let role = field("role").as_str().filter(|role| ROLES.contains(role))?;
    let format = field("format").as_str().filter(|format| is_slug(format, 16, false))?;
    let name = field("name").as_str()?;
    // Zod's `max(255)` counts UTF-16 units.
    if name.is_empty() || name.encode_utf16().count() > 255 {
        return None;
    }
    let encoding = match field("encoding").as_str()? {
        "raw" => Encoding::Raw,
        "brotli" => Encoding::Brotli,
        _ => return None,
    };
    Some(Rep {
        id: id.into(),
        role: role.into(),
        format: format.into(),
        name: name.into(),
        cid: non_empty(field("cid"))?,
        bytes: size(field("bytes"))?,
        encoding,
        derived_from: optional(object.get("derived_from"), derived_from).ok()?,
        tool: optional(object.get("tool"), detail).ok()?,
        profile: optional(object.get("profile"), detail).ok()?,
        stats: optional(object.get("stats"), detail).ok()?,
        created_at: field("created_at").as_f64()?,
    })
}

// ---- The list ---------------------------------------------------------------------

/// The stored entries, valid or not: none unless the list is an array.
fn stored(manifest: &Value) -> &[Value] {
    manifest["representations"]
        .as_array()
        .map_or(&[], Vec::as_slice)
}

/// `listRepresentations`: the valid entries in stored order. An entry that
/// doesn't parse is left out, never an error.
pub(crate) fn list(manifest: &Value) -> Vec<Rep> {
    stored(manifest).iter().filter_map(parse_entry).collect()
}

/// How many entries are stored, valid or not (what `hasRoomFor` counts).
pub(crate) fn stored_count(manifest: &Value) -> usize {
    stored(manifest).len()
}

/// The `id` of each stored entry that has a string one, valid or not
/// (`hasRoomFor`'s `entry?.id`): every id decision reads these.
pub(crate) fn stored_ids(manifest: &Value) -> Vec<String> {
    stored(manifest)
        .iter()
        .filter_map(|entry| entry["id"].as_str().map(String::from))
        .collect()
}

// ---- Staleness ----------------------------------------------------------------------

/// `currentCidOf`: the model's `/scene/nodes/0/source/cid` when it is a
/// non-empty string, or the `cid` of the first **valid** entry with the id
/// `reference`; `None` when it names nothing.
pub(crate) fn current_cid_of(manifest: &Value, reference: &str) -> Option<String> {
    if reference == PRIMARY {
        let node = js_member(&js_member(&js_member(manifest, "scene")?, "nodes")?, "0")?;
        return js_member(&js_member(&node, "source")?, "cid")
            .as_ref()
            .and_then(non_empty);
    }
    list(manifest)
        .into_iter()
        .find(|rep| rep.id == reference)
        .map(|rep| rep.cid)
}

const IDENTITY: [f64; 16] = [
    1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
];

/// `primaryTransformOf`: `None` without a node 0, its `transform_matrix`
/// (16 values, each through `ToNumber`) or the identity.
pub(crate) fn primary_transform_of(manifest: &Value) -> Option<Vec<f64>> {
    let node = js_member(&js_member(&js_member(manifest, "scene")?, "nodes")?, "0")?;
    if !truthy(&node) {
        return None;
    }
    match js_member(&node, "transform_matrix") {
        Some(Value::Array(matrix)) if matrix.len() == 16 => {
            Some(matrix.iter().map(to_number).collect())
        }
        _ => Some(IDENTITY.to_vec()),
    }
}

/// What staleness compares against, read once per manifest: the model's
/// CID and placement, and the CID of the first valid entry of each id (as
/// `currentCidOf` finds it). Each entry's check is then a lookup, so a
/// long list costs one pass, never one per entry.
pub(crate) struct Current {
    primary_cid: Option<String>,
    primary_transform: Option<Vec<f64>>,
    cids: HashMap<String, String>,
}

impl Current {
    pub(crate) fn of(manifest: &Value, reps: &[Rep]) -> Self {
        let mut cids = HashMap::new();
        for rep in reps {
            cids.entry(rep.id.clone()).or_insert_with(|| rep.cid.clone());
        }
        Current {
            primary_cid: current_cid_of(manifest, PRIMARY),
            primary_transform: primary_transform_of(manifest),
            cids,
        }
    }

    fn cid_of(&self, reference: &str) -> Option<&str> {
        if reference == PRIMARY {
            self.primary_cid.as_deref()
        } else {
            self.cids.get(reference).map(String::as_str)
        }
    }
}

/// `isStale`: no `derived_from` is never stale; a ref whose current CID
/// differs (or is gone) is; for `primary` with a recorded matrix, a
/// different placement is.
#[cfg_attr(not(test), allow(dead_code))] // The view and the command use `is_stale_in`.
pub(crate) fn is_stale(manifest: &Value, rep: &Rep) -> bool {
    is_stale_in(&Current::of(manifest, &list(manifest)), rep)
}

/// [`is_stale`] against a manifest's [`Current`].
pub(crate) fn is_stale_in(current: &Current, rep: &Rep) -> bool {
    let Some(from) = &rep.derived_from else {
        return false;
    };
    if current.cid_of(&from.reference) != Some(from.cid.as_str()) {
        return true;
    }
    let (true, Some(recorded)) = (from.reference == PRIMARY, &from.transform_matrix) else {
        return false;
    };
    match &current.primary_transform {
        // f64 equality, as `===`: NaN never equals, -0 equals 0.
        Some(now) => now.len() != recorded.len() || now.iter().zip(recorded).any(|(a, b)| a != b),
        None => true,
    }
}

/// `derivedFromOf`: what a new entry made from `reference` records (the
/// ref, its current CID and, for `primary`, the model's placement); `None`
/// when the ref names nothing.
pub(crate) fn derived_from_of(manifest: &Value, reference: &str) -> Option<Value> {
    let cid = current_cid_of(manifest, reference)?;
    let mut from = json!({ "ref": reference, "cid": cid });
    if reference == PRIMARY
        && let Some(matrix) = primary_transform_of(manifest)
    {
        // As JavaScript writes the numbers: `2`, never `2.0`.
        from["transform_matrix"] = Value::Array(matrix.into_iter().map(js_value).collect());
    }
    Some(from)
}

/// `n` as `JSON.stringify` writes it: a whole number without a fraction,
/// NaN and the infinities as `null`.
pub(crate) fn js_value(n: f64) -> Value {
    if n.fract() == 0.0 && n.abs() < 9_007_199_254_740_992.0 {
        json!(n as i64)
    } else {
        serde_json::Number::from_f64(n).map_or(Value::Null, Value::Number)
    }
}

// ---- JavaScript's reading of a value ----------------------------------------------------

/// `value?.[key]` as JavaScript reads it: an object's key, an array's
/// index, a string's character; `None` for `undefined`.
fn js_member(value: &Value, key: &str) -> Option<Value> {
    match value {
        Value::Object(map) => map.get(key).cloned(),
        Value::Array(items) => key.parse::<usize>().ok().and_then(|i| items.get(i)).cloned(),
        Value::String(text) => {
            let units: Vec<u16> = text.encode_utf16().collect();
            let unit = *units.get(key.parse::<usize>().ok()?)?;
            Some(Value::String(String::from_utf16_lossy(&[unit])))
        }
        _ => None,
    }
}

fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|n| n != 0.0 && !n.is_nan()),
        Value::String(text) => !text.is_empty(),
        Value::Array(_) | Value::Object(_) => true,
    }
}

/// ECMAScript's `ToNumber` of a JSON value.
pub(crate) fn to_number(value: &Value) -> f64 {
    match value {
        Value::Null => 0.0,
        Value::Bool(b) => f64::from(u8::from(*b)),
        Value::Number(n) => n.as_f64().unwrap_or(f64::NAN),
        Value::String(text) => string_to_number(text),
        Value::Array(_) => string_to_number(&js_string(value)),
        Value::Object(_) => f64::NAN,
    }
}

/// `String(value)` as JavaScript writes it, where it decides a number: an
/// array is its elements joined with `,` (`null` as empty).
fn js_string(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => js_number(n.as_f64().unwrap_or(f64::NAN)),
        Value::String(text) => text.clone(),
        Value::Array(items) => items.iter().map(js_string).collect::<Vec<_>>().join(","),
        Value::Object(_) => "[object Object]".into(),
    }
}

/// `String(number)`: an integer without a fraction, an exponent from 1e21
/// up and below 1e-6, the shortest digits that read back otherwise.
pub(crate) fn js_number(n: f64) -> String {
    if n.is_nan() {
        return "NaN".into();
    }
    if n.is_infinite() {
        return if n > 0.0 { "Infinity" } else { "-Infinity" }.into();
    }
    if n == 0.0 {
        return "0".into();
    }
    let magnitude = n.abs();
    if (1e-6..1e21).contains(&magnitude) {
        return format!("{n}");
    }
    let exp = format!("{n:e}");
    match exp.split_once('e') {
        Some((digits, power)) if !power.starts_with('-') => format!("{digits}e+{power}"),
        _ => exp,
    }
}

/// JavaScript's white space and line terminators, which `StringToNumber` trims.
fn js_space(c: char) -> bool {
    matches!(
        c,
        '\u{9}' | '\u{a}' | '\u{b}' | '\u{c}' | '\u{d}' | ' ' | '\u{a0}' | '\u{1680}'
            | '\u{2000}'..='\u{200a}'
            | '\u{2028}' | '\u{2029}' | '\u{202f}' | '\u{205f}' | '\u{3000}' | '\u{feff}'
    )
}

/// ECMAScript's `StringToNumber`; never Rust's `parse::<f64>` on its own,
/// which reads `inf`, `nan` and `infinity` too.
pub(crate) fn string_to_number(text: &str) -> f64 {
    let text = text.trim_matches(js_space);
    if text.is_empty() {
        return 0.0;
    }
    for (prefix, radix) in [("0x", 16), ("0X", 16), ("0o", 8), ("0O", 8), ("0b", 2), ("0B", 2)] {
        if let Some(digits) = text.strip_prefix(prefix) {
            return radix_number(digits, radix);
        }
    }
    match text {
        "Infinity" | "+Infinity" => return f64::INFINITY,
        "-Infinity" => return f64::NEG_INFINITY,
        _ => {}
    }
    if decimal_literal(text) {
        text.parse().unwrap_or(f64::NAN)
    } else {
        f64::NAN
    }
}

/// Digits of `radix`, at least one, unsigned.
fn radix_number(digits: &str, radix: u32) -> f64 {
    if digits.is_empty() {
        return f64::NAN;
    }
    let mut value = 0.0_f64;
    for c in digits.chars() {
        match c.to_digit(radix) {
            Some(digit) => value = value * f64::from(radix) + f64::from(digit),
            None => return f64::NAN,
        }
    }
    value
}

/// `StrDecimalLiteral` without `Infinity`: an optional sign, digits with
/// an optional fraction (`1`, `5.`, `-.5`, not `.`), an optional exponent.
fn decimal_literal(text: &str) -> bool {
    let body = text.strip_prefix(['+', '-']).unwrap_or(text);
    let (mantissa, exponent) = match body.find(['e', 'E']) {
        Some(at) => (&body[..at], Some(&body[at + 1..])),
        None => (body, None),
    };
    let digits = |part: &str| part.bytes().all(|b| b.is_ascii_digit());
    let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    let mantissa_ok = digits(whole) && digits(fraction) && !(whole.is_empty() && fraction.is_empty());
    let exponent_ok = exponent.is_none_or(|exp| {
        let exp = exp.strip_prefix(['+', '-']).unwrap_or(exp);
        !exp.is_empty() && digits(exp)
    });
    mantissa_ok && exponent_ok
}

// ---- Versions -----------------------------------------------------------------

/// besk's `versionOf`: the manifest's top-level `version` when it is a JSON
/// number (any form: `3`, `3.0`, `-1`), else 1 (a string, missing).
pub(crate) fn version_of(manifest: &Value) -> f64 {
    manifest["version"].as_f64().unwrap_or(1.0)
}

/// The version after `manifest`'s (`version_of + 1`), written as an
/// integer when it is one (`4`, not `4.0`), as JavaScript writes it.
pub(crate) fn next_version(manifest: &Value) -> Value {
    let next = version_of(manifest) + 1.0;
    // Whole numbers JavaScript would print without a fraction, as i64.
    let whole = next.fract() == 0.0 && next.abs() < 9_007_199_254_740_992.0;
    if whole { json!(next as i64) } else { json!(next) }
}

// ---- Carrying the list forward ---------------------------------------------------

/// What a new version of `manifest`'s model carries forward: its
/// `representations` value verbatim when it is a non-empty JSON array, as
/// besk's `nextModelEntry` does; otherwise nothing (no key, `[]`, `null`,
/// an object or any other non-array), so the key is left out rather than
/// written as something `manifestSchema` would refuse.
pub(crate) fn carried(manifest: &Value) -> Option<Value> {
    match &manifest["representations"] {
        Value::Array(list) if !list.is_empty() => Some(Value::Array(list.clone())),
        _ => None,
    }
}

// ---- Format tables -------------------------------------------------------------------

/// `defaultRole`: the role a file of this format usually plays (a 3MF is a
/// `mesh`: only a sliced one is `machine`, which the caller says).
pub(crate) fn default_role(format: &str) -> &'static str {
    match format.to_lowercase().as_str() {
        "blend" | "step" | "stp" | "f3d" | "fcstd" | "scad" | "iges" | "igs" => "source",
        "stl" | "obj" | "ply" | "3mf" => "mesh",
        "glb" | "gltf" | "usdz" => "view",
        "gcode" | "bgcode" => "machine",
        _ => "other",
    }
}

/// `encodingFor`: text formats are stored brotli-framed, the rest raw.
pub(crate) fn encoding_for(format: &str) -> Encoding {
    const TEXT: [&str; 15] = [
        "gcode", "gltf", "obj", "mtl", "step", "stp", "iges", "igs", "scad", "dxf", "svg", "ply",
        "txt", "json", "csv",
    ];
    if TEXT.contains(&format.to_lowercase().as_str()) {
        Encoding::Brotli
    } else {
        Encoding::Raw
    }
}

/// asset-core's `slug`: lower case, each run of anything but `[a-z0-9]` one
/// `-`, no `-` at either end.
fn slug(text: &str) -> String {
    let mut out = String::new();
    for c in text.to_lowercase().chars() {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            out.push(c);
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_matches('-').to_string()
}

/// `representationId`: `source` for a source file, else the format, then
/// the profile's printer and material when it names them
/// (`gcode-original-prusa-mk4-petg`), at most 64 characters.
pub(crate) fn representation_id(role: &str, format: &str, profile: Option<&Map<String, Value>>) -> String {
    let base = match (role, slug(format)) {
        ("source", _) => "source".to_string(),
        (_, format) if !format.is_empty() => format,
        _ => role.to_string(),
    };
    let named = |key: &str| -> Option<String> {
        match profile?.get(key)? {
            Value::String(text) if !text.is_empty() => Some(text.clone()),
            Value::Number(n) => Some(js_number(n.as_f64()?)),
            _ => None,
        }
    };
    let parts: Vec<String> = [Some(base), named("printer"), named("material")]
        .into_iter()
        .flatten()
        .map(|part| slug(&part))
        .filter(|part| !part.is_empty())
        .collect();
    let id = parts.join("-");
    let cut: String = id.chars().take(64).collect();
    cut.trim_end_matches('-').to_string()
}

// ---- Encoding the bytes -------------------------------------------------------------------

/// `decodeRepresentation`, by the entry's `encoding` and never by sniffing:
/// a `raw` file as it is (whatever its first bytes); a `brotli` one must
/// start with the `ARB\x01` frame and is unpacked, stopping past `cap`.
pub(crate) fn decode(bytes: Vec<u8>, encoding: Encoding, cap: u64) -> Result<Vec<u8>, String> {
    let out = match encoding {
        Encoding::Raw => bytes,
        Encoding::Brotli => {
            let packed = bytes.strip_prefix(BROTLI_MAGIC).ok_or(DAMAGED)?;
            let mut out = Vec::new();
            brotli::Decompressor::new(packed, 4096)
                .take(cap + 1)
                .read_to_end(&mut out)
                .map_err(|_| DAMAGED.to_string())?;
            out
        }
    };
    if out.len() as u64 > cap {
        return Err(TOO_LARGE.into());
    }
    Ok(out)
}

/// `encodeRepresentation`: brotli-framed for a text format (quality 5,
/// window 22, what `compressAuto` writes with the `binary` hint), raw
/// otherwise; and the encoding to record.
pub(crate) fn encode(bytes: &[u8], format: &str) -> (Vec<u8>, Encoding) {
    match encoding_for(format) {
        Encoding::Raw => (bytes.to_vec(), Encoding::Raw),
        Encoding::Brotli => (
            crate::decompose::brotli_framed(bytes, BROTLI_QUALITY),
            Encoding::Brotli,
        ),
    }
}

// ---- What the view and the command show --------------------------------------------------

/// The listing's summary of `manifest`'s other files (spec F2): the valid
/// entries, how many are stale, their distinct formats in stored order (at
/// most [`MAX_REPRESENTATIONS`] of them, however long a list a client
/// stored); `None` when it has none. One pass over the list.
pub(crate) fn summary(manifest: &Value) -> Option<places::Representations> {
    let reps = list(manifest);
    if reps.is_empty() {
        return None;
    }
    let current = Current::of(manifest, &reps);
    let mut seen = HashSet::new();
    let formats: Vec<String> = reps
        .iter()
        .filter(|rep| seen.insert(rep.format.as_str()))
        .take(MAX_REPRESENTATIONS)
        .map(|rep| rep.format.clone())
        .collect();
    Some(places::Representations {
        count: reps.len() as u32,
        stale: reps.iter().filter(|rep| is_stale_in(&current, rep)).count() as u32,
        formats,
    })
}

/// One file of an asset as `arbesk_representations` answers it (spec F3).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RepresentationInfo {
    pub id: String,
    pub role: String,
    pub format: String,
    pub name: String,
    pub bytes: u64,
    pub stale: bool,
    /// The ref it was made from (`primary` or another entry's id).
    pub derived_from: Option<String>,
    pub tool: Option<Map<String, Value>>,
    pub profile: Option<Map<String, Value>>,
    pub stats: Option<Map<String, Value>>,
    pub created_at: f64,
    /// Its path one segment below the asset, pinned to the entry's CID
    /// (spec F4): what Copy puts on the clipboard and Open With fetches,
    /// refused once the entry holds another file.
    pub path: String,
}

/// Each valid entry of `manifest`, in stored order, with its staleness
/// and its pinned path below the asset at `asset_path`.
pub(crate) fn infos(manifest: &Value, asset_path: &str) -> Vec<RepresentationInfo> {
    let reps = list(manifest);
    let current = Current::of(manifest, &reps);
    reps.into_iter()
        .map(|rep| RepresentationInfo {
            stale: is_stale_in(&current, &rep),
            path: crate::location::file_path(asset_path, &rep.id, Some(&rep.cid), &rep.name),
            derived_from: rep.derived_from.as_ref().map(|from| from.reference.clone()),
            id: rep.id,
            role: rep.role,
            format: rep.format,
            name: rep.name,
            bytes: rep.bytes,
            tool: rep.tool,
            profile: rep.profile,
            stats: rep.stats,
            created_at: rep.created_at,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Value {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/representations-stale.json");
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
    }

    fn cases<'a>(fixture: &'a Value, key: &str) -> &'a Vec<Value> {
        fixture[key].as_array().unwrap()
    }

    #[test]
    fn the_fixture_names_the_arbesk_commit_it_came_from() {
        let commit = fixture()["arbesk_commit"].as_str().unwrap().to_string();
        assert_eq!(commit.len(), 40);
        assert!(commit.bytes().all(|b| b.is_ascii_hexdigit()));
    }

    #[test]
    fn each_entry_parses_as_asset_core_parses_it() {
        let fixture = fixture();
        for case in cases(&fixture, "entries") {
            let parsed = parse_entry(&case["entry"]);
            assert_eq!(parsed.is_some(), case["valid"].as_bool().unwrap(), "{}", case["label"]);
        }
    }

    #[test]
    fn a_list_keeps_what_asset_core_keeps_and_counts_every_stored_entry() {
        let fixture = fixture();
        for case in cases(&fixture, "lists") {
            let manifest = &case["manifest"];
            let stored = stored(manifest);
            let kept: Vec<Value> = case["kept"]
                .as_array()
                .unwrap()
                .iter()
                .map(|i| stored[i.as_u64().unwrap() as usize].clone())
                .collect();
            let listed: Vec<Rep> = list(manifest);
            let expected: Vec<Rep> = kept.iter().map(|e| parse_entry(e).unwrap()).collect();
            assert_eq!(listed, expected, "{}", case["label"]);
            assert_eq!(stored_count(manifest) as u64, case["stored_count"].as_u64().unwrap());
            let ids: Vec<String> = serde_json::from_value(case["stored_ids"].clone()).unwrap();
            assert_eq!(stored_ids(manifest), ids, "{}", case["label"]);
        }
    }

    #[test]
    fn current_cids_are_asset_cores() {
        let fixture = fixture();
        for case in cases(&fixture, "cids") {
            let cid = current_cid_of(&case["manifest"], case["ref"].as_str().unwrap());
            assert_eq!(cid.as_deref(), case["cid"].as_str(), "{}", case["label"]);
        }
    }

    /// Asset-core's `isStale`, matrices that need `ToNumber` and a ref to
    /// an entry this client can't parse included.
    #[test]
    fn staleness_is_asset_cores() {
        let fixture = fixture();
        let stale = cases(&fixture, "stale");
        assert!(stale.len() > 90);
        for case in stale {
            let rep = parse_entry(&case["rep"]).unwrap();
            let answer = is_stale(&case["manifest"], &rep);
            assert_eq!(answer, case["stale"].as_bool().unwrap(), "{}", case["label"]);
        }
    }

    #[test]
    fn ids_and_format_tables_are_besks() {
        let fixture = fixture();
        for case in cases(&fixture, "ids") {
            let profile = case["profile"].as_object();
            let id = representation_id(
                case["role"].as_str().unwrap(),
                case["format"].as_str().unwrap(),
                profile,
            );
            assert_eq!(id, case["id"].as_str().unwrap(), "{case}");
        }
        for case in cases(&fixture, "tables") {
            let format = case["format"].as_str().unwrap();
            assert_eq!(default_role(format), case["role"].as_str().unwrap(), "{format}");
            assert_eq!(json!(encoding_for(format)), case["encoding"], "{format}");
        }
    }

    #[test]
    fn to_number_reads_as_javascript() {
        for (text, number) in [
            ("", 0.0),
            (" \t", 0.0),
            ("0x10", 16.0),
            ("0b101", 5.0),
            ("0o17", 15.0),
            ("1e3", 1000.0),
            ("-.5", -0.5),
            ("5.", 5.0),
            ("+1", 1.0),
            ("\u{2028}7\u{feff}", 7.0),
        ] {
            assert_eq!(string_to_number(text), number, "{text:?}");
        }
        for text in ["inf", "nan", "NaN", "infinity", "1_000", ".", "1e", "-0x1", "0x", "e5", "1 1", "--1"] {
            assert!(string_to_number(text).is_nan(), "{text:?}");
        }
        assert_eq!(string_to_number("-Infinity"), f64::NEG_INFINITY);
        for (value, number) in [
            (json!([]), 0.0),
            (json!([7]), 7.0),
            (json!(["7"]), 7.0),
            (json!([null]), 0.0),
            (json!(true), 1.0),
            (Value::Null, 0.0),
        ] {
            assert_eq!(to_number(&value), number, "{value}");
        }
        assert!(to_number(&json!([1, 2])).is_nan());
        assert!(to_number(&json!({})).is_nan());
        assert_eq!(js_number(1e21), "1e+21");
        assert_eq!(js_number(1.5e-7), "1.5e-7");
        assert_eq!(js_number(0.2), "0.2");
        assert_eq!(js_number(12.0), "12");
    }

    /// An asset's path, for the pinned paths `infos` gives its files.
    const ASSET: &str = "arbesk://0x00000000000000000000000000000000000000ab/1~Shapes/box~Box.gltf";

    fn entry() -> Value {
        json!({ "id": "stl", "role": "mesh", "format": "stl", "name": "Box.stl", "cid": "bafyStl",
            "bytes": 10, "encoding": "raw", "created_at": 1 })
    }

    #[test]
    fn a_size_is_an_integer_valued_number() {
        let mut whole = entry();
        whole["bytes"] = json!(10.0);
        assert_eq!(parse_entry(&whole).unwrap().bytes, 10);
        for bytes in [json!(1.5), json!(-1), json!(MAX_BYTES + 1), json!("10"), Value::Null] {
            let mut bad = entry();
            bad["bytes"] = bytes.clone();
            assert!(parse_entry(&bad).is_none(), "{bytes}");
        }
    }

    #[test]
    fn decoding_follows_the_encoding_and_never_sniffs() {
        // A raw file that looks like a frame, or like gzip, stays as it is.
        for raw in [b"ARB\x01\x0b\x02\x80hi\x03".to_vec(), vec![0x1f, 0x8b, 8, 0, 1, 2]] {
            assert_eq!(decode(raw.clone(), Encoding::Raw, 1 << 20).unwrap(), raw);
        }
        assert_eq!(decode(b"G1 X1".to_vec(), Encoding::Brotli, 1 << 20).unwrap_err(), DAMAGED);
        assert_eq!(decode(b"ARB\x01not brotli".to_vec(), Encoding::Brotli, 1 << 20).unwrap_err(), DAMAGED);
        let text = "G1 X10 Y10 E0.5\n".repeat(2000).into_bytes();
        let (stored, encoding) = encode(&text, "gcode");
        assert_eq!(encoding, Encoding::Brotli);
        assert!(stored.starts_with(BROTLI_MAGIC) && stored.len() < text.len());
        assert_eq!(decode(stored.clone(), encoding, 1 << 20).unwrap(), text);
        // A bomb stops at the cap.
        assert_eq!(decode(stored, encoding, 100).unwrap_err(), TOO_LARGE);
        assert_eq!(decode(vec![0; 101], Encoding::Raw, 100).unwrap_err(), TOO_LARGE);
        let (blend, encoding) = encode(b"BLENDER", "blend");
        assert_eq!((blend.as_slice(), encoding), (b"BLENDER".as_slice(), Encoding::Raw));
    }

    #[test]
    fn a_summary_counts_valid_entries_and_their_formats() {
        let mut stale = entry();
        stale["id"] = json!("gcode");
        stale["format"] = json!("gcode");
        stale["derived_from"] = json!({ "ref": "primary", "cid": "bafyOld" });
        let manifest = json!({ "scene": { "nodes": [{ "source": { "cid": "bafyNew" } }] },
            "representations": [entry(), { "id": "BAD" }, stale, { "id": "stl-2", "role": "mesh",
                "format": "stl", "name": "B.stl", "cid": "x", "bytes": 1, "encoding": "raw",
                "created_at": 2 }] });
        let summary = summary(&manifest).unwrap();
        assert_eq!((summary.count, summary.stale), (3, 1));
        assert_eq!(summary.formats, ["stl", "gcode"]);
        assert!(super::summary(&json!({})).is_none());
        assert!(super::summary(&json!({ "representations": [{ "id": "BAD" }] })).is_none());
        let infos = infos(&manifest, ASSET);
        assert_eq!(infos.len(), 3);
        assert!(infos[1].stale && infos[1].derived_from.as_deref() == Some("primary"));
        assert_eq!(
            serde_json::to_value(&infos[0]).unwrap(),
            json!({ "id": "stl", "role": "mesh", "format": "stl", "name": "Box.stl", "bytes": 10,
                "stale": false, "derived_from": null, "tool": null, "profile": null, "stats": null,
                "created_at": 1.0, "path": format!("{ASSET}/stl@bafyStl~Box.stl") })
        );
    }

    /// A list no writer would store (50,000 entries, each made from
    /// another) still sums up in one pass, with a bounded row.
    #[test]
    fn a_huge_list_sums_up_quickly_and_briefly() {
        let reps: Vec<Value> = (0..50_000)
            .map(|i| {
                let mut rep = entry();
                rep["id"] = json!(format!("r{i}"));
                rep["format"] = json!(format!("f{i}"));
                rep["cid"] = json!(format!("bafy{i}"));
                rep["derived_from"] = json!({ "ref": format!("r{}", (i + 1) % 50_000), "cid": format!("bafy{}", (i + 1) % 50_000) });
                rep
            })
            .collect();
        let mut manifest = json!({ "representations": reps });
        manifest["representations"][1]["derived_from"]["cid"] = json!("bafyOld");
        let started = std::time::Instant::now();
        let summary = summary(&manifest).unwrap();
        assert_eq!((summary.count, summary.stale), (50_000, 1));
        assert_eq!(summary.formats.len(), MAX_REPRESENTATIONS);
        assert_eq!(summary.formats[..2], ["f0", "f1"]);
        assert_eq!(infos(&manifest, ASSET).iter().filter(|info| info.stale).count(), 1);
        assert!(started.elapsed() < std::time::Duration::from_secs(10), "{:?}", started.elapsed());
    }

    /// A ref names the first valid entry with its id, as `currentCidOf`.
    #[test]
    fn a_ref_reads_the_first_valid_entry_with_its_id() {
        let mut first = entry();
        first["id"] = json!("dup");
        let mut second = first.clone();
        second["cid"] = json!("bafyOther");
        let mut made = entry();
        made["derived_from"] = json!({ "ref": "dup", "cid": "bafyStl" });
        let manifest = json!({ "representations": [{ "id": "dup", "cid": "bafyBad" }, first, second, made] });
        let rep = parse_entry(&manifest["representations"][3]).unwrap();
        assert!(!is_stale(&manifest, &rep));
        assert_eq!(summary(&manifest).unwrap().stale, 0);
    }

    #[test]
    fn derived_from_records_the_ref_cid_and_placement() {
        let manifest = json!({ "scene": { "nodes": [{ "source": { "cid": "bafyM" } }] },
            "representations": [entry()] });
        // Written as JavaScript writes the numbers (`1`, not `1.0`), as besk's entries are.
        let identity = json!([1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1]);
        assert_eq!(
            derived_from_of(&manifest, PRIMARY),
            Some(json!({ "ref": "primary", "cid": "bafyM", "transform_matrix": identity }))
        );
        assert_eq!(derived_from_of(&manifest, "stl"), Some(json!({ "ref": "stl", "cid": "bafyStl" })));
        assert_eq!(derived_from_of(&manifest, "gone"), None);
        assert_eq!(derived_from_of(&json!({}), PRIMARY), None);
        let placed = json!({ "scene": { "nodes": [{ "source": { "cid": "bafyM" },
            "transform_matrix": [0.5, "2", null, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1] }] } });
        let from = derived_from_of(&placed, PRIMARY).unwrap();
        assert_eq!(from["transform_matrix"].to_string(), "[0.5,2,0,0,0,1,0,0,0,0,1,0,0,0,0,1]");
    }

    #[test]
    fn version_of_reads_any_json_number_else_one() {
        for (version, of, next) in [
            (json!(3), 3.0, json!(4)),
            (json!(3.0), 3.0, json!(4)),
            (json!(-1), -1.0, json!(0)),
            (json!(2.5), 2.5, json!(3.5)),
            (json!("3"), 1.0, json!(2)),
            (Value::Null, 1.0, json!(2)),
        ] {
            let manifest = json!({ "version": version });
            assert_eq!(version_of(&manifest), of, "{version}");
            assert_eq!(next_version(&manifest), next, "{version}");
        }
        assert_eq!(version_of(&json!({})), 1.0);
        assert_eq!(next_version(&json!({})), json!(2));
        // `4`, never `4.0`.
        assert_eq!(next_version(&json!({ "version": 3.0 })).to_string(), "4");
    }

    #[test]
    fn only_a_non_empty_array_is_carried() {
        let list = json!([{ "id": "stl", "future": { "kept": true } }, "not even an entry"]);
        assert_eq!(carried(&json!({ "representations": list })), Some(list));
        for value in [json!([]), Value::Null, json!({}), json!("x"), json!(3)] {
            assert_eq!(carried(&json!({ "representations": value })), None, "{value}");
        }
        assert_eq!(carried(&json!({})), None);
    }
}
