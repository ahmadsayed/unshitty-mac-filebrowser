//! Adding files to an asset and removing them (spec F10–F12): each paste,
//! drop or Remove… is one new asset version, written as `besk attach` and
//! `besk detach` write theirs (`packages/besk/src/representations.ts`).
//!
//! Everything is checked before the first upload: the session, the editor
//! proof, each file, the ids (settled for the whole paste against the
//! **stored** ids) and the room. Then the files are uploaded, the next
//! asset manifest is built from the stored JSON (every key and entry kept,
//! the model too) and uploaded, and under the collection's hold the
//! collection is read once settled and changed only while its entry for the
//! asset still names the version the write started from (compare-and-swap).
//! A conflict is retried once from the new tip, reusing the uploaded files;
//! a second one changes nothing. A failure anywhere leaves the asset as it
//! was: what was uploaded stays on IPFS, unreferenced.

use std::collections::{HashMap, HashSet};
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::chain::Site;
use crate::compose::{self, Format};
use crate::decompose::{sanitize_file_name, stored_json};
use crate::http::Http;
use crate::location::{self, ArbeskPath};
use crate::progress::{Phase, Progress};
use crate::representations::{self as reps, PRIMARY, RepresentationInfo};
use crate::service::Reader;
use crate::signin::now_ms;
use crate::write::{self, CollectionLocks, Outcome, Relaying, Target, Uploader};

// ---- Wording ----------------------------------------------------------------

/// No session, an expired one, or Arbesk's 401, for an attach.
pub(crate) const SIGN_IN: &str = "Sign in to Arbesk again to add files to it.";
/// The same for a remove.
pub(crate) const SIGN_IN_REMOVE: &str = "Sign in to Arbesk again to remove files from it.";
/// Anything but a regular file on this computer.
pub(crate) const ONLY_LOCAL: &str = "Only files on this computer can be added to an asset.";
/// More entries than an asset may hold, once the ids are settled.
pub(crate) const FULL: &str = "An asset holds at most 32 files.";
/// A removed file whose entry is gone already.
pub(crate) const GONE_FILE: &str = "This file is no longer in the asset.";
/// A removed file whose entry holds another file than the one shown.
pub(crate) const CHANGED_FILE: &str = "This file was replaced in the asset since it was shown. Nothing was removed.";

fn no_type(name: &str) -> String {
    format!("“{name}” has no file type to record.")
}

fn too_large(name: &str) -> String {
    format!("“{name}” is larger than 256 MB.")
}

fn unrecordable(name: &str) -> String {
    format!("“{name}” can't be recorded as a file of the asset.")
}

/// An id an entry this app can't parse holds: never replaced.
pub(crate) fn held_by_unreadable(id: &str) -> String {
    format!("This asset already has a file “{id}” that File Browser can't show.")
}

fn nothing_added(asset: &str) -> String {
    format!("Nothing was added to “{asset}”.")
}

fn changed_while_adding(asset: &str) -> String {
    format!("“{asset}” changed while the files were being added. Nothing was changed; try again.")
}

fn changed_while_removing(asset: &str) -> String {
    format!("“{asset}” changed while the file was being removed. Nothing was changed; try again.")
}

fn not_in_collection(asset: &str) -> String {
    format!("“{asset}” is no longer in the collection.")
}

/// A `representations` value that isn't a list (a newer schema's, or a
/// broken write): never overwritten or dropped.
fn unreadable_files(asset: &str) -> String {
    format!("“{asset}” has files File Browser can't show.")
}

/// Refuse a write to `manifest` when its `representations` is there but
/// not a list: the write would replace or drop what this app can't read.
fn list_readable(manifest: &Value, listed: &str) -> Result<(), String> {
    match manifest.get("representations") {
        None | Some(Value::Null | Value::Array(_)) => Ok(()),
        Some(_) => Err(unreadable_files(listed)),
    }
}

// ---- A local file -------------------------------------------------------------

/// How much of a G-code's start and end is read for what its slicer wrote.
const GCODE_HEAD: u64 = 4 * 1024;
const GCODE_TAIL: u64 = 64 * 1024;

/// One file on this computer to add, as it will be recorded: its format,
/// role, details and the id `besk attach` would give it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct LocalFile {
    pub(crate) path: String,
    pub(crate) name: String,
    pub(crate) format: String,
    pub(crate) role: &'static str,
    pub(crate) bytes: u64,
    pub(crate) details: Details,
    /// `representation_id(role, format, profile)`, before any clash.
    pub(crate) id: String,
}

/// What a slicer wrote in a G-code's comments, each map left out when
/// nothing in it parsed.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Details {
    pub(crate) tool: Option<Map<String, Value>>,
    pub(crate) profile: Option<Map<String, Value>>,
    pub(crate) stats: Option<Map<String, Value>>,
}

/// The file at `path` as it will be recorded; refused unless it is a
/// regular file with a format (`[a-z0-9]{1,16}`, its lower-case
/// extension) of at most 256 MiB.
pub(crate) fn local_file(path: &str) -> Result<LocalFile, String> {
    let file = Path::new(path);
    let name = file
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string());
    let meta = std::fs::metadata(file).map_err(|_| ONLY_LOCAL.to_string())?;
    if !file.is_absolute() || !meta.is_file() {
        return Err(ONLY_LOCAL.into());
    }
    let format = format_of(&name).ok_or_else(|| no_type(&name))?;
    if meta.len() > reps::MAX_BYTES {
        return Err(too_large(&name));
    }
    let role = reps::default_role(&format);
    let details = match format.as_str() {
        "gcode" => gcode_details(file),
        _ => Details::default(),
    };
    let id = reps::representation_id(role, &format, details.profile.as_ref());
    Ok(LocalFile {
        path: path.to_string(),
        name,
        format,
        role,
        bytes: meta.len(),
        details,
        id,
    })
}

/// The lower-case extension of `name` when it names a format: `[a-z0-9]{1,16}`.
fn format_of(name: &str) -> Option<String> {
    let (stem, ext) = name.rsplit_once('.')?;
    let ext = ext.to_ascii_lowercase();
    let ok = !stem.is_empty()
        && (1..=16).contains(&ext.len())
        && ext.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit());
    ok.then_some(ext)
}

// ---- What a slicer wrote ---------------------------------------------------------

/// The tool, profile and stats PrusaSlicer, SuperSlicer, OrcaSlicer, Bambu
/// Studio and Cura write in a G-code's first 4 KB and last 64 KB, best
/// effort: anything that can't be read is left out.
fn gcode_details(path: &Path) -> Details {
    let text = gcode_ends(path).unwrap_or_default();
    details_of(&text)
}

/// The start and end of a G-code, as text.
fn gcode_ends(path: &Path) -> std::io::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let len = file.metadata()?.len();
    let mut head = Vec::new();
    (&mut file).take(GCODE_HEAD).read_to_end(&mut head)?;
    let mut tail = Vec::new();
    if len > GCODE_HEAD {
        file.seek(SeekFrom::Start(len.saturating_sub(GCODE_TAIL).max(GCODE_HEAD)))?;
        file.take(GCODE_TAIL).read_to_end(&mut tail)?;
    }
    let mut text = String::from_utf8_lossy(&head).into_owned();
    text.push('\n');
    text.push_str(&String::from_utf8_lossy(&tail));
    Ok(text)
}

/// The `key = value` (or `key: value`) comments of a G-code, keys in lower
/// case; a line Orca writes with several (`; a: 1; b: 2`) gives each.
fn comments(text: &str) -> HashMap<String, String> {
    let mut found = HashMap::new();
    for line in text.lines() {
        let Some(comment) = line.trim_start().strip_prefix(';') else {
            continue;
        };
        for part in comment.split("; ") {
            let part = part.trim().trim_start_matches(';').trim();
            let split = part.split_once(" = ").or_else(|| part.split_once('=')).or_else(|| part.split_once(':'));
            if let Some((key, value)) = split {
                let value = value.trim().trim_matches('"').trim();
                found.entry(key.trim().to_ascii_lowercase()).or_insert_with(|| value.to_string());
            }
        }
    }
    found
}

/// The slicer and its version, from `generated by PrusaSlicer 2.9.2+…` or
/// Cura's `Generated with Cura_SteamEngine 5.7.1`.
fn tool_of(text: &str) -> Option<Map<String, Value>> {
    text.lines().find_map(|line| {
        let comment = line.trim_start().strip_prefix(';')?.trim();
        let lower = comment.to_ascii_lowercase();
        let rest = ["generated by ", "generated with "]
            .iter()
            .find_map(|lead| lower.starts_with(lead).then(|| &comment[lead.len()..]))?;
        let mut words = rest.split_whitespace();
        let name = words.next()?;
        let name = if name.starts_with("Cura_") { "Cura" } else { name };
        let version = words.next()?;
        let version = version.split('+').next().unwrap_or(version);
        let ok = version.starts_with(|c: char| c.is_ascii_digit());
        let mut tool = Map::new();
        tool.insert("name".into(), json!(name));
        if ok {
            tool.insert("version".into(), json!(version));
        }
        Some(tool)
    })
}

/// The first of `keys` the comments hold.
fn first<'a>(found: &'a HashMap<String, String>, keys: &[&str]) -> Option<&'a str> {
    keys.iter().find_map(|key| found.get(*key).map(String::as_str)).filter(|v| !v.is_empty())
}

/// A positive number, the first of a list (`0.4,0.4`).
fn positive(value: &str) -> Option<f64> {
    let first = value.split([',', ';']).next()?.trim();
    let n: f64 = first.parse().ok()?;
    (n.is_finite() && n > 0.0).then_some(n)
}

/// `1d 2h 3m 4s` (any of them) in seconds; a bare number is seconds (Cura).
fn duration(value: &str) -> Option<f64> {
    if let Ok(seconds) = value.trim().parse::<f64>() {
        return (seconds.is_finite() && seconds > 0.0).then_some(seconds.round());
    }
    let mut total = 0.0;
    let mut any = false;
    for word in value.split_whitespace() {
        let (digits, unit) = word.split_at(word.find(|c: char| !c.is_ascii_digit()).unwrap_or(word.len()));
        let n: f64 = digits.parse().ok()?;
        let scale = match unit {
            "d" => 86_400.0,
            "h" => 3_600.0,
            "m" => 60.0,
            "s" => 1.0,
            _ => return None,
        };
        total += n * scale;
        any = true;
    }
    (any && total > 0.0).then_some(total)
}

/// `Original Prusa MK4 0.4 nozzle` → `Original Prusa MK4`: the printer, not
/// its nozzle, as besk's `--printer` names it.
fn printer_name(settings: &str) -> String {
    let words: Vec<&str> = settings.split_whitespace().collect();
    let nozzle = words.len() >= 3
        && words[words.len() - 1].eq_ignore_ascii_case("nozzle")
        && words[words.len() - 2].parse::<f64>().is_ok();
    let kept = if nozzle { &words[..words.len() - 2] } else { &words[..] };
    kept.join(" ")
}

fn map_of(pairs: Vec<(&str, Option<Value>)>) -> Option<Map<String, Value>> {
    let map: Map<String, Value> = pairs
        .into_iter()
        .filter_map(|(key, value)| value.map(|value| (key.to_string(), value)))
        .collect();
    (!map.is_empty()).then_some(map)
}

/// What a slicer wrote, from a G-code's text.
pub(crate) fn details_of(text: &str) -> Details {
    let found = comments(text);
    let printer = first(&found, &["printer_settings_id"])
        .map(printer_name)
        .or_else(|| first(&found, &["printer_model", "target_machine.name"]).map(String::from))
        .filter(|name| !name.is_empty());
    let material = first(&found, &["filament_type"])
        .and_then(|value| value.split(';').next())
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let number = |keys: &[&str]| first(&found, keys).and_then(positive).map(|n| json!(n));
    let profile = map_of(vec![
        ("printer", printer.map(|p| json!(p))),
        ("material", material.map(|m| json!(m))),
        ("layer_mm", number(&["layer_height", "layer height"])),
        ("nozzle_mm", number(&["nozzle_diameter", "extruder_train.0.nozzle.diameter"])),
    ]);
    let time = first(
        &found,
        &["total estimated time", "estimated printing time (normal mode)", "model printing time", "time"],
    )
    .and_then(duration);
    let grams = number(&["total filament used [g]", "total filament weight [g]", "filament used [g]"]);
    let stats = map_of(vec![("print_time_s", time.map(|t| json!(t as u64))), ("filament_g", grams)]);
    Details {
        tool: tool_of(text),
        profile,
        stats,
    }
}

// ---- What the frontend asks before adding -------------------------------------------

/// A file of a paste whose id an entry already has.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Clash {
    /// A file the asset shows: Replace, Skip or Keep Both. `cid` is what
    /// Replace agrees to replace.
    Valid { cid: String, existing: RepresentationInfo },
    /// An entry this app can't read: refused, never replaced.
    Unreadable { message: String },
}

/// One file of a paste as it would be added (spec F10), with its clash.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PlannedFile {
    pub path: String,
    pub name: String,
    pub format: String,
    pub role: String,
    pub bytes: u64,
    pub id: String,
    pub clash: Option<Clash>,
}

/// What adding `paths` to the asset at `asset_path` would do, before
/// anything is asked: each file as it would be recorded and its clash with
/// a stored entry. A file that can't be added at all refuses the paste.
pub fn plan(
    http: &dyn Http,
    site: &Site,
    asset_path: &str,
    paths: &[String],
    cache_dir: &Path,
) -> Result<Vec<PlannedFile>, String> {
    let (_, token_id, asset_id) = asset_ids(asset_path)?;
    let files = paths.iter().map(|path| local_file(path)).collect::<Result<Vec<_>, _>>()?;
    let reader = Reader::open(http, site, cache_dir)?;
    let (cid, manifest) = reader.asset_manifest(&token_id, &asset_id)?;
    let listed = reader.name_of(&manifest, &asset_id, &cid)?;
    let base = based_on(manifest, &cid, &listed, &asset_id)?;
    list_readable(&base, &listed)?;
    let stored = Stored::of(&base);
    let infos = reps::infos(&base, asset_path);
    Ok(files
        .into_iter()
        .map(|file| PlannedFile {
            clash: stored.clash(&file.id, &infos),
            path: file.path,
            name: file.name,
            format: file.format,
            role: file.role.into(),
            bytes: file.bytes,
            id: file.id,
        })
        .collect())
}

/// The stored entries' ids, as every id decision reads them.
struct Stored {
    ids: HashSet<String>,
    /// Ids an entry this app can't parse holds.
    unreadable: HashSet<String>,
    /// The CID of the first valid entry of each id.
    valid: HashMap<String, String>,
    count: usize,
}

impl Stored {
    fn of(manifest: &Value) -> Self {
        let entries = manifest["representations"].as_array().map_or(&[][..], Vec::as_slice);
        let mut unreadable = HashSet::new();
        for entry in entries {
            if let (Some(id), None) = (entry["id"].as_str(), reps::parse_entry(entry)) {
                unreadable.insert(id.to_string());
            }
        }
        let mut valid = HashMap::new();
        for rep in reps::list(manifest) {
            valid.entry(rep.id).or_insert(rep.cid);
        }
        Stored {
            ids: reps::stored_ids(manifest).into_iter().collect(),
            unreadable,
            valid,
            count: reps::stored_count(manifest),
        }
    }

    fn clash(&self, id: &str, infos: &[RepresentationInfo]) -> Option<Clash> {
        if self.unreadable.contains(id) {
            return Some(Clash::Unreadable { message: held_by_unreadable(id) });
        }
        let cid = self.valid.get(id)?;
        let existing = infos.iter().find(|info| info.id == id)?.clone();
        Some(Clash::Valid { cid: cid.clone(), existing })
    }
}

// ---- Settling the ids --------------------------------------------------------------

/// What the user answered for a file whose id is taken.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Choice {
    /// Replace the entry, while it still holds the file `cid`.
    Replace { cid: String },
    KeepBoth,
    Skip,
}

/// One file to add and what was answered for it.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct Adding {
    pub path: String,
    #[serde(default)]
    pub choice: Option<Choice>,
}

/// The ids of a paste, settled against `manifest`'s stored ids: for each
/// file (by its index in the paste) the id it is recorded under, and the
/// words of each file refused.
#[derive(Debug, PartialEq)]
struct Settled {
    ids: Vec<(usize, String)>,
    refused: Vec<String>,
}

/// Settle each file's id (spec F10): an id an unreadable entry holds is
/// refused; one a valid entry holds is replaced on Replace (only while it
/// still holds the file Replace agreed to: else, and for a clash nobody
/// answered, as on a retry, Keep Both, which overwrites nothing), skipped on
/// Skip, and Keep Both takes the first free `<id>-2`, `<id>-3`…; an id an
/// earlier file of the paste took is Keep Both too, without asking.
fn settle(manifest: &Value, files: &[LocalFile], choices: &[Option<Choice>]) -> Settled {
    let stored = Stored::of(manifest);
    let mut taken: HashSet<String> = HashSet::new();
    let mut settled = Settled { ids: Vec::new(), refused: Vec::new() };
    for (i, file) in files.iter().enumerate() {
        let id = &file.id;
        if stored.unreadable.contains(id) {
            settled.refused.push(held_by_unreadable(id));
            continue;
        }
        let choice = choices.get(i).cloned().flatten();
        let chosen = match (stored.valid.get(id), choice) {
            (Some(_), Some(Choice::Skip)) => continue,
            (Some(cid), Some(Choice::Replace { cid: agreed })) if *cid == agreed && !taken.contains(id) => id.clone(),
            (Some(_), _) => free_id(id, &stored.ids, &taken),
            (None, _) if taken.contains(id) || stored.ids.contains(id) => free_id(id, &stored.ids, &taken),
            (None, _) => id.clone(),
        };
        taken.insert(chosen.clone());
        settled.ids.push((i, chosen));
    }
    settled
}

/// The first `<id>-2`, `<id>-3`… neither stored nor taken by the paste,
/// the base cut so the whole stays within 64 characters.
fn free_id(id: &str, stored: &HashSet<String>, taken: &HashSet<String>) -> String {
    (2..)
        .map(|n| {
            let suffix = format!("-{n}");
            let base: String = id.chars().take(64 - suffix.len()).collect();
            format!("{}{suffix}", base.trim_end_matches('-'))
        })
        .find(|candidate| !stored.contains(candidate) && !taken.contains(candidate))
        .expect("a free id")
}

// ---- The next manifest ----------------------------------------------------------------

/// A placeholder CID for checking an entry before its file is uploaded
/// (`checkEntry`).
const PENDING: &str = "pending";

/// The entry `file` is recorded as under `id`, as `besk attach` records it:
/// `derived_from` the model for a mesh or machine file (none, silently,
/// when the model has no source CID), its details, `created_at` now.
fn entry_of(manifest: &Value, file: &LocalFile, id: &str, cid: &str, bytes: u64, encoding: reps::Encoding, now: i64) -> Value {
    let mut entry = Map::new();
    entry.insert("id".into(), json!(id));
    entry.insert("role".into(), json!(file.role));
    entry.insert("format".into(), json!(file.format));
    entry.insert("name".into(), json!(file.name));
    entry.insert("bytes".into(), json!(bytes));
    entry.insert("encoding".into(), json!(encoding));
    if matches!(file.role, "machine" | "mesh")
        && let Some(from) = reps::derived_from_of(manifest, PRIMARY)
    {
        entry.insert("derived_from".into(), from);
    }
    for (key, value) in [("tool", &file.details.tool), ("profile", &file.details.profile), ("stats", &file.details.stats)] {
        if let Some(map) = value {
            entry.insert(key.into(), Value::Object(map.clone()));
        }
    }
    entry.insert("created_at".into(), json!(now));
    entry.insert("cid".into(), json!(cid));
    Value::Object(entry)
}

/// `manifest`'s next version (besk's `nextVersion`): every key kept,
/// `version_of + 1`, linked back to `cid`, saved `now`.
fn next_version_of(manifest: &Value, cid: &str, now: i64) -> Value {
    let mut next = manifest.clone();
    next["version"] = reps::next_version(manifest);
    next["prev_asset_manifest_cid"] = json!(cid);
    next["timestamp"] = json!(now);
    next
}

/// `entries` added to `manifest`'s list or replacing the stored entries
/// with their ids, in place, as `withRepresentation` does one by one;
/// entries this app can't parse stay where they are.
fn with_entries(mut manifest: Value, entries: Vec<Value>) -> Value {
    let mut list = manifest["representations"].as_array().cloned().unwrap_or_default();
    for entry in entries {
        let id = entry["id"].clone();
        let at = list.iter().position(|stored| stored["id"] == id);
        list.retain(|stored| stored["id"] != id);
        list.insert(at.unwrap_or(list.len()).min(list.len()), entry);
    }
    manifest["representations"] = Value::Array(list);
    manifest
}

/// `manifest` without the entries `id` names, the key dropped when the
/// list empties (`withoutRepresentation`).
fn without_entry(mut manifest: Value, id: &str) -> Value {
    let list = manifest["representations"].as_array().cloned().unwrap_or_default();
    let kept: Vec<Value> = list.into_iter().filter(|stored| stored["id"] != id).collect();
    if let Some(object) = manifest.as_object_mut() {
        if kept.is_empty() {
            object.remove("representations");
        } else {
            object.insert("representations".into(), Value::Array(kept));
        }
    }
    manifest
}

/// The asset manifest a write builds on: `manifest` itself, or, for an
/// entry `besk upload` stored as a bare composite (no `scene.nodes`
/// array), the wrapper besk's `compositeWrapper` makes for it, field for
/// field: named after the composite, else the listed name without its
/// extension; numbered as the composite.
pub(crate) fn based_on(manifest: Value, cid: &str, listed: &str, asset_id: &str) -> Result<Value, String> {
    if manifest.pointer("/scene/nodes").is_some_and(Value::is_array) {
        return Ok(manifest);
    }
    let format = compose::format_of(&manifest)?;
    let path = match format {
        Format::Gltf => "composite.gltf",
        Format::ThreeMf => "composite.3mf.json",
    };
    let stem = listed.rsplit_once('.').map_or(listed, |(stem, _)| stem);
    // `current.name ?? target.name`.
    let name = match &manifest["name"] {
        Value::Null => json!(stem),
        name => name.clone(),
    };
    Ok(json!({
        "type": "asset",
        "name": name,
        "asset_id": asset_id,
        "version": manifest["version"].as_f64().map_or(json!(1), |_| manifest["version"].clone()),
        "scene": { "nodes": [{
            "node_id": "node_1",
            "transform_matrix": [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1],
            "source": { "cid": cid, "format": format.ext(), "path": path },
        }] },
    }))
}

// ---- The write ------------------------------------------------------------------------

/// What a write leaves: the asset's files in its new version (for an open
/// Files… page), its listing summary (for its emblem) and the words of
/// each file refused on the way (a clash with an entry File Browser can't
/// show).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Attached {
    pub files: Vec<RepresentationInfo>,
    pub representations: Option<places::Representations>,
    pub refused: Vec<String>,
}

/// Who writes, where.
pub(crate) struct Writer<'a> {
    pub(crate) http: &'a dyn Http,
    pub(crate) site: &'a Site,
    pub(crate) session: &'a str,
    pub(crate) cache_dir: &'a Path,
    pub(crate) locks: &'a CollectionLocks,
    pub(crate) progress: &'a Progress,
}

/// The asset path's address, collection and asset ids.
fn asset_ids(path: &str) -> Result<(String, String, String), String> {
    match location::parse(path)? {
        ArbeskPath::Asset { address, token_id, asset_id } => Ok((address, token_id, asset_id)),
        _ => Err(places::ADD_TO_AN_ASSET.into()),
    }
}

/// One write's asset and what it builds on.
struct Base {
    /// The asset manifest's CID in the collection when the write read it.
    cid: String,
    /// That manifest, wrapped when it is a bare composite.
    manifest: Value,
    /// Its listed name, for the words.
    listed: String,
}

/// A write on one asset: its collection, the reader, the editor proof.
struct AssetWrite<'a> {
    writer: &'a Writer<'a>,
    reader: Reader<'a>,
    target: Target,
    asset_id: String,
    asset_path: String,
    /// The name the asset's path shows, for words said before its
    /// manifest is read.
    shown: String,
    editor_proof: Vec<String>,
}

impl<'a> AssetWrite<'a> {
    fn open(writer: &'a Writer<'a>, asset_path: &str) -> Result<Self, String> {
        let (address, token_id, asset_id) = asset_ids(asset_path)?;
        let reader = Reader::open_fresh(writer.http, writer.site, writer.cache_dir)?;
        let target = Target { address, token_id, name: String::new() };
        let editor_proof = write::editor_proof(&reader, &target)?;
        let shown = location::shown_name(asset_path).unwrap_or_else(|| asset_id.clone());
        Ok(Self {
            writer,
            reader,
            target,
            asset_id,
            asset_path: asset_path.to_string(),
            shown,
            editor_proof,
        })
    }

    /// Step 1's base: under the collection's hold, the settled collection's
    /// entry for the asset (the hold let go before anything is uploaded),
    /// then its manifest.
    fn base(&self) -> Result<Base, String> {
        let held = self.writer.locks.take(&self.target.address, &self.target.token_id);
        let (_, collection) =
            write::settled_collection(&self.reader, &self.target.token_id, &held, self.writer.progress)?;
        drop(held);
        let cid = collection["assets"][&self.asset_id]
            .as_str()
            .ok_or_else(|| not_in_collection(&self.shown))?
            .to_string();
        let manifest = self.reader.manifest_at(&cid)?;
        let listed = self.reader.name_of(&manifest, &self.asset_id, &cid)?;
        let manifest = based_on(manifest, &cid, &listed, &self.asset_id)?;
        Ok(Base { cid, manifest, listed })
    }

    /// Upload the next asset manifest, then, under the collection's hold,
    /// change the collection's entry for the asset from `base.cid` to it,
    /// only while it still names `base.cid`. `Ok(None)` on a conflict.
    fn swap(&self, uploader: &mut Uploader, base: &Base, next: &Value) -> Result<Option<()>, String> {
        let progress = self.writer.progress;
        let file_name = format!("asset_{}_composite.gltf", sanitize_file_name(&self.asset_id));
        progress.check()?;
        let next_cid = uploader.upload(&file_name, &stored_json(next, false))?;
        let held = self.writer.locks.take(&self.target.address, &self.target.token_id);
        let (current_cid, collection) =
            write::settled_collection(&self.reader, &self.target.token_id, &held, progress)?;
        match collection["assets"][&self.asset_id].as_str() {
            None => return Err(not_in_collection(&base.listed)),
            Some(now) if now != base.cid => return Ok(None),
            Some(_) => {}
        }
        let collection_next = write::next_collection(&collection, &current_cid, &self.asset_id, &next_cid)?;
        let collection_file = format!("collect_{}.json", sanitize_file_name(&write::collection_key(&collection_next)));
        progress.check()?;
        let collection_cid = uploader.upload(&collection_file, &stored_json(&collection_next, false))?;
        progress.set_phase(Phase::Saving);
        let to = Relaying {
            http: self.writer.http,
            site: self.writer.site,
            session: self.writer.session,
            token_id: &self.target.token_id,
            editor_proof: &self.editor_proof,
        };
        let outcome = write::relay(&to, &collection_cid, &self.asset_id, &base.listed);
        held.remember(&current_cid, &collection_cid, &outcome);
        match outcome {
            Outcome::Landed => Ok(Some(())),
            Outcome::Unsettled(error) | Outcome::Refused(error) => Err(error),
        }
    }

    fn attached(&self, next: &Value, refused: Vec<String>) -> Attached {
        Attached {
            files: reps::infos(next, &self.asset_path),
            representations: reps::summary(next),
            refused,
        }
    }
}

/// Every check before an upload, for `files` on `base`: the ids settled,
/// room for them, each entry as it will be recorded; then the entries and
/// the refusals. Nothing to add is refused.
fn checked(base: &Base, files: &[LocalFile], choices: &[Option<Choice>]) -> Result<Settled, String> {
    list_readable(&base.manifest, &base.listed)?;
    let settled = settle(&base.manifest, files, choices);
    if settled.ids.is_empty() {
        return Err(nothing_added(&base.listed));
    }
    let stored = Stored::of(&base.manifest);
    let adds = settled.ids.iter().filter(|(_, id)| !stored.ids.contains(id)).count();
    if stored.count + adds > reps::MAX_REPRESENTATIONS {
        return Err(FULL.into());
    }
    for (i, id) in &settled.ids {
        let file = &files[*i];
        let entry = entry_of(&base.manifest, file, id, PENDING, file.bytes, reps::encoding_for(&file.format), 0);
        if reps::parse_entry(&entry).is_none() {
            return Err(unrecordable(&file.name));
        }
    }
    Ok(settled)
}

/// Add the local files `adding` to the asset at `asset_path` as one new
/// version (spec F10, F11): its files afterwards.
pub(crate) fn attach(writer: &Writer, asset_path: &str, adding: &[Adding]) -> Result<Attached, String> {
    let files = adding.iter().map(|file| local_file(&file.path)).collect::<Result<Vec<_>, _>>()?;
    let choices: Vec<Option<Choice>> = adding.iter().map(|file| file.choice.clone()).collect();
    let write = AssetWrite::open(writer, asset_path)?;
    let mut base = write.base()?;
    let mut settled = checked(&base, &files, &choices)?;
    let named = base.listed.clone();
    let mut uploader = Uploader::new(writer.http, writer.site, writer.session, settled.ids.len() + 2, &named)?;
    let mut uploaded: HashMap<usize, Uploaded> = HashMap::new();
    writer.progress.set_total(upload_total(&files, &settled, &uploaded));
    for attempt in 0..2 {
        upload_files(writer.progress, &mut uploader, &files, &settled, &mut uploaded)?;
        let now = now_ms();
        let entries = settled
            .ids
            .iter()
            .map(|(i, id)| {
                let done = &uploaded[i];
                entry_of(&base.manifest, &files[*i], id, &done.cid, done.bytes, done.encoding, now)
            })
            .collect();
        let next = with_entries(next_version_of(&base.manifest, &base.cid, now), entries);
        if write.swap(&mut uploader, &base, &next)?.is_some() {
            return Ok(write.attached(&next, settled.refused));
        }
        if attempt == 1 {
            break;
        }
        // Another save landed: once more from the new tip, its ids
        // settled again; the files already uploaded keep their CIDs.
        base = write.base()?;
        settled = checked(&base, &files, &choices)?;
        // A file the first settle refused may be settled now.
        writer.progress.set_total(upload_total(&files, &settled, &uploaded));
    }
    Err(changed_while_adding(&base.listed))
}

/// The bytes a write uploads: the files uploaded already and those settled.
fn upload_total(files: &[LocalFile], settled: &Settled, uploaded: &HashMap<usize, Uploaded>) -> u64 {
    let mut counted: HashSet<usize> = uploaded.keys().copied().collect();
    counted.extend(settled.ids.iter().map(|(i, _)| *i));
    counted.iter().map(|i| files[*i].bytes).sum()
}

/// A file uploaded once: its CID, decoded size and encoding.
struct Uploaded {
    cid: String,
    bytes: u64,
    encoding: reps::Encoding,
}

/// Upload each settled file not uploaded yet, encoded as its format is
/// stored, counting its bytes; a cancel stops between files.
fn upload_files(
    progress: &Progress,
    uploader: &mut Uploader,
    files: &[LocalFile],
    settled: &Settled,
    uploaded: &mut HashMap<usize, Uploaded>,
) -> Result<(), String> {
    for (i, _) in &settled.ids {
        if uploaded.contains_key(i) {
            continue;
        }
        let file = &files[*i];
        progress.check()?;
        let bytes = std::fs::read(&file.path).map_err(|error| format!("Couldn't read “{}”: {error}", file.name))?;
        if bytes.len() as u64 > reps::MAX_BYTES {
            return Err(too_large(&file.name));
        }
        let (stored, encoding) = reps::encode(&bytes, &file.format);
        let cid = uploader.upload_shown(&file.name, &stored, &file.name)?;
        progress.add_bytes(bytes.len() as u64);
        uploaded.insert(*i, Uploaded { cid, bytes: bytes.len() as u64, encoding });
    }
    Ok(())
}

/// Whether `manifest` still holds the file `cid` under `id`, the one the
/// user saw and agreed to remove: gone, or replaced by another file, is
/// refused; so is an id an entry this app can't read also holds, even
/// beside the file shown, since the remove drops every entry of the id.
fn still_holds(manifest: &Value, id: &str, cid: &str) -> Result<(), String> {
    let stored = Stored::of(manifest);
    if stored.unreadable.contains(id) {
        return Err(CHANGED_FILE.into());
    }
    match stored.valid.get(id) {
        Some(now) if now == cid => Ok(()),
        None if !stored.ids.contains(id) => Err(GONE_FILE.into()),
        _ => Err(CHANGED_FILE.into()),
    }
}

/// Remove the file `id` from the asset at `asset_path` as one new version
/// (spec F12), only while its entry still holds the file `cid` the user
/// saw: its files afterwards. Nothing is uploaded but the two manifests;
/// the same hold, settle and compare-and-swap as an attach.
pub(crate) fn detach(writer: &Writer, asset_path: &str, id: &str, cid: &str) -> Result<Attached, String> {
    let write = AssetWrite::open(writer, asset_path)?;
    let mut base = write.base()?;
    let named = base.listed.clone();
    let mut uploader = Uploader::new(writer.http, writer.site, writer.session, 2, &named)?;
    for attempt in 0..2 {
        if attempt == 1 {
            base = write.base()?;
        }
        list_readable(&base.manifest, &base.listed)?;
        still_holds(&base.manifest, id, cid)?;
        let next = without_entry(next_version_of(&base.manifest, &base.cid, now_ms()), id);
        if write.swap(&mut uploader, &base, &next)?.is_some() {
            return Ok(write.attached(&next, Vec::new()));
        }
    }
    Err(changed_while_removing(&base.listed))
}

#[cfg(test)]
mod tests;
