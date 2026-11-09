//! Writing a model file into an Arbesk collection, as the Studio's upload
//! (`library-ops.ts`'s `uploadFileToCollection`) does: split the file as
//! Arbesk stores it (`decompose`), upload its parts and composite to IPFS
//! through the pre-signed URLs Arbesk hands out, then its asset manifest;
//! then, under a lock per collection, read `tokenURI` again, upload the
//! collection manifest with the asset in it and ask Arbesk's wallet relay
//! to point the token at it (spec D7). The session goes only to Arbesk's
//! own endpoints, never to the upload URLs, and never into an error.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::Path;
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::chain::Site;
use crate::compose::Format;
use crate::decompose::{Decomposed, decompose, fill, sanitize_file_name, stored_json};
use crate::http::{Http, HttpError};
use crate::location::{asset_path_named, collection_path};
use crate::service::{Reader, collection_name};
use crate::signin::now_ms;
use crate::progress::{Phase, Progress};

// ---- Wording ----------------------------------------------------------------

/// No session, an expired one, or Arbesk's 401.
pub(crate) const SIGN_IN: &str = "Sign in to Arbesk again to copy files into it.";
pub(crate) const FORBIDDEN: &str = "You can't add to this collection.";
pub(crate) const BUSY: &str = "Arbesk is busy. Try again in a minute.";
/// The relay's 502: most often an expired CDP delegation.
pub const RELAY_FAILED: &str =
    "Arbesk couldn't save the change. Sign in again to renew its permission.";
/// Upload credentials for a local deployment's Kubo, which only its own
/// machine reaches.
pub const UNSUPPORTED_STORAGE: &str = "This Arbesk server's storage isn't supported.";
const UNREADABLE: &str = "Arbesk's answer can't be read.";
/// An earlier write from this app may still change the collection.
pub const EARLIER_UNSETTLED: &str =
    "Arbesk is still saving an earlier copy into this collection. Try again in a minute.";

fn exists(name: &str) -> String {
    format!("Target already exists: {name}")
}

fn still_saving(name: &str) -> String {
    format!("Arbesk is still saving “{name}”. It appears once the change is confirmed.")
}

fn not_saved(name: &str) -> String {
    format!("Arbesk couldn't save “{name}”.")
}

/// What a failed request to Arbesk itself means; `step` words any other
/// failure, saying which step it was.
fn refused(error: HttpError, step: impl FnOnce(&str) -> String) -> String {
    match error.status {
        Some(401) => SIGN_IN.into(),
        Some(403) => FORBIDDEN.into(),
        Some(429) => BUSY.into(),
        _ => step(&error.message),
    }
}

// ---- Manifests ----------------------------------------------------------------

/// What `updateUri` scopes a collection's change to: none.
const NO_SCOPE: &str = "0x0000000000000000000000000000000000000000000000000000000000000000";
const IDENTITY: [u8; 16] = [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1];

/// Where a file goes: a collection, and the name it is listed under there
/// (`Box.gltf` for a `Box.glb`).
pub struct Target {
    pub address: String,
    pub token_id: String,
    pub name: String,
}

/// The asset manifest the Studio's upload writes: one `source_asset` node
/// holding the stored source, unmoved and uncoloured; `prev` (the asset's
/// manifest before, on Replace) links its history.
pub fn asset_manifest(
    name: &str,
    asset_id: &str,
    version: u64,
    prev: Option<&str>,
    timestamp_ms: i64,
    source_cid: &str,
    format: Format,
) -> Value {
    let path = match format {
        Format::Gltf => "composite.gltf",
        Format::ThreeMf => "composite.3mf.json",
    };
    let node = json!({
        "node_id": "node_1",
        "type": "source_asset",
        "name": name,
        "source": { "cid": source_cid, "path": path, "format": format.ext() },
        "transform_matrix": IDENTITY,
        "post_processor": { "color": null, "scale": { "x": 1, "y": 1, "z": 1 } },
    });
    let mut manifest = json!({
        "type": "asset",
        "name": name,
        "asset_id": asset_id,
        "version": version,
        "timestamp": timestamp_ms,
        "scene": { "nodes": [node] },
    });
    if let Some(prev) = prev {
        manifest["prev_asset_manifest_cid"] = json!(prev);
    }
    manifest
}

/// The collection manifest `current` (at `current_cid`) with `asset_id` at
/// `asset_cid`, as asset-core's `applyCollectionMutation` makes it: the
/// rest kept, `version` + 1, `prev_asset_manifest_cid` = `current_cid`.
/// A manifest that isn't a JSON object is refused: its name and type
/// would be lost.
pub fn next_collection(
    current: &Value,
    current_cid: &str,
    asset_id: &str,
    asset_cid: &str,
) -> Result<Value, String> {
    if !current.is_object() {
        return Err(UNREADABLE.into());
    }
    let mut next = current.clone();
    let mut assets = current["assets"].as_object().cloned().unwrap_or_default();
    assets.insert(asset_id.into(), json!(asset_cid));
    next["assets"] = Value::Object(assets);
    next["version"] = json!(current["version"].as_u64().unwrap_or(0) + 1);
    next["prev_asset_manifest_cid"] = json!(current_cid);
    Ok(next)
}

/// An asset's name from its listed name: the Studio's
/// `baseNameWithoutExtension`.
fn asset_name(listed: &str) -> String {
    let base = match listed.rsplit_once('.') {
        Some((base, ext)) if !ext.is_empty() => base,
        _ => listed,
    };
    if base.is_empty() {
        "Uploaded Asset".into()
    } else {
        base.into()
    }
}

/// A new asset's id, as the Studio makes one (`asset_<ms>`), not yet in
/// `collection`.
fn fresh_id(collection: &Value, mut ms: i64) -> String {
    loop {
        let id = format!("asset_{ms}");
        if collection["assets"].get(&id).is_none() {
            return id;
        }
        ms += 1;
    }
}

// ---- The lock ---------------------------------------------------------------

/// One write at a time per collection, from reading its `tokenURI` to the
/// relay's answer, so two copies from this app never lose each other's
/// asset. (A Studio save in the same seconds still can: Arbesk has no
/// compare-and-swap.)
///
/// The relay's answer isn't always the end: a change whose receipt isn't
/// known yet (or whose answer was lost) may still land, and a confirmed
/// one may not show yet on a node a block behind. So each collection's
/// last relayed change is remembered, and the next write builds only on
/// a `tokenURI` that has settled ([`Hold::settled`]), re-reading `reads`
/// times `every` apart, else refuses. An unsettled change is forgotten
/// `expiry` after the relay answered: by then it has landed or never will.
pub struct CollectionLocks {
    held: Mutex<HashSet<String>>,
    freed: Condvar,
    relayed: Mutex<HashMap<String, Relayed>>,
    reads: usize,
    every: Duration,
    expiry: Duration,
}

/// A collection's last change relayed from this app.
struct Relayed {
    /// The collection manifest it replaced, and the one it points to.
    replaced: String,
    relayed: String,
    /// Its receipt said it was mined.
    landed: bool,
    at: Instant,
}

impl Default for CollectionLocks {
    fn default() -> Self {
        Self {
            held: Mutex::default(),
            freed: Condvar::default(),
            relayed: Mutex::default(),
            reads: 6,
            every: Duration::from_secs(5),
            expiry: Duration::from_secs(300),
        }
    }
}

/// A write's hold on its collection, let go when dropped.
struct Hold<'a> {
    locks: &'a CollectionLocks,
    key: String,
}

impl CollectionLocks {
    /// Wait for collection `token_id` of `address`, then hold it.
    #[cfg(test)]
    pub fn hold(&self, address: &str, token_id: &str) -> impl Drop + '_ {
        self.take(address, token_id)
    }

    fn take(&self, address: &str, token_id: &str) -> Hold<'_> {
        let key = format!("{}/{token_id}", address.to_ascii_lowercase());
        let mut held = self.held.lock().unwrap_or_else(|e| e.into_inner());
        while held.contains(&key) {
            held = self.freed.wait(held).unwrap_or_else(|e| e.into_inner());
        }
        held.insert(key.clone());
        Hold { locks: self, key }
    }
}

impl Hold<'_> {
    /// Whether a write may build on `current`, the CID `tokenURI` answers
    /// now: nothing relayed from here is in doubt, the last change shows,
    /// a confirmed one isn't hidden by a stale read, or it is too old to
    /// wait for.
    fn settled(&self, current: &str) -> bool {
        let mut relayed = self.locks.relayed.lock().unwrap_or_else(|e| e.into_inner());
        let Some(last) = relayed.get(&self.key) else {
            return true;
        };
        let shows = current == last.relayed || (last.landed && current != last.replaced);
        let settled = shows || last.at.elapsed() >= self.locks.expiry;
        if settled {
            relayed.remove(&self.key);
        }
        settled
    }

    /// Remember the change from `replaced` to `relayed` the relay answered
    /// with `outcome`; one it refused or that reverted changed nothing.
    fn remember(&self, replaced: &str, relayed: &str, outcome: &Outcome) {
        let landed = match outcome {
            Outcome::Landed => true,
            Outcome::Unsettled(_) => false,
            Outcome::Refused(_) => return,
        };
        let change = Relayed {
            replaced: replaced.into(),
            relayed: relayed.into(),
            landed,
            at: Instant::now(),
        };
        let mut remembered = self.locks.relayed.lock().unwrap_or_else(|e| e.into_inner());
        remembered.insert(self.key.clone(), change);
    }
}

impl Drop for Hold<'_> {
    fn drop(&mut self) {
        let mut held = self.locks.held.lock().unwrap_or_else(|e| e.into_inner());
        held.remove(&self.key);
        self.locks.freed.notify_all();
    }
}

// ---- Uploads ----------------------------------------------------------------

/// Pinata's pre-signed URLs expire after 60 s: one older than this is
/// replaced before use.
const MAX_AGE: Duration = Duration::from_secs(45);
/// The most credentials Arbesk's `upload-urls` hands out at once.
const MAX_MINT: usize = 200;
const PRESIGNED: &str = "presigned-put";

/// Uploads to IPFS through Arbesk's one-use credentials, minted together
/// and replaced when stale.
pub(crate) struct Uploader<'a> {
    http: &'a dyn Http,
    mint_url: String,
    session: &'a str,
    /// What the errors call the file being copied.
    name: &'a str,
    fresh: VecDeque<String>,
    minted_at: Instant,
    /// Uploads still to come, this one included.
    left: usize,
    max_age: Duration,
}

impl<'a> Uploader<'a> {
    /// Credentials for `count` uploads, minted in one call.
    pub(crate) fn new(
        http: &'a dyn Http,
        site: &Site,
        session: &'a str,
        count: usize,
        name: &'a str,
    ) -> Result<Self, String> {
        let mut uploader = Self {
            http,
            mint_url: format!("{}/api/v1/ipfs/upload-urls", site.origin),
            session,
            name,
            fresh: VecDeque::new(),
            minted_at: Instant::now(),
            left: count,
            max_age: MAX_AGE,
        };
        uploader.mint()?;
        Ok(uploader)
    }

    fn mint(&mut self) -> Result<(), String> {
        let count = self.left.clamp(1, MAX_MINT);
        let step = |why: &str| format!("Couldn't prepare the upload of “{}”: {why}", self.name);
        let answer = self
            .http
            .post_json(
                &self.mint_url,
                &json!({ "count": count }),
                Some(self.session),
            )
            .map_err(|error| refused(error, step))?;
        self.fresh = upload_urls(&answer).map_err(|why| match why.as_str() {
            UNSUPPORTED_STORAGE => why,
            _ => step(&why),
        })?;
        self.minted_at = Instant::now();
        Ok(())
    }

    /// The next credential, minted afresh when none is left or they are stale.
    fn next_url(&mut self) -> Result<String, String> {
        if self.fresh.is_empty() || self.minted_at.elapsed() >= self.max_age {
            self.mint()?;
        }
        self.left = self.left.saturating_sub(1);
        self.fresh.pop_front().ok_or_else(|| UNREADABLE.to_string())
    }

    /// Upload `bytes` as `file_name`; the CID IPFS gave it.
    pub(crate) fn upload(&mut self, file_name: &str, bytes: &[u8]) -> Result<String, String> {
        let url = self.next_url()?;
        let fields = [("network", "public")];
        let failed = |why: &str| format!("Couldn't upload “{}” to IPFS: {why}", self.name);
        let answer = self
            .http
            .post_multipart(&url, file_name, bytes, &fields, None)
            .map_err(|error| failed(&error.message))?;
        uploaded_cid(&answer).ok_or_else(|| failed("IPFS sent no CID."))
    }
}

/// The pre-signed URLs in `upload-urls`' answer; any other kind of
/// credential (Kubo's API) is refused.
fn upload_urls(answer: &[u8]) -> Result<VecDeque<String>, String> {
    let answer: Value = serde_json::from_slice(answer).map_err(|_| UNREADABLE)?;
    let credentials = answer["credentials"]
        .as_array()
        .filter(|credentials| !credentials.is_empty())
        .ok_or(UNREADABLE)?;
    credentials
        .iter()
        .map(
            |credential| match (credential["strategy"].as_str(), credential["url"].as_str()) {
                (Some(PRESIGNED), Some(url)) => Ok(url.to_string()),
                _ => Err(UNSUPPORTED_STORAGE.to_string()),
            },
        )
        .collect()
}

/// The CID in an upload's answer (`data.cid`, else `cid`, as asset-core
/// reads it), if it looks like one.
fn uploaded_cid(answer: &[u8]) -> Option<String> {
    let answer: Value = serde_json::from_slice(answer).ok()?;
    let cid = answer["data"]["cid"].as_str().or(answer["cid"].as_str())?;
    let plain = !cid.is_empty() && cid.chars().all(|c| c.is_ascii_alphanumeric());
    plain.then(|| cid.to_string())
}

// ---- The write ----------------------------------------------------------------

/// Copy the local model `file` into `target`'s collection as the asset
/// listed as `target.name`; the new asset's `arbesk://` path. A name
/// already listed is refused before any upload unless `overwrite`, which
/// saves the file as that asset's next version (spec D3). Cancel works
/// until the collection manifest is uploaded; what was uploaded before
/// stays on IPFS, unreferenced, and the collection is unchanged.
#[allow(clippy::too_many_arguments)]
pub fn add(
    http: &dyn Http,
    site: &Site,
    session: &str,
    target: &Target,
    file: &Path,
    overwrite: bool,
    progress: &Progress,
    cache_dir: &Path,
    locks: &CollectionLocks,
) -> Result<String, String> {
    let name = asset_name(&target.name);
    let split = decompose(file, &name)?;
    let reader = Reader::open(http, site, cache_dir)?;
    let (_, collection) = an_object(reader.collection_manifest_with_cid(&target.token_id)?)?;
    claim(&reader, &collection, target, overwrite)?;
    let uploader = Uploader::new(http, site, session, split.parts.len() + 3, &target.name)?;
    let mut write = Write {
        http,
        site,
        session,
        target,
        overwrite,
        progress,
        reader,
        uploader,
        name,
    };
    let source = write.upload_source(&split)?;
    let held = locks.take(&target.address, &target.token_id);
    write.save(&held, &source, split.format)
}

/// A collection read whose manifest is a JSON object, as every collection's is.
fn an_object(read: (String, Value)) -> Result<(String, Value), String> {
    match read.1.is_object() {
        true => Ok(read),
        false => Err(UNREADABLE.into()),
    }
}

/// What the relay's answer says of a change.
pub(crate) enum Outcome {
    /// Mined.
    Landed,
    /// May still land: no receipt status yet, no answer, or the relay
    /// failed (502) after perhaps sending it.
    Unsettled(String),
    /// Refused before it was sent, or reverted: nothing changed.
    Refused(String),
}

/// The asset listed as `target.name` in `collection` (its id and manifest
/// CID), which Replace saves over; a taken name without `overwrite` is
/// refused.
fn claim(
    reader: &Reader,
    collection: &Value,
    target: &Target,
    overwrite: bool,
) -> Result<Option<(String, String)>, String> {
    let taken = reader
        .listed_names(collection)?
        .into_iter()
        .find(|(_, _, listed)| *listed == target.name);
    match taken {
        Some(_) if !overwrite => Err(exists(&target.name)),
        Some((id, cid, _)) => Ok(Some((id, cid))),
        None => Ok(None),
    }
}

/// One file's write, once it is split.
struct Write<'a> {
    http: &'a dyn Http,
    site: &'a Site,
    session: &'a str,
    target: &'a Target,
    overwrite: bool,
    progress: &'a Progress,
    reader: Reader<'a>,
    uploader: Uploader<'a>,
    /// The asset's name.
    name: String,
}

impl Write<'_> {
    /// Upload each part, then the composite pointing at them; the
    /// composite's CID.
    fn upload_source(&mut self, split: &Decomposed) -> Result<String, String> {
        let parts: u64 = split
            .parts
            .iter()
            .map(|part| part.stored.len() as u64)
            .sum();
        let guess = stored_json(&split.composite, split.composite_compressed).len() as u64;
        self.progress.set_total(parts + guess);
        let mut cids = Vec::with_capacity(split.parts.len());
        for part in &split.parts {
            cids.push(self.upload_counted(&part.file_name, &part.stored)?);
        }
        let composite = stored_json(&fill(&split.composite, split.format, &cids), split.composite_compressed);
        self.progress.set_total(parts + composite.len() as u64);
        self.upload_counted(&split.composite_name, &composite)
    }

    fn upload_counted(&mut self, file_name: &str, bytes: &[u8]) -> Result<String, String> {
        self.progress.check()?;
        let cid = self.uploader.upload(file_name, bytes)?;
        self.progress.add_bytes(bytes.len() as u64);
        Ok(cid)
    }

    /// Under the collection's lock: the asset manifest, then the
    /// collection's, then the relay; the asset's path.
    fn save(&mut self, held: &Hold, source: &str, format: Format) -> Result<String, String> {
        let token_id = &self.target.token_id;
        let (current_cid, collection) = self.settled_collection(held)?;
        // Checked again: another write may have taken the name meanwhile.
        let replaced = claim(&self.reader, &collection, self.target, self.overwrite)?;
        let (asset_id, asset_cid) = self.put_asset(&collection, replaced, source, format)?;
        let next = next_collection(&collection, &current_cid, &asset_id, &asset_cid)?;
        let file_name = format!(
            "collect_{}.json",
            sanitize_file_name(&collection_key(&next))
        );
        self.progress.check()?;
        let next_cid = self
            .uploader
            .upload(&file_name, &stored_json(&next, false))?;
        self.progress.set_phase(Phase::Saving);
        let outcome = self.relay(&next_cid, &asset_id);
        held.remember(&current_cid, &next_cid, &outcome);
        match outcome {
            Outcome::Landed => {}
            Outcome::Unsettled(error) | Outcome::Refused(error) => return Err(error),
        }
        let folder = collection_path(&self.target.address, token_id, &collection_name(&next));
        let listed = format!("{}.{}", self.name, format.ext());
        Ok(asset_path_named(&folder, &asset_id, &listed))
    }

    /// The collection's CID and manifest once this app's last change to
    /// it has settled ([`Hold::settled`]), re-reading `tokenURI` while it
    /// hasn't; refused when it still hasn't.
    fn settled_collection(&self, held: &Hold) -> Result<(String, Value), String> {
        let token_id = &self.target.token_id;
        let read = || an_object(self.reader.collection_manifest_with_cid(token_id)?);
        let mut current = read()?;
        for _ in 0..held.locks.reads {
            if held.settled(&current.0) {
                return Ok(current);
            }
            self.progress.check()?;
            std::thread::sleep(held.locks.every);
            current = read()?;
        }
        match held.settled(&current.0) {
            true => Ok(current),
            false => Err(EARLIER_UNSETTLED.into()),
        }
    }

    /// Upload the asset manifest: a new asset, or the next version of
    /// `replaced`. Its id and CID.
    fn put_asset(
        &mut self,
        collection: &Value,
        replaced: Option<(String, String)>,
        source: &str,
        format: Format,
    ) -> Result<(String, String), String> {
        let (asset_id, version, prev) = match replaced {
            Some((id, cid)) => {
                let old = self.reader.manifest_at(&cid)?;
                (id, old["version"].as_u64().map_or(2, |v| v + 1), Some(cid))
            }
            None => (fresh_id(collection, now_ms()), 1, None),
        };
        let manifest = asset_manifest(
            &self.name,
            &asset_id,
            version,
            prev.as_deref(),
            now_ms(),
            source,
            format,
        );
        // The file name the Studio's `writeJSONToIPFS` gives an asset manifest.
        let file_name = format!("asset_{}_composite.gltf", sanitize_file_name(&asset_id));
        self.progress.check()?;
        let cid = self
            .uploader
            .upload(&file_name, &stored_json(&manifest, false))?;
        Ok((asset_id, cid))
    }

    /// Point the collection's token at `collection_cid` through Arbesk's
    /// wallet relay, as the account's only editor (`proof: []`, spec D9).
    fn relay(&self, collection_cid: &str, asset_id: &str) -> Outcome {
        let body = json!({
            "op": "updateUri",
            "tokenId": self.target.token_id,
            "params": {
                "newUri": collection_cid,
                "proof": [],
                "assetScope": NO_SCOPE,
                "assetId": asset_id,
            },
        });
        let url = format!("{}/api/v1/wallet/relay", self.site.origin);
        let name = &self.target.name;
        match self.http.post_json_slow(&url, &body, Some(self.session)) {
            Ok(answer) => receipt_outcome(&answer, name),
            Err(error) => relay_failure(error, name),
        }
    }
}

/// What a relay's receipt says: `status` true mined, false reverted, null
/// (or no receipt to read) not known yet.
fn receipt_outcome(answer: &[u8], name: &str) -> Outcome {
    let answer: Value = serde_json::from_slice(answer).unwrap_or_default();
    match answer["receipt"]["status"].as_bool() {
        Some(true) => Outcome::Landed,
        Some(false) => Outcome::Refused(not_saved(name)),
        None => Outcome::Unsettled(still_saving(name)),
    }
}

/// What a failed relay request means. Only a 4xx is a refusal before
/// anything was sent (Arbesk's 400, 401, 403, 429 and its other checks).
/// Anything else may come after the transaction was sent, so it may still
/// land: no answer at all, the relay's own 502, another 5xx (Cloudflare,
/// in front of Arbesk, answers 524 after 100 s while the relay may wait
/// three minutes, and 520 when the origin drops the answer), and a 2xx
/// whose body couldn't be read.
fn relay_failure(error: HttpError, name: &str) -> Outcome {
    let step = |why: &str| format!("Couldn't save “{name}” in the collection: {why}");
    match error.status {
        Some(400..=499) => Outcome::Refused(refused(error, step)),
        Some(502) => Outcome::Unsettled(RELAY_FAILED.into()),
        Some(_) => Outcome::Unsettled(still_saving(name)),
        None => Outcome::Unsettled(step(&error.message)),
    }
}

/// What the Studio names a collection manifest's file after: its
/// `asset_id`, else the time.
fn collection_key(collection: &Value) -> String {
    collection["asset_id"]
        .as_str()
        .map_or_else(|| now_ms().to_string(), String::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compose::testing::assert_same_model;
    use crate::decompose::decompose;
    use crate::http::HttpError;
    use crate::http::fake::{FakeHttp, Request};
    use crate::location::asset_path;
    use crate::service::{self, Builds, Thumbnails};
    use crate::testing::{ADDRESS, Relay, World, arbesk, fixture, shapes, site, world};
    use crate::progress::Phase;
    use std::collections::BTreeMap;
    use std::io::Read;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    /// The session Arbesk accepts: it must never show in an error.
    const SESSION: &str = "s3ss10n-t0k3n";

    fn signed_in() -> (Arc<Mutex<World>>, FakeHttp, tempfile::TempDir) {
        let mut w = world();
        w.sessions.insert(SESSION.into());
        let state = Arc::new(Mutex::new(w));
        (state.clone(), arbesk(state), tempfile::tempdir().unwrap())
    }

    fn into(token_id: &str, name: &str) -> Target {
        Target {
            address: ADDRESS.into(),
            token_id: token_id.into(),
            name: name.into(),
        }
    }

    fn add_to(
        http: &dyn Http,
        cache: &Path,
        target: &Target,
        file: &Path,
        overwrite: bool,
        progress: &Progress,
    ) -> Result<String, String> {
        let locks = CollectionLocks::default();
        let added = add(
            http,
            &site(),
            SESSION,
            target,
            file,
            overwrite,
            progress,
            cache,
            &locks,
        );
        if let Err(error) = &added {
            assert!(!error.contains(SESSION), "{error}");
        }
        added
    }

    fn glb() -> PathBuf {
        fixture("glb-box").join("input.glb")
    }

    fn three_mf() -> PathBuf {
        fixture("3mf-box").join("input.3mf")
    }

    fn json_of(path: &Path) -> Value {
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
    }

    fn listed(http: &dyn Http, cache: &Path) -> Vec<String> {
        service::list(http, &site(), &shapes(), cache, &Thumbnails::default())
            .unwrap()
            .into_iter()
            .map(|entry| entry.name)
            .collect()
    }

    fn export(http: &dyn Http, cache: &Path, path: &str) -> PathBuf {
        let progress = Progress::default();
        service::export(http, &site(), path, cache, &progress, &Builds::default()).unwrap()
    }

    fn sent(http: &FakeHttp, to: &str) -> Vec<Request> {
        http.requests()
            .into_iter()
            .filter(|request| request.url.starts_with(to))
            .collect()
    }

    const UPLOAD_URLS: &str = "https://arbesk.test/api/v1/ipfs/upload-urls";
    const RELAY: &str = "https://arbesk.test/api/v1/wallet/relay";
    const UPLOADS: &str = "https://up.test/";

    fn zip_entries(path: &Path) -> BTreeMap<String, Vec<u8>> {
        let file = std::fs::File::open(path).unwrap();
        let mut archive = zip::ZipArchive::new(file).unwrap();
        (0..archive.len())
            .map(|i| {
                let mut entry = archive.by_index(i).unwrap();
                let mut bytes = Vec::new();
                entry.read_to_end(&mut bytes).unwrap();
                (entry.name().unwrap().into_owned(), bytes)
            })
            .collect()
    }

    /// `gltf` (every part a `data:` URI) written to `dir` as `model/scene.gltf`
    /// with its buffers and images in files beside it.
    fn split_out(gltf: &Value, dir: &Path) -> PathBuf {
        let model = dir.join("model");
        std::fs::create_dir_all(model.join("bin")).unwrap();
        std::fs::create_dir_all(model.join("textures")).unwrap();
        let mut split = gltf.clone();
        for (key, folder) in [
            ("buffers", "bin/part %d.bin"),
            ("images", "textures/%d.png"),
        ] {
            for (i, item) in split[key].as_array_mut().unwrap().iter_mut().enumerate() {
                let uri = item["uri"].as_str().unwrap();
                let (_, data) = uri.split_once(',').unwrap();
                let bytes = data_encoding::BASE64.decode(data.as_bytes()).unwrap();
                let name = folder.replace("%d", &i.to_string());
                std::fs::write(model.join(&name), bytes).unwrap();
                item["uri"] = json!(name.replace(' ', "%20"));
            }
        }
        let file = model.join("scene.gltf");
        std::fs::write(&file, serde_json::to_vec(&split).unwrap()).unwrap();
        file
    }

    /// An [`Http`] in front of the fake Arbesk: the relay can be slowed
    /// down, uploads can fail, and the progress can be cancelled after a
    /// number of uploads.
    struct Watched<'a> {
        inner: &'a FakeHttp,
        relay_delay: Duration,
        failing_uploads: bool,
        /// The relay gets the request, but its answer is lost: this
        /// error comes back instead.
        lost_relay_answer: Option<HttpError>,
        cancel_after: Option<(usize, Progress)>,
        /// Runs once, before the first upload.
        before_uploads: Mutex<Option<Box<dyn FnOnce() + Send + 'a>>>,
        uploads: AtomicUsize,
    }

    impl<'a> Watched<'a> {
        fn new(inner: &'a FakeHttp) -> Self {
            Self {
                inner,
                relay_delay: Duration::ZERO,
                failing_uploads: false,
                lost_relay_answer: None,
                cancel_after: None,
                before_uploads: Mutex::new(None),
                uploads: AtomicUsize::new(0),
            }
        }
    }

    impl Http for Watched<'_> {
        fn get(&self, url: &str) -> Result<Vec<u8>, HttpError> {
            self.inner.get(url)
        }

        fn post_json(
            &self,
            url: &str,
            body: &Value,
            session: Option<&str>,
        ) -> Result<Vec<u8>, HttpError> {
            self.inner.post_json(url, body, session)
        }

        fn post_json_slow(
            &self,
            url: &str,
            body: &Value,
            session: Option<&str>,
        ) -> Result<Vec<u8>, HttpError> {
            std::thread::sleep(self.relay_delay);
            let answer = self.inner.post_json_slow(url, body, session);
            if let Some(error) = &self.lost_relay_answer {
                return Err(error.clone());
            }
            answer
        }

        fn post_multipart(
            &self,
            url: &str,
            file_name: &str,
            bytes: &[u8],
            fields: &[(&str, &str)],
            session: Option<&str>,
        ) -> Result<Vec<u8>, HttpError> {
            if self.failing_uploads {
                return Err(HttpError::new(Some(500), "The server answered 500."));
            }
            if let Some(before) = self.before_uploads.lock().unwrap().take() {
                before();
            }
            let answer = self
                .inner
                .post_multipart(url, file_name, bytes, fields, session);
            let done = self.uploads.fetch_add(1, Ordering::SeqCst) + 1;
            if let Some((after, progress)) = &self.cancel_after
                && done == *after
            {
                progress.cancel();
            }
            answer
        }

        fn delete(&self, url: &str, session: Option<&str>) -> Result<(), HttpError> {
            self.inner.delete(url, session)
        }
    }

    #[test]
    fn a_glb_added_reads_back_as_the_same_model() {
        let (_, http, cache) = signed_in();
        let progress = Progress::default();
        let path = add_to(
            &http,
            cache.path(),
            &into("1", "Crate.gltf"),
            &glb(),
            false,
            &progress,
        )
        .unwrap();
        assert!(path.starts_with(&format!("{}/asset_", shapes())), "{path}");
        assert!(path.ends_with("~Crate.gltf"), "{path}");

        let theirs = json_of(&fixture("glb-box").join("expected.gltf"));
        let preview = service::build(
            &http,
            &site(),
            &path,
            cache.path(),
            &Progress::default(),
            &Builds::default(),
        )
        .unwrap();
        assert_same_model(&std::fs::read(preview).unwrap(), &theirs);
        let exported = export(&http, cache.path(), &path);
        assert_eq!(exported.file_name().unwrap(), "Crate.gltf");
        assert_eq!(json_of(&exported), theirs);
        assert!(listed(&http, cache.path()).contains(&"Crate.gltf".to_string()));

        // Every byte uploaded is counted, and the relay was the last step.
        let snapshot = progress.snapshot();
        assert_eq!(snapshot.total, Some(snapshot.bytes));
        assert!(snapshot.bytes > 0);
        assert_eq!(snapshot.phase, Phase::Saving);
    }

    #[test]
    fn a_split_gltf_and_a_3mf_read_back() {
        let (_, http, cache) = signed_in();
        let folder = tempfile::tempdir().unwrap();
        let source = json_of(&fixture("gltf-image").join("input.gltf"));
        let split = split_out(&source, folder.path());
        let progress = Progress::default();
        let path = add_to(
            &http,
            cache.path(),
            &into("1", "Table.gltf"),
            &split,
            false,
            &progress,
        )
        .unwrap();
        assert_eq!(json_of(&export(&http, cache.path(), &path)), source);

        let path = add_to(
            &http,
            cache.path(),
            &into("1", "Plate.3mf"),
            &three_mf(),
            false,
            &progress,
        )
        .unwrap();
        let exported = export(&http, cache.path(), &path);
        assert_eq!(exported.file_name().unwrap(), "Plate.3mf");
        assert_eq!(zip_entries(&exported), zip_entries(&three_mf()));
    }

    #[test]
    fn a_new_asset_and_a_replace_build_their_manifests() {
        let new = asset_manifest("Crate", "asset_5", 1, None, 1234, "bafySrc", Format::Gltf);
        assert_eq!(
            new,
            json!({ "type": "asset", "name": "Crate", "asset_id": "asset_5", "version": 1,
                "timestamp": 1234, "scene": { "nodes": [{
                    "node_id": "node_1", "type": "source_asset", "name": "Crate",
                    "source": { "cid": "bafySrc", "path": "composite.gltf", "format": "gltf" },
                    "transform_matrix": [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1],
                    "post_processor": { "color": null, "scale": { "x": 1, "y": 1, "z": 1 } }
                }] } })
        );
        let replace = asset_manifest(
            "Plate",
            "a2",
            3,
            Some("bafyA2"),
            9,
            "bafyS",
            Format::ThreeMf,
        );
        assert_eq!(replace["asset_id"], "a2");
        assert_eq!(replace["version"], 3);
        assert_eq!(replace["prev_asset_manifest_cid"], "bafyA2");
        assert_eq!(
            replace["scene"]["nodes"][0]["source"],
            json!({ "cid": "bafyS", "path": "composite.3mf.json", "format": "3mf" })
        );

        let current = json!({ "type": "collection", "name": "Shapes", "version": 4,
            "timestamp": 7, "assets": { "a1": "bafyA1" } });
        assert_eq!(
            next_collection(&current, "bafyC1", "asset_5", "bafyNew").unwrap(),
            json!({ "type": "collection", "name": "Shapes", "version": 5, "timestamp": 7,
                "prev_asset_manifest_cid": "bafyC1",
                "assets": { "a1": "bafyA1", "asset_5": "bafyNew" } })
        );
        assert_eq!(
            next_collection(&json!({ "type": "collection" }), "bafyC", "x", "bafyX").unwrap(),
            json!({ "type": "collection", "version": 1, "prev_asset_manifest_cid": "bafyC",
                "assets": { "x": "bafyX" } })
        );

        // Replace: Shapes' Box.gltf (a1, saved without a version) gets version 2.
        let (world, http, cache) = signed_in();
        let path = add_to(
            &http,
            cache.path(),
            &into("1", "Box.gltf"),
            &glb(),
            true,
            &Progress::default(),
        )
        .unwrap();
        assert_eq!(path, asset_path(&shapes(), "a1", "Box", "gltf"));
        let w = world.lock().unwrap();
        let read = |cid: &str| -> Value { serde_json::from_slice(&w.files[cid]).unwrap() };
        let collection = read(&w.tokens["1"]);
        assert_eq!(collection["type"], "collection");
        assert_eq!(collection["name"], "Shapes");
        assert_eq!(collection["version"], 1);
        assert_eq!(collection["prev_asset_manifest_cid"], "bafyC1");
        assert_eq!(collection["assets"].as_object().unwrap().len(), 5);
        assert_eq!(collection["assets"]["a2"], "bafyA2");
        let asset = read(collection["assets"]["a1"].as_str().unwrap());
        assert_eq!(asset["asset_id"], "a1");
        assert_eq!(asset["name"], "Box");
        assert_eq!(asset["version"], 2);
        assert_eq!(asset["prev_asset_manifest_cid"], "bafyA1");
        drop(w);
        let saves = service::versions(&http, &site(), &path, cache.path()).unwrap();
        assert_eq!(saves.len(), 2);
    }

    #[test]
    fn an_asset_is_named_as_the_studio_names_it() {
        for (listed, name) in [
            ("Box.gltf", "Box"),
            ("Box (copy).3mf", "Box (copy)"),
            ("a.b.gltf", "a.b"),
            (".gltf", "Uploaded Asset"),
            ("Box.", "Box."),
            ("", "Uploaded Asset"),
        ] {
            assert_eq!(asset_name(listed), name, "{listed}");
        }
    }

    #[test]
    fn a_taken_name_without_overwrite_is_refused_before_any_upload() {
        let (world, http, cache) = signed_in();
        let error = add_to(
            &http,
            cache.path(),
            &into("1", "Box.gltf"),
            &glb(),
            false,
            &Progress::default(),
        )
        .unwrap_err();
        assert_eq!(error, "Target already exists: Box.gltf");
        assert!(sent(&http, UPLOAD_URLS).is_empty());
        assert!(sent(&http, UPLOADS).is_empty());
        assert!(sent(&http, RELAY).is_empty());
        assert_eq!(world.lock().unwrap().tokens["1"], "bafyC1");
    }

    #[test]
    fn a_name_taken_while_uploading_is_refused_under_the_lock() {
        let (world, http, cache) = signed_in();
        let mut watched = Watched::new(&http);
        let taker = world.clone();
        // Another writer adds a Crate.gltf while this one uploads.
        *watched.before_uploads.get_mut().unwrap() = Some(Box::new(move || {
            let mut w = taker.lock().unwrap();
            w.put_asset("bafyA9", Some("Crate"), "bafySrcGltf");
            w.put(
                "bafyC1b",
                json!({ "type": "collection", "name": "Shapes", "assets": { "a9": "bafyA9" } }),
            );
            w.tokens.insert("1".into(), "bafyC1b".into());
        }));
        let error = add_to(
            &watched,
            cache.path(),
            &into("1", "Crate.gltf"),
            &glb(),
            false,
            &Progress::default(),
        )
        .unwrap_err();
        assert_eq!(error, "Target already exists: Crate.gltf");
        assert!(sent(&http, RELAY).is_empty());
        assert_eq!(world.lock().unwrap().tokens["1"], "bafyC1b");
    }

    #[test]
    fn relay_status_is_checked() {
        for (relay, wording) in [
            (
                Relay::Pending,
                "Arbesk is still saving “Crate.gltf”. It appears once the change is confirmed.",
            ),
            (Relay::Failed, "Arbesk couldn't save “Crate.gltf”."),
        ] {
            let (world, http, cache) = signed_in();
            world.lock().unwrap().relay = relay;
            let error = add_to(
                &http,
                cache.path(),
                &into("1", "Crate.gltf"),
                &glb(),
                false,
                &Progress::default(),
            )
            .unwrap_err();
            assert_eq!(error, wording);
            assert_eq!(world.lock().unwrap().tokens["1"], "bafyC1");
        }
    }

    #[test]
    fn arbesk_refusals_are_said_in_words() {
        type Setup = fn(&mut World);
        let cases: [(&str, Setup, &str); 5] = [
            (
                "1",
                |w| w.sessions.clear(),
                "Sign in to Arbesk again to copy files into it.",
            ),
            ("4", |_| {}, "You can't add to this collection."),
            (
                "1",
                |w| w.busy = true,
                "Arbesk is busy. Try again in a minute.",
            ),
            (
                "1",
                |w| w.relay = Relay::Status(502),
                "Arbesk couldn't save the change. Sign in again to renew its permission.",
            ),
            (
                "1",
                |w| w.kubo = true,
                "This Arbesk server's storage isn't supported.",
            ),
        ];
        for (token, setup, wording) in cases {
            let (world, http, cache) = signed_in();
            setup(&mut world.lock().unwrap());
            let error = add_to(
                &http,
                cache.path(),
                &into(token, "Crate.gltf"),
                &glb(),
                false,
                &Progress::default(),
            )
            .unwrap_err();
            assert_eq!(error, wording);
        }
        // Any other failure says which step failed.
        let (_, http, cache) = signed_in();
        let mut failing = Watched::new(&http);
        failing.failing_uploads = true;
        let error = add_to(
            &failing,
            cache.path(),
            &into("1", "Crate.gltf"),
            &glb(),
            false,
            &Progress::default(),
        )
        .unwrap_err();
        assert_eq!(
            error,
            "Couldn't upload “Crate.gltf” to IPFS: The server answered 500."
        );
    }

    #[test]
    fn two_writes_to_one_collection_keep_both() {
        let (_, http, cache) = signed_in();
        let mut slow = Watched::new(&http);
        slow.relay_delay = Duration::from_millis(150);
        let locks = CollectionLocks::default();
        let write = |name: &str, file: PathBuf| {
            let progress = Progress::default();
            add(
                &slow,
                &site(),
                SESSION,
                &into("1", name),
                &file,
                false,
                &progress,
                cache.path(),
                &locks,
            )
        };
        let (one, two) = std::thread::scope(|s| {
            let one = s.spawn(|| write("One.gltf", glb()));
            let two = s.spawn(|| write("Two.3mf", three_mf()));
            (one.join().unwrap().unwrap(), two.join().unwrap().unwrap())
        });
        assert_ne!(one, two);
        let names = listed(&http, cache.path());
        assert!(names.contains(&"One.gltf".to_string()), "{names:?}");
        assert!(names.contains(&"Two.3mf".to_string()), "{names:?}");
    }

    const EARLIER: &str =
        "Arbesk is still saving an earlier copy into this collection. Try again in a minute.";

    /// Locks that re-read the chain 3 times, 1 ms apart, and forget an
    /// unsettled change after `expiry`.
    fn quick_locks(expiry: Duration) -> CollectionLocks {
        CollectionLocks {
            reads: 3,
            every: Duration::from_millis(1),
            expiry,
            ..CollectionLocks::default()
        }
    }

    fn add_with(
        http: &dyn Http,
        cache: &Path,
        locks: &CollectionLocks,
        name: &str,
        file: PathBuf,
    ) -> Result<String, String> {
        let progress = Progress::default();
        let target = into("1", name);
        let added = add(
            http,
            &site(),
            SESSION,
            &target,
            &file,
            false,
            &progress,
            cache,
            locks,
        );
        if let Err(error) = &added {
            assert!(!error.contains(SESSION), "{error}");
        }
        added
    }

    fn relayed_uri(http: &FakeHttp, nth: usize) -> String {
        let relays = sent(http, RELAY);
        let body = relays[nth].body.as_ref().unwrap();
        body["params"]["newUri"].as_str().unwrap().to_string()
    }

    #[test]
    fn a_stale_chain_read_waits_for_the_last_write() {
        let (world, http, cache) = signed_in();
        let locks = quick_locks(Duration::from_secs(300));
        // Two reads after each confirmed relay still answer the CID before:
        // the next write's pre-check and its first read under the lock.
        world.lock().unwrap().lag = 2;
        add_with(&http, cache.path(), &locks, "One.gltf", glb()).unwrap();
        add_with(&http, cache.path(), &locks, "Two.3mf", three_mf()).unwrap();
        world.lock().unwrap().lagging.clear();
        let names = listed(&http, cache.path());
        assert!(names.contains(&"One.gltf".to_string()), "{names:?}");
        assert!(names.contains(&"Two.3mf".to_string()), "{names:?}");

        // A chain that stays behind past the wait: refused, not built on.
        let (world, http, cache) = signed_in();
        world.lock().unwrap().lag = 10;
        add_with(&http, cache.path(), &locks, "One.gltf", glb()).unwrap();
        let error = add_with(&http, cache.path(), &locks, "Two.3mf", three_mf()).unwrap_err();
        assert_eq!(error, EARLIER);
        assert_eq!(sent(&http, RELAY).len(), 1);
    }

    #[test]
    fn an_unsettled_write_holds_back_the_next_until_it_lands() {
        type Setup = Box<dyn Fn(&mut World, &mut Watched)>;
        let saving = "Arbesk is still saving “One.gltf”. It appears once the change is confirmed.";
        let lost = |status: Option<u16>, why: &str| {
            let error = HttpError::new(status, why);
            move |w: &mut World, watched: &mut Watched| {
                w.relay = Relay::Pending;
                watched.lost_relay_answer = Some(error.clone());
            }
        };
        let timed_out = lost(None, "operation timed out");
        let unreadable = lost(Some(200), "error decoding response body");
        let cases: [(Setup, &str); 7] = [
            (Box::new(|w, _| w.relay = Relay::Pending), saving),
            (
                Box::new(|w, _| w.relay = Relay::Status(502)),
                "Arbesk couldn't save the change. Sign in again to renew its permission.",
            ),
            // Cloudflare in front of Arbesk: origin timeout, dropped origin.
            (Box::new(|w, _| w.relay = Relay::Status(524)), saving),
            (Box::new(|w, _| w.relay = Relay::Status(520)), saving),
            (Box::new(|w, _| w.relay = Relay::Status(503)), saving),
            (
                Box::new(timed_out),
                "Couldn't save “One.gltf” in the collection: operation timed out",
            ),
            (Box::new(unreadable), saving),
        ];
        for (setup, wording) in cases {
            let (world, http, cache) = signed_in();
            let locks = quick_locks(Duration::from_secs(300));
            let mut watched = Watched::new(&http);
            setup(&mut world.lock().unwrap(), &mut watched);
            let error = add_with(&watched, cache.path(), &locks, "One.gltf", glb()).unwrap_err();
            assert_eq!(error, wording);
            world.lock().unwrap().relay = Relay::Ok;
            let error = add_with(&http, cache.path(), &locks, "Two.3mf", three_mf()).unwrap_err();
            assert_eq!(error, EARLIER);
            assert_eq!(sent(&http, RELAY).len(), 1);
            // The first write lands after all; the next one builds on it.
            let landed = relayed_uri(&http, 0);
            world.lock().unwrap().tokens.insert("1".into(), landed);
            add_with(&http, cache.path(), &locks, "Two.3mf", three_mf()).unwrap();
            let names = listed(&http, cache.path());
            assert!(names.contains(&"One.gltf".to_string()), "{names:?}");
            assert!(names.contains(&"Two.3mf".to_string()), "{names:?}");
        }
    }

    #[test]
    fn a_reverted_or_refused_write_holds_nothing_back() {
        // Reverted, or refused by Arbesk before anything was sent.
        for (relay, wording) in [
            (Relay::Failed, "Arbesk couldn't save “One.gltf”."),
            (
                Relay::Status(400),
                "Couldn't save “One.gltf” in the collection: The server answered 400.",
            ),
            (
                Relay::Status(401),
                "Sign in to Arbesk again to copy files into it.",
            ),
            (Relay::Status(403), "You can't add to this collection."),
            (Relay::Status(429), "Arbesk is busy. Try again in a minute."),
        ] {
            let (world, http, cache) = signed_in();
            let locks = quick_locks(Duration::from_secs(300));
            world.lock().unwrap().relay = relay;
            let error = add_with(&http, cache.path(), &locks, "One.gltf", glb()).unwrap_err();
            assert_eq!(error, wording);
            world.lock().unwrap().relay = Relay::Ok;
            add_with(&http, cache.path(), &locks, "Two.3mf", three_mf()).unwrap();
        }
    }

    #[test]
    fn an_unsettled_write_is_forgotten_after_a_while() {
        let (world, http, cache) = signed_in();
        let locks = quick_locks(Duration::ZERO);
        world.lock().unwrap().relay = Relay::Pending;
        add_with(&http, cache.path(), &locks, "One.gltf", glb()).unwrap_err();
        world.lock().unwrap().relay = Relay::Ok;
        add_with(&http, cache.path(), &locks, "Two.3mf", three_mf()).unwrap();
    }

    #[test]
    fn a_collection_that_isnt_an_object_is_refused() {
        assert_eq!(
            next_collection(&json!(["bafyA1"]), "bafyC", "x", "bafyX").unwrap_err(),
            "Arbesk's answer can't be read."
        );
        let (world, http, cache) = signed_in();
        {
            let mut w = world.lock().unwrap();
            w.put("bafyList", json!(["bafyA1"]));
            w.tokens.insert("6".into(), "bafyList".into());
            w.owned.push("6".into());
        }
        let error = add_to(
            &http,
            cache.path(),
            &into("6", "Crate.gltf"),
            &glb(),
            false,
            &Progress::default(),
        )
        .unwrap_err();
        assert_eq!(error, "Arbesk's answer can't be read.");
        assert!(sent(&http, UPLOADS).is_empty());
        assert_eq!(world.lock().unwrap().tokens["6"], "bafyList");
    }

    #[test]
    fn the_lock_is_per_collection() {
        let locks = CollectionLocks::default();
        let held = locks.hold(ADDRESS, "1");
        // Another collection doesn't wait.
        drop(locks.hold(ADDRESS, "2"));
        let waited = std::thread::scope(|s| {
            let waiter = s.spawn(|| {
                let begun = std::time::Instant::now();
                drop(locks.hold(&ADDRESS.to_uppercase().replace("0X", "0x"), "1"));
                begun.elapsed()
            });
            std::thread::sleep(Duration::from_millis(100));
            drop(held);
            waiter.join().unwrap()
        });
        assert!(waited >= Duration::from_millis(90), "{waited:?}");
    }

    #[test]
    fn a_cancel_stops_before_the_collection_changes() {
        let (world, http, cache) = signed_in();
        let progress = Progress::default();
        progress.cancel();
        let error = add_to(
            &http,
            cache.path(),
            &into("1", "Crate.gltf"),
            &glb(),
            false,
            &progress,
        )
        .unwrap_err();
        assert_eq!(error, "Cancelled.");
        assert!(sent(&http, UPLOADS).is_empty());

        // Cancelled once the asset manifest is up: the collection manifest
        // never goes up and the relay is never asked.
        let parts = decompose(&glb(), "Crate").unwrap().parts.len();
        let progress = Progress::default();
        let mut watched = Watched::new(&http);
        watched.cancel_after = Some((parts + 2, progress.clone()));
        let error = add_to(
            &watched,
            cache.path(),
            &into("1", "Crate.gltf"),
            &glb(),
            false,
            &progress,
        )
        .unwrap_err();
        assert_eq!(error, "Cancelled.");
        assert_eq!(sent(&http, UPLOADS).len(), parts + 2);
        assert!(sent(&http, RELAY).is_empty());
        assert_eq!(world.lock().unwrap().tokens["1"], "bafyC1");
    }

    #[test]
    fn credentials_are_minted_in_one_call() {
        let (_, http, cache) = signed_in();
        add_to(
            &http,
            cache.path(),
            &into("1", "Crate.gltf"),
            &glb(),
            false,
            &Progress::default(),
        )
        .unwrap();
        let parts = decompose(&glb(), "Crate").unwrap().parts.len();
        let mints = sent(&http, UPLOAD_URLS);
        assert_eq!(mints.len(), 1);
        assert_eq!(mints[0].body.as_ref().unwrap()["count"], parts + 3);
        assert_eq!(mints[0].session.as_deref(), Some(SESSION));
        let uploads = sent(&http, UPLOADS);
        assert_eq!(uploads.len(), parts + 3);
        for upload in uploads {
            // Pinata never sees the session.
            assert_eq!(upload.session, None);
            assert_eq!(
                upload.fields,
                [("network".to_string(), "public".to_string())]
            );
        }
        let relayed = sent(&http, RELAY);
        assert_eq!(relayed.len(), 1);
        assert_eq!(relayed[0].session.as_deref(), Some(SESSION));
        let body = relayed[0].body.as_ref().unwrap();
        assert_eq!(body["op"], "updateUri");
        assert_eq!(body["tokenId"], "1");
        let params = &body["params"];
        assert_eq!(params["proof"], json!([]));
        assert_eq!(params["assetScope"], format!("0x{}", "0".repeat(64)));
        assert!(params["assetId"].as_str().unwrap().starts_with("asset_"));
        assert!(params["newUri"].as_str().unwrap().starts_with("bafyUp"));
    }

    #[test]
    fn a_stale_credential_is_replaced_before_use() {
        let (_, http, _) = signed_in();
        let mut uploader = Uploader::new(&http, &site(), SESSION, 3, "Crate.gltf").unwrap();
        uploader.max_age = Duration::ZERO;
        uploader.upload("a.bin", b"a").unwrap();
        uploader.upload("b.bin", b"b").unwrap();
        let counts: Vec<Value> = sent(&http, UPLOAD_URLS)
            .into_iter()
            .map(|mint| mint.body.unwrap()["count"].clone())
            .collect();
        assert_eq!(counts, [json!(3), json!(3), json!(2)]);
    }
}
