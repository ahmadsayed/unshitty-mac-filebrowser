//! Arbesk as one of the app's places (design
//! `2026-11-10-provider-crates-design.md`, step 3): [`Client`] implements
//! `places::Provider`, its `Transfer` (a local model file into a
//! collection, an asset's own file out) and the extensions only Arbesk has
//! today: thumbnails, an asset's saves, the preview's scene and parts,
//! sharing a collection and the accounts. Each answer is the `Client`
//! method the app called before, with the same words; a failure is
//! `Error::Other` with them, so a command returns exactly the same text.
//!
//! Arbesk is read only but for copies in, new collections (`create` in an
//! account, which its capabilities allow there only) and sharing:
//! renaming, trashing and restoring are refused with “Arbesk is read
//! only.”, as `place::writable` refuses them in the app.

use std::path::{Path, PathBuf};

use places::{
    Capabilities, Cx, Error, Extensions, Fetch, Fetched, Json, ListOptions, NewKind, Operation,
    PathInfo, Put, Refused, Replace, Result, Shared, SignIn, Trashed, Version, ext,
};

use crate::location::{self, ArbeskPath};
use crate::{Client, Entry};

/// The scheme of Arbesk's paths, `arbesk://…`.
pub const SCHEME: &str = "arbesk";

/// Every write but a copy in, a new collection and sharing.
const READ_ONLY: &str = "Arbesk is read only.";
/// The local-only gate's words for every Arbesk path (`place::local`).
const NOT_LOCAL: &str = "This action is not available for Arbesk.";
/// Copying out takes one asset at a time.
const COLLECTION_COPY: &str = "Copying Arbesk collections comes next.";
/// Copying in goes into a collection.
const INTO_COLLECTION: &str = "Paste into a collection.";
/// A collection is created in an account, nowhere else (the client's words).
const IN_ACCOUNT: &str = "Create collections in the account's folder.";

/// The client's errors are words; they stay these words.
fn words(message: String) -> Error {
    Error::Other(message)
}

fn read_only<T>() -> Result<T> {
    Err(Error::ReadOnly(READ_ONLY.into()))
}

/// A lookup no command routes through a provider yet.
fn not_yet<T>(what: &str) -> Result<T> {
    Err(Error::Unsupported(format!(
        "{what} doesn't go through the places registry yet."
    )))
}

/// `refused` with each operation that has words.
fn refusals(each: [(Operation, Option<String>); 6]) -> Vec<Refused> {
    each.into_iter()
        .filter_map(|(operation, message)| message.map(|message| Refused { operation, message }))
        .collect()
}

/// What an Arbesk path allows. Arbesk is read only but for copies: a local
/// model file into a collection, and an asset out (built first). Its sizes
/// are known only once an asset is built, and a save is a new version.
/// The copy refusals are the app's route table's (`place::copy`), word for
/// word: a malformed path is refused by its parser.
///
/// Creating is what [`places::Provider::create`] does: a folder in an
/// account is a new collection (New Collection), so an account allows it,
/// a collection or an asset refuses it in the client's words, and nothing
/// else is ever created (a new file is refused as read only by `create`
/// itself). The account stays `read_only`: nothing in it is renamed,
/// trashed or copied in.
///
/// An asset's other file (spec F4) allows what its asset does, but has no
/// thumbnail of its own.
pub fn capabilities(path: &str) -> Capabilities {
    let parsed = location::parse(path);
    let is_file = matches!(parsed, Ok(ArbeskPath::File { .. }));
    let is_asset = is_file || matches!(parsed, Ok(ArbeskPath::Asset { .. }));
    let create = match &parsed {
        Ok(ArbeskPath::Account { .. }) => None,
        Ok(_) => Some(IN_ACCOUNT.to_string()),
        Err(error) => Some(error.clone()),
    };
    let copy_in = match &parsed {
        Ok(ArbeskPath::Collection { .. }) => None,
        Ok(_) => Some(INTO_COLLECTION.to_string()),
        Err(error) => Some(error.clone()),
    };
    let copy_out = match &parsed {
        Ok(ArbeskPath::Asset { .. } | ArbeskPath::File { .. }) => None,
        Ok(_) => Some(COLLECTION_COPY.to_string()),
        Err(error) => Some(error.clone()),
    };
    let read_only = || Some(READ_ONLY.to_string());
    Capabilities {
        read_only: true,
        can_create: create.is_none(),
        can_rename: false,
        can_trash: false,
        can_copy_out: is_asset,
        knows_sizes: false,
        knows_counts: false,
        searchable: false,
        has_thumbnails: !is_file,
        // Nothing is renamed here.
        rename_atomic: false,
        replace: Replace::NewVersion,
        names_unique: true,
        // A save lands whole, once Arbesk has it.
        put_is_whole: true,
        refused: refusals([
            (Operation::Create, create),
            (Operation::Rename, read_only()),
            (Operation::Trash, read_only()),
            (Operation::CopyIn, copy_in),
            (Operation::CopyOut, copy_out),
            (Operation::LocalOnly, Some(NOT_LOCAL.into())),
        ]),
    }
}

/// A listing row as one of the app's entries, with an asset's summary of
/// its other files. Only Google Drive sets the `cloud` key (the frontend
/// reads one as a Drive item).
fn place_entry(entry: Entry) -> places::Entry {
    places::Entry {
        name: entry.name,
        path: entry.path,
        is_dir: entry.is_dir,
        size: entry.size,
        modified: entry.modified,
        is_hidden: entry.is_hidden,
        is_symlink: entry.is_symlink,
        cloud: None,
        representations: entry.representations,
    }
}

// ---- The provider --------------------------------------------------------------

impl places::Provider for Client {
    fn scheme(&self) -> &'static str {
        SCHEME
    }
    fn capabilities(&self, path: &str) -> Capabilities {
        capabilities(path)
    }
    /// An account's collections, or a collection's assets.
    fn list(&self, dir: &str, _: &ListOptions, _: &Cx) -> Result<Vec<places::Entry>> {
        let entries = Client::list(self, dir).map_err(words)?;
        Ok(entries.into_iter().map(place_entry).collect())
    }
    fn stat(&self, _: &str, _: &Cx) -> Result<Option<places::Entry>> {
        not_yet("Looking up an item")
    }
    fn lookup(&self, _: &str, _: &Cx) -> Result<Vec<places::Entry>> {
        not_yet("Looking up a name")
    }
    /// A name pending in a collection exists when the collection lists it.
    fn exists(&self, path: &str, _: &Cx) -> bool {
        Client::exists(self, path)
    }
    /// Accounts and collections are folders, assets files; nothing for a
    /// path Arbesk can't read.
    fn path_info(&self, paths: &[String], _: &Cx) -> Vec<Option<PathInfo>> {
        let info = |path: &String| location::is_folder(path).map(|is_dir| PathInfo { is_dir, device: 0 });
        paths.iter().map(info).collect()
    }
    /// An asset opens in Arbesk's Studio, in the browser; the preview gets
    /// its file built in the app cache.
    ///
    /// An asset's other file (spec F4) is decoded into the app cache for
    /// either, with progress (spec F5).
    fn fetch(&self, path: &str, purpose: Fetch, cx: &Cx) -> Result<Fetched> {
        if let Ok(ArbeskPath::File { .. }) = location::parse(path) {
            return self.file_at(path, &cx.progress).map(Fetched::File).map_err(words);
        }
        match purpose {
            Fetch::Open => self.studio_url(path).map(Fetched::Url).map_err(words),
            Fetch::Preview => self.build(path, &cx.progress).map(Fetched::File).map_err(words),
        }
    }
    /// A new collection in the account `parent` (New Collection): its path.
    /// Nothing else is created on Arbesk.
    fn create(&self, parent: &str, name: &str, kind: NewKind, _: &Cx) -> Result<String> {
        match kind {
            NewKind::Folder => self.create_collection(parent, name).map_err(words),
            NewKind::EmptyFile => read_only(),
        }
    }
    fn rename(&self, _: &str, _: &str, _: &Cx) -> Result<String> {
        read_only()
    }
    fn trash(&self, paths: &[&str], _: &Cx) -> Vec<Trashed> {
        let kept = |path: &&str| Trashed { path: path.to_string(), result: read_only() };
        paths.iter().map(kept).collect()
    }
    fn restore(&self, _: &str, _: &Cx) -> Result<String> {
        read_only()
    }
    /// The client itself: its primitives keep nothing between steps.
    fn transfer(&self, _: &Cx, work: &mut dyn FnMut(&dyn places::Transfer)) {
        work(self)
    }
    fn extensions(&self) -> Extensions<'_> {
        Extensions {
            thumbnails: Some(self),
            versions: Some(self),
            accounts: Some(self),
            sharing: Some(self),
            preview: Some(self),
            representations: Some(self),
            properties: Some(self),
            ..Extensions::default()
        }
    }
}

// ---- Copies in and out ---------------------------------------------------------

/// A copy in is `add` (a new asset, or a new version of the one it
/// replaces); a copy out reads the asset's own file, built in the app
/// cache (`export`). Nothing else is written on Arbesk.
/// Why an asset isn't read into a target: a copy out reads its export.
const EXPORTED: &str = "An Arbesk asset is copied from its exported file.";
/// Why a copy into Arbesk looks nothing up first.
const LANDING: &str = "A copy into Arbesk finds the asset it replaces as it saves.";

impl places::Transfer for Client {
    /// The local model file `source` saved at the pending name `target` in
    /// a collection: the new asset's path.
    fn write(&self, source: &Path, target: &str, replace: bool, cx: &Cx) -> Result<Put> {
        let path = self.add(target, source, replace, &cx.progress).map_err(words)?;
        Ok(Put { path, skipped: Vec::new() })
    }
    fn read_into(&self, _: &str, _: &Path, _: &Cx) -> Result<Put> {
        Err(Error::Unsupported(EXPORTED.into()))
    }
    fn read(&self, _: &str, _: &mut dyn FnMut(&dyn places::Reading) -> Result<()>, _: &Cx) -> Result<()> {
        Err(Error::Unsupported(EXPORTED.into()))
    }
    /// A copy in may target a name pending in a collection (`add` makes
    /// the rest of its refusals); nothing else is written on Arbesk.
    fn check(&self, from: Option<&str>, to: Option<&str>, _: &Cx) -> Result<()> {
        match (from, to) {
            (None, Some(to)) => location::parse_target(to).map(|_| ()).map_err(words),
            _ => read_only(),
        }
    }
    /// `write` finds the asset a copy in replaces as it saves (a new
    /// version of it), so there is nothing to look up first.
    fn landing(&self, _: Option<&str>, _: &str, _: &Cx) -> Result<places::Landing> {
        Err(Error::Unsupported(LANDING.into()))
    }
    /// The name pending in the collection `parent`, as `parse_target`
    /// reads it.
    fn pending(&self, parent: &str, name: &str) -> Result<String> {
        match location::parse(parent) {
            Ok(ArbeskPath::Collection { .. }) => Ok(location::asset_path_named(parent, "", name)),
            // A name pending in a collection reads as a target, not a
            // collection: it is refused like any other non-collection.
            _ => Err(words(location::parse_target(parent).err().unwrap_or_else(|| INTO_COLLECTION.to_string()))),
        }
    }
    fn export(&self, path: &str, cx: &Cx) -> Result<PathBuf> {
        Client::export(self, path, &cx.progress).map_err(words)
    }
    fn copy_within(&self, _: &str, _: &str, _: &Cx) -> Result<Put> {
        read_only()
    }
    fn rename_new(&self, _: &str, _: &str, _: &Cx) -> Result<String> {
        read_only()
    }
    fn move_to(&self, _: &str, _: &str, _: &Cx) -> Result<String> {
        read_only()
    }
    fn trash_for_replace(&self, _: &str, _: &Cx) -> Result<String> {
        read_only()
    }
    fn take_back(&self, _: &str, _: &str, _: &Cx) -> Result<()> {
        read_only()
    }
    fn discard(&self, _: &str, _: &Cx) -> Result<()> {
        read_only()
    }
}

// ---- Extensions ----------------------------------------------------------------

impl ext::Thumbnails for Client {
    /// The thumbnail the asset was saved with, if any.
    fn thumbnail(&self, path: &str, size: u32, stop: &dyn Fn() -> bool) -> Vec<u8> {
        Client::thumbnail(self, path, size, stop)
    }
}

impl ext::Versions for Client {
    fn versions(&self, path: &str) -> Result<Vec<Version>> {
        Client::versions(self, path).map_err(words)
    }
}

impl ext::Preview for Client {
    fn scene(&self, path: &str, version: Option<&str>) -> Result<Json> {
        Json::of(&Client::scene(self, path, version).map_err(words)?)
    }
    fn part(&self, path: &str, version: Option<&str>, key: &str, cx: &Cx) -> Result<PathBuf> {
        Client::part(self, path, version, key, &cx.progress).map_err(words)
    }
}

impl ext::Representations for Client {
    fn representations(&self, path: &str, version: Option<&str>) -> Result<Json> {
        Json::of(&Client::representations(self, path, version).map_err(words)?)
    }
    fn file(&self, path: &str, version: Option<&str>, id: &str, cx: &Cx) -> Result<PathBuf> {
        self.representation_file(path, version, id, &cx.progress).map_err(words)
    }
}

impl ext::Properties for Client {
    /// An asset's name, collection, save date and version (spec F7).
    fn properties(&self, path: &str) -> Result<places::FileProperties> {
        Client::properties(self, path).map_err(words)
    }
}

impl ext::Sharing for Client {
    fn share(&self, path: &str, email: &str, role: u8) -> Result<Shared> {
        self.share_collection(path, email, role).map_err(words)
    }
}

impl ext::Accounts for Client {
    fn accounts(&self) -> Vec<Json> {
        Client::accounts(self).iter().filter_map(|account| Json::of(account).ok()).collect()
    }
    fn sign_in_start(&self, email: &str) -> Result<SignIn> {
        let (flow_id, url) = self.connect_start(email).map_err(words)?;
        Ok(SignIn { flow_id, url })
    }
    fn sign_in_finish(&self, flow_id: &str) -> Result<Json> {
        Json::of(&self.connect_finish(flow_id).map_err(words)?)
    }
    fn sign_in_cancel(&self, flow_id: &str) {
        self.connect_cancel(flow_id);
    }
    fn signed_in(&self, path: &str) -> bool {
        Client::signed_in(self, path)
    }
    fn disconnect(&self, account: &str) -> Result<()> {
        Client::disconnect(self, account).map_err(words)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use places::Provider;

    use super::*;
    use crate::session::{MemorySessions, Session, SessionStore};
    use crate::{Account, signin, testing};

    const ACCOUNT: &str = "arbesk://0x00000000000000000000000000000000000000ab";
    const COLLECTION: &str = "arbesk://0x00000000000000000000000000000000000000ab/12~Set";
    const ASSET: &str = "arbesk://0x00000000000000000000000000000000000000ab/12~Set/a1~Box.gltf";

    /// A client over the fake Arbesk, with `testing::ADDRESS` connected and
    /// signed in, and the sessions it keeps.
    fn client(dir: &Path) -> (Client, Account, MemorySessions) {
        client_with(dir, |_| {})
    }

    /// [`client`] over the fake Arbesk as `edit` leaves it.
    fn client_with(dir: &Path, edit: impl FnOnce(&mut testing::World)) -> (Client, Account, MemorySessions) {
        let mut world = testing::world();
        edit(&mut world);
        world.sessions.insert("t".into());
        let http = Arc::new(testing::arbesk(Arc::new(Mutex::new(world))));
        let sessions = MemorySessions::default();
        let client = Client::with_sessions(
            http,
            dir.join("cache"),
            dir.join("arbesk_accounts.json"),
            Box::new(sessions.clone()),
        );
        let account = client
            .accounts
            .lock()
            .unwrap()
            .connect("me@example.com", testing::ADDRESS, &testing::site())
            .unwrap();
        let kept = Session { token: "t".into(), expires_at_ms: signin::now_ms() + 3_600_000 };
        sessions.save(&account.id, &kept).unwrap();
        (client, account, sessions)
    }

    fn refusal(caps: &Capabilities, operation: Operation) -> Option<String> {
        caps.refused.iter().find(|r| r.operation == operation).map(|r| r.message.clone())
    }

    /// Read only, with thumbnails, a save as a new version; a collection
    /// created only in an account; copies in only into a collection and out
    /// only of an asset, each other path refused as the app's route table
    /// refuses it.
    #[test]
    fn what_each_arbesk_path_allows() {
        for path in [ACCOUNT, COLLECTION, ASSET, "arbesk://", "arbesk://bad"] {
            let caps = capabilities(path);
            assert!(caps.read_only && !caps.can_rename && !caps.can_trash, "{path}");
            assert_eq!(caps.can_create, path == ACCOUNT, "{path}");
            assert!(!caps.knows_sizes && !caps.knows_counts && !caps.searchable && caps.has_thumbnails);
            assert_eq!((caps.replace, caps.put_is_whole, caps.names_unique), (Replace::NewVersion, true, true));
            for operation in [Operation::Rename, Operation::Trash] {
                assert_eq!(refusal(&caps, operation).as_deref(), Some(READ_ONLY), "{path}");
            }
            assert_eq!(refusal(&caps, Operation::LocalOnly).as_deref(), Some(NOT_LOCAL));
            assert_eq!(caps.can_copy_out, path == ASSET, "{path}");
        }
        assert_eq!(refusal(&capabilities(ACCOUNT), Operation::Create), None);
        assert_eq!(refusal(&capabilities(COLLECTION), Operation::Create).as_deref(), Some(IN_ACCOUNT));
        assert_eq!(refusal(&capabilities(ASSET), Operation::Create).as_deref(), Some(IN_ACCOUNT));
        assert_eq!(refusal(&capabilities(COLLECTION), Operation::CopyIn), None);
        assert_eq!(refusal(&capabilities(ACCOUNT), Operation::CopyIn).as_deref(), Some(INTO_COLLECTION));
        assert_eq!(refusal(&capabilities(ASSET), Operation::CopyIn).as_deref(), Some(INTO_COLLECTION));
        assert_eq!(refusal(&capabilities(ASSET), Operation::CopyOut), None);
        assert_eq!(refusal(&capabilities(COLLECTION), Operation::CopyOut).as_deref(), Some(COLLECTION_COPY));
        let bad = location::parse("arbesk://bad").unwrap_err();
        assert_eq!(refusal(&capabilities("arbesk://bad"), Operation::Create), Some(bad.clone()));
        assert_eq!(refusal(&capabilities("arbesk://bad"), Operation::CopyIn), Some(bad.clone()));
        assert_eq!(refusal(&capabilities("arbesk://bad"), Operation::CopyOut), Some(bad));
    }

    /// An asset's other file allows what its asset allows, but has no
    /// thumbnail of its own (spec F4).
    #[test]
    fn an_assets_other_file_allows_what_its_asset_does() {
        let file = format!("{ASSET}/stl@bafyStl~Box.stl");
        let (caps, asset) = (capabilities(&file), capabilities(ASSET));
        assert!(caps.can_copy_out && caps.read_only && !caps.can_create && !caps.has_thumbnails);
        assert!(asset.has_thumbnails);
        for operation in [Operation::Create, Operation::Rename, Operation::Trash, Operation::CopyIn, Operation::CopyOut, Operation::LocalOnly] {
            assert_eq!(refusal(&caps, operation), refusal(&asset, operation), "{operation:?}");
        }
    }

    const GCODE: &str = "G1 X10 Y10 E0.5\n";

    /// Shapes' Box.gltf (a1) with other files: a G-code stored
    /// brotli-framed, a raw `.blend` whose bytes start like a frame, one
    /// whose `bytes` is wrong, and one named to leave its folder.
    fn with_files(world: &mut testing::World) {
        let text = GCODE.repeat(500).into_bytes();
        let (stored, _) = crate::representations::encode(&text, "gcode");
        world.files.insert("bafyGcode".into(), stored);
        let blend = b"ARB\x01not a frame, a blend".to_vec();
        world.files.insert("bafyBlend".into(), blend.clone());
        let entry = |id: &str, format: &str, name: &str, cid: &str, bytes: usize, encoding: &str| {
            serde_json::json!({ "id": id, "role": "other", "format": format, "name": name, "cid": cid,
                "bytes": bytes, "encoding": encoding, "created_at": 1 })
        };
        let mut manifest: serde_json::Value = serde_json::from_slice(&world.files["bafyA1"]).unwrap();
        manifest["representations"] = serde_json::json!([
            entry("gcode", "gcode", "Bracket.gcode", "bafyGcode", text.len(), "brotli"),
            entry("source", "blend", "../Bracket.blend", "bafyBlend", blend.len(), "raw"),
            entry("short", "blend", "Short.blend", "bafyBlend", blend.len() + 1, "raw"),
        ]);
        world.put("bafyA1", manifest);
    }

    fn opened(client: &Client, path: &str) -> Result<PathBuf> {
        match client.fetch(path, Fetch::Open, &Cx::default())? {
            Fetched::File(file) => Ok(file),
            Fetched::Url(url) => panic!("{path} opened {url}"),
        }
    }

    /// Spec F5: a file is fetched without unpacking, decoded by its entry's
    /// encoding, checked and saved in the app cache with the mark; an
    /// unchanged copy is reused, an edited one kept beside a fresh copy.
    #[test]
    fn an_assets_other_file_opens_from_the_app_cache() {
        let dir = tempfile::tempdir().unwrap();
        let (client, _, _) = client_with(dir.path(), with_files);
        let asset = location::asset_path(&testing::shapes(), "a1", "Box", "gltf");
        let gcode = location::file_path(&asset, "gcode", None, "Bracket.gcode");
        let file = opened(&client, &gcode).unwrap();
        assert_eq!(file, dir.path().join("cache/arbesk/files/bafyGcode/Bracket.gcode"));
        assert_eq!(std::fs::read_to_string(&file).unwrap(), GCODE.repeat(500));
        assert!(places::is_unchanged(&file));
        assert_eq!(opened(&client, &gcode).unwrap(), file, "an unchanged copy is reused");
        let preview = client.fetch(&gcode, Fetch::Preview, &Cx::default()).unwrap();
        assert_eq!(preview, Fetched::File(file.clone()));
        std::fs::write(&file, "edited").unwrap();
        let fresh = opened(&client, &gcode).unwrap();
        assert_ne!(fresh, file);
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "edited", "an edited copy is kept");
        assert_eq!(std::fs::read_to_string(&fresh).unwrap(), GCODE.repeat(500));

        // A raw file is never unpacked, and its name stays one file name.
        let blend = opened(&client, &location::file_path(&asset, "source", Some("bafyBlend"), "x")).unwrap();
        assert_eq!(blend, dir.path().join("cache/arbesk/files/bafyBlend/.._Bracket.blend"));
        assert!(std::fs::read(&blend).unwrap().starts_with(b"ARB\x01not a frame"));
    }

    /// Spec F9: a copy out of an asset's other file exports it as an open
    /// fetches it (into `arbesk/files/<cid>/`), and a pinned path whose
    /// entry moved since it was copied is refused in the same words.
    #[test]
    fn an_assets_other_file_exports_as_it_opens() {
        let dir = tempfile::tempdir().unwrap();
        let (client, _, _) = client_with(dir.path(), with_files);
        let asset = location::asset_path(&testing::shapes(), "a1", "Box", "gltf");
        let pinned = location::file_path(&asset, "gcode", Some("bafyGcode"), "Bracket.gcode");
        let exported = places::Transfer::export(&client, &pinned, &Cx::default()).unwrap();
        assert_eq!(exported, dir.path().join("cache/arbesk/files/bafyGcode/Bracket.gcode"));
        assert_eq!(std::fs::read_to_string(&exported).unwrap(), GCODE.repeat(500));
        assert_eq!(exported, opened(&client, &pinned).unwrap(), "the same cached copy");
        let moved = location::file_path(&asset, "gcode", Some("bafyOld"), "Bracket.gcode");
        assert_eq!(
            places::Transfer::export(&client, &moved, &Cx::default()).unwrap_err(),
            Error::Other("“Bracket.gcode” changed in Arbesk since it was copied.".into())
        );
    }

    /// Each listed file carries its path pinned to its CID (spec F4, F9).
    #[test]
    fn listed_files_carry_their_pinned_paths() {
        let dir = tempfile::tempdir().unwrap();
        let (client, _, _) = client_with(dir.path(), with_files);
        let asset = location::asset_path(&testing::shapes(), "a1", "Box", "gltf");
        let listed = Client::representations(&client, &asset, None).unwrap();
        assert_eq!(listed[0].path, location::file_path(&asset, "gcode", Some("bafyGcode"), "Bracket.gcode"));
        assert_eq!(listed[1].path, location::file_path(&asset, "source", Some("bafyBlend"), "../Bracket.blend"));
        assert!(listed[1].path.ends_with("/source@bafyBlend~.._Bracket.blend"), "{}", listed[1].path);
    }

    #[test]
    fn a_file_that_changed_moved_or_was_damaged_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let (client, _, _) = client_with(dir.path(), with_files);
        let asset = location::asset_path(&testing::shapes(), "a1", "Box", "gltf");
        let refused = |path: String| opened(&client, &path).unwrap_err();
        let changed = Error::Other("“Bracket.gcode” changed in Arbesk since it was copied.".into());
        assert_eq!(refused(location::file_path(&asset, "gcode", Some("bafyOld"), "Bracket.gcode")), changed);
        let gone = Error::Other("“Old.gcode” changed in Arbesk since it was copied.".into());
        assert_eq!(refused(location::file_path(&asset, "gone", Some("bafyGcode"), "Old.gcode")), gone);
        assert_eq!(
            refused(location::file_path(&asset, "gone", None, "Old.gcode")),
            Error::Other("This file is no longer in the asset.".into())
        );
        assert_eq!(
            refused(location::file_path(&asset, "short", None, "Short.blend")),
            Error::Other("Arbesk sent a damaged file.".into())
        );
        assert!(!dir.path().join("cache/arbesk/files/bafyBlend/Short.blend").exists());
    }

    /// The extension lists an asset's files and fetches one of a given save.
    #[test]
    fn the_extension_lists_and_fetches_an_assets_files() {
        let dir = tempfile::tempdir().unwrap();
        let (client, _, _) = client_with(dir.path(), with_files);
        let files = client.extensions().representations.unwrap();
        let asset = location::asset_path(&testing::shapes(), "a1", "Box", "gltf");
        let listed: serde_json::Value = serde_json::from_str(files.representations(&asset, None).unwrap().as_str()).unwrap();
        let ids: Vec<&str> = listed.as_array().unwrap().iter().map(|f| f["id"].as_str().unwrap()).collect();
        assert_eq!(ids, ["gcode", "source", "short"]);
        assert_eq!(listed[0]["name"], "Bracket.gcode");
        assert_eq!(listed[0]["stale"], false);
        let at_version: serde_json::Value = serde_json::from_str(files.representations(&asset, Some("bafyA1")).unwrap().as_str()).unwrap();
        assert_eq!(at_version, listed);
        let before: serde_json::Value = serde_json::from_str(files.representations(&asset, Some("bafyA2")).unwrap().as_str()).unwrap();
        assert_eq!(before, serde_json::json!([]));
        let file = files.file(&asset, Some("bafyA1"), "gcode", &Cx::default()).unwrap();
        assert_eq!(file, dir.path().join("cache/arbesk/files/bafyGcode/Bracket.gcode"));
        assert_eq!(files.file(&asset, Some("bafyA2"), "gcode", &Cx::default()).unwrap_err(), Error::Other("This file is no longer in the asset.".into()));
        assert!(files.representations(&testing::shapes(), None).is_err());
    }

    /// Properties for an asset: its listed name, its collection, the save's
    /// date and its version while that is a whole number from 0 up; an
    /// account, a collection and an asset's other file are refused (F7).
    #[test]
    fn an_asset_has_properties_with_its_version() {
        let versions = [(serde_json::json!(3), Some(3)), (serde_json::json!(3.0), Some(3)), (serde_json::json!(2.5), None),
            (serde_json::json!(-1), None), (serde_json::json!("7"), Some(1)), (serde_json::Value::Null, Some(1))];
        for (version, shown) in versions {
            let dir = tempfile::tempdir().unwrap();
            let (client, _, _) = client_with(dir.path(), |world| {
                let mut manifest: serde_json::Value = serde_json::from_slice(&world.files["bafyA1"]).unwrap();
                manifest["version"] = version.clone();
                world.files.insert("bafyA1".into(), serde_json::to_vec(&manifest).unwrap());
            });
            let described = client.extensions().properties.unwrap();
            let asset = location::asset_path(&testing::shapes(), "a1", "Box", "gltf");
            let props = described.properties(&asset).unwrap();
            assert_eq!(props.version, shown, "{version}");
            assert_eq!((props.name.as_str(), props.parent.as_str()), ("Box.gltf", testing::shapes().as_str()));
            assert_eq!(props.modified, testing::STAMP_MS / 1000);
            assert!(!props.is_dir && props.size == 0 && props.owner.is_empty() && props.mode.is_empty());
            for path in [testing::shapes(), format!("{asset}/stl~Box.stl")] {
                assert_eq!(
                    described.properties(&path).unwrap_err(),
                    Error::Other("Properties aren't available for Arbesk yet.".into()),
                    "{path}"
                );
            }
        }
    }

    /// The JSON the frontend gets for a listed asset, byte for byte, with no
    /// `cloud` key: the frontend reads one as a Drive item.
    #[test]
    fn an_arbesk_listing_row_becomes_an_app_entry() {
        let entry = place_entry(Entry {
            name: "Box.gltf".into(),
            path: "arbesk://0xab/1~Set/a1~Box.gltf".into(),
            is_dir: false,
            size: 0,
            modified: 1_762_000_000,
            is_hidden: false,
            is_symlink: false,
            representations: None,
        });
        assert_eq!(entry.name, "Box.gltf");
        assert_eq!(entry.path, "arbesk://0xab/1~Set/a1~Box.gltf");
        assert!(!entry.is_dir && entry.size == 0 && entry.modified == 1_762_000_000);
        assert!(!entry.is_hidden && !entry.is_symlink && entry.cloud.is_none());
        assert_eq!(
            serde_json::to_string(&entry).unwrap(),
            concat!(
                r#"{"name":"Box.gltf","path":"arbesk://0xab/1~Set/a1~Box.gltf","is_dir":false,"#,
                r#""size":0,"modified":1762000000,"is_hidden":false,"is_symlink":false}"#
            ),
            "no cloud key: the frontend reads one as a Drive item"
        );
    }

    /// The provider lists what the client lists, knows a pending name its
    /// collection lists, opens an asset in the Studio and builds its
    /// preview file.
    #[test]
    fn the_provider_reads_as_the_client_reads() {
        let dir = tempfile::tempdir().unwrap();
        let (client, _, _) = client(dir.path());
        let cx = Cx::default();
        let shapes = testing::shapes();
        let listed = Provider::list(&client, &shapes, &ListOptions::default(), &cx).unwrap();
        let theirs = Client::list(&client, &shapes).unwrap();
        assert_eq!(serde_json::to_string(&listed).unwrap(), serde_json::to_string(&theirs).unwrap());
        assert!(Provider::exists(&client, &format!("{shapes}/~Box.gltf"), &cx));
        assert!(!Provider::exists(&client, &format!("{shapes}/~Other.glb"), &cx));
        let asset = location::asset_path(&shapes, "a1", "Box", "gltf");
        let studio = client.studio_url(&asset).unwrap();
        assert_eq!(client.fetch(&asset, Fetch::Open, &cx).unwrap(), Fetched::Url(studio));
        let Fetched::File(built) = client.fetch(&asset, Fetch::Preview, &cx).unwrap() else {
            panic!("a preview is a file");
        };
        assert_eq!(built, client.build(&asset, &places::Progress::default()).unwrap());
        assert_eq!(
            client.fetch(&shapes, Fetch::Preview, &cx).unwrap_err(),
            Error::Other(client.build(&shapes, &places::Progress::default()).unwrap_err())
        );
        let thumbnails = client.extensions().thumbnails.unwrap();
        assert!(thumbnails.thumbnail("arbesk://bad", 64, &|| false).is_empty());
    }

    #[test]
    fn accounts_and_collections_are_folders_and_assets_files() {
        let dir = tempfile::tempdir().unwrap();
        let (client, _, _) = client(dir.path());
        let paths = [ACCOUNT, COLLECTION, ASSET, "arbesk://bad"].map(String::from);
        let kinds: Vec<Option<bool>> =
            client.path_info(&paths, &Cx::default()).iter().map(|i| i.as_ref().map(|i| i.is_dir)).collect();
        assert_eq!(kinds, vec![Some(true), Some(true), Some(false), None]);
    }

    /// Nothing on Arbesk is renamed, trashed or restored; only a collection
    /// is created, in an account, with the client's own refusals.
    #[test]
    fn only_a_new_collection_is_created() {
        let dir = tempfile::tempdir().unwrap();
        let (client, _, _) = client(dir.path());
        let cx = Cx::default();
        let read_only = Err(Error::ReadOnly(READ_ONLY.into()));
        assert_eq!(client.rename(ASSET, "b", &cx), read_only);
        assert_eq!(client.restore(ASSET, &cx), read_only);
        assert_eq!(client.trash(&[ASSET], &cx), vec![Trashed { path: ASSET.into(), result: Err(Error::ReadOnly(READ_ONLY.into())) }]);
        assert_eq!(client.create(ACCOUNT, "x", NewKind::EmptyFile, &cx), read_only);
        assert!(matches!(client.stat(ASSET, &cx), Err(Error::Unsupported(_))));
        assert!(matches!(client.lookup(ASSET, &cx), Err(Error::Unsupported(_))));
        let root = format!("arbesk://{}", testing::ADDRESS);
        let made = client.create(&root, "My Set", NewKind::Folder, &cx).unwrap();
        let token = crate::create::named_collection_id(testing::ADDRESS, "My Set").unwrap();
        assert_eq!(made, location::collection_path(testing::ADDRESS, &token, "My Set"));
        assert_eq!(
            client.create(&made, "Another", NewKind::Folder, &cx),
            Err(Error::Other("Create collections in the account's folder.".into()))
        );
    }

    /// `create` and the capabilities agree: a collection is created where
    /// they allow it, and anywhere else `create` refuses in the very words
    /// of their Create refusal, before it asks Arbesk anything.
    #[test]
    fn a_new_collection_goes_where_the_capabilities_allow_it() {
        let dir = tempfile::tempdir().unwrap();
        let (client, _, _) = client(dir.path());
        let cx = Cx::default();
        let root = format!("arbesk://{}", testing::ADDRESS);
        let made = client.create(&root, "My Set", NewKind::Folder, &cx).unwrap();
        let asset = format!("{made}/a1~Box.gltf");
        assert!(capabilities(&root).can_create);
        for path in [made.as_str(), asset.as_str(), "arbesk://", "arbesk://bad"] {
            let caps = capabilities(path);
            assert!(!caps.can_create, "{path}");
            let refused = refusal(&caps, Operation::Create).unwrap();
            assert_eq!(client.create(path, "x", NewKind::Folder, &cx), Err(Error::Other(refused)), "{path}");
        }
    }

    /// A copy in saves the model file in the collection; a copy out is the
    /// asset's exported file.
    #[test]
    fn a_copy_goes_in_by_writing_and_out_by_exporting() {
        let dir = tempfile::tempdir().unwrap();
        let (client, _, _) = client(dir.path());
        let cx = Cx::default();
        places::with_transfer(&client, &cx, |transfer| copies_in_and_out(&client, transfer, dir.path())).unwrap();
    }

    fn copies_in_and_out(client: &Client, transfer: &dyn places::Transfer, dir: &Path) {
        let cx = Cx::default();
        let file = testing::fixture("glb-box").join("input.glb");
        let to = format!("{}/~Fresh.glb", testing::shapes());
        let put = transfer.write(&file, &to, false, &cx).unwrap();
        assert!(put.skipped.is_empty());
        assert!(put.path.ends_with("~Fresh.gltf"), "{}", put.path);
        assert!(Client::list(client, &testing::shapes()).unwrap().iter().any(|e| e.path == put.path));
        let outside = format!("{ACCOUNT}/~Fresh.glb");
        let refused = transfer.write(&file, &outside, false, &cx).unwrap_err();
        assert_eq!(refused, Error::Other(client.add(&outside, &file, false, &places::Progress::default()).unwrap_err()));

        let asset = location::asset_path(&testing::shapes(), "a1", "Box", "gltf");
        let exported = transfer.export(&asset, &cx).unwrap();
        assert_eq!(exported, client.export(&asset, &places::Progress::default()).unwrap());
        let collection = transfer.export(&testing::shapes(), &cx).unwrap_err();
        assert_eq!(collection, Error::Other(client.export(&testing::shapes(), &places::Progress::default()).unwrap_err()));
        assert!(matches!(transfer.read_into(&asset, dir, &cx), Err(Error::Unsupported(_))));
        let read_only = Err::<String, _>(Error::ReadOnly(READ_ONLY.into()));
        assert_eq!(transfer.rename_new(&asset, &to, &cx), read_only);
        assert_eq!(transfer.move_to(&asset, &to, &cx), read_only);
        assert_eq!(transfer.trash_for_replace(&asset, &cx), read_only);
        assert_eq!(transfer.take_back(&asset, &asset, &cx), Err(Error::ReadOnly(READ_ONLY.into())));
        assert_eq!(transfer.discard(&asset, &cx), Err(Error::ReadOnly(READ_ONLY.into())));
        assert_eq!(transfer.copy_within(&asset, &to, &cx), Err(Error::ReadOnly(READ_ONLY.into())));
    }

    /// A copy in may target a name pending in a collection, which `pending`
    /// builds as `parse_target` reads it; anything else is refused, and
    /// `landing` leaves the asset a copy replaces to `write`.
    #[test]
    fn a_copy_in_targets_a_name_pending_in_a_collection() {
        let dir = tempfile::tempdir().unwrap();
        let (client, _, _) = client(dir.path());
        let cx = Cx::default();
        let pending = places::Transfer::pending(&client, COLLECTION, "My Box.gltf").unwrap();
        assert_eq!(pending, format!("{COLLECTION}/~My%20Box.gltf"));
        let (_, token, name) = location::parse_target(&pending).unwrap();
        assert_eq!((token.as_str(), name.as_str()), ("12", "My Box.gltf"));
        assert_eq!(places::Transfer::check(&client, None, Some(&pending), &cx), Ok(()));
        let into = Error::Other("Paste into a collection.".into());
        assert_eq!(places::Transfer::pending(&client, ACCOUNT, "x.glb"), Err(into.clone()));
        assert_eq!(places::Transfer::pending(&client, ASSET, "x.glb"), Err(into.clone()));
        assert_eq!(places::Transfer::pending(&client, &pending, "y.glb"), Err(into.clone()));
        assert_eq!(places::Transfer::check(&client, None, Some(&format!("{ACCOUNT}/~x.glb")), &cx), Err(into));
        let read_only = || Err(Error::ReadOnly(READ_ONLY.into()));
        assert_eq!(places::Transfer::check(&client, Some(ASSET), Some(&pending), &cx), read_only());
        assert_eq!(places::Transfer::check(&client, Some(ASSET), None, &cx), read_only());
        let landing = places::Transfer::landing(&client, None, &pending, &cx);
        assert_eq!(landing, Err(Error::Unsupported(LANDING.into())));
    }

    /// The saves, the scene and its parts, and sharing answer as the client
    /// does; a scene is handed on as the client serialises it.
    #[test]
    fn versions_preview_and_sharing_are_the_clients() {
        let dir = tempfile::tempdir().unwrap();
        let (client, _, _) = client(dir.path());
        let extensions = client.extensions();
        let asset = location::asset_path(&testing::shapes(), "a1", "Box", "gltf");
        let versions = extensions.versions.unwrap().versions(&asset).unwrap();
        assert_eq!(versions, Client::versions(&client, &asset).unwrap());
        assert!(extensions.versions.unwrap().versions(ACCOUNT).is_err());

        let preview = extensions.preview.unwrap();
        let scene = Client::scene(&client, &asset, None).unwrap();
        assert_eq!(preview.scene(&asset, None).unwrap().as_str(), serde_json::to_string(&scene).unwrap());
        let key = &scene.parts[0].key;
        let part = preview.part(&asset, None, key, &Cx::default()).unwrap();
        assert_eq!(part, Client::part(&client, &asset, None, key, &places::Progress::default()).unwrap());
        assert_eq!(
            preview.part(&asset, None, "bafyNotHere", &Cx::default()).unwrap_err(),
            Error::Other(Client::part(&client, &asset, None, "bafyNotHere", &places::Progress::default()).unwrap_err())
        );

        let sharing = extensions.sharing.unwrap();
        let shared = sharing.share(&testing::shapes(), "alice@example.com", 1).unwrap();
        assert_eq!(shared, Shared { address: testing::ALICE.into(), role: 1, added: true });
        assert_eq!(
            sharing.share(&testing::shapes(), "alice@example.com", 1).unwrap_err(),
            Error::Other("That account is already a viewer.".into())
        );
        assert_eq!(sharing.share(ACCOUNT, "a@example.com", 1).unwrap_err(), Error::Other("Share a collection.".into()));
    }

    /// The accounts are the client's, each as its JSON; signing in starts,
    /// cancels and is refused as the client's flow is, and disconnecting
    /// forgets the account and its session.
    #[test]
    fn the_accounts_are_the_clients() {
        let dir = tempfile::tempdir().unwrap();
        let (client, account, sessions) = client(dir.path());
        let accounts = client.extensions().accounts.unwrap();
        let listed: Vec<String> = accounts.accounts().iter().map(|a| a.as_str().to_string()).collect();
        assert_eq!(listed, vec![serde_json::to_string(&account).unwrap()]);
        assert!(accounts.signed_in(ACCOUNT));
        assert!(!accounts.signed_in("arbesk://0x00000000000000000000000000000000000000cd"));

        let started = accounts.sign_in_start("me@example.com").unwrap();
        assert!(started.url.contains("/api/v1/cli-auth?email=me%40example.com&port="), "{}", started.url);
        accounts.sign_in_cancel(&started.flow_id);
        assert_eq!(
            accounts.sign_in_finish(&started.flow_id).unwrap_err(),
            Error::Other("This sign-in attempt is no longer active.".into())
        );

        accounts.disconnect(&account.id).unwrap();
        assert!(accounts.accounts().is_empty());
        assert!(!accounts.signed_in(ACCOUNT));
        assert_eq!(sessions.load(&account.id), None);
    }
}
