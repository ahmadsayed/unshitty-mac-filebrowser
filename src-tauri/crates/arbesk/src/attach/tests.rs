//! The attach and detach writes against the fake Arbesk (`testing`), with
//! the uploads and relay calls counted.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};

use super::*;
use crate::http::HttpError;
use crate::http::fake::{FakeHttp, Request};
use crate::location::asset_path;
use crate::testing::{self, World, arbesk, shapes, site};

const SESSION: &str = "s3ss10n";
const UPLOADS: &str = "https://up.test/";
const RELAY: &str = "https://arbesk.test/api/v1/wallet/relay";

/// Shapes' Box.gltf (a1) with other files: an STL made from the model, an
/// entry this app can't read, a `.blend`; and keys this app doesn't know.
fn with_files(w: &mut World) {
    let mut manifest: Value = serde_json::from_slice(&w.files["bafyA1"]).unwrap();
    manifest["version"] = json!(3);
    manifest["metadata"] = json!({ "tags": ["box"], "computed": { "faces": 12 } });
    manifest["comments"] = json!([{ "by": "alice", "text": "nice" }]);
    manifest["future"] = json!({ "kept": true });
    manifest["representations"] = json!([
        { "id": "stl", "role": "mesh", "format": "stl", "name": "Box.stl", "cid": "bafyOldStl", "bytes": 3,
          "encoding": "raw", "created_at": 1, "derived_from": { "ref": "primary", "cid": "bafySrcGltf" } },
        { "id": "plan", "role": "someday", "format": "json", "newer": [1, 2] },
        { "id": "source", "role": "source", "format": "blend", "name": "Box.blend", "cid": "bafyBlend", "bytes": 5,
          "encoding": "raw", "created_at": 2 },
    ]);
    w.put("bafyA1", manifest);
}

struct Fixture {
    world: Arc<Mutex<World>>,
    http: FakeHttp,
    cache: tempfile::TempDir,
    dir: tempfile::TempDir,
    locks: CollectionLocks,
    site: Site,
}

fn fixture(edit: impl FnOnce(&mut World)) -> Fixture {
    let mut w = testing::world();
    w.sessions.insert(SESSION.into());
    edit(&mut w);
    let world = Arc::new(Mutex::new(w));
    Fixture {
        http: arbesk(world.clone()),
        world,
        cache: tempfile::tempdir().unwrap(),
        dir: tempfile::tempdir().unwrap(),
        locks: CollectionLocks::quick(3, Duration::from_millis(1), Duration::from_secs(300)),
        site: site(),
    }
}

impl Fixture {
    fn file(&self, name: &str, bytes: &[u8]) -> String {
        let path = self.dir.path().join(name);
        std::fs::write(&path, bytes).unwrap();
        path.to_string_lossy().into_owned()
    }

    fn writer<'a>(&'a self, http: &'a dyn Http, progress: &'a Progress) -> Writer<'a> {
        Writer {
            http,
            site: &self.site,
            session: SESSION,
            cache_dir: self.cache.path(),
            locks: &self.locks,
            progress,
        }
    }

    fn attach_with(&self, http: &dyn Http, adding: &[Adding]) -> Result<Attached, String> {
        let progress = Progress::default();
        attach(&self.writer(http, &progress), &box_path(), adding)
    }

    fn attach(&self, adding: &[Adding]) -> Result<Attached, String> {
        self.attach_with(&self.http, adding)
    }

    fn detach(&self, id: &str, cid: &str) -> Result<Attached, String> {
        self.detach_with(&self.http, id, cid)
    }

    fn detach_with(&self, http: &dyn Http, id: &str, cid: &str) -> Result<Attached, String> {
        let progress = Progress::default();
        detach(&self.writer(http, &progress), &box_path(), id, cid)
    }

    /// The asset manifest Shapes lists for a1 now, and its CID.
    fn box_manifest(&self) -> (String, Value) {
        let w = self.world.lock().unwrap();
        let collection: Value = serde_json::from_slice(&w.files[&w.tokens["1"]]).unwrap();
        let cid = collection["assets"]["a1"].as_str().unwrap().to_string();
        let manifest = serde_json::from_slice(&w.files[&cid]).unwrap();
        (cid, manifest)
    }

    fn token(&self) -> String {
        self.world.lock().unwrap().tokens["1"].clone()
    }

    fn sent(&self, to: &str) -> Vec<Request> {
        self.http.requests().into_iter().filter(|r| r.url.starts_with(to)).collect()
    }
}

fn box_path() -> String {
    asset_path(&shapes(), "a1", "Box", "gltf")
}

fn adding(paths: &[&String]) -> Vec<Adding> {
    paths.iter().map(|path| Adding { path: path.to_string(), choice: None }).collect()
}

fn ids(manifest: &Value) -> Vec<String> {
    manifest["representations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["id"].as_str().unwrap_or_default().to_string())
        .collect()
}

/// The keys an attach or detach owns: everything else stays as it was.
fn without_owned(manifest: &Value) -> Value {
    let mut rest = manifest.clone();
    for key in ["version", "prev_asset_manifest_cid", "timestamp", "representations"] {
        rest.as_object_mut().unwrap().remove(key);
    }
    rest
}

// ---- What is written ----------------------------------------------------------

#[test]
fn an_attach_keeps_everything_but_what_it_owns() {
    let f = fixture(with_files);
    let (before_cid, before) = f.box_manifest();
    let gcode = f.file("Box.gcode", &b"G1 X1 Y1\n".repeat(200));
    let attached = f.attach(&adding(&[&gcode])).unwrap();
    let (after_cid, after) = f.box_manifest();
    assert_ne!(after_cid, before_cid);
    assert_eq!(without_owned(&after), without_owned(&before));
    assert_eq!(after["version"], json!(4));
    assert_eq!(after["prev_asset_manifest_cid"], json!(before_cid));
    assert!(after["timestamp"].as_i64().unwrap() > testing::STAMP_MS);
    // Every stored entry stays, in place (the unreadable one too); the new
    // one goes at the end.
    let list = after["representations"].as_array().unwrap();
    assert_eq!(&list[..3], before["representations"].as_array().unwrap().as_slice());
    assert_eq!(ids(&after), ["stl", "plan", "source", "gcode"]);
    let gcode_entry = &list[3];
    assert_eq!(gcode_entry["role"], "machine");
    assert_eq!(gcode_entry["encoding"], "brotli");
    assert_eq!(gcode_entry["bytes"], 1800);
    assert_eq!(gcode_entry["name"], "Box.gcode");
    assert_eq!(gcode_entry["derived_from"]["ref"], "primary");
    assert_eq!(gcode_entry["derived_from"]["cid"], "bafySrcGltf");
    assert_eq!(gcode_entry["derived_from"]["transform_matrix"].as_array().unwrap().len(), 16);
    // Stored brotli-framed, decoded back to the file.
    let stored = f.world.lock().unwrap().files[gcode_entry["cid"].as_str().unwrap()].clone();
    assert_eq!(reps::decode(stored, reps::Encoding::Brotli, 1 << 20).unwrap(), std::fs::read(&gcode).unwrap());
    // One relay, one version; what the Files… page redraws from.
    assert_eq!(f.sent(RELAY).len(), 1);
    assert_eq!(attached.files.iter().map(|file| file.id.as_str()).collect::<Vec<_>>(), ["stl", "source", "gcode"]);
    assert_eq!(attached.representations.as_ref().unwrap().count, 3);
    assert!(attached.refused.is_empty());
}

#[test]
fn several_files_are_one_version_and_a_raw_file_is_stored_as_it_is() {
    let f = fixture(with_files);
    let stl = f.file("Lid.stl", b"solid lid");
    let blend = f.file("Lid.blend", b"ARB\x01looks framed");
    // `stl` and `source` are stored: Keep Both is the answer nobody gave.
    let attached = f.attach(&adding(&[&stl, &blend])).unwrap();
    let (_, after) = f.box_manifest();
    assert_eq!(ids(&after), ["stl", "plan", "source", "stl-2", "source-2"]);
    assert_eq!(f.sent(RELAY).len(), 1);
    let list = after["representations"].as_array().unwrap();
    let blend_cid = list[4]["cid"].as_str().unwrap();
    assert_eq!(list[4]["encoding"], "raw");
    assert!(list[4].get("derived_from").is_none(), "a source is made from nothing");
    assert_eq!(f.world.lock().unwrap().files[blend_cid], b"ARB\x01looks framed");
    assert_eq!(attached.representations.unwrap().count, 4);
}

#[test]
fn replace_swaps_the_entry_by_its_stored_id_in_place() {
    let f = fixture(with_files);
    let stl = f.file("Box.stl", b"solid new");
    let replace = Adding { path: stl, choice: Some(Choice::Replace { cid: "bafyOldStl".into() }) };
    f.attach(&[replace]).unwrap();
    let (_, after) = f.box_manifest();
    assert_eq!(ids(&after), ["stl", "plan", "source"]);
    assert_ne!(after["representations"][0]["cid"], "bafyOldStl");
    assert_eq!(after["representations"][0]["bytes"], 9);
}

#[test]
fn replace_of_a_file_that_changed_since_keeps_both() {
    let f = fixture(with_files);
    let stl = f.file("Box.stl", b"solid new");
    // The user agreed to replace another file than the one stored now.
    let replace = Adding { path: stl, choice: Some(Choice::Replace { cid: "bafyElse".into() }) };
    f.attach(&[replace]).unwrap();
    let (_, after) = f.box_manifest();
    assert_eq!(ids(&after), ["stl", "plan", "source", "stl-2"]);
    assert_eq!(after["representations"][0]["cid"], "bafyOldStl");
}

#[test]
fn two_files_of_one_paste_with_one_id_get_two() {
    let f = fixture(|_| {});
    let a = f.file("A.stl", b"solid a");
    let b = f.file("B.stl", b"solid b");
    f.attach(&adding(&[&a, &b])).unwrap();
    let (_, after) = f.box_manifest();
    assert_eq!(ids(&after), ["stl", "stl-2"]);
    // A paste of two that both replace: the second can't take the id too.
    let f = fixture(with_files);
    let a = f.file("A.stl", b"solid a");
    let b = f.file("B.stl", b"solid b");
    let replace = |path: &String| Adding { path: path.clone(), choice: Some(Choice::Replace { cid: "bafyOldStl".into() }) };
    f.attach(&[replace(&a), replace(&b)]).unwrap();
    let (_, after) = f.box_manifest();
    assert_eq!(ids(&after), ["stl", "plan", "source", "stl-2"]);
    assert_eq!(after["representations"][0]["name"], "A.stl");
}

#[test]
fn an_id_an_unreadable_entry_holds_is_refused_and_the_rest_goes_on() {
    let f = fixture(with_files);
    let plan_file = f.file("Print.plan", b"later");
    let stl = f.file("Other.stl", b"solid");
    // `plan` is what an unreadable entry holds: `.plan` files get `plan`.
    let attached = f.attach(&adding(&[&plan_file, &stl])).unwrap();
    assert_eq!(attached.refused, ["This asset already has a file “plan” that File Browser can't show."]);
    let (_, after) = f.box_manifest();
    assert_eq!(after["representations"][1], json!({ "id": "plan", "role": "someday", "format": "json", "newer": [1, 2] }));
    assert_eq!(ids(&after), ["stl", "plan", "source", "stl-2"]);
    // Alone, nothing is added and nothing is written.
    let f = fixture(with_files);
    let plan_file = f.file("Print.plan", b"later");
    let error = f.attach(&adding(&[&plan_file])).unwrap_err();
    assert_eq!(error, "Nothing was added to “Box.gltf”.");
    assert!(f.sent(UPLOADS).is_empty());
}

#[test]
fn every_file_skipped_writes_nothing() {
    let f = fixture(with_files);
    let stl = f.file("Box.stl", b"solid");
    let error = f.attach(&[Adding { path: stl, choice: Some(Choice::Skip) }]).unwrap_err();
    assert_eq!(error, "Nothing was added to “Box.gltf”.");
    assert!(f.sent(UPLOADS).is_empty() && f.sent(RELAY).is_empty());
    assert_eq!(f.token(), "bafyC1");
}

#[test]
fn room_is_counted_after_the_ids_are_settled() {
    let full = |w: &mut World| {
        with_files(w);
        let mut manifest: Value = serde_json::from_slice(&w.files["bafyA1"]).unwrap();
        let list = manifest["representations"].as_array_mut().unwrap();
        for n in list.len()..32 {
            list.push(json!({ "id": format!("x{n}"), "broken": true }));
        }
        w.put("bafyA1", manifest);
    };
    let f = fixture(full);
    let stl = f.file("Box.stl", b"solid");
    assert_eq!(f.attach(&adding(&[&stl])).unwrap_err(), "An asset holds at most 32 files.");
    assert!(f.sent(UPLOADS).is_empty());
    // Replacing adds no entry: it fits a full asset.
    let replace = Adding { path: stl, choice: Some(Choice::Replace { cid: "bafyOldStl".into() }) };
    f.attach(&[replace]).unwrap();
    assert_eq!(f.box_manifest().1["representations"].as_array().unwrap().len(), 32);
}

#[test]
fn every_refusal_comes_before_the_first_upload() {
    let f = fixture(with_files);
    let readme = f.file("README", b"hello");
    let odd = f.file("Model.Ex_t", b"x");
    let folder = f.dir.path().join("Parts");
    std::fs::create_dir(&folder).unwrap();
    let folder = folder.to_string_lossy().into_owned();
    let huge = f.dir.path().join("Print.gcode");
    std::fs::File::create(&huge).unwrap().set_len(reps::MAX_BYTES + 1).unwrap();
    let huge = huge.to_string_lossy().into_owned();
    let fine = f.file("Fine.stl", b"solid");
    let cases = [
        (readme, "“README” has no file type to record."),
        (odd, "“Model.Ex_t” has no file type to record."),
        (folder, "Only files on this computer can be added to an asset."),
        ("relative/Fine.stl".to_string(), "Only files on this computer can be added to an asset."),
        (huge, "“Print.gcode” is larger than 256 MB."),
    ];
    for (path, words) in cases {
        assert_eq!(f.attach(&adding(&[&fine, &path])).unwrap_err(), words, "{path}");
    }
    assert!(f.sent(UPLOADS).is_empty() && f.sent(RELAY).is_empty());
}

// ---- Failures leave the asset as it was ----------------------------------------------

/// The fake Arbesk, with something run after the n-th upload and uploads
/// that fail from a count on.
struct Watched<'a> {
    inner: &'a FakeHttp,
    uploads: AtomicUsize,
    after: Mutex<Vec<(usize, Box<dyn FnOnce() + Send + 'a>)>>,
    failing_from: Option<usize>,
}

impl<'a> Watched<'a> {
    fn new(inner: &'a FakeHttp) -> Self {
        Self { inner, uploads: AtomicUsize::new(0), after: Mutex::new(Vec::new()), failing_from: None }
    }

    fn after(self, upload: usize, run: impl FnOnce() + Send + 'a) -> Self {
        self.after.lock().unwrap().push((upload, Box::new(run)));
        self
    }
}

impl Http for Watched<'_> {
    fn get(&self, url: &str) -> Result<Vec<u8>, HttpError> {
        self.inner.get(url)
    }
    fn get_streamed(&self, url: &str, expected: u64, chunk: &mut dyn FnMut(&[u8]) -> bool) -> Result<(), HttpError> {
        self.inner.get_streamed(url, expected, chunk)
    }
    fn post_json(&self, url: &str, body: &Value, session: Option<&str>) -> Result<Vec<u8>, HttpError> {
        self.inner.post_json(url, body, session)
    }
    fn post_json_slow(&self, url: &str, body: &Value, session: Option<&str>) -> Result<Vec<u8>, HttpError> {
        self.inner.post_json_slow(url, body, session)
    }
    fn post_multipart(
        &self,
        url: &str,
        file_name: &str,
        bytes: &[u8],
        fields: &[(&str, &str)],
        session: Option<&str>,
    ) -> Result<Vec<u8>, HttpError> {
        let n = self.uploads.fetch_add(1, Ordering::SeqCst) + 1;
        if self.failing_from.is_some_and(|from| n >= from) {
            return Err(HttpError::new(Some(500), "The server answered 500."));
        }
        let answer = self.inner.post_multipart(url, file_name, bytes, fields, session);
        let mut after = self.after.lock().unwrap();
        if let Some(at) = after.iter().position(|(upload, _)| *upload == n) {
            let (_, run) = after.remove(at);
            drop(after);
            run();
        }
        answer
    }
    fn delete(&self, url: &str, session: Option<&str>) -> Result<(), HttpError> {
        self.inner.delete(url, session)
    }
}

/// Shapes' collection changed by `edit`, saved under a new CID
/// (`<cid>C`) the token points at, as a save elsewhere lands.
fn new_collection(w: &mut World, cid: &str, edit: impl FnOnce(&mut Value)) {
    let mut collection: Value = serde_json::from_slice(&w.files[&w.tokens["1"]]).unwrap();
    edit(&mut collection);
    let collection_cid = format!("{cid}C");
    w.put(&collection_cid, collection);
    w.tokens.insert("1".into(), collection_cid);
}

/// Another client's save of a1 lands: a new manifest (with a key of its
/// own and a new model) at `cid`, and the collection points at it.
fn another_save(world: &Arc<Mutex<World>>, cid: &str) {
    another_save_with(world, cid, |_| {});
}

/// The same, its manifest also changed by `edit`.
fn another_save_with(world: &Arc<Mutex<World>>, cid: &str, edit: impl FnOnce(&mut Value)) {
    let mut w = world.lock().unwrap();
    let collection: Value = serde_json::from_slice(&w.files[&w.tokens["1"]]).unwrap();
    let current = collection["assets"]["a1"].as_str().unwrap().to_string();
    let mut manifest: Value = serde_json::from_slice(&w.files[&current]).unwrap();
    manifest["edited_by"] = json!(cid);
    manifest["scene"]["nodes"][0]["source"]["cid"] = json!(format!("{cid}Src"));
    let source = w.files["bafySrcGltf"].clone();
    w.files.insert(format!("{cid}Src"), source);
    manifest["prev_asset_manifest_cid"] = json!(current);
    edit(&mut manifest);
    w.put(cid, manifest);
    new_collection(&mut w, cid, |collection| collection["assets"]["a1"] = json!(cid));
}

#[test]
fn a_failed_upload_leaves_the_asset_unchanged() {
    let f = fixture(with_files);
    let a = f.file("A.stl", b"solid a");
    let b = f.file("B.stl", b"solid b");
    let before = f.box_manifest();
    // Each failure names what was going up: a pasted file, then the asset.
    for (from, named) in [(1, "A.stl"), (2, "B.stl"), (3, "Box.gltf"), (4, "Box.gltf")] {
        let mut watched = Watched::new(&f.http);
        watched.failing_from = Some(from);
        let error = f.attach_with(&watched, &adding(&[&a, &b])).unwrap_err();
        assert_eq!(error, format!("Couldn't upload “{named}” to IPFS: The server answered 500."));
        assert_eq!(f.box_manifest(), before, "failing from upload {from}");
        assert_eq!(f.token(), "bafyC1");
    }
    assert!(f.sent(RELAY).is_empty());
}

#[test]
fn a_cancel_stops_before_the_collection_changes() {
    let f = fixture(with_files);
    let stl = f.file("A.stl", b"solid a");
    for after in [0, 1, 2] {
        let progress = Progress::default();
        let cancel = progress.clone();
        let watched = Watched::new(&f.http).after(after, move || cancel.cancel());
        if after == 0 {
            progress.cancel();
        }
        let error = attach(&f.writer(&watched, &progress), &box_path(), &adding(&[&stl])).unwrap_err();
        assert_eq!(error, "Cancelled.");
        assert_eq!(f.token(), "bafyC1");
    }
    assert!(f.sent(RELAY).is_empty());
}

#[test]
fn a_first_conflict_is_retried_from_the_new_tip_without_uploading_again() {
    let f = fixture(with_files);
    let stl = f.file("A.stl", b"solid a");
    let world = f.world.clone();
    // Uploads: the file, the asset manifest; then another save lands.
    let watched = Watched::new(&f.http).after(2, move || another_save(&world, "bafyTheirs"));
    let attached = f.attach_with(&watched, &adding(&[&stl])).unwrap();
    let (_, after) = f.box_manifest();
    // Built on their save: their key and model kept, linked back to it.
    assert_eq!(after["edited_by"], "bafyTheirs");
    assert_eq!(after["prev_asset_manifest_cid"], "bafyTheirs");
    assert_eq!(ids(&after), ["stl", "plan", "source", "stl-2"]);
    // `derived_from` recomputed from their model.
    assert_eq!(after["representations"][3]["derived_from"]["cid"], "bafyTheirsSrc");
    // The file went up once: file, manifest, manifest again, collection.
    let uploads = f.sent(UPLOADS);
    assert_eq!(uploads.len(), 4);
    assert_eq!(uploads.iter().filter(|r| r.data.as_deref() == Some(&b"solid a"[..])).count(), 1);
    assert_eq!(f.sent(RELAY).len(), 1);
    assert_eq!(attached.files.len(), 3);
}

#[test]
fn a_second_conflict_changes_nothing() {
    let f = fixture(with_files);
    let stl = f.file("A.stl", b"solid a");
    let (one, two) = (f.world.clone(), f.world.clone());
    let watched = Watched::new(&f.http)
        .after(2, move || another_save(&one, "bafyTheirs"))
        .after(3, move || another_save(&two, "bafyTheirs2"));
    let error = f.attach_with(&watched, &adding(&[&stl])).unwrap_err();
    assert_eq!(error, "“Box.gltf” changed while the files were being added. Nothing was changed; try again.");
    assert!(f.sent(RELAY).is_empty());
    // The collection is the other save's, untouched by this one.
    assert_eq!(f.token(), "bafyTheirs2C");
    assert_eq!(f.box_manifest().0, "bafyTheirs2");
}

#[test]
fn an_asset_gone_from_its_collection_is_said() {
    let f = fixture(with_files);
    let stl = f.file("A.stl", b"solid a");
    let world = f.world.clone();
    let watched = Watched::new(&f.http).after(2, move || {
        let mut w = world.lock().unwrap();
        new_collection(&mut w, "bafyNoBox", |collection| {
            collection["assets"].as_object_mut().unwrap().remove("a1");
        });
    });
    let error = f.attach_with(&watched, &adding(&[&stl])).unwrap_err();
    assert_eq!(error, "“Box.gltf” is no longer in the collection.");
    assert!(f.sent(RELAY).is_empty());
    // Gone before the write first reads it: the same words, nothing uploaded.
    let f = fixture(|w| {
        new_collection(w, "bafyNoBox", |collection| {
            collection["assets"].as_object_mut().unwrap().remove("a1");
        });
    });
    let stl = f.file("A.stl", b"solid a");
    assert_eq!(f.attach(&adding(&[&stl])).unwrap_err(), "“Box.gltf” is no longer in the collection.");
    assert_eq!(f.detach("stl", "bafyOldStl").unwrap_err(), "“Box.gltf” is no longer in the collection.");
    assert!(f.sent(UPLOADS).is_empty() && f.sent(RELAY).is_empty());
}

#[test]
fn a_file_refused_first_and_settled_on_the_retry_counts_in_the_total() {
    let f = fixture(with_files);
    let plan_file = f.file("Print.plan", b"later");
    let stl = f.file("A.stl", b"solid a");
    let world = f.world.clone();
    // The other save drops the entry that held `plan`: on the retry the
    // `.plan` file settles too.
    let watched = Watched::new(&f.http).after(2, move || {
        another_save_with(&world, "bafyTheirs", |manifest| {
            manifest["representations"].as_array_mut().unwrap().retain(|entry| entry["id"] != "plan");
        });
    });
    let progress = Progress::default();
    let attached = attach(&f.writer(&watched, &progress), &box_path(), &adding(&[&plan_file, &stl])).unwrap();
    assert!(attached.refused.is_empty());
    let done = progress.snapshot();
    assert_eq!(done.total, Some(5 + 7));
    assert_eq!(done.bytes, 5 + 7);
}

#[test]
fn two_attaches_in_a_row_from_this_app_dont_conflict() {
    let f = fixture(with_files);
    // A node a block behind: two reads after each relay still answer the
    // collection before it, so the second attach waits for the first.
    f.world.lock().unwrap().lag = 2;
    let a = f.file("A.stl", b"solid a");
    let b = f.file("B.gcode", b"G1 X1\n");
    f.attach(&adding(&[&a])).unwrap();
    f.attach(&adding(&[&b])).unwrap();
    f.world.lock().unwrap().lagging.clear();
    let (_, after) = f.box_manifest();
    assert_eq!(ids(&after), ["stl", "plan", "source", "stl-2", "gcode"]);
    assert_eq!(after["version"], json!(5));
    assert_eq!(f.sent(RELAY).len(), 2);
}

#[test]
fn a_refused_relay_leaves_the_asset_unchanged() {
    let f = fixture(with_files);
    f.world.lock().unwrap().relay = testing::Relay::Failed;
    let stl = f.file("A.stl", b"solid a");
    assert_eq!(f.attach(&adding(&[&stl])).unwrap_err(), "Arbesk couldn't save “Box.gltf”.");
    assert_eq!(f.token(), "bafyC1");
    f.world.lock().unwrap().relay = testing::Relay::Status(502);
    assert_eq!(
        f.attach(&adding(&[&stl])).unwrap_err(),
        "Arbesk couldn't save the change. Sign in again to renew its permission."
    );
    assert_eq!(f.token(), "bafyC1");
}

#[test]
fn a_viewer_or_a_lost_session_uploads_nothing() {
    let f = fixture(|w| {
        w.editor_lists.insert("1".into(), ("bafyEditors4".into(), 1));
    });
    let stl = f.file("A.stl", b"solid a");
    assert_eq!(f.attach(&adding(&[&stl])).unwrap_err(), "You can't add to this collection.");
    assert!(f.sent(UPLOADS).is_empty());
    let f = fixture(|w| w.sessions.clear());
    let stl = f.file("A.stl", b"solid a");
    assert_eq!(f.attach(&adding(&[&stl])).unwrap_err(), write::SIGN_IN);
    assert!(f.sent(UPLOADS).is_empty());
}

// ---- A bare composite ----------------------------------------------------------------

/// `besk upload` stored a1 as its bare composite.
fn bare(w: &mut World) {
    let composite = w.files["bafySrcGltf"].clone();
    w.files.insert("bafyBare".into(), composite);
    w.add_asset("1", "a1", "bafyBare");
}

#[test]
fn a_bare_composite_is_wrapped_as_besk_wraps_it() {
    let f = fixture(bare);
    let stl = f.file("Box.stl", b"solid");
    f.attach(&adding(&[&stl])).unwrap();
    let (_, after) = f.box_manifest();
    let composite: Value = serde_json::from_slice(&f.world.lock().unwrap().files["bafyBare"]).unwrap();
    assert_eq!(after["type"], "asset");
    assert_eq!(after["asset_id"], "a1");
    // Named after the composite, else as the listing names it (its id,
    // without a name), without the extension.
    assert_eq!(after["name"], composite.get("name").cloned().unwrap_or(json!("a1")));
    assert_eq!(after["version"], json!(2));
    assert_eq!(after["prev_asset_manifest_cid"], "bafyBare");
    let node = &after["scene"]["nodes"][0];
    assert_eq!(node["node_id"], "node_1");
    assert_eq!(node["source"], json!({ "cid": "bafyBare", "format": "gltf", "path": "composite.gltf" }));
    // Made from the composite: derived from it.
    assert_eq!(after["representations"][0]["derived_from"]["cid"], "bafyBare");
}

#[test]
fn a_child_ref_asset_records_no_derived_from() {
    let f = fixture(|w| w.add_asset("1", "a1", "bafyA3"));
    let stl = f.file("Box.stl", b"solid");
    f.attach(&adding(&[&stl])).unwrap();
    let (_, after) = f.box_manifest();
    assert!(after["representations"][0].get("derived_from").is_none());
    assert_eq!(after["scene"], json!({ "nodes": [{ "child_ref": { "assetID": "x" } }] }));
}

// ---- Removing ----------------------------------------------------------------------

#[test]
fn a_detach_writes_the_next_version_without_the_entry() {
    let f = fixture(with_files);
    let (before_cid, before) = f.box_manifest();
    let attached = f.detach("stl", "bafyOldStl").unwrap();
    let (_, after) = f.box_manifest();
    assert_eq!(without_owned(&after), without_owned(&before));
    assert_eq!(ids(&after), ["plan", "source"]);
    assert_eq!(after["prev_asset_manifest_cid"], json!(before_cid));
    assert_eq!(after["version"], json!(4));
    assert_eq!(attached.files.len(), 1);
    // Uploads: the two manifests, nothing else.
    assert_eq!(f.sent(UPLOADS).len(), 2);
    assert_eq!(f.detach("stl", "bafyOldStl").unwrap_err(), "This file is no longer in the asset.");
}

#[test]
fn a_detach_removes_only_the_file_the_user_saw() {
    const CHANGED: &str = "This file was replaced in the asset since it was shown. Nothing was removed.";
    // Replaced before the page was opened again: refused before anything goes up.
    let f = fixture(with_files);
    assert_eq!(f.detach("stl", "bafyElse").unwrap_err(), CHANGED);
    // An entry File Browser can't show is never removed from here.
    assert_eq!(f.detach("plan", "").unwrap_err(), CHANGED);
    assert!(f.sent(UPLOADS).is_empty() && f.sent(RELAY).is_empty());
    // Replaced by another save while the remove was written: the retry
    // from the new tip finds another file under `stl` and changes nothing.
    let f = fixture(with_files);
    let world = f.world.clone();
    let watched = Watched::new(&f.http).after(1, move || {
        another_save_with(&world, "bafyTheirs", |manifest| {
            manifest["representations"][0]["cid"] = json!("bafyTheirStl");
        });
    });
    assert_eq!(f.detach_with(&watched, "stl", "bafyOldStl").unwrap_err(), CHANGED);
    assert!(f.sent(RELAY).is_empty());
    let (cid, after) = f.box_manifest();
    assert_eq!(cid, "bafyTheirs");
    assert_eq!(after["representations"][0]["cid"], "bafyTheirStl");
}

#[test]
fn a_list_that_isnt_one_is_never_overwritten_or_dropped() {
    let f = fixture(|w| {
        with_files(w);
        let mut manifest: Value = serde_json::from_slice(&w.files["bafyA1"]).unwrap();
        manifest["representations"] = json!({ "schema": 2, "items": [] });
        w.put("bafyA1", manifest);
    });
    let words = "“Box.gltf” has files File Browser can't show.";
    let stl = f.file("Box.stl", b"solid");
    assert_eq!(plan(&f.http, &f.site, &box_path(), std::slice::from_ref(&stl), f.cache.path()).unwrap_err(), words);
    assert_eq!(f.attach(&adding(&[&stl])).unwrap_err(), words);
    assert_eq!(f.detach("stl", "bafyOldStl").unwrap_err(), words);
    assert!(f.sent(UPLOADS).is_empty() && f.sent(RELAY).is_empty());
    assert_eq!(f.token(), "bafyC1");
}

#[test]
fn the_list_goes_once_it_empties() {
    let f = fixture(|w| {
        with_files(w);
        let mut manifest: Value = serde_json::from_slice(&w.files["bafyA1"]).unwrap();
        manifest["representations"] = json!([manifest["representations"][0].clone()]);
        w.put("bafyA1", manifest);
    });
    let attached = f.detach("stl", "bafyOldStl").unwrap();
    let (_, after) = f.box_manifest();
    assert!(after.get("representations").is_none());
    assert!(attached.files.is_empty() && attached.representations.is_none());
}

// ---- Planning and settling -------------------------------------------------------------

#[test]
fn a_plan_says_each_files_record_and_clash() {
    let f = fixture(with_files);
    let stl = f.file("Box.stl", b"solid");
    let plan_file = f.file("Print.plan", b"x");
    let blend = f.file("Lid.BLEND", b"x");
    let three = f.file("Lid.3mf", b"x");
    let planned = plan(&f.http, &f.site, &box_path(), &[stl.clone(), plan_file, blend, three], f.cache.path()).unwrap();
    let rows: Vec<(&str, &str, &str)> = planned.iter().map(|p| (p.id.as_str(), p.role.as_str(), p.format.as_str())).collect();
    assert_eq!(rows, [("stl", "mesh", "stl"), ("plan", "other", "plan"), ("source", "source", "blend"), ("3mf", "mesh", "3mf")]);
    let Some(Clash::Valid { cid, existing }) = &planned[0].clash else { panic!("{:?}", planned[0].clash) };
    assert_eq!((cid.as_str(), existing.name.as_str()), ("bafyOldStl", "Box.stl"));
    assert_eq!(planned[1].clash, Some(Clash::Unreadable { message: held_by_unreadable("plan") }));
    assert!(matches!(planned[2].clash, Some(Clash::Valid { .. })));
    assert_eq!(planned[3].clash, None);
    assert_eq!(planned[0].bytes, 5);
    // A plan writes nothing.
    assert!(f.sent(UPLOADS).is_empty());
    let readme = f.file("README", b"x");
    assert_eq!(plan(&f.http, &f.site, &box_path(), &[readme], f.cache.path()).unwrap_err(), "“README” has no file type to record.");
}

#[test]
fn a_free_id_stays_within_64_characters() {
    let long = "a".repeat(64);
    let stored: HashSet<String> = [long.clone(), format!("{}-2", "a".repeat(62))].into();
    assert_eq!(free_id(&long, &stored, &HashSet::new()), format!("{}-3", "a".repeat(62)));
    let taken: HashSet<String> = ["stl-2".to_string()].into();
    assert_eq!(free_id("stl", &HashSet::new(), &taken), "stl-3");
}

// ---- What a slicer wrote ------------------------------------------------------------------

const PRUSA: &str = "; generated by PrusaSlicer 2.9.2+linux-x64-GTK3 on 2025-03-01 at 10:00:00 UTC\n\
G1 X1 Y1\n\
; filament used [mm] = 12873.45\n\
; filament used [g] = 38.50\n\
; total filament used [g] = 38.50\n\
; estimated printing time (normal mode) = 2h 3m 0s\n\
; prusaslicer_config = begin\n\
; filament_type = PETG\n\
; layer_height = 0.2\n\
; nozzle_diameter = 0.4,0.4\n\
; printer_model = MK4\n\
; printer_settings_id = Original Prusa MK4 0.4 nozzle\n\
; prusaslicer_config = end\n";

const CURA: &str = ";FLAVOR:Marlin\n\
;TIME:6666\n\
;Filament used: 2.34567m\n\
;Layer height: 0.2\n\
;TARGET_MACHINE.NAME:Creality Ender-3\n\
;Generated with Cura_SteamEngine 5.7.1\n\
G1 X1\n";

#[test]
fn a_prusaslicer_gcode_says_its_tool_profile_and_stats() {
    let details = details_of(PRUSA);
    assert_eq!(Value::Object(details.tool.unwrap()), json!({ "name": "PrusaSlicer", "version": "2.9.2" }));
    let profile = details.profile.unwrap();
    assert_eq!(
        Value::Object(profile.clone()),
        json!({ "printer": "Original Prusa MK4", "material": "PETG", "layer_mm": 0.2, "nozzle_mm": 0.4 })
    );
    assert_eq!(Value::Object(details.stats.unwrap()), json!({ "print_time_s": 7380, "filament_g": 38.5 }));
    assert_eq!(reps::representation_id("machine", "gcode", Some(&profile)), "gcode-original-prusa-mk4-petg");
}

#[test]
fn a_cura_gcode_says_what_it_writes() {
    let details = details_of(CURA);
    assert_eq!(Value::Object(details.tool.unwrap()), json!({ "name": "Cura", "version": "5.7.1" }));
    assert_eq!(Value::Object(details.profile.unwrap()), json!({ "printer": "Creality Ender-3", "layer_mm": 0.2 }));
    assert_eq!(Value::Object(details.stats.unwrap()), json!({ "print_time_s": 6666 }));
}

#[test]
fn an_orca_header_and_values_that_dont_parse() {
    let orca = "; generated by OrcaSlicer 2.1.1 on 2025-01-01\n; model printing time: 1h 2m 3s; total estimated time: 1h 5m 0s\n\
; total filament weight [g] : 12.25\n; layer_height = abc\n; nozzle_diameter = -1\n; printer_settings_id = Bambu Lab X1 Carbon 0.4 nozzle\n";
    let details = details_of(orca);
    assert_eq!(Value::Object(details.profile.unwrap()), json!({ "printer": "Bambu Lab X1 Carbon" }));
    assert_eq!(Value::Object(details.stats.unwrap()), json!({ "print_time_s": 3900, "filament_g": 12.25 }));
    let nothing = details_of("G1 X1\n; just a comment\n");
    assert_eq!(nothing, Details::default());
}

#[test]
fn a_gcode_file_is_read_at_both_ends_only() {
    let f = fixture(|_| {});
    let mut text = PRUSA.lines().take(1).collect::<String>() + "\n";
    text.push_str(&"G1 X1 Y1 E0.1\n".repeat(20_000));
    text.push_str(&PRUSA.lines().skip(2).map(|l| format!("{l}\n")).collect::<String>());
    let path = f.file("Bracket.gcode", text.as_bytes());
    let file = local_file(&path).unwrap();
    assert_eq!(file.id, "gcode-original-prusa-mk4-petg");
    assert_eq!(file.role, "machine");
    assert_eq!(file.bytes, text.len() as u64);
}

// ---- Against besk -------------------------------------------------------------------------

/// `besk attach`'s own output for the same inputs, made by
/// `scripts/make-attach-fixture.ts` from an Arbesk checkout.
fn besk_fixture() -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/attach.json");
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

/// Each case's file, made on disk from its text, recorded through the real
/// path (`local_file`: format, role, the G-code's details and id; `settle`:
/// Replace or Keep Both) and added to the next version as an attach adds
/// it. besk records only the profile its flags give, so the tool and stats
/// File Browser reads from a slicer's G-code are checked on their own.
#[test]
fn the_next_manifest_is_besks() {
    let fixture = besk_fixture();
    let dir = tempfile::tempdir().unwrap();
    for (n, case) in fixture["cases"].as_array().unwrap().iter().enumerate() {
        let name = case["name"].as_str().unwrap();
        let (cid, now) = (case["cid"].as_str().unwrap(), case["now"].as_i64().unwrap());
        let base = based_on(case["current"].clone(), cid, case["listed"].as_str().unwrap(), case["asset_id"].as_str().unwrap()).unwrap();
        let mut next = next_version_of(&base, cid, now);
        for file in case["files"].as_array().unwrap() {
            let folder = dir.path().join(n.to_string());
            std::fs::create_dir_all(&folder).unwrap();
            let path = folder.join(file["name"].as_str().unwrap());
            std::fs::write(&path, file["text"].as_str().unwrap()).unwrap();
            let local = local_file(&path.to_string_lossy()).unwrap();
            assert_eq!(local.bytes, file["bytes"].as_u64().unwrap(), "{name}");
            let choice: Option<Choice> = serde_json::from_value(file["choice"].clone()).unwrap();
            let settled = settle(&base, std::slice::from_ref(&local), &[choice]);
            let id = &settled.ids[0].1;
            let encoding = reps::encoding_for(&local.format);
            let mut entry = entry_of(&next, &local, id, file["cid"].as_str().unwrap(), local.bytes, encoding, now);
            let fields = entry.as_object_mut().unwrap();
            let (tool, stats) = (fields.remove("tool"), fields.remove("stats"));
            if local.details.tool.is_some() {
                assert_eq!(tool, Some(json!({ "name": "PrusaSlicer", "version": "2.9.2" })), "{name}");
                assert_eq!(stats, Some(json!({ "print_time_s": 7380, "filament_g": 38.5 })), "{name}");
            }
            next = with_entries(next, vec![entry]);
        }
        assert_eq!(next, case["next"], "{name}");
    }
}
