//! An Arbesk account as folders and files: its collections, a collection's
//! assets, an asset built into the app cache to open or preview, and the
//! thumbnail an asset was saved with.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Condvar, Mutex};

use serde_json::Value;

use crate::chain::{self, CHAIN_UNREACHABLE, ChainConfig, Site};
use crate::compose::{
    Format, UNKNOWN_FORMAT, compose, compose_preview, format_of, is_child_ref, source_cid,
};
use crate::http::Http;
use crate::ipfs::{Gateway, ManifestCache, STORAGE_UNREACHABLE, in_parallel};
use crate::location::{ArbeskPath, asset_path_named, collection_path, parse};
use crate::scene::{Links, Scene, resolve};
use crate::Entry;
use crate::progress::Progress;
use crate::image::png_from_bytes;

/// How many requests a listing runs at once.
const PARALLEL: usize = 8;
/// The manifest cache's cap.
const MANIFEST_CACHE_BYTES: u64 = 64 * 1024 * 1024;
const UNTITLED_COLLECTION: &str = "Untitled Collection";

/// The folder or file entries at `path`: an account's collections, or a
/// collection's assets. Rows that can't be read are left out. Each asset
/// listed has its thumbnail, or its lack of one, kept in `thumbs`.
pub fn list(
    http: &dyn Http,
    site: &Site,
    path: &str,
    cache_dir: &Path,
    thumbs: &Thumbnails,
) -> Result<Vec<Entry>, String> {
    let reader = Reader::open(http, site, cache_dir)?;
    match parse(path)? {
        ArbeskPath::Account { address } => reader.collections(&address),
        ArbeskPath::Collection { address, token_id } => {
            thumbs.forget_collection(&address, &token_id);
            reader.assets(path, &address, &token_id, thumbs)
        }
        ArbeskPath::Asset { .. } => Err(format!("“{path}” is a file, not a folder.")),
    }
}

/// The thumbnail the asset at `path` was saved with, as PNG at most `size`
/// pixels square. `None` when it has none, the picture can't be read, or
/// `stop` says the view no longer wants it: the view keeps the file's icon,
/// as for any file without a thumbnail.
pub fn thumbnail(
    http: &dyn Http,
    site: &Site,
    path: &str,
    size: u32,
    cache_dir: &Path,
    thumbs: &Thumbnails,
    stop: &dyn Fn() -> bool,
) -> Option<Vec<u8>> {
    let ArbeskPath::Asset {
        address,
        token_id,
        asset_id,
    } = parse(path).ok()?
    else {
        return None;
    };
    if stop() {
        return None;
    }
    let key = asset_key(&address, &token_id, &asset_id);
    let stored = match thumbs.known(&key) {
        Some(known) => known,
        // Not kept when the chain or the storage didn't answer, so a later
        // request tries again.
        None => {
            let found = find_thumbnail(http, site, &token_id, &asset_id, cache_dir).ok()?;
            thumbs.remember(key, found.clone());
            found
        }
    }?;
    if stop() {
        return None;
    }
    let gateway = Gateway::new(http, &stored.gateway, manifest_cache(cache_dir));
    png_from_bytes(&gateway.image(&stored.cid).ok()?, size, stop)
}

/// The thumbnail of an asset no listing has shown, found through its collection.
fn find_thumbnail(
    http: &dyn Http,
    site: &Site,
    token_id: &str,
    asset_id: &str,
    cache_dir: &Path,
) -> Result<Option<Stored>, String> {
    let reader = Reader::open(http, site, cache_dir)?;
    let (_, manifest) = reader.asset_manifest(token_id, asset_id)?;
    Ok(reader.stored_thumbnail(&manifest))
}

/// Where each asset a listing showed keeps its thumbnail: the picture's
/// gateway and CID, or `None` for an asset saved without one. A thumbnail
/// then costs one request to the gateway. Keyed by the asset's ids
/// ([`asset_key`]), never by names a collection's owner chooses.
#[derive(Default)]
pub struct Thumbnails {
    known: Mutex<HashMap<String, Option<Stored>>>,
}

/// An asset's key in [`Thumbnails`]: its account, token and asset ids. A
/// token id is a number, so the collection's assets share a prefix.
fn asset_key(address: &str, token_id: &str, asset_id: &str) -> String {
    format!("{address}/{token_id}/{asset_id}")
}

/// A thumbnail on an account's gateway.
#[derive(Clone)]
struct Stored {
    gateway: String,
    cid: String,
}

impl Thumbnails {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Option<Stored>>> {
        self.known.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn remember(&self, key: String, stored: Option<Stored>) {
        self.lock().insert(key, stored);
    }

    /// What the last listing said about the asset's thumbnail, if one showed it.
    fn known(&self, key: &str) -> Option<Option<Stored>> {
        self.lock().get(key).cloned()
    }

    /// Forget a collection's assets before its listing records them again,
    /// so one that left the collection isn't shown with its old picture.
    fn forget_collection(&self, address: &str, token_id: &str) {
        let prefix = asset_key(address, token_id, "");
        self.lock().retain(|key, _| !key.starts_with(&prefix));
    }

    /// Forget every thumbnail: an account was disconnected.
    pub fn clear(&self) {
        self.lock().clear();
    }
}

/// The CID of an asset's thumbnail, read as Arbesk reads it
/// (`extractThumbnailCid`): the object's `cid`, else its `source.cid`;
/// older manifests may hold the CID itself.
fn thumbnail_cid(manifest: &Value) -> Option<&str> {
    let thumbnail = &manifest["thumbnail"];
    [thumbnail, &thumbnail["cid"], &thumbnail["source"]["cid"]]
        .into_iter()
        .find_map(text)
}

fn manifest_cache(cache_dir: &Path) -> ManifestCache {
    ManifestCache::new(cache_dir.join("arbesk-manifests"), MANIFEST_CACHE_BYTES)
}

/// Where Arbesk's Studio opens the asset at `path`: the deep link
/// `besk show` opens, `{origin}/studio?asset=<token>&assetId=<id>`.
pub fn studio_url(site: &Site, path: &str) -> Result<String, String> {
    let ArbeskPath::Asset {
        token_id, asset_id, ..
    } = parse(path)?
    else {
        return Err(format!("“{path}” is a folder, not a file."));
    };
    let query = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("asset", &token_id)
        .append_pair("assetId", &asset_id)
        .finish();
    Ok(format!("{}/studio?{query}", site.origin))
}

/// The file the Space preview loads for the asset at `path` (a `.glb` for a
/// glTF asset, the `.3mf` for a 3MF one) in `cache_dir/arbesk`, built once
/// per version (its manifest CID) and reused while nobody has changed it.
pub fn build(
    http: &dyn Http,
    site: &Site,
    path: &str,
    cache_dir: &Path,
    progress: &Progress,
    builds: &Builds,
) -> Result<PathBuf, String> {
    build_as(http, site, path, cache_dir, progress, builds, Output::Preview)
}

/// The asset at `path` as Arbesk builds it (a `.gltf` with every part
/// inside, or the `.3mf`), under its listed name, to copy out: in
/// `cache_dir/arbesk` beside the preview's file, built once per version
/// and reused while nobody has changed it.
pub fn export(
    http: &dyn Http,
    site: &Site,
    path: &str,
    cache_dir: &Path,
    progress: &Progress,
    builds: &Builds,
) -> Result<PathBuf, String> {
    build_as(http, site, path, cache_dir, progress, builds, Output::Export)
}

/// Which file of an asset [`build_as`] makes.
#[derive(Clone, Copy)]
enum Output {
    /// What the Space preview loads (`compose_preview`).
    Preview,
    /// The asset's own file (`compose`).
    Export,
}

fn build_as(
    http: &dyn Http,
    site: &Site,
    path: &str,
    cache_dir: &Path,
    progress: &Progress,
    builds: &Builds,
    output: Output,
) -> Result<PathBuf, String> {
    progress.check()?;
    let ArbeskPath::Asset {
        token_id, asset_id, ..
    } = parse(path)?
    else {
        return Err(format!("“{path}” is a folder, not a file."));
    };
    let reader = Reader::open(http, site, cache_dir)?;
    let (cid, manifest) = reader.asset_manifest(&token_id, &asset_id)?;
    let cid = cid.as_str();
    if is_child_ref(&manifest) {
        return Err(UNKNOWN_FORMAT.into());
    }
    let source = reader.gateway.json(&source_cid(&manifest, cid))?;
    let format = format_of(&source)?;
    let ext = match output {
        Output::Preview => format.preview_ext(),
        Output::Export => format.ext(),
    };
    let name = file_name(&display_name(&manifest, &asset_id), ext);
    let dir = cache_dir.join("arbesk").join(cid);
    build_file(&dir, &name, progress, builds, || match output {
        Output::Preview => compose_preview(&reader.gateway, &source),
        Output::Export => compose(&reader.gateway, &source),
    })
}

/// The scene of the asset at `path`: each stored model once, with every
/// place it goes (`scene::resolve`).
pub fn scene(
    http: &dyn Http,
    site: &Site,
    path: &str,
    version: Option<&str>,
    cache_dir: &Path,
) -> Result<Scene, String> {
    let (reader, cid) = open_asset(http, site, path, cache_dir)?;
    let Some(version) = version else {
        return Ok(resolve(&cid, &reader));
    };
    if !chain(&reader, &cid).iter().any(|(id, _)| id == version) {
        return Err("This version is not in the asset.".into());
    }
    Ok(resolve(version, &reader))
}

/// How many saves of an asset are read back, as the Studio's `walkManifestChain`.
pub const MAX_VERSIONS: usize = 50;

/// One save of an asset: its manifest's CID, and when it was saved (ms).
#[derive(serde::Serialize, Debug, Clone, PartialEq)]
pub struct Version {
    pub id: String,
    pub timestamp: Option<i64>,
}

/// The saves of the asset at `path`, oldest first: its manifest and the ones
/// before it (`prev_asset_manifest_cid`), the newest `MAX_VERSIONS`, up to
/// one that can't be read.
pub fn versions(
    http: &dyn Http,
    site: &Site,
    path: &str,
    cache_dir: &Path,
) -> Result<Vec<Version>, String> {
    let (reader, cid) = open_asset(http, site, path, cache_dir)?;
    let saves = chain(&reader, &cid)
        .into_iter()
        .map(|(id, manifest)| Version {
            id,
            timestamp: manifest["timestamp"].as_i64(),
        });
    Ok(saves.collect())
}

/// The manifests from `cid` back through `prev_asset_manifest_cid`, oldest first.
fn chain(reader: &Reader, cid: &str) -> Vec<(String, Value)> {
    let mut saves = Vec::new();
    let mut next = Some(cid.to_string());
    while let Some(id) = next.filter(|_| saves.len() < MAX_VERSIONS) {
        let Ok(manifest) = reader.gateway.json(&id) else {
            break;
        };
        next = manifest["prev_asset_manifest_cid"]
            .as_str()
            .map(String::from);
        saves.push((id, manifest));
    }
    saves.reverse();
    saves
}

/// The file the preview loads for the part `key` of `scene` (as `scene`
/// resolved it): `cache_dir/arbesk/<key>/part.<glb|3mf>`, built once and
/// reused while nobody has changed it. A key the scene doesn't hold is refused.
pub fn part(
    http: &dyn Http,
    site: &Site,
    scene: &Scene,
    key: &str,
    cache_dir: &Path,
    progress: &Progress,
    builds: &Builds,
) -> Result<PathBuf, String> {
    progress.check()?;
    let found = scene.parts.iter().find(|part| part.key == key);
    let found = found.ok_or("This part is not in the asset.")?;
    let reader = Reader::open(http, site, cache_dir)?;
    let dir = cache_dir.join("arbesk").join(key);
    let name = format!("part.{}", found.format);
    if found.raw {
        let format = found.format;
        return build_file(&dir, &name, progress, builds, || {
            raw_part(&reader.gateway, key, format)
        });
    }
    let source = reader.gateway.json(key)?;
    build_file(&dir, &name, progress, builds, || {
        compose_preview(&reader.gateway, &source)
    })
}

/// The model file at `cid` as an older save stored it: a GLB, or a zipped
/// 3MF, as `format` says; bytes that are neither are refused.
fn raw_part(gateway: &Gateway, cid: &str, format: &str) -> Result<Vec<u8>, String> {
    let bytes = gateway.bytes(cid)?;
    let magic: &[u8] = if format == "glb" {
        b"glTF"
    } else {
        b"PK\x03\x04"
    };
    if bytes.starts_with(magic) {
        Ok(bytes)
    } else {
        Err(UNKNOWN_FORMAT.into())
    }
}

/// The reader for the asset at `path`, and its manifest's CID.
fn open_asset<'a>(
    http: &'a dyn Http,
    site: &'a Site,
    path: &str,
    cache_dir: &Path,
) -> Result<(Reader<'a>, String), String> {
    let ArbeskPath::Asset {
        token_id, asset_id, ..
    } = parse(path)?
    else {
        return Err(format!("“{path}” is a folder, not a file."));
    };
    let mut reader = Reader::open(http, site, cache_dir)?;
    let (cid, _) = reader.asset_manifest(&token_id, &asset_id)?;
    reader.own_token = Some(token_id);
    Ok((reader, cid))
}

/// `dir/name`, made by `make` unless an unchanged copy is there. Two builds
/// of one file take turns (Space, then Open): the second reuses the first's.
fn build_file(
    dir: &Path,
    name: &str,
    progress: &Progress,
    builds: &Builds,
    make: impl FnOnce() -> Result<Vec<u8>, String>,
) -> Result<PathBuf, String> {
    let target = dir.join(name);
    let _held = builds.hold(dir.to_path_buf());
    if crate::cache::is_unchanged(&target) {
        return Ok(target);
    }
    // Someone edited the copy at `target`: theirs stays, a fresh one goes beside it.
    let target = if target.exists() {
        dir.join(unique_folder()).join(name)
    } else {
        target
    };
    let bytes = make()?;
    progress.check()?;
    save(&target, &bytes)?;
    Ok(target)
}

impl Links for Reader<'_> {
    fn manifest(&self, cid: &str) -> Result<Value, String> {
        self.gateway.json(cid)
    }

    /// The asset a `child_ref` names, as the Studio resolves it: its
    /// collection token's manifest through the link's own contract (the
    /// deployment's when it names none), then the asset in it. Links to
    /// another chain aren't followed.
    fn linked(&self, child_ref: &Value) -> Result<String, String> {
        let manifest = match &child_ref["collection"] {
            // The opened asset's own collection, as the Studio's "self".
            Value::String(own) if own == "self" => {
                self.collection_manifest(self.own_token.as_deref().ok_or(UNKNOWN_FORMAT)?)?
            }
            collection => self.linked_collection(collection)?,
        };
        let asset = child_ref["assetID"].as_str().unwrap_or_default();
        manifest["assets"][asset]
            .as_str()
            .map(String::from)
            .ok_or_else(|| "This asset is no longer in the collection.".into())
    }

    fn format(&self, source_cid: &str) -> Result<Format, String> {
        format_of(&self.gateway.json(source_cid)?)
    }
}

impl Reader<'_> {
    /// The manifest of the collection a link names, through the link's own
    /// contract (the deployment's when it names none).
    fn linked_collection(&self, collection: &Value) -> Result<Value, String> {
        if number(&collection["chainId"]) != Some(self.site.chain_id) {
            return Err("This link is to another chain.".into());
        }
        let token = number(&collection["tokenId"]).map(|id| id.to_string());
        let token = collection["tokenId"]
            .as_str()
            .map(String::from)
            .or(token)
            .ok_or(UNKNOWN_FORMAT)?;
        let mut chain = self.chain.clone();
        if let Some(contract) = collection["contractAddress"].as_str() {
            chain.contract = contract.to_string();
        }
        self.gateway
            .json(&chain::token_uri(self.http, &chain, &token)?)
    }
}

/// A number written as a number or as a decimal string.
fn number(value: &Value) -> Option<u64> {
    value.as_u64().or_else(|| value.as_str()?.parse().ok())
}

/// The builds under way, by the folder they build into: a second build of
/// one asset waits for the first, then finds its file.
#[derive(Default)]
pub struct Builds {
    busy: Mutex<HashSet<PathBuf>>,
    done: Condvar,
}

/// One build's hold on its folder, let go when dropped.
struct Held<'a> {
    builds: &'a Builds,
    folder: PathBuf,
}

impl Builds {
    fn hold(&self, folder: PathBuf) -> Held<'_> {
        let mut busy = self.busy.lock().unwrap_or_else(|e| e.into_inner());
        while busy.contains(&folder) {
            busy = self.done.wait(busy).unwrap_or_else(|e| e.into_inner());
        }
        busy.insert(folder.clone());
        Held {
            builds: self,
            folder,
        }
    }
}

impl Drop for Held<'_> {
    fn drop(&mut self) {
        let mut busy = self.builds.busy.lock().unwrap_or_else(|e| e.into_inner());
        busy.remove(&self.folder);
        self.builds.done.notify_all();
    }
}

/// What an account's chain and gateway say, read through one manifest cache.
pub(crate) struct Reader<'a> {
    http: &'a dyn Http,
    site: &'a Site,
    chain: ChainConfig,
    gateway: Gateway<'a>,
    /// The opened asset's collection token, for links to "self".
    own_token: Option<String>,
}

impl<'a> Reader<'a> {
    pub(crate) fn open(
        http: &'a dyn Http,
        site: &'a Site,
        cache_dir: &Path,
    ) -> Result<Self, String> {
        let chain = chain::config(http, site)?;
        let gateway = Gateway::new(http, &chain.gateway, manifest_cache(cache_dir));
        Ok(Self {
            http,
            site,
            chain,
            gateway,
            own_token: None,
        })
    }

    fn collection_manifest(&self, token_id: &str) -> Result<Value, String> {
        Ok(self.collection_manifest_with_cid(token_id)?.1)
    }

    /// The CID collection `token_id` points at now (its `tokenURI`, read
    /// afresh) and the manifest there.
    pub(crate) fn collection_manifest_with_cid(
        &self,
        token_id: &str,
    ) -> Result<(String, Value), String> {
        let cid = chain::token_uri(self.http, &self.chain, token_id)?;
        let manifest = self.gateway.json(&cid)?;
        Ok((cid, manifest))
    }

    /// The asset ids, manifest CIDs and listed names (as [`list`] shows
    /// them) of `collection`'s assets that can be read, in id order.
    pub(crate) fn listed_names(
        &self,
        collection: &Value,
    ) -> Result<Vec<(String, String, String)>, String> {
        let assets = asset_cids(collection);
        let rows = in_parallel(&assets, PARALLEL, |(id, cid)| {
            self.listed_name(id, cid)
                .map(|name| (id.clone(), cid.clone(), name))
        });
        let mut named = Vec::new();
        for row in rows {
            match row {
                Ok(row) => named.push(row),
                Err(error) if unreachable(&error) => return Err(error),
                Err(_) => {}
            }
        }
        Ok(named)
    }

    /// The asset manifest at `cid`.
    pub(crate) fn manifest_at(&self, cid: &str) -> Result<Value, String> {
        self.gateway.json(cid)
    }

    /// An asset's listed name: its name with its format's extension; a live
    /// link shows as glTF, an unknown format without an extension.
    fn listed_name(&self, asset_id: &str, cid: &str) -> Result<String, String> {
        let manifest = self.gateway.json(cid)?;
        self.name_of(&manifest, asset_id, cid)
    }

    fn name_of(&self, manifest: &Value, asset_id: &str, cid: &str) -> Result<String, String> {
        let ext = if is_child_ref(manifest) {
            Some("gltf")
        } else {
            format_of(&self.gateway.json(&source_cid(manifest, cid))?)
                .ok()
                .map(|format| format.ext())
        };
        let base = display_name(manifest, asset_id);
        Ok(ext.map_or_else(|| base.clone(), |ext| format!("{base}.{ext}")))
    }

    /// The CID and manifest of asset `asset_id` in collection `token_id`.
    fn asset_manifest(&self, token_id: &str, asset_id: &str) -> Result<(String, Value), String> {
        let collection = self.collection_manifest(token_id)?;
        let cid = collection["assets"][asset_id]
            .as_str()
            .ok_or("This asset is no longer in the collection.")?
            .to_string();
        let manifest = self.gateway.json(&cid)?;
        Ok((cid, manifest))
    }

    /// Where the asset `manifest` keeps its thumbnail, if it has one.
    fn stored_thumbnail(&self, manifest: &Value) -> Option<Stored> {
        thumbnail_cid(manifest).map(|cid| Stored {
            gateway: self.chain.gateway.clone(),
            cid: cid.to_string(),
        })
    }

    fn collections(&self, address: &str) -> Result<Vec<Entry>, String> {
        let ids = chain::collection_ids(self.http, self.site, address)?;
        readable(in_parallel(&ids, PARALLEL, |id| {
            self.collection_entry(address, id)
        }))
    }

    fn collection_entry(&self, address: &str, token_id: &str) -> Result<Entry, String> {
        let manifest = self.collection_manifest(token_id)?;
        let name = collection_name(&manifest);
        let path = collection_path(address, token_id, &name);
        Ok(entry(name, path, true, &manifest))
    }

    fn assets(
        &self,
        path: &str,
        address: &str,
        token_id: &str,
        thumbs: &Thumbnails,
    ) -> Result<Vec<Entry>, String> {
        let manifest = self.collection_manifest(token_id)?;
        let assets = asset_cids(&manifest);
        readable(in_parallel(&assets, PARALLEL, |(id, cid)| {
            let entry = self.asset_entry(path, id, cid)?;
            thumbs.remember(asset_key(address, token_id, id), entry.1);
            Ok(entry.0)
        }))
    }

    /// An asset's row, under its listed name ([`Reader::listed_name`]);
    /// with it, where its thumbnail is.
    fn asset_entry(
        &self,
        collection: &str,
        asset_id: &str,
        cid: &str,
    ) -> Result<(Entry, Option<Stored>), String> {
        let manifest = self.gateway.json(cid)?;
        let name = self.name_of(&manifest, asset_id, cid)?;
        let path = asset_path_named(collection, asset_id, &name);
        let thumbnail = self.stored_thumbnail(&manifest);
        Ok((entry(name, path, false, &manifest), thumbnail))
    }
}

/// The rows that could be read. A row that can't be read is left out, but
/// one the chain or the storage didn't answer fails the listing: an account
/// offline would otherwise look empty.
fn readable(rows: Vec<Result<Entry, String>>) -> Result<Vec<Entry>, String> {
    let mut entries = Vec::new();
    for row in rows {
        match row {
            Ok(entry) => entries.push(entry),
            Err(error) if unreachable(&error) => return Err(error),
            Err(_) => {}
        }
    }
    Ok(entries)
}

/// The chain or the storage didn't answer: not a row's own failure.
fn unreachable(error: &str) -> bool {
    error.starts_with(CHAIN_UNREACHABLE) || error.starts_with(STORAGE_UNREACHABLE)
}

/// A collection manifest's assets: each id and its manifest's CID.
fn asset_cids(collection: &Value) -> Vec<(String, String)> {
    collection["assets"]
        .as_object()
        .into_iter()
        .flatten()
        .filter_map(|(id, cid)| Some((id.clone(), cid.as_str()?.to_string())))
        .collect()
}

/// A collection's name as its folder shows it.
pub(crate) fn collection_name(manifest: &Value) -> String {
    text(&manifest["name"])
        .unwrap_or(UNTITLED_COLLECTION)
        .to_string()
}

fn text(value: &Value) -> Option<&str> {
    value.as_str().filter(|text| !text.trim().is_empty())
}

/// An asset's name, or its id when it has none.
fn display_name(manifest: &Value, asset_id: &str) -> String {
    text(&manifest["name"]).unwrap_or(asset_id).to_string()
}

/// A listing row; the date is the manifest's `timestamp` (milliseconds).
fn entry(name: String, path: String, is_dir: bool, manifest: &Value) -> Entry {
    Entry {
        name,
        path,
        is_dir,
        size: 0,
        modified: manifest["timestamp"].as_i64().map_or(0, |ms| ms / 1000),
        is_hidden: false,
        is_symlink: false,
    }
}

/// `<name>.<ext>` as a file name: no `/` or NUL, never empty, `.` or `..`.
fn file_name(name: &str, ext: &str) -> String {
    let safe: String = name
        .chars()
        .map(|c| if c == '/' || c == '\0' { '_' } else { c })
        .collect();
    let safe = if safe.is_empty() || safe == "." || safe == ".." {
        "_".to_string()
    } else {
        safe
    };
    format!("{safe}.{ext}")
}

fn unique_folder() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    format!("{nanos}-{}", std::process::id())
}

/// Write `bytes` beside `target` under a `.part` name, then rename it in
/// with `rename_new`, so a file only ever appears whole and never replaces
/// one. A failure leaves nothing behind.
fn save(target: &Path, bytes: &[u8]) -> Result<(), String> {
    let failed = |error: std::io::Error| format!("The file could not be saved: {error}");
    let dir = target.parent().ok_or("The file has no folder.")?;
    std::fs::create_dir_all(dir).map_err(failed)?;
    let name = target.file_name().unwrap_or_default().to_string_lossy();
    // Its own staging name, so no other writer can truncate it.
    let part = dir.join(format!(".{name}.{}.part", unique_folder()));
    let written =
        std::fs::write(&part, bytes).and_then(|()| crate::cache::rename_new(&part, target));
    if let Err(error) = written {
        let _ = std::fs::remove_file(&part);
        // Another build placed the same file first: it is the one to open.
        if error.kind() == std::io::ErrorKind::AlreadyExists
            && crate::cache::is_unchanged(target)
        {
            return Ok(());
        }
        return Err(failed(error));
    }
    crate::cache::mark(target);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chain::CHAIN_UNREACHABLE;
    use crate::compose::UNKNOWN_FORMAT;
    use crate::http::fake::FakeHttp;
    use crate::ipfs::STORAGE_UNREACHABLE;
    use crate::location::{asset_path, collection_path};
    use crate::testing::{ADDRESS, ORIGIN, STAMP_MS, arbesk, fixture, shapes, site, world};
    use serde_json::{Value, json};
    use std::collections::HashSet;
    use std::path::Path;
    use std::sync::{Arc, Mutex};

    fn rooms() -> String {
        collection_path(ADDRESS, "4", "Rooms")
    }

    fn table() -> String {
        asset_path(&rooms(), "t1", "Table", "gltf")
    }

    fn history() -> String {
        asset_path(
            &collection_path(ADDRESS, "5", "History"),
            "h1",
            "Chair",
            "gltf",
        )
    }

    fn table_scene(http: &FakeHttp, cache_dir: &Path) -> Scene {
        scene(http, &site(), &table(), None, cache_dir).unwrap()
    }

    fn names(entries: &[Entry]) -> Vec<String> {
        let mut names: Vec<String> = entries.iter().map(|entry| entry.name.clone()).collect();
        names.sort();
        names
    }

    fn files_under(dir: &Path) -> Vec<std::path::PathBuf> {
        let mut found = Vec::new();
        for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            if entry.path().is_dir() {
                found.extend(files_under(&entry.path()));
            } else {
                found.push(entry.path());
            }
        }
        found
    }

    #[test]
    fn an_account_lists_its_readable_collections() {
        let cache = tempfile::tempdir().unwrap();
        let http = arbesk(Arc::new(Mutex::new(world())));
        let entries = list(
            &http,
            &site(),
            &format!("arbesk://{ADDRESS}"),
            cache.path(),
            &Thumbnails::default(),
        )
        .unwrap();
        assert_eq!(names(&entries), ["Shapes", "Untitled Collection"]);
        let shapes_entry = entries.iter().find(|entry| entry.name == "Shapes").unwrap();
        assert!(shapes_entry.is_dir);
        assert_eq!(shapes_entry.path, shapes());
        assert_eq!(shapes_entry.modified, STAMP_MS / 1000);
        let untitled = entries
            .iter()
            .find(|entry| entry.name == "Untitled Collection")
            .unwrap();
        assert_eq!(untitled.modified, 0);
    }

    #[test]
    fn a_collection_lists_its_assets_as_files() {
        let cache = tempfile::tempdir().unwrap();
        let http = arbesk(Arc::new(Mutex::new(world())));
        let entries = list(
            &http,
            &site(),
            &shapes(),
            cache.path(),
            &Thumbnails::default(),
        )
        .unwrap();
        assert_eq!(
            names(&entries),
            ["Box.3mf", "Box.gltf", "Linked.gltf", "Odd"]
        );
        assert!(entries.iter().all(|entry| !entry.is_dir && entry.size == 0));
        let gltf = entries
            .iter()
            .find(|entry| entry.name == "Box.gltf")
            .unwrap();
        assert_eq!(gltf.path, asset_path(&shapes(), "a1", "Box", "gltf"));
        assert_eq!(gltf.modified, STAMP_MS / 1000);
        let paths: HashSet<&str> = entries.iter().map(|entry| entry.path.as_str()).collect();
        assert_eq!(paths.len(), entries.len());
    }

    #[test]
    fn an_unreachable_chain_fails_the_listing_instead_of_emptying_it() {
        let cache = tempfile::tempdir().unwrap();
        let world = Arc::new(Mutex::new(world()));
        world.lock().unwrap().rpc_down = true;
        let http = arbesk(world);
        let account = list(
            &http,
            &site(),
            &format!("arbesk://{ADDRESS}"),
            cache.path(),
            &Thumbnails::default(),
        );
        assert!(account.unwrap_err().starts_with(CHAIN_UNREACHABLE));
        assert!(
            list(
                &http,
                &site(),
                &shapes(),
                cache.path(),
                &Thumbnails::default()
            )
            .unwrap_err()
            .starts_with(CHAIN_UNREACHABLE)
        );
    }

    #[test]
    fn an_unreachable_gateway_fails_the_listing_instead_of_emptying_it() {
        let cache = tempfile::tempdir().unwrap();
        let world = Arc::new(Mutex::new(world()));
        world
            .lock()
            .unwrap()
            .failing
            .extend(["bafyC1", "bafyC2", "bafyA1"].map(String::from));
        let http = arbesk(world.clone());
        let account = list(
            &http,
            &site(),
            &format!("arbesk://{ADDRESS}"),
            cache.path(),
            &Thumbnails::default(),
        );
        assert!(account.unwrap_err().starts_with(STORAGE_UNREACHABLE));
        // One asset unreachable fails the collection too: Reload tries again.
        world.lock().unwrap().failing = ["bafyA1".to_string()].into();
        assert!(
            list(
                &http,
                &site(),
                &shapes(),
                cache.path(),
                &Thumbnails::default()
            )
            .unwrap_err()
            .starts_with(STORAGE_UNREACHABLE)
        );
    }

    #[test]
    fn an_asset_opens_at_its_studio_link() {
        let asset = asset_path(&shapes(), "a1", "Box", "gltf");
        assert_eq!(
            studio_url(&site(), &asset).unwrap(),
            format!("{ORIGIN}/studio?asset=1&assetId=a1")
        );
        // Ids are query values, encoded as the Studio's URLSearchParams reads them.
        let odd = asset_path(&shapes(), "a/b~c d", "Box", "gltf");
        assert_eq!(
            studio_url(&site(), &odd).unwrap(),
            format!("{ORIGIN}/studio?asset=1&assetId=a%2Fb%7Ec+d")
        );
    }

    #[test]
    fn only_an_asset_has_a_studio_link() {
        for path in [
            shapes(),
            format!("arbesk://{ADDRESS}"),
            "arbesk://bad".to_string(),
        ] {
            assert!(studio_url(&site(), &path).is_err(), "{path}");
        }
    }

    #[test]
    fn the_table_asset_resolves_to_two_parts() {
        let cache = tempfile::tempdir().unwrap();
        let http = arbesk(Arc::new(Mutex::new(world())));
        let resolved = scene(&http, &site(), &table(), None, cache.path()).unwrap();
        let parts: Vec<(&str, &str, usize)> = resolved
            .parts
            .iter()
            .map(|p| (p.key.as_str(), p.format, p.placements.len()))
            .collect();
        // The link to another chain is left out: two Box copies, not three.
        assert_eq!(
            parts,
            [("bafySrcTable", "glb", 1), ("bafySrcGltf", "glb", 2)]
        );
        assert_eq!(resolved.parts[1].placements[0].matrix[12], 2.57);
        assert!(scene(&http, &site(), &rooms(), None, cache.path()).is_err());
    }

    #[test]
    fn a_link_resolves_through_its_own_contract() {
        let cache = tempfile::tempdir().unwrap();
        let mut w = world();
        w.only_via.insert(1, "0xLINK".into());
        let http = arbesk(Arc::new(Mutex::new(w)));
        let resolved = scene(&http, &site(), &table(), None, cache.path()).unwrap();
        assert_eq!(resolved.parts.len(), 2);
    }

    #[test]
    fn an_assets_versions_are_its_manifest_chain_oldest_first() {
        let cache = tempfile::tempdir().unwrap();
        let http = arbesk(Arc::new(Mutex::new(world())));
        let found = versions(&http, &site(), &history(), cache.path()).unwrap();
        let ids: Vec<&str> = found.iter().map(|v| v.id.as_str()).collect();
        assert_eq!(ids, ["bafyH1", "bafyH2", "bafyH3"]);
        let stamps: Vec<Option<i64>> = found.iter().map(|v| v.timestamp).collect();
        assert_eq!(stamps, [Some(1000), Some(2000), Some(3000)]);
        // An asset saved once has one version.
        assert_eq!(
            versions(&http, &site(), &table(), cache.path())
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn a_chain_stops_at_fifty() {
        let cache = tempfile::tempdir().unwrap();
        let mut w = world();
        for i in 0..60 {
            let mut manifest = json!({ "timestamp": i, "scene": { "nodes": [] } });
            if i > 0 {
                manifest["prev_asset_manifest_cid"] = json!(format!("bafyL{}", i - 1));
            }
            w.put(&format!("bafyL{i}"), manifest);
        }
        w.put(
            "bafyC5",
            json!({ "type": "collection", "assets": { "h1": "bafyL59" } }),
        );
        let http = arbesk(Arc::new(Mutex::new(w)));
        let found = versions(&http, &site(), &history(), cache.path()).unwrap();
        assert_eq!(found.len(), MAX_VERSIONS);
        assert_eq!(found.last().unwrap().id, "bafyL59");
        assert_eq!(found[0].id, "bafyL10");
    }

    #[test]
    fn a_chain_stops_where_a_manifest_cant_be_read() {
        let cache = tempfile::tempdir().unwrap();
        let mut w = world();
        w.failing.insert("bafyH1".into());
        let http = arbesk(Arc::new(Mutex::new(w)));
        let found = versions(&http, &site(), &history(), cache.path()).unwrap();
        let ids: Vec<&str> = found.iter().map(|v| v.id.as_str()).collect();
        assert_eq!(ids, ["bafyH2", "bafyH3"]);
    }

    #[test]
    fn an_older_version_resolves_its_own_scene() {
        let cache = tempfile::tempdir().unwrap();
        let http = arbesk(Arc::new(Mutex::new(world())));
        let keys = |version: Option<&str>| {
            let resolved = scene(&http, &site(), &history(), version, cache.path()).unwrap();
            resolved
                .parts
                .iter()
                .map(|p| (p.key.clone(), p.format))
                .collect::<Vec<_>>()
        };
        assert_eq!(keys(Some("bafyH1")), [("bafySrc3mf".to_string(), "3mf")]);
        assert_eq!(keys(None), [("bafySrcGltf".to_string(), "glb")]);
        assert_eq!(keys(Some("bafyH3")), keys(None));
    }

    #[test]
    fn a_version_outside_the_chain_is_refused() {
        let cache = tempfile::tempdir().unwrap();
        let http = arbesk(Arc::new(Mutex::new(world())));
        let error = scene(&http, &site(), &history(), Some("bafyA1"), cache.path()).unwrap_err();
        assert_eq!(error, "This version is not in the asset.");
        let read = |cid: &str| http.requests().iter().any(|r| r.url.ends_with(cid));
        assert!(!read("bafyA1"));
    }

    #[test]
    fn a_link_to_self_is_in_the_assets_own_collection() {
        let cache = tempfile::tempdir().unwrap();
        let mut w = world();
        w.put_asset("bafyBench", Some("Bench"), "bafySrc3mf");
        w.put(
            "bafyCorner",
            json!({ "name": "Corner", "timestamp": STAMP_MS, "scene": { "nodes": [
            { "child_ref": { "collection": "self", "assetID": "t2" } }
        ] } }),
        );
        w.put("bafyC4", json!({ "type": "collection", "name": "Rooms", "assets": { "t1": "bafyTable", "t2": "bafyBench", "t3": "bafyCorner" } }));
        let http = arbesk(Arc::new(Mutex::new(w)));
        let corner = asset_path(&rooms(), "t3", "Corner", "3mf");
        let resolved = scene(&http, &site(), &corner, None, cache.path()).unwrap();
        let keys: Vec<&str> = resolved.parts.iter().map(|p| p.key.as_str()).collect();
        assert_eq!(keys, ["bafySrc3mf"]);
    }

    #[test]
    fn a_part_does_not_resolve_the_scene_again() {
        let cache = tempfile::tempdir().unwrap();
        let http = arbesk(Arc::new(Mutex::new(world())));
        let resolved = scene(&http, &site(), &table(), None, cache.path()).unwrap();
        let chain_calls = |http: &FakeHttp| {
            http.requests()
                .iter()
                .filter(|r| r.url == "https://rpc.test")
                .count()
        };
        let before = chain_calls(&http);
        part(
            &http,
            &site(),
            &resolved,
            "bafySrcGltf",
            cache.path(),
            &Progress::default(),
            &Builds::default(),
        )
        .unwrap();
        assert_eq!(chain_calls(&http), before);
    }

    /// A raw GLB and a zipped 3MF, as older saves store the model file itself.
    fn raw_glb() -> Vec<u8> {
        let mut glb = b"glTF".to_vec();
        glb.extend_from_slice(&[2, 0, 0, 0, 12, 0, 0, 0]);
        glb
    }

    #[test]
    fn an_older_saves_raw_file_previews_as_it_is() {
        let cache = tempfile::tempdir().unwrap();
        let mut w = world();
        w.files.insert("bafyRawGlb".into(), raw_glb());
        w.files
            .insert("bafyRawZip".into(), b"PK\x03\x04rest".to_vec());
        w.files.insert("bafyNotGlb".into(), b"<html>".to_vec());
        w.put(
            "bafyH1",
            json!({ "timestamp": 1000, "scene": { "nodes": [
            { "source": { "cid": "bafyRawGlb", "path": "asset.glb", "format": "glb" } },
            { "source": { "cid": "bafyRawZip", "path": "asset.3mf", "format": "3mf" } },
            { "source": { "cid": "bafyNotGlb", "path": "asset.glb", "format": "glb" } }
        ] } }),
        );
        let http = arbesk(Arc::new(Mutex::new(w)));
        let older = scene(&http, &site(), &history(), Some("bafyH1"), cache.path()).unwrap();
        let build = |key: &str| {
            part(
                &http,
                &site(),
                &older,
                key,
                cache.path(),
                &Progress::default(),
                &Builds::default(),
            )
        };
        let glb = build("bafyRawGlb").unwrap();
        assert_eq!(glb, cache.path().join("arbesk/bafyRawGlb/part.glb"));
        assert_eq!(std::fs::read(&glb).unwrap(), raw_glb());
        assert_eq!(
            std::fs::read(build("bafyRawZip").unwrap()).unwrap(),
            b"PK\x03\x04rest"
        );
        // Bytes that aren't what the node says are refused, not shown as a model.
        assert_eq!(build("bafyNotGlb").unwrap_err(), UNKNOWN_FORMAT);
    }

    #[test]
    fn a_part_is_built_once_and_reused() {
        let cache = tempfile::tempdir().unwrap();
        let http = arbesk(Arc::new(Mutex::new(world())));
        let built = part(
            &http,
            &site(),
            &table_scene(&http, cache.path()),
            "bafySrcGltf",
            cache.path(),
            &Progress::default(),
            &Builds::default(),
        )
        .unwrap();
        assert_eq!(built, cache.path().join("arbesk/bafySrcGltf/part.glb"));
        assert!(crate::cache::is_unchanged(&built));
        let theirs: Value = serde_json::from_slice(
            &std::fs::read(fixture("glb-box").join("expected.gltf")).unwrap(),
        )
        .unwrap();
        crate::compose::testing::assert_same_model(
            &std::fs::read(&built).unwrap(),
            &theirs,
        );
        let fetches = |http: &FakeHttp| {
            http.requests()
                .iter()
                .filter(|r| r.url.ends_with("bafymem00000000"))
                .count()
        };
        let before = fetches(&http);
        let again = part(
            &http,
            &site(),
            &table_scene(&http, cache.path()),
            "bafySrcGltf",
            cache.path(),
            &Progress::default(),
            &Builds::default(),
        )
        .unwrap();
        assert_eq!((again, fetches(&http)), (built, before));
    }

    #[test]
    fn a_key_outside_the_scene_is_refused() {
        let cache = tempfile::tempdir().unwrap();
        let http = arbesk(Arc::new(Mutex::new(world())));
        for key in ["bafySrc3mf", "../x", ""] {
            let error = part(
                &http,
                &site(),
                &table_scene(&http, cache.path()),
                key,
                cache.path(),
                &Progress::default(),
                &Builds::default(),
            )
            .unwrap_err();
            assert_eq!(error, "This part is not in the asset.", "{key}");
        }
    }

    #[test]
    fn the_table_asset_lists_as_gltf() {
        let cache = tempfile::tempdir().unwrap();
        let http = arbesk(Arc::new(Mutex::new(world())));
        let entries = list(
            &http,
            &site(),
            &rooms(),
            cache.path(),
            &Thumbnails::default(),
        )
        .unwrap();
        assert_eq!(names(&entries), ["Table.gltf"]);
        assert_eq!(entries[0].path, table());
    }

    #[test]
    fn listing_an_asset_is_refused() {
        let cache = tempfile::tempdir().unwrap();
        let http = arbesk(Arc::new(Mutex::new(world())));
        let asset = asset_path(&shapes(), "a1", "Box", "gltf");
        assert!(list(&http, &site(), &asset, cache.path(), &Thumbnails::default()).is_err());
    }

    #[test]
    fn a_built_asset_is_the_composed_file_and_is_reused() {
        let cache = tempfile::tempdir().unwrap();
        let world = Arc::new(Mutex::new(world()));
        let http = arbesk(world.clone());
        let asset = asset_path(&shapes(), "a1", "Box", "gltf");
        let built = build(
            &http,
            &site(),
            &asset,
            cache.path(),
            &Progress::default(),
            &Builds::default(),
        )
        .unwrap();
        assert!(built.starts_with(cache.path().join("arbesk")));
        // The preview's file is a GLB: what Arbesk's .gltf holds, without base64.
        assert_eq!(built.file_name().unwrap(), "Box.glb");
        let theirs: Value = serde_json::from_slice(
            &std::fs::read(fixture("glb-box").join("expected.gltf")).unwrap(),
        )
        .unwrap();
        crate::compose::testing::assert_same_model(
            &std::fs::read(&built).unwrap(),
            &theirs,
        );
        assert!(crate::cache::is_unchanged(&built));

        let part_fetches = |http: &FakeHttp| {
            http.requests()
                .iter()
                .filter(|r| r.url.ends_with("bafymem00000000"))
                .count()
        };
        let before = part_fetches(&http);
        let again = build(
            &http,
            &site(),
            &asset,
            cache.path(),
            &Progress::default(),
            &Builds::default(),
        )
        .unwrap();
        assert_eq!(again, built);
        assert_eq!(part_fetches(&http), before);
    }

    #[test]
    fn an_edited_copy_is_never_served_again() {
        let cache = tempfile::tempdir().unwrap();
        let http = arbesk(Arc::new(Mutex::new(world())));
        let asset = asset_path(&shapes(), "a1", "Box", "gltf");
        let built = build(
            &http,
            &site(),
            &asset,
            cache.path(),
            &Progress::default(),
            &Builds::default(),
        )
        .unwrap();
        std::fs::write(&built, b"{\"edited\":true}").unwrap();
        let fresh = build(
            &http,
            &site(),
            &asset,
            cache.path(),
            &Progress::default(),
            &Builds::default(),
        )
        .unwrap();
        assert_ne!(fresh, built);
        assert_eq!(std::fs::read(&built).unwrap(), b"{\"edited\":true}");
        assert_ne!(std::fs::read(&fresh).unwrap(), b"{\"edited\":true}");
    }

    #[test]
    fn a_failed_build_leaves_nothing_and_the_next_one_works() {
        let cache = tempfile::tempdir().unwrap();
        let world = Arc::new(Mutex::new(world()));
        world
            .lock()
            .unwrap()
            .failing
            .insert("bafymem00000000".into());
        let http = arbesk(world.clone());
        let asset = asset_path(&shapes(), "a1", "Box", "gltf");
        assert!(
            build(
                &http,
                &site(),
                &asset,
                cache.path(),
                &Progress::default(),
                &Builds::default()
            )
            .is_err()
        );
        assert_eq!(
            files_under(&cache.path().join("arbesk")),
            Vec::<std::path::PathBuf>::new()
        );

        world.lock().unwrap().failing.clear();
        let built = build(
            &http,
            &site(),
            &asset,
            cache.path(),
            &Progress::default(),
            &Builds::default(),
        )
        .unwrap();
        assert!(built.exists());
    }

    #[test]
    fn a_3mf_asset_builds() {
        let cache = tempfile::tempdir().unwrap();
        let http = arbesk(Arc::new(Mutex::new(world())));
        let asset = asset_path(&shapes(), "a2", "Box", "3mf");
        let built = build(
            &http,
            &site(),
            &asset,
            cache.path(),
            &Progress::default(),
            &Builds::default(),
        )
        .unwrap();
        assert_eq!(built.file_name().unwrap(), "Box.3mf");
        assert!(std::fs::read(&built).unwrap().starts_with(b"PK"));
    }

    #[test]
    fn an_exported_asset_is_the_file_arbesk_builds_and_is_reused() {
        let cache = tempfile::tempdir().unwrap();
        let http = arbesk(Arc::new(Mutex::new(world())));
        let export_of = |asset: &str| {
            export(&http, &site(), asset, cache.path(), &Progress::default(), &Builds::default())
        };
        let gltf = asset_path(&shapes(), "a1", "Box", "gltf");
        let exported = export_of(&gltf).unwrap();
        assert!(exported.starts_with(cache.path().join("arbesk")));
        // The listed name, holding Arbesk's own .gltf with every part inside.
        assert_eq!(exported.file_name().unwrap(), "Box.gltf");
        let ours: Value = serde_json::from_slice(&std::fs::read(&exported).unwrap()).unwrap();
        let theirs: Value = serde_json::from_slice(
            &std::fs::read(fixture("glb-box").join("expected.gltf")).unwrap(),
        )
        .unwrap();
        assert_eq!(ours, theirs);
        assert!(crate::cache::is_unchanged(&exported));
        assert_eq!(export_of(&gltf).unwrap(), exported);
        // The preview's GLB is another file beside it.
        let preview = build(&http, &site(), &gltf, cache.path(), &Progress::default(), &Builds::default()).unwrap();
        assert_eq!(preview.parent(), exported.parent());
        assert_ne!(preview, exported);

        let three_mf = export_of(&asset_path(&shapes(), "a2", "Box", "3mf")).unwrap();
        assert_eq!(three_mf.file_name().unwrap(), "Box.3mf");
        assert!(std::fs::read(&three_mf).unwrap().starts_with(b"PK"));
    }

    #[test]
    fn a_collection_linked_or_unknown_asset_is_not_exported() {
        let cache = tempfile::tempdir().unwrap();
        let http = arbesk(Arc::new(Mutex::new(world())));
        let export_of = |asset: &str| {
            export(&http, &site(), asset, cache.path(), &Progress::default(), &Builds::default())
        };
        for (id, name) in [("a3", "Linked.gltf"), ("a4", "Odd")] {
            assert_eq!(export_of(&format!("{}/{id}~{name}", shapes())).unwrap_err(), UNKNOWN_FORMAT);
        }
        assert_eq!(
            export_of(&shapes()).unwrap_err(),
            format!("“{}” is a folder, not a file.", shapes())
        );
        let progress = Progress::default();
        progress.cancel();
        let gltf = asset_path(&shapes(), "a1", "Box", "gltf");
        assert!(export(&http, &site(), &gltf, cache.path(), &progress, &Builds::default()).is_err());
        assert_eq!(files_under(&cache.path().join("arbesk")), Vec::<std::path::PathBuf>::new());
    }

    #[test]
    fn a_linked_or_unknown_asset_is_refused() {
        let cache = tempfile::tempdir().unwrap();
        let http = arbesk(Arc::new(Mutex::new(world())));
        for (id, name) in [("a3", "Linked.gltf"), ("a4", "Odd")] {
            let asset = format!("{}/{id}~{name}", shapes());
            assert_eq!(
                build(
                    &http,
                    &site(),
                    &asset,
                    cache.path(),
                    &Progress::default(),
                    &Builds::default()
                )
                .unwrap_err(),
                UNKNOWN_FORMAT
            );
        }
    }

    #[test]
    fn an_asset_gone_from_its_collection_is_an_error() {
        let cache = tempfile::tempdir().unwrap();
        let http = arbesk(Arc::new(Mutex::new(world())));
        let asset = asset_path(&shapes(), "nope", "Box", "gltf");
        let error = build(
            &http,
            &site(),
            &asset,
            cache.path(),
            &Progress::default(),
            &Builds::default(),
        )
        .unwrap_err();
        assert_eq!(error, "This asset is no longer in the collection.");
    }

    #[test]
    fn a_cancelled_build_writes_nothing() {
        let cache = tempfile::tempdir().unwrap();
        let http = arbesk(Arc::new(Mutex::new(world())));
        let progress = Progress::default();
        progress.cancel();
        let asset = asset_path(&shapes(), "a1", "Box", "gltf");
        assert!(
            build(
                &http,
                &site(),
                &asset,
                cache.path(),
                &progress,
                &Builds::default()
            )
            .is_err()
        );
        assert_eq!(
            files_under(&cache.path().join("arbesk")),
            Vec::<std::path::PathBuf>::new()
        );
    }

    #[test]
    fn two_builds_of_one_asset_at_once_make_one_file() {
        let cache = tempfile::tempdir().unwrap();
        let world = Arc::new(Mutex::new(world()));
        let slow = arbesk(world);
        // The part arrives slowly, so both builds are under way together.
        let http = FakeHttp::new(move |request| {
            if request.url.ends_with("bafymem00000000") {
                std::thread::sleep(std::time::Duration::from_millis(150));
            }
            match &request.body {
                Some(body) => crate::http::Http::post_json(&slow, &request.url, body, None),
                None => crate::http::Http::get(&slow, &request.url),
            }
        });
        let builds = Builds::default();
        let asset = asset_path(&shapes(), "a1", "Box", "gltf");
        let (first, second) = std::thread::scope(|scope| {
            let one = scope.spawn(|| {
                build(
                    &http,
                    &site(),
                    &asset,
                    cache.path(),
                    &Progress::default(),
                    &builds,
                )
            });
            let two = scope.spawn(|| {
                build(
                    &http,
                    &site(),
                    &asset,
                    cache.path(),
                    &Progress::default(),
                    &builds,
                )
            });
            (one.join().unwrap(), two.join().unwrap())
        });
        assert_eq!(first.as_ref().unwrap(), second.as_ref().unwrap());
        let parts = http
            .requests()
            .iter()
            .filter(|r| r.url.ends_with("bafymem00000000"))
            .count();
        assert_eq!(parts, 1);
        assert_eq!(
            files_under(&cache.path().join("arbesk"))
                .iter()
                .filter(|f| !f.to_string_lossy().contains(".downloaded"))
                .count(),
            1
        );
        assert!(crate::cache::is_unchanged(first.as_ref().unwrap()));
    }

    #[test]
    fn a_name_that_is_not_a_file_name_is_made_one() {
        assert_eq!(file_name("a/b", "gltf"), "a_b.gltf");
        assert_eq!(file_name("..", "3mf"), "_.3mf");
        assert_eq!(file_name("", "gltf"), "_.gltf");
    }

    fn box_gltf() -> String {
        asset_path(&shapes(), "a1", "Box", "gltf")
    }

    fn box_3mf() -> String {
        asset_path(&shapes(), "a2", "Box", "3mf")
    }

    fn png_size(png: &[u8]) -> (u32, u32) {
        use image::GenericImageView;
        assert_eq!(image::guess_format(png).unwrap(), image::ImageFormat::Png);
        image::load_from_memory(png).unwrap().dimensions()
    }

    /// The URLs asked for since request number `from`.
    fn asked_since(http: &FakeHttp, from: usize) -> Vec<String> {
        http.requests()[from..]
            .iter()
            .map(|request| request.url.clone())
            .collect()
    }

    #[test]
    fn a_listed_asset_shows_its_own_thumbnail_after_one_request() {
        let cache = tempfile::tempdir().unwrap();
        let http = arbesk(Arc::new(Mutex::new(world())));
        let thumbs = Thumbnails::default();
        list(&http, &site(), &shapes(), cache.path(), &thumbs).unwrap();
        let before = http.requests().len();
        let png = thumbnail(
            &http,
            &site(),
            &box_gltf(),
            256,
            cache.path(),
            &thumbs,
            &|| false,
        )
        .expect("Box.gltf was saved with a thumbnail");
        assert_eq!(png_size(&png), (256, 192));
        // Neither the backend nor the chain: the listing found where it is.
        assert_eq!(
            asked_since(&http, before),
            ["https://gw.test/ipfs/bafyThumb"]
        );
    }

    #[test]
    fn an_asset_saved_without_a_thumbnail_has_none_and_asks_nobody() {
        let cache = tempfile::tempdir().unwrap();
        let http = arbesk(Arc::new(Mutex::new(world())));
        let thumbs = Thumbnails::default();
        list(&http, &site(), &shapes(), cache.path(), &thumbs).unwrap();
        let before = http.requests().len();
        assert!(
            thumbnail(
                &http,
                &site(),
                &box_3mf(),
                256,
                cache.path(),
                &thumbs,
                &|| false
            )
            .is_none()
        );
        assert!(asked_since(&http, before).is_empty());
    }

    #[test]
    fn an_asset_not_listed_yet_finds_its_thumbnail_through_its_collection() {
        let cache = tempfile::tempdir().unwrap();
        let http = arbesk(Arc::new(Mutex::new(world())));
        let thumbs = Thumbnails::default();
        let png = thumbnail(
            &http,
            &site(),
            &box_gltf(),
            128,
            cache.path(),
            &thumbs,
            &|| false,
        )
        .expect("found through the chain");
        assert_eq!(png_size(&png), (128, 96));
        // Found once, remembered: the next one goes straight to the picture.
        let before = http.requests().len();
        assert!(
            thumbnail(
                &http,
                &site(),
                &box_gltf(),
                128,
                cache.path(),
                &thumbs,
                &|| false
            )
            .is_some()
        );
        assert_eq!(
            asked_since(&http, before),
            ["https://gw.test/ipfs/bafyThumb"]
        );
    }

    #[test]
    fn a_new_version_saved_without_a_thumbnail_drops_the_old_one() {
        let cache = tempfile::tempdir().unwrap();
        let world = Arc::new(Mutex::new(world()));
        let http = arbesk(world.clone());
        let thumbs = Thumbnails::default();
        list(&http, &site(), &shapes(), cache.path(), &thumbs).unwrap();
        assert!(
            thumbnail(
                &http,
                &site(),
                &box_gltf(),
                256,
                cache.path(),
                &thumbs,
                &|| false
            )
            .is_some()
        );
        {
            let mut w = world.lock().unwrap();
            w.put_asset("bafyA1v2", Some("Box"), "bafySrcGltf");
            let mut collection: Value = serde_json::from_slice(&w.files["bafyC1"]).unwrap();
            collection["assets"]["a1"] = json!("bafyA1v2");
            w.put("bafyC1v2", collection);
            w.tokens.insert(1, "bafyC1v2".into());
        }
        list(&http, &site(), &shapes(), cache.path(), &thumbs).unwrap();
        assert!(
            thumbnail(
                &http,
                &site(),
                &box_gltf(),
                256,
                cache.path(),
                &thumbs,
                &|| false
            )
            .is_none()
        );
    }

    #[test]
    fn a_thumbnail_that_cannot_be_read_is_no_thumbnail() {
        let cache = tempfile::tempdir().unwrap();
        let world = Arc::new(Mutex::new(world()));
        let http = arbesk(world.clone());
        let thumbs = Thumbnails::default();
        list(&http, &site(), &shapes(), cache.path(), &thumbs).unwrap();
        world.lock().unwrap().failing.insert("bafyThumb".into());
        assert!(
            thumbnail(
                &http,
                &site(),
                &box_gltf(),
                256,
                cache.path(),
                &thumbs,
                &|| false
            )
            .is_none()
        );
        {
            let mut w = world.lock().unwrap();
            w.failing.clear();
            w.put("bafyThumb", json!({ "not": "a picture" }));
        }
        assert!(
            thumbnail(
                &http,
                &site(),
                &box_gltf(),
                256,
                cache.path(),
                &thumbs,
                &|| false
            )
            .is_none()
        );
        // Folders, assets gone from the collection and other accounts' paths have none either.
        for path in [
            shapes(),
            asset_path(&shapes(), "a9", "Gone", "gltf"),
            "arbesk://not-an-address/1~x/a1~y.gltf".to_string(),
        ] {
            assert!(
                thumbnail(&http, &site(), &path, 256, cache.path(), &thumbs, &|| false).is_none(),
                "{path}"
            );
        }
    }

    #[test]
    fn a_cancelled_thumbnail_asks_nobody() {
        let cache = tempfile::tempdir().unwrap();
        let http = arbesk(Arc::new(Mutex::new(world())));
        let thumbs = Thumbnails::default();
        list(&http, &site(), &shapes(), cache.path(), &thumbs).unwrap();
        let before = http.requests().len();
        assert!(
            thumbnail(
                &http,
                &site(),
                &box_gltf(),
                256,
                cache.path(),
                &thumbs,
                &|| true
            )
            .is_none()
        );
        assert!(asked_since(&http, before).is_empty());
    }

    #[test]
    fn a_renamed_asset_keeps_its_thumbnail_by_its_ids() {
        let cache = tempfile::tempdir().unwrap();
        let http = arbesk(Arc::new(Mutex::new(world())));
        let thumbs = Thumbnails::default();
        list(&http, &site(), &shapes(), cache.path(), &thumbs).unwrap();
        let before = http.requests().len();
        let renamed = asset_path(
            &collection_path(ADDRESS, "1", "Old name"),
            "a1",
            "Crate",
            "gltf",
        );
        assert!(
            thumbnail(
                &http,
                &site(),
                &renamed,
                128,
                cache.path(),
                &thumbs,
                &|| false
            )
            .is_some()
        );
        assert_eq!(
            asked_since(&http, before),
            ["https://gw.test/ipfs/bafyThumb"]
        );
    }

    #[test]
    fn listing_a_collection_again_forgets_assets_no_longer_in_it() {
        let cache = tempfile::tempdir().unwrap();
        let world = Arc::new(Mutex::new(world()));
        let http = arbesk(world.clone());
        let thumbs = Thumbnails::default();
        list(&http, &site(), &shapes(), cache.path(), &thumbs).unwrap();
        {
            let mut w = world.lock().unwrap();
            let mut collection: Value = serde_json::from_slice(&w.files["bafyC1"]).unwrap();
            collection["assets"].as_object_mut().unwrap().remove("a1");
            w.put("bafyC1v3", collection);
            w.tokens.insert(1, "bafyC1v3".into());
        }
        list(&http, &site(), &shapes(), cache.path(), &thumbs).unwrap();
        // Not the remembered picture: Box.gltf left the collection.
        assert!(
            thumbnail(
                &http,
                &site(),
                &box_gltf(),
                256,
                cache.path(),
                &thumbs,
                &|| false
            )
            .is_none()
        );
    }

    #[test]
    fn forgetting_every_thumbnail_leaves_none_known() {
        let cache = tempfile::tempdir().unwrap();
        let http = arbesk(Arc::new(Mutex::new(world())));
        let thumbs = Thumbnails::default();
        list(&http, &site(), &shapes(), cache.path(), &thumbs).unwrap();
        let key = asset_key(ADDRESS, "1", "a1");
        assert!(thumbs.known(&key).is_some());
        thumbs.clear();
        assert!(thumbs.known(&key).is_none());
    }

    #[test]
    fn the_thumbnail_cid_reads_every_shape_arbesk_has_saved() {
        // Today an object; older manifests kept a bare CID or a `source`.
        assert_eq!(
            thumbnail_cid(&json!({ "thumbnail": { "cid": "bafyT" } })),
            Some("bafyT")
        );
        assert_eq!(
            thumbnail_cid(&json!({ "thumbnail": "bafyT" })),
            Some("bafyT")
        );
        assert_eq!(
            thumbnail_cid(&json!({ "thumbnail": { "source": { "cid": "bafyT" } } })),
            Some("bafyT")
        );
        assert_eq!(
            thumbnail_cid(&json!({ "thumbnail": { "cid": "", "source": { "cid": "bafyT" } } })),
            Some("bafyT")
        );
        for none in [
            json!({}),
            json!({ "thumbnail": null }),
            json!({ "thumbnail": "" }),
            json!({ "thumbnail": { "cid": 7 } }),
            json!({ "thumbnail": { "path": "thumbnail.webp" } }),
        ] {
            assert_eq!(thumbnail_cid(&none), None, "{none}");
        }
    }
}
