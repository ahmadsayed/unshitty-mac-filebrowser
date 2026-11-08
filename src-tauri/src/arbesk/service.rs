//! An Arbesk account as folders and files: its collections, a collection's
//! assets, and an asset built into the app cache to open or preview.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Condvar, Mutex};

use serde_json::Value;

use crate::arbesk::chain::{self, CHAIN_UNREACHABLE, ChainConfig, Site};
use crate::arbesk::compose::{UNKNOWN_FORMAT, compose, format_of, is_child_ref, source_cid};
use crate::arbesk::http::Http;
use crate::arbesk::ipfs::{Gateway, ManifestCache, STORAGE_UNREACHABLE, in_parallel};
use crate::arbesk::location::{ArbeskPath, asset_path_named, collection_path, parse};
use crate::fs_ops::Entry;
use crate::progress::Progress;

/// How many requests a listing runs at once.
const PARALLEL: usize = 8;
/// The manifest cache's cap.
const MANIFEST_CACHE_BYTES: u64 = 64 * 1024 * 1024;
const UNTITLED_COLLECTION: &str = "Untitled Collection";

/// The folder or file entries at `path`: an account's collections, or a
/// collection's assets. Rows that can't be read are left out.
pub fn list(
    http: &dyn Http,
    site: &Site,
    path: &str,
    cache_dir: &Path,
) -> Result<Vec<Entry>, String> {
    let reader = Reader::open(http, site, cache_dir)?;
    match parse(path)? {
        ArbeskPath::Account { address } => reader.collections(&address),
        ArbeskPath::Collection { token_id, .. } => reader.assets(path, &token_id),
        ArbeskPath::Asset { .. } => Err(format!("“{path}” is a file, not a folder.")),
    }
}

/// The asset at `path` as a file in `cache_dir/arbesk`, built once per
/// version (its manifest CID) and reused while nobody has changed it.
pub fn build(
    http: &dyn Http,
    site: &Site,
    path: &str,
    cache_dir: &Path,
    progress: &Progress,
    builds: &Builds,
) -> Result<PathBuf, String> {
    progress.check()?;
    let ArbeskPath::Asset {
        token_id, asset_id, ..
    } = parse(path)?
    else {
        return Err(format!("“{path}” is a folder, not a file."));
    };
    let reader = Reader::open(http, site, cache_dir)?;
    let collection = reader.collection_manifest(&token_id)?;
    let cid = collection["assets"][&asset_id]
        .as_str()
        .ok_or("This asset is no longer in the collection.")?;
    let manifest = reader.gateway.json(cid)?;
    if is_child_ref(&manifest) {
        return Err(UNKNOWN_FORMAT.into());
    }
    let source = reader.gateway.json(&source_cid(&manifest, cid))?;
    let name = file_name(
        &display_name(&manifest, &asset_id),
        format_of(&source)?.ext(),
    );
    let dir = cache_dir.join("arbesk").join(cid);
    let target = dir.join(&name);
    // Space then Open asks twice at once: the second waits, then reuses the file.
    let _held = builds.hold(dir.clone());
    if crate::downloads::is_unchanged(&target) {
        return Ok(target);
    }
    // Someone edited the copy at `target`: theirs stays, a fresh one goes beside it.
    let target = if target.exists() {
        dir.join(unique_folder()).join(&name)
    } else {
        target
    };
    let bytes = compose(&reader.gateway, &source)?;
    progress.check()?;
    save(&target, &bytes)?;
    Ok(target)
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
struct Reader<'a> {
    http: &'a dyn Http,
    site: &'a Site,
    chain: ChainConfig,
    gateway: Gateway<'a>,
}

impl<'a> Reader<'a> {
    fn open(http: &'a dyn Http, site: &'a Site, cache_dir: &Path) -> Result<Self, String> {
        let chain = chain::config(http, site)?;
        let cache = ManifestCache::new(cache_dir.join("arbesk-manifests"), MANIFEST_CACHE_BYTES);
        let gateway = Gateway::new(http, &chain.gateway, cache);
        Ok(Self {
            http,
            site,
            chain,
            gateway,
        })
    }

    fn collection_manifest(&self, token_id: &str) -> Result<Value, String> {
        self.gateway
            .json(&chain::token_uri(self.http, &self.chain, token_id)?)
    }

    fn collections(&self, address: &str) -> Result<Vec<Entry>, String> {
        let ids = chain::collection_ids(self.http, self.site, address)?;
        readable(in_parallel(&ids, PARALLEL, |id| {
            self.collection_entry(address, id)
        }))
    }

    fn collection_entry(&self, address: &str, token_id: &str) -> Result<Entry, String> {
        let manifest = self.collection_manifest(token_id)?;
        let name = text(&manifest["name"])
            .unwrap_or(UNTITLED_COLLECTION)
            .to_string();
        let path = collection_path(address, token_id, &name);
        Ok(entry(name, path, true, &manifest))
    }

    fn assets(&self, path: &str, token_id: &str) -> Result<Vec<Entry>, String> {
        let manifest = self.collection_manifest(token_id)?;
        let assets: Vec<(String, String)> = manifest["assets"]
            .as_object()
            .into_iter()
            .flatten()
            .filter_map(|(id, cid)| Some((id.clone(), cid.as_str()?.to_string())))
            .collect();
        readable(in_parallel(&assets, PARALLEL, |(id, cid)| {
            self.asset_entry(path, id, cid)
        }))
    }

    /// An asset's row: its name with its format's extension; a live link
    /// shows as glTF, an unknown format without an extension.
    fn asset_entry(&self, collection: &str, asset_id: &str, cid: &str) -> Result<Entry, String> {
        let manifest = self.gateway.json(cid)?;
        let ext = if is_child_ref(&manifest) {
            Some("gltf")
        } else {
            format_of(&self.gateway.json(&source_cid(&manifest, cid))?)
                .ok()
                .map(|format| format.ext())
        };
        let base = display_name(&manifest, asset_id);
        let name = ext.map_or_else(|| base.clone(), |ext| format!("{base}.{ext}"));
        let path = asset_path_named(collection, asset_id, &name);
        Ok(entry(name, path, false, &manifest))
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
            Err(error)
                if error.starts_with(CHAIN_UNREACHABLE)
                    || error.starts_with(STORAGE_UNREACHABLE) =>
            {
                return Err(error);
            }
            Err(_) => {}
        }
    }
    Ok(entries)
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
        cloud: None,
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
        std::fs::write(&part, bytes).and_then(|()| crate::placement::rename_new(&part, target));
    if let Err(error) = written {
        let _ = std::fs::remove_file(&part);
        // Another build placed the same file first: it is the one to open.
        if error.kind() == std::io::ErrorKind::AlreadyExists
            && crate::downloads::is_unchanged(target)
        {
            return Ok(());
        }
        return Err(failed(error));
    }
    crate::downloads::mark(target);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arbesk::chain::CHAIN_UNREACHABLE;
    use crate::arbesk::chain::testing::abi_string;
    use crate::arbesk::compose::UNKNOWN_FORMAT;
    use crate::arbesk::http::HttpError;
    use crate::arbesk::http::fake::{FakeHttp, not_found};
    use crate::arbesk::ipfs::STORAGE_UNREACHABLE;
    use crate::arbesk::location::{asset_path, collection_path};
    use serde_json::{Value, json};
    use std::collections::{HashMap, HashSet};
    use std::path::Path;
    use std::sync::{Arc, Mutex};

    const ORIGIN: &str = "https://arbesk.test";
    const ADDRESS: &str = "0x00000000000000000000000000000000000000ab";
    const STAMP_MS: i64 = 1_762_000_000_000;

    fn site() -> Site {
        Site {
            origin: ORIGIN.into(),
            chain_id: 84532,
        }
    }

    /// What the fake Arbesk holds: IPFS content by CID, each token's
    /// manifest CID, the index's answer, and CIDs whose fetch fails.
    #[derive(Default)]
    struct World {
        files: HashMap<String, Vec<u8>>,
        tokens: HashMap<u64, String>,
        owned: Vec<String>,
        failing: HashSet<String>,
        rpc_down: bool,
    }

    fn fixture(case: &str) -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/arbesk")
            .join(case)
    }

    impl World {
        fn put(&mut self, cid: &str, value: Value) {
            self.files
                .insert(cid.into(), value.to_string().into_bytes());
        }

        /// A fixture's stored source and parts, under `cid` for the source.
        /// A source that holds all its parts has no `cids/` (git keeps no
        /// empty folder).
        fn put_fixture(&mut self, case: &str, cid: &str) {
            self.files.insert(
                cid.into(),
                std::fs::read(fixture(case).join("source.json")).unwrap(),
            );
            for part in std::fs::read_dir(fixture(case).join("cids"))
                .into_iter()
                .flatten()
                .flatten()
            {
                let name = part.file_name().to_string_lossy().into_owned();
                self.files.insert(name, std::fs::read(part.path()).unwrap());
            }
        }

        /// An asset manifest that wraps the source at `source`.
        fn put_asset(&mut self, cid: &str, name: Option<&str>, source: &str) {
            let mut manifest = json!({ "type": "asset", "timestamp": STAMP_MS, "scene": { "nodes": [{ "source": { "cid": source } }] } });
            if let Some(name) = name {
                manifest["name"] = json!(name);
            }
            self.put(cid, manifest);
        }
    }

    /// Shapes (token 1): Box as glTF and as 3MF, a live link, an asset of an
    /// unknown format and one whose manifest is gone; an untitled collection
    /// (token 2); token 3 points at a manifest that can't be read.
    fn world() -> World {
        let mut w = World::default();
        w.put_fixture("glb-box", "bafySrcGltf");
        w.put_fixture("3mf-box", "bafySrc3mf");
        w.put(
            "bafySrcOdd",
            json!({ "arbesk_format": "composite-example" }),
        );
        w.put_asset("bafyA1", Some("Box"), "bafySrcGltf");
        w.put_asset("bafyA2", Some("Box"), "bafySrc3mf");
        w.put_asset("bafyA4", Some("Odd"), "bafySrcOdd");
        w.put("bafyA3", json!({ "name": "Linked", "scene": { "nodes": [{ "child_ref": { "assetID": "x" } }] } }));
        w.put(
            "bafyC1",
            json!({ "type": "collection", "name": "Shapes", "timestamp": STAMP_MS, "assets": {
                "a1": "bafyA1", "a2": "bafyA2", "a3": "bafyA3", "a4": "bafyA4", "a5": "bafyGone"
            } }),
        );
        w.put(
            "bafyC2",
            json!({ "type": "collection", "assets": { "u1": "bafyA1" } }),
        );
        w.tokens.insert(1, "bafyC1".into());
        w.tokens.insert(2, "bafyC2".into());
        w.tokens.insert(3, "bafyGone".into());
        w.owned = vec!["1".into(), "2".into(), "3".into()];
        w
    }

    fn arbesk(world: Arc<Mutex<World>>) -> FakeHttp {
        FakeHttp::new(move |request| {
            let w = world.lock().unwrap();
            let url = request.url.as_str();
            if url == format!("{ORIGIN}/api/v1/config") {
                let config = json!({
                    "ipfsGatewayUrl": "https://gw.test/ipfs/",
                    "networkConfigs": { "84532": { "contractAddress": "0xC0", "rpcUrl": "https://rpc.test" } }
                });
                return Ok(config.to_string().into_bytes());
            }
            if url.starts_with(&format!("{ORIGIN}/api/v1/indexer/owned")) {
                return Ok(json!({ "owned": w.owned }).to_string().into_bytes());
            }
            if url.starts_with(&format!("{ORIGIN}/api/v1/indexer/shared")) {
                return Ok(json!({ "shared": [] }).to_string().into_bytes());
            }
            if url == "https://rpc.test" && w.rpc_down {
                return Err(HttpError::new(None, "connection refused"));
            }
            if url == "https://rpc.test" {
                let data = request.body.as_ref().unwrap()["params"][0]["data"]
                    .as_str()
                    .unwrap()
                    .to_string();
                let token = u64::from_str_radix(&data[data.len() - 16..], 16).unwrap();
                let answer = match w.tokens.get(&token) {
                    Some(cid) => json!({ "result": abi_string(&format!("ipfs://{cid}")) }),
                    None => json!({ "error": { "message": "execution reverted" } }),
                };
                return Ok(answer.to_string().into_bytes());
            }
            let cid = url
                .strip_prefix("https://gw.test/ipfs/")
                .ok_or_else(not_found)?;
            if w.failing.contains(cid) {
                return Err(HttpError::new(Some(502), "Bad gateway"));
            }
            w.files.get(cid).cloned().ok_or_else(not_found)
        })
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

    fn shapes() -> String {
        collection_path(ADDRESS, "1", "Shapes")
    }

    #[test]
    fn an_account_lists_its_readable_collections() {
        let cache = tempfile::tempdir().unwrap();
        let http = arbesk(Arc::new(Mutex::new(world())));
        let entries = list(&http, &site(), &format!("arbesk://{ADDRESS}"), cache.path()).unwrap();
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
        let entries = list(&http, &site(), &shapes(), cache.path()).unwrap();
        assert_eq!(
            names(&entries),
            ["Box.3mf", "Box.gltf", "Linked.gltf", "Odd"]
        );
        assert!(
            entries
                .iter()
                .all(|entry| !entry.is_dir && entry.size == 0 && entry.cloud.is_none())
        );
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
        let account = list(&http, &site(), &format!("arbesk://{ADDRESS}"), cache.path());
        assert!(account.unwrap_err().starts_with(CHAIN_UNREACHABLE));
        assert!(
            list(&http, &site(), &shapes(), cache.path())
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
        let account = list(&http, &site(), &format!("arbesk://{ADDRESS}"), cache.path());
        assert!(account.unwrap_err().starts_with(STORAGE_UNREACHABLE));
        // One asset unreachable fails the collection too: Reload tries again.
        world.lock().unwrap().failing = ["bafyA1".to_string()].into();
        assert!(
            list(&http, &site(), &shapes(), cache.path())
                .unwrap_err()
                .starts_with(STORAGE_UNREACHABLE)
        );
    }

    #[test]
    fn listing_an_asset_is_refused() {
        let cache = tempfile::tempdir().unwrap();
        let http = arbesk(Arc::new(Mutex::new(world())));
        let asset = asset_path(&shapes(), "a1", "Box", "gltf");
        assert!(list(&http, &site(), &asset, cache.path()).is_err());
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
        assert_eq!(built.file_name().unwrap(), "Box.gltf");
        let ours: Value = serde_json::from_slice(&std::fs::read(&built).unwrap()).unwrap();
        let theirs: Value = serde_json::from_slice(
            &std::fs::read(fixture("glb-box").join("expected.gltf")).unwrap(),
        )
        .unwrap();
        assert_eq!(ours, theirs);
        assert!(crate::downloads::is_unchanged(&built));

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
                Some(body) => crate::arbesk::http::Http::post_json(&slow, &request.url, body, None),
                None => crate::arbesk::http::Http::get(&slow, &request.url),
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
        assert!(crate::downloads::is_unchanged(first.as_ref().unwrap()));
    }

    #[test]
    fn a_name_that_is_not_a_file_name_is_made_one() {
        assert_eq!(file_name("a/b", "gltf"), "a_b.gltf");
        assert_eq!(file_name("..", "3mf"), "_.3mf");
        assert_eq!(file_name("", "gltf"), "_.gltf");
    }
}
