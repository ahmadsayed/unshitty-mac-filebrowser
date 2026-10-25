# Folder Sharing over iroh Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Share a local folder read-only with "anyone with the link" or a specific person, browsed live by the recipient under Network, directly between two apps over iroh.

**Architecture:** A new platform-neutral `src-tauri/src/share/` module. The sharer walks the folder into structure manifests (postcard, canonical) whose root hash names the tree; file content is hashed on first request and served by reference through `iroh-blobs`, behind a gate that admits only allowed keys and published hashes. A thin control protocol (`filebrowser/share/1`) carries `Claim`, `Root`, `Content` and `Watch`. The recipient sees shares as `share://<received-id>/…` locations that reuse the existing list / open / transfer dispatch in `lib.rs`.

**Tech Stack:** Tauri 2, Rust (edition 2024), `iroh` 1.x, `iroh-blobs` 0.103 (`default-features = false`, `fs-store`), `postcard`, `data-encoding`, `notify` (already present), TypeScript + Vite, Vitest + jsdom.

**Spec:** `docs/superpowers/specs/2026-10-25-iroh-sharing-design.md` (D1–D15, Command surface, Data flows, UI, Error handling, Testing, Delivery).

## Global Constraints

- Platform agnostic: no `cfg!(target_os)` in `src-tauri/src/share/`; only `lib.rs` uses `AppHandle` or other Tauri types.
- **Read only:** no command or protocol message lets a recipient change a byte of the sharer's files.
- Visitors send hashes and entry names, never paths (D8). The sharer resolves names from its own manifest map.
- Never served: `.versions/`, symlinks resolving outside the shared folder, sockets, FIFOs, device files, unreadable items, non-UTF-8 names (D8; non-UTF-8 is this plan's ruling, see Review Focus 3).
- Recipient-side writes stage and rename; nothing is ever overwritten without the user's Replace (D11). Every manifest name is validated before writing: no `/`, empty, `.`, `..`, NUL.
- A person-link binding is written to disk before the `Claim` reply (D6).
- Limits (D13): manifest ≤ 64 MiB (`MAX_MANIFEST = 64 * 1024 * 1024`), control messages ≤ 64 KiB (`MAX_MESSAGE = 64 * 1024`), ≤ 32 visitor connections (`MAX_VISITORS = 32`), connect timeout 10 s (`CONNECT_TIMEOUT = Duration::from_secs(10)`).
- Files: `share-key` (0600, locked while sharing), `shares.json`, `received.json` (both 0600) in the app config dir; recipient blob store in the app cache dir, emptied on startup.
- ALPN `b"filebrowser/share/1"`; link prefix `share://`; base32 lowercase, no padding.
- GNOME wording exactly as the spec's UI and Error handling sections give it (e.g. "This link was already used on another computer.", "This link is no longer valid.", "Ahmed Is Offline").
- Commands that touch many files or the network are `async fn` in `lib.rs`; file work goes through `blocking(…)`. Never `#[tauri::command(async)]` on a plain fn.
- Every new Tauri command has a mock in `dev/mock-tauri/core.ts` and a case in `tests/mock-core.test.ts`.
- New crates only as listed in Tech Stack; update `Cargo.lock` minimally and check the diff only adds entries. Never commit `package-lock.json`.
- Before each commit: `npx tsc --noEmit`, `npm run build`, `npm test`, `cd src-tauri && cargo test`; the pre-commit CRAP gate (≤ 30) must pass. No `--no-verify`.
- Network tests run over loopback with relays and discovery off, each under a hard timeout; the one real-relay test is `#[ignore]`.

## Review Focus

1. **A shared folder with a very large directory** (10,000 entries in one folder): the walk finishes, the manifest stays under the cap, and listing it on the recipient side is one manifest fetch (Task 3 test `a_ten_thousand_entry_folder_fits_one_manifest`).
2. **Sharing a folder that can't be shared** — a file, the Trash folder, a folder inside `.versions/`, a `share://` or `gdrive://` path: `share_folder` refuses with "Only folders on this computer can be shared." (Task 4 test `only_local_folders_can_be_shared`).
3. **Non-UTF-8 file names on Linux:** they can't round-trip through a `String` name, so they are left out of manifests quietly rather than mangled (Task 3 test `non_utf8_names_are_left_out`).
4. **A link pasted from chat with surrounding spaces, a newline, or wrapped in `<…>`:** it decodes (Task 2 test `decode_tolerates_chat_wrapping`).
5. **The sharer restarts the app:** the key, shares, links and person bindings survive; a recipient's existing share works again without a new link (Task 5 test `links_survive_a_restart`).

---

### Task 1: Spike — dependencies, binary size, serving by reference

Throwaway findings, kept as one smoke test. Answers the spec's two open questions before anything is built on them.

**Files:**
- Modify: `src-tauri/Cargo.toml`, `src-tauri/Cargo.lock`
- Create: `src-tauri/src/share/mod.rs` (module declaration and the smoke test only)
- Modify: `src-tauri/src/lib.rs` (`mod share;`)
- Modify: `docs/superpowers/specs/2026-10-25-iroh-sharing-design.md` (answer "Open questions", adjust D4 if needed)

**Interfaces:**
- Produces: dependencies `iroh = "1"`, `iroh-blobs = { version = "0.103", default-features = false, features = ["fs-store"] }`, `postcard = { version = "1", features = ["use-std"] }`, `data-encoding = "2"`; dev-dependency `tokio = { version = "1", features = ["macros", "rt-multi-thread", "time"] }`.
- Produces: `share::test_endpoint() -> iroh::Endpoint` (`#[cfg(test)]`, `pub(crate)`): binds on loopback with relays and discovery off. Every later network test uses it.

- [ ] **Step 1: Record the baseline size.** Run `npm run build && cd src-tauri && cargo build --release && stat -c %s target/release/filebrowser`. Expected: a byte count (about 12 MB today). Note it in the ledger.

- [ ] **Step 2: Write the smoke test** `share::tests::a_file_round_trips_between_two_loopback_endpoints`: endpoint A adds a 3 MB file to an `FsStore` **by reference** (an import mode that does not copy, if `iroh-blobs` has one), serves it with `BlobsProtocol`; endpoint B downloads by `(hash, A's EndpointAddr)` and exports; the bytes match. Then assert that A's store directory holds less than 1 MB (so the file was referenced, not copied). Wrap in `tokio::time::timeout(Duration::from_secs(20), …)`.

- [ ] **Step 3: Run it.** `cd src-tauri && cargo test share::tests -- --nocapture`. Expected: compile errors until the deps are added; then PASS. If the size assertion fails because `FsStore` always copies, **stop and report to the user** (the spec's D4 open question): the alternative is a custom `iroh-blobs` store, which is a design change.

- [ ] **Step 4: Measure.** Same command as Step 1. Note the new size and the growth in the ledger and in the spec's "Open questions" (answered, with numbers). If growth is above 15 MB, ask the user whether to put sharing behind a Cargo feature before Task 2.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/Cargo.lock src-tauri/src/share/mod.rs src-tauri/src/lib.rs docs/superpowers/specs/2026-10-25-iroh-sharing-design.md
git commit -m "chore: add iroh and iroh-blobs for folder sharing"
```

---

### Task 2: Links and identity

**Files:**
- Create: `src-tauri/src/share/link.rs`, `src-tauri/src/share/identity.rs`
- Modify: `src-tauri/src/share/mod.rs`

**Interfaces:**
- Produces (`link.rs`):
  - `pub type ShareId = [u8; 16]; pub type Token = [u8; 32];`
  - `pub enum LinkKind { Anyone, Person }` (serde)
  - `pub struct ShareLink { pub addr: iroh::EndpointAddr, pub share: ShareId, pub token: Token, pub kind: LinkKind, pub sharer: String }`
  - `pub const LINK_PREFIX: &str = "share://";`
  - `impl ShareLink { pub fn encode(&self) -> String; pub fn decode(text: &str) -> Result<ShareLink, String> }` — postcard struct with a leading `version: u8 = 1`.
  - `pub fn random_id() -> ShareId; pub fn random_token() -> Token` (via `getrandom`, already a dependency).
- Produces (`identity.rs`):
  - `pub struct Identity { pub key: iroh::SecretKey, _lock: std::fs::File }`
  - `pub fn load_or_create(dir: &Path) -> Result<Identity, String>` — creates `share-key` with mode 0600 if missing; takes an exclusive non-blocking lock; a held lock → `Err("Sharing is active in another Files window.")`.
  - `pub fn config_dir_override() -> Option<PathBuf>` — `FILEBROWSER_CONFIG_DIR` in debug builds (`cfg!(debug_assertions)`), always `None` in release (D15).

- [ ] **Step 1: Write failing tests**

```rust
// link.rs
#[test] fn a_link_round_trips()                 // encode → decode equals, starts with "share://"
#[test] fn decode_tolerates_chat_wrapping()     // "  <share://…>\n" decodes to the same link
#[test] fn decode_rejects_garbage()             // "hello", "share://!!!", "" → Err("This isn't a share link.")
#[test] fn decode_rejects_a_truncated_link()    // drop the last 10 chars → Err("This isn't a share link.")
#[test] fn decode_rejects_another_version()     // version byte 2 → Err("This link needs a newer version of Files.")
// identity.rs
#[test] fn the_key_is_created_once_and_reused() // two loads in sequence (lock dropped between) → same public key
#[test] fn the_key_file_is_private()            // mode & 0o777 == 0o600
#[test] fn a_second_instance_is_refused()       // load while another Identity lives → that exact message
```

- [ ] **Step 2: Run** `cd src-tauri && cargo test share::link share::identity`. Expected: FAIL (items not defined).

- [ ] **Step 3: Implement** `link.rs` and `identity.rs` as in Interfaces. `decode` trims whitespace and one surrounding `<…>` pair before checking the prefix. The lock uses `std::fs::File::try_lock` (stable since Rust 1.89).

- [ ] **Step 4: Run** the Step 2 command. Expected: PASS.

- [ ] **Step 5: Commit** — `feat: add share links and the sharing identity`

---

### Task 3: Manifests and the tree builder

**Files:**
- Create: `src-tauri/src/share/manifest.rs`
- Modify: `src-tauri/src/share/mod.rs`

**Interfaces:**
- Produces:
  - `pub enum Kind { File, Folder }`
  - `pub struct ManifestEntry { pub name: String, pub kind: Kind, pub size: u64, pub mtime: i64, pub folder: Option<iroh_blobs::Hash> }` — `folder` is `Some(child manifest hash)` exactly for folders.
  - `pub struct Manifest { pub entries: Vec<ManifestEntry> }` with `encode(&self) -> Vec<u8>` (leading version byte 1, entries sorted by name bytes), `decode(bytes: &[u8]) -> Result<Manifest, String>`, `hash(&self) -> Hash` (`Hash::new(encode())`, BLAKE3).
  - `pub const MAX_MANIFEST: usize = 64 * 1024 * 1024;`
  - `pub struct Tree { pub root: Hash, /* private maps */ }` with:
    - `pub fn build(folder: &Path) -> Result<Tree, String>` — full walk.
    - `pub fn rebuild(&mut self, folder: &Path, changed: &Path) -> Result<(), String>` — rebuilds `changed`'s folder and its ancestors only; `changed` is any path under `folder`.
    - `pub fn manifest(&self, hash: &Hash) -> Option<&[u8]>` — encoded bytes of a published manifest.
    - `pub fn entry_path(&self, manifest: &Hash, name: &str) -> Option<(PathBuf, &ManifestEntry)>` — the file's absolute path, from the builder's map.
    - `pub fn manifests(&self) -> impl Iterator<Item = Hash> + '_`.
  - `pub struct ContentCache` with `pub fn hash(&mut self, path: &Path, versions_hint: Option<Hash>) -> Result<Option<(Hash, u64)>, String>` — returns `None` when size or mtime differ before and after reading (the "changed" answer); a hit by (path, size, mtime) does no read; a `versions_hint` is used only when the caller has checked it matches the current size and mtime.

- [ ] **Step 1: Write failing tests**

```rust
#[test] fn the_same_folder_gives_the_same_root()            // build twice → equal roots; create files in a different order → equal
#[test] fn a_rename_changes_that_folder_and_its_parents_only() // a/b/x → a/b/y: root and a, a/b manifests change; sibling a/c's hash unchanged
#[test] fn a_size_or_mtime_change_changes_the_root()
#[test] fn rebuild_matches_a_fresh_build()                  // after rebuild(changed) the root equals Tree::build's
#[test] fn excluded_items_never_appear()                    // .versions/, symlink to /etc, FIFO, chmod 000 file → absent; symlink to a sibling inside → present as the target's kind
#[test] fn non_utf8_names_are_left_out()                    // Linux only via #[cfg(unix)] OsStr::from_bytes(b"bad\xff")
#[test] fn a_ten_thousand_entry_folder_fits_one_manifest()  // 10_000 empty files → one manifest, encoded len < MAX_MANIFEST
#[test] fn an_oversized_manifest_is_refused()               // a Manifest whose encoding exceeds the cap → build Err("“name” has too many items to share.")
#[test] fn entry_path_resolves_only_names_in_that_manifest() // unknown name → None; "../x" → None; folder entry → Some with kind Folder
#[test] fn the_content_cache_skips_unchanged_files()        // second hash() of same file does no read (count via a test hook or by deleting read permission after first hash)
#[test] fn a_file_that_changes_while_hashing_is_not_hashed() // hook between reads changes the file → Ok(None)
#[test] fn keep_versions_and_iroh_hash_alike()              // blake3::hash(b"…").as_bytes() == iroh_blobs::Hash::new(b"…").as_bytes()
```

- [ ] **Step 2: Run** `cd src-tauri && cargo test share::manifest`. Expected: FAIL.

- [ ] **Step 3: Implement** `manifest.rs`. Walk with `std::fs::read_dir` + `symlink_metadata`; resolve symlinks with `canonicalize` and keep them only if the result starts with the canonical shared folder. Reuse `crate::versions::VERSIONS_DIR` for the exclusion. Use `postcard` for encoding.

- [ ] **Step 4: Run** the Step 2 command. Expected: PASS.

- [ ] **Step 5: Commit** — `feat: build structure manifests for shared folders`

---

### Task 4: The share registry and claim rules

**Files:**
- Create: `src-tauri/src/share/registry.rs`
- Modify: `src-tauri/src/share/mod.rs`

**Interfaces:**
- Consumes: `ShareId`, `Token`, `LinkKind`, `random_id`, `random_token` (Task 2).
- Produces:
  - `pub struct AccessEntry { pub id: String, pub kind: LinkKind, pub name: String, pub token: Token, pub bound: Option<iroh::PublicKey>, pub visitors: Vec<iroh::PublicKey> }` — `bound` is used by person links, `visitors` by anyone links.
  - `pub struct Share { pub id: ShareId, pub path: PathBuf, pub access: Vec<AccessEntry> }`
  - `pub struct Registry { pub name: Option<String>, pub shares: Vec<Share> }` — `shares.json`, written with `config_store::write_json` then chmod 0600.
  - `pub fn load(path: &Path) -> Registry; pub fn save(&self, path: &Path) -> Result<(), String>`
  - `pub fn share_folder(&mut self, path: &Path, is_trash: impl Fn(&Path) -> bool) -> Result<ShareId, String>` — idempotent per path.
  - `pub fn create_link(&mut self, share: &ShareId, kind: LinkKind, person: Option<&str>) -> Result<AccessEntry, String>` — at most one Anyone entry per share (returns the existing one).
  - `pub enum Claimed { Ok, Denied(&'static str) }`
  - `pub fn claim(&mut self, share: &ShareId, token: &Token, visitor: iroh::PublicKey, persist: impl FnOnce(&Registry) -> Result<(), String>) -> Claimed` — calls `persist` before returning `Ok` for any change.
  - `pub fn allowed(&self, share: &ShareId, visitor: &iroh::PublicKey) -> bool`
  - `pub fn remove_access(&mut self, share: &ShareId, entry: &str) -> Vec<iroh::PublicKey>` — returns keys that lost access (for dropping connections).
  - `pub fn stop_sharing(&mut self, share: &ShareId) -> Vec<iroh::PublicKey>`
  - `pub fn follow_move(&mut self, from: &Path, to: &Path)` — rewrites share paths at or under `from`.
  - `pub struct Received { pub id: String, pub link: ShareLink, pub folder: String, pub sharer: String }` and `pub struct ReceivedList { pub items: Vec<Received> }` for `received.json` with `load`, `save`, `add(link, folder, sharer, own_key: &PublicKey) -> Result<Received, String>`, `remove(id)`.

- [ ] **Step 1: Write failing tests**

```rust
#[test] fn an_unused_person_link_binds_to_the_first_visitor()
#[test] fn the_same_visitor_can_claim_again()
#[test] fn a_used_person_link_refuses_another_computer()     // Denied("This link was already used on another computer.")
#[test] fn a_removed_link_is_no_longer_valid()               // Denied("This link is no longer valid.")
#[test] fn a_wrong_token_is_no_longer_valid()
#[test] fn the_binding_is_saved_before_the_reply()           // persist closure records registry state; it already holds `bound`; a failing persist → Denied("Sharing settings couldn't be saved.") and no binding in memory
#[test] fn anyone_links_record_each_visitor()
#[test] fn removing_the_anyone_link_cuts_off_its_visitors()  // returned keys == visitors; allowed() false for each
#[test] fn removing_one_person_keeps_the_others()
#[test] fn only_local_folders_can_be_shared()                // a file, the Trash (is_trash true), …/.versions/x, "share://a/b", "gdrive://a/b" → Err("Only folders on this computer can be shared.")
#[test] fn sharing_a_folder_twice_gives_the_same_share()
#[test] fn a_moved_folder_keeps_its_share()                  // follow_move on the folder and on a parent
#[test] fn the_registry_file_is_private()                    // 0600
#[test] fn the_same_link_twice_is_refused()                  // ReceivedList::add → Err("This folder is already in Network.")
#[test] fn your_own_link_is_refused()                        // link.addr.id == own_key → Err("This is your own shared folder.")
```

- [ ] **Step 2: Run** `cd src-tauri && cargo test share::registry`. Expected: FAIL.

- [ ] **Step 3: Implement** `registry.rs`. Serialize keys and tokens as lowercase hex strings in JSON.

- [ ] **Step 4: Run** the Step 2 command. Expected: PASS.

- [ ] **Step 5: Commit** — `feat: keep shares, links and claims in shares.json`

---

### Task 5: The host — control protocol, gate, serving

**Files:**
- Create: `src-tauri/src/share/host.rs`, `src-tauri/src/share/proto.rs`
- Modify: `src-tauri/src/share/mod.rs`

**Interfaces:**
- Consumes: `Identity` (Task 2), `Tree`, `ContentCache` (Task 3), `Registry` (Task 4), `share::test_endpoint()` (Task 1).
- Produces (`proto.rs`):
  - `pub const ALPN: &[u8] = b"filebrowser/share/1"; pub const MAX_MESSAGE: usize = 64 * 1024;`
  - `pub enum Request { Claim { share: ShareId, token: Token }, Root { share: ShareId }, Content { share: ShareId, root: Hash, names: Vec<String> }, Watch { share: ShareId } }`
  - `pub enum Reply { Ok { folder: String, sharer: String }, Root { root: Hash, folder: String, sharer: String }, Content { hash: Hash, size: u64 }, Changed, Missing, Denied(String), Busy }` — `Missing` answers `Root` and `Content` while the shared folder is gone (Task 10)
  - `pub async fn write_msg<T: Serialize>(send: &mut SendStream, msg: &T) -> Result<(), String>` and `read_msg<T: DeserializeOwned>(recv: &mut RecvStream) -> Result<T, String>` — u32 length prefix, refuse > `MAX_MESSAGE`.
- Produces (`host.rs`):
  - `pub struct Host` with `pub async fn start(identity: Identity, registry_path: PathBuf, store_dir: PathBuf, endpoint: iroh::Endpoint) -> Result<Host, String>` (tests pass `test_endpoint()`; the app passes the default-preset endpoint).
  - `pub fn addr(&self) -> iroh::EndpointAddr`
  - `pub async fn share_folder(&self, path: &Path) -> Result<ShareId, String>` — registers and builds the `Tree` on a blocking thread.
  - `pub async fn create_link(&self, share: &ShareId, kind: LinkKind, person: Option<String>) -> Result<(String, AccessEntry), String>` — returns the encoded link.
  - `pub async fn remove_access(&self, share: &ShareId, entry: &str)`; `pub async fn stop_sharing(&self, share: &ShareId)` — both close the affected visitors' connections.
  - `pub fn online(&self) -> Vec<iroh::PublicKey>` — keys with an open connection.
  - `pub async fn shutdown(self)`.
- Gate: an `iroh_blobs::provider::events::EventSender` that rejects `ClientConnected` unless the key is allowed on some share, and rejects get requests unless the hash is published for a share that key is allowed on (`AbortReason::Permission`). Published = `Tree::manifests()` plus hashes returned by `Content`. Connections beyond `MAX_VISITORS` get `Reply::Busy` on the control protocol and `AbortReason::RateLimited` on blobs.
- Serving: manifests are added to the store as bytes; a `Content` hit imports the file by reference (Task 1's mode) after `ContentCache::hash` confirms it; before a get is served, size and mtime are checked against the manifest entry and a mismatch aborts the request.

- [ ] **Step 1: Write failing tests** (all under `tokio::time::timeout(Duration::from_secs(20), …)`, two `test_endpoint()`s; a small test client in `#[cfg(test)]` that sends one `Request` and reads one `Reply`)

```rust
#[tokio::test] async fn claim_then_root_returns_the_tree_root()     // Reply::Root.root == Tree::build(folder).root
#[tokio::test] async fn root_without_a_claim_is_denied()
#[tokio::test] async fn content_hashes_a_file_and_it_downloads()    // Content → hash; blobs download of hash → bytes match a 3 MB file
#[tokio::test] async fn content_refuses_names_outside_the_manifest() // unknown name, "../x", a folder entry, a root that is neither current nor previous → Denied
#[tokio::test] async fn content_answers_changed_after_a_modification() // modify file after the Root → Reply::Changed
#[tokio::test] async fn an_unpublished_hash_is_not_served()          // blobs get of blake3(other file's bytes) → error
#[tokio::test] async fn share_a_hashes_are_denied_to_a_share_b_visitor()
#[tokio::test] async fn removing_access_drops_the_connection()       // open control connection closes; next Root → Denied
#[tokio::test] async fn the_thirty_third_visitor_is_busy()           // MAX_VISITORS lowered via a #[cfg(test)] constructor arg to 1; second visitor → Reply::Busy
#[tokio::test] async fn links_survive_a_restart()                    // shutdown, Host::start again on the same dirs, same addr id; the bound visitor's Root → Ok
```

- [ ] **Step 2: Run** `cd src-tauri && cargo test share::host`. Expected: FAIL.

- [ ] **Step 3: Implement** `proto.rs` and `host.rs`. The control handler is a `iroh::protocol::ProtocolHandler` registered next to `BlobsProtocol` on one `Router`. Keep the request handlers as one small function per `Request` variant (CRAP gate).

- [ ] **Step 4: Run** the Step 2 command. Expected: PASS.

- [ ] **Step 5: Commit** — `feat: serve shared folders over iroh with an access gate`

---

### Task 6: The guest — connect, browse, download

**Files:**
- Create: `src-tauri/src/share/guest.rs`, `src-tauri/src/share/location.rs`
- Modify: `src-tauri/src/share/mod.rs`

**Interfaces:**
- Consumes: `ShareLink`, `ReceivedList`, `Received` (Tasks 2, 4), `proto` (Task 5).
- Produces (`location.rs`):
  - `pub const PREFIX: &str = "share://";`
  - `pub fn is_share(path: &str) -> bool`
  - `pub struct ShareLocation { pub received: String, pub names: Vec<String> }` with `parse(path: &str) -> Option<ShareLocation>`, `to_path(&self) -> String`, `child(&self, name: &str) -> ShareLocation`. Names percent-encoded with the same set as `cloud::location`'s `NAME_SET` (reuse it).
  - `pub fn safe_name(name: &str) -> Option<&str>` — `None` for empty, `.`, `..`, or containing `/` or NUL.
- Produces (`guest.rs`):
  - `pub enum Status { Online, Offline, NoAccess, Missing }` (serde lowercase, `NoAccess` as `"no-access"`).
  - `pub struct Guest` with `pub fn new(endpoint: iroh::Endpoint, cache_dir: PathBuf) -> Guest` (empties `cache_dir/share-store` first).
  - `pub async fn claim(&self, link: &ShareLink) -> Result<(String, String), String>` — `(folder, sharer)`.
  - `pub async fn status(&self, r: &Received) -> Status` — `Root` with `CONNECT_TIMEOUT`; timeout or unreachable → `Offline`.
  - `pub async fn list(&self, r: &Received, loc: &ShareLocation, show_hidden: bool) -> Result<Vec<crate::fs_ops::Entry>, String>` — walk manifests from the current root by name; drop entries failing `safe_name`; `is_hidden` = leading `.`; `path` = `loc.child(name).to_path()`.
  - `pub async fn fetch(&self, r: &Received, loc: &ShareLocation, dest: &Path) -> Result<(), String>` — `Content`, blobs download, export to `dest` (the caller supplies a staging path).
  - Error strings: offline → "<sharer> is offline."; `Changed` → "The file changed while downloading. Try again."; version mismatch (ALPN refused) → "<sharer>'s Files app uses a different sharing version. Both of you need the same version."; `Busy` → "<sharer>'s computer is busy. Try again in a moment."; denied after a claim → "<sharer> stopped sharing this folder with you."
  - Manifests are cached in memory by hash (`HashMap<Hash, Manifest>`).

- [ ] **Step 1: Write failing tests** (`location.rs` unit tests plus integration tests against a `Host` from Task 5, same timeout rule)

```rust
#[test] fn a_share_path_round_trips()                      // "share://r1/Photos/2024%20Trip" parse → to_path
#[test] fn unsafe_names_are_rejected()                     // "", ".", "..", "a/b", "a\0b" → None
#[tokio::test] async fn listing_shows_the_shared_folder()  // names, sizes, is_dir match the sharer's folder; subfolder listing works
#[tokio::test] async fn hidden_entries_follow_show_hidden()
#[tokio::test] async fn a_malicious_manifest_name_is_dropped() // a host test hook publishes a manifest with "../evil" → absent from list
#[tokio::test] async fn fetch_writes_a_verified_copy()     // bytes equal; dest only exists after success
#[tokio::test] async fn a_stopped_host_is_offline_within_the_timeout() // CONNECT_TIMEOUT lowered via #[cfg(test)] to 1 s; status → Offline in < 3 s
#[tokio::test] async fn a_revoked_guest_sees_no_access()   // remove_access → status NoAccess; list Err with the "stopped sharing" message
#[tokio::test] async fn a_different_protocol_version_is_explained() // host built with a #[cfg(test)] ALPN b"filebrowser/share/2" → the "different sharing version" message
```

- [ ] **Step 2: Run** `cd src-tauri && cargo test share::guest share::location`. Expected: FAIL.

- [ ] **Step 3: Implement** `location.rs` and `guest.rs`.

- [ ] **Step 4: Run** the Step 2 command. Expected: PASS.

- [ ] **Step 5: Commit** — `feat: browse and download shared folders as a guest`

---

### Task 7: Tauri commands, state, mocks (M1 backend surface)

**Files:**
- Create: `src-tauri/src/share/state.rs`
- Modify: `src-tauri/src/lib.rs` (commands, `list_dir` dispatch, `open_entry`-style `share_open`, `.manage`, config-dir override)
- Modify: `src/api.ts`, `src/paths.ts`
- Modify: `dev/mock-tauri/core.ts`, `tests/mock-core.test.ts`, `tests/paths.test.ts`

**Interfaces:**
- Consumes: `Host`, `Guest`, `Registry`, `ReceivedList`, `ShareLocation` (Tasks 4–6).
- Produces (`state.rs`): `pub struct ShareState` holding `config_dir`, `cache_dir` and a lazily started `tokio::sync::OnceCell<(Host, Guest)>` on one endpoint (D1: started on first need — any share or received share at startup, or the first `share_folder` / `add_received`). Plain async methods mirroring the commands below; `lib.rs` only forwards.
- Produces (commands, exact names and shapes from the spec's Command surface): `share_identity`, `set_share_name`, `share_folder`, `share_info`, `list_shares`, `create_link`, `share_link`, `remove_access`, `stop_sharing`, `add_received`, `list_received`, `remove_received`, plus `share_open(path) -> String` (local file path in the app cache to open).
- Produces (TS, `src/paths.ts`): `SHARE_PREFIX = "share://"`, `isSharePath(path)`, `isRemotePath(path) = isCloudPath(path) || isSharePath(path)`, `joinSharePath(dir, name)` (same encoding as `joinCloudPath`).
- Produces (TS, `src/api.ts`): typed wrappers for every command above; types `ShareInfo`, `AccessEntry`, `ShareSummary`, `ReceivedShare` exactly as in the spec; `openFile` dispatches `share://` to `share_open` then `openPath`.
- `lib.rs`: `list_dir` gains `if share::location::is_share(&path) { … }` before the Drive branch; `config_file` and the setup's `config_dir` use `share::identity::config_dir_override()` when set.

- [ ] **Step 1: Write failing tests**

```ts
// tests/paths.test.ts
test("share paths are remote but not Drive", …)  // isSharePath, isRemotePath true, isCloudPath false
test("joinSharePath encodes names like Drive", …) // "share://r1" + "a b" → "share://r1/a%20b"
// tests/mock-core.test.ts — one case per command
test("share_folder then create_link gives a link and an access entry", …)
test("a person link shows as unused until claimed", …)
test("add_received rejects an invalid link with the backend's message", …) // "This isn't a share link."
test("add_received refuses the same link twice", …)                       // "This folder is already in Network."
test("list_dir lists a received share", …)
test("an offline share reports offline", …)                               // mock helper takes the fake sharer offline
test("remove_access and stop_sharing update share_info", …)
test("share_open returns a local path", …)
```

Rust: `state.rs` test `the_node_starts_only_when_needed` — a fresh `ShareState` with no shares and no received shares has not created `share-key` after `list_shares()` and `list_received()`.

- [ ] **Step 2: Run** `npm test -- tests/paths.test.ts tests/mock-core.test.ts` and `cd src-tauri && cargo test share::state`. Expected: FAIL.

- [ ] **Step 3: Implement** `state.rs`, the commands in `lib.rs` (all `async fn`; manifest builds and file exports through `blocking(…)`), the TS wrappers, and the mock: an in-memory sharer tree with `__mockShare.setOnline(bool)`, `__mockShare.change(path)` and the same message strings as the backend.

- [ ] **Step 4: Run** the Step 2 commands. Expected: PASS.

- [ ] **Step 5: Commit** — `feat: expose folder sharing to the frontend`

---

### Task 8: Share dialog, Network view, read-only browsing (M1 UI)

**Files:**
- Modify: `src/main.ts`, `src/styles.css`, `src/icons.ts` (network and share emblems drawn in the Adwaita style, from scratch)
- Create: `tests/share.test.ts`

**Interfaces:**
- Consumes: Task 7's API and `isSharePath` / `isRemotePath`.
- Produces: `PSEUDO.network` becomes a real view (no longer "Network locations aren't supported yet."); `isReadOnlyLocation(path): boolean` (true for `share://`) used by every menu and drop guard.

- [ ] **Step 1: Write failing UI tests** in `tests/share.test.ts`

```ts
test("Share… asks for your name the first time", …)         // dialog text "People you share with will see this name."; pre-filled
test("Share… is offered only for local folders", …)         // absent on a file, in Trash, in Drive, in a share
test("Copy Link copies an anyone link", …)                  // clipboard starts with "share://"; toast "Link copied"
test("a person link appears as hasn't opened yet", …)       // "Hasn't opened the link yet"
test("Stop Sharing confirms with Cancel as the default", …) // Enter → still shared; exact dialog text from the spec
test("Network is empty with a paste bar", …)                // "No Network Locations", "Paste a share link below to add a shared folder."
test("pasting a bad link shows the reason inline", …)       // "This isn't a share link."
test("pasting your own link opens the local folder", …)     // "This is your own shared folder." then the location is the local path
test("a received share shows who shared it", …)             // "Shared by Ahmed"
test("an offline share opens to the offline page", …)       // "Ahmed Is Offline" and the spec's sentence
test("inside a share, menus are read only", …)              // no Cut, Rename, Move to Trash, New Folder, Paste, Star; Copy present
test("dropping onto a share is refused", …)
test("losing access shows the No Access page", …)           // "Ahmed stopped sharing this folder with you."
test("Remove from Network forgets the share", …)
```

- [ ] **Step 2: Run** `npm test -- tests/share.test.ts`. Expected: FAIL.

- [ ] **Step 3: Implement** the Share dialog (with `openDialog`; "Preparing…" until `share_folder` resolves), the name prompt, the Network view (grid and list, Shared by and Status columns, dimmed offline items, bottom paste bar), the offline / no-access / not-available status pages, path bar and subtitle for `share://`, and the read-only guards. Escape every name and path with `escapeHtml`.

- [ ] **Step 4: Run** `npm test`. Expected: PASS. Then look at it in `npm run dev:mock` (Network empty, received share, offline page, Share dialog).

- [ ] **Step 5: Commit** — `feat: share folders and browse shares under Network`

---

### Task 9: Open and copy out (M2)

**Files:**
- Modify: `src-tauri/src/share/guest.rs`, `src-tauri/src/share/state.rs`, `src-tauri/src/lib.rs` (`copy_entry` dispatch for `share://` → local; `move_entry` from a share refused)
- Modify: `src/main.ts` (`runTransfer` and `dropOp` treat shares as copy-only sources), `src/api.ts` (progress event, `share_cancel`)
- Modify: `dev/mock-tauri/core.ts`, `tests/mock-core.test.ts`, `tests/share.test.ts`
- Modify: `docs/superpowers/specs/2026-10-25-iroh-sharing-design.md` (Command surface: add `share_cancel` and `share-progress`)

**Interfaces:**
- Consumes: `Guest::fetch`, `ShareLocation`, `fs_ops::staging_path`, `fs_ops::replace_with_staged`, `fs_ops::remove_any` (crate-visible since the Drive work).
- Produces:
  - `pub async fn copy_out(&self, r: &Received, from: &ShareLocation, to: &Path, overwrite: bool, job: &Job) -> Result<String, String>` in `guest.rs` — file or folder; follows the root read at the start; each file staged then renamed; returns the final path.
  - `pub struct Job { pub id: String, cancel: AtomicBool, progress: Box<dyn Fn(u64, u64) + Send + Sync> }`.
  - Event `share-progress { job, done, total }`; command `share_cancel(job)`.
  - Errors: the sharer going offline mid-folder → "<sharer> went offline. N of M items were copied."; changed files → copy continues, then "These items changed while copying: …".
  - `move_entry` with a `share://` source → "Shared folders are read only."

- [ ] **Step 1: Write failing tests**

```rust
#[tokio::test] async fn a_folder_copies_out_completely()            // tree equal
#[tokio::test] async fn copy_out_never_overwrites()                 // existing target, overwrite=false → Err; target bytes unchanged
#[tokio::test] async fn replace_stages_then_trashes_the_old_file()  // overwrite=true → replace_with_staged: old file in the Trash, new bytes in place, no staging file remains
#[tokio::test] async fn a_host_that_stops_mid_copy_leaves_whole_files() // stop after first file: copied files complete, no staging files, message "N of M"
#[tokio::test] async fn a_file_changed_mid_copy_is_skipped_and_named()
#[tokio::test] async fn cancel_removes_the_staging_file()
#[test] fn moving_out_of_a_share_is_refused()                       // lib-level dispatch helper returns "Shared folders are read only."
```

```ts
test("opening a file in a share shows download progress", …)  // toast "Downloading “video.mp4”… 45%" with Cancel
test("dragging from a share to a folder copies", …)            // drop op is copy, never move
test("a clash when copying out asks Replace / Skip / Keep Both", …)
```

- [ ] **Step 2: Run** `cd src-tauri && cargo test share::guest` and `npm test -- tests/share.test.ts`. Expected: FAIL.

- [ ] **Step 3: Implement** `copy_out`, `Job`, the `copy_entry` / `move_entry` branches, `share_open` progress, `share_cancel`, the frontend progress toast and copy-only drops; add the two items to the spec's Command surface.

- [ ] **Step 4: Run** the full gates: `npx tsc --noEmit && npm run build && npm test && (cd src-tauri && cargo test)`. Expected: all PASS.

- [ ] **Step 5: Commit** — `feat: open and copy files out of shared folders`

---

### Task 10: Live updates and connection lifecycle (M3 backend)

**Files:**
- Modify: `src-tauri/src/share/host.rs` (watcher, `Watch`), `src-tauri/src/share/guest.rs` (watch loop with backoff), `src-tauri/src/share/state.rs`, `src-tauri/src/lib.rs` (emit events)
- Modify: `dev/mock-tauri/core.ts`, `tests/mock-core.test.ts`

**Interfaces:**
- Consumes: `Tree::rebuild` (Task 3), `notify` (as `versions/watch.rs` uses it).
- Produces:
  - Host: one `notify::RecommendedWatcher` per share, recursive; events debounced 500 ms; each batch → `Tree::rebuild` for each changed path on a blocking thread → if the root changed, push it to every `Watch` stream of that share. A vanished shared folder sets the share's status to `missing`; it rebuilds and returns to `ok` when the folder reappears at the same path.
  - Guest: `pub async fn watch(&self, r: &Received, on_root: impl Fn(Hash) + Send + 'static)` — reconnects with backoff 1 s, 2 s, 4 s … capped at 30 s; reports status changes.
  - Events (from the spec): `share-root-changed {received, root}`, `share-status-changed {received, status}`, `share-access-changed {share}`.
  - `follow_move` wired into `lib.rs`'s `rename_entry` / `move_entry` local branches (the same place stars follow moves).

- [ ] **Step 1: Write failing tests**

```rust
#[tokio::test] async fn adding_a_file_pushes_a_new_root()      // within 5 s
#[tokio::test] async fn renaming_a_file_pushes_a_new_root()
#[tokio::test] async fn the_watch_reconnects_after_the_host_restarts() // status Offline then Online
#[tokio::test] async fn a_deleted_shared_folder_is_missing_then_returns() // guest status Missing ("Not Available — the shared folder was moved or deleted."), then Online
#[test] fn renaming_a_shared_folder_in_the_app_keeps_the_share()   // registry follow_move through the lib helper
```

- [ ] **Step 2: Run** `cd src-tauri && cargo test share::host share::guest`. Expected: FAIL.

- [ ] **Step 3: Implement** the watcher, `Watch`, the guest loop and the events.

- [ ] **Step 4: Run** the Step 2 command. Expected: PASS.

- [ ] **Step 5: Commit** — `feat: push live changes from shared folders`

---

### Task 11: Sharer lifecycle UI, live refresh, docs (M3 frontend)

**Files:**
- Modify: `src/main.ts`, `src/styles.css`
- Modify: `tests/share.test.ts`
- Create: `docs/sharing.md` (user guide: sharing, link kinds, what the other person sees, offline, back up `share-key`, IP visibility)
- Modify: `README.md` (features, "Not supported yet": write access, offline access, clickable links, single files), `AGENTS.md` (layout row for `src-tauri/src/share/`), the spec's Status line

**Interfaces:**
- Consumes: Task 10's events; Task 7's `list_shares`.
- Produces: share emblem in grid and list views; **Shared** sidebar row next to Starred (a pseudo place like Starred) with "Folder not found" for missing shares; live refresh of an open `share://` view that keeps selection and scroll position by name; Share dialog rows update on `share-access-changed` (Online / Offline per person from `Host::online`).

- [ ] **Step 1: Write failing UI tests**

```ts
test("shared folders carry a share emblem", …)
test("the Shared row lists shared folders", …)
test("a missing shared folder says Folder not found", …)
test("a change on the sharer refreshes the open share and keeps the selection", …) // __mockShare.change(...)
test("the Share dialog shows a person as online", …)
```

- [ ] **Step 2: Run** `npm test -- tests/share.test.ts`. Expected: FAIL.

- [ ] **Step 3: Implement** the emblem, Shared row, live refresh, dialog updates; write `docs/sharing.md`, update README, AGENTS.md and the spec's Status (M1–M3 done; Mac-only checks pending).

- [ ] **Step 4: Run** the full gates: `npx tsc --noEmit && npm run build && npm test && (cd src-tauri && cargo test)`. Expected: all PASS. Then a look in `npm run dev:mock`, and the manual two-instance check: `FILEBROWSER_CONFIG_DIR=/tmp/fb2 npm run tauri dev` beside a normal instance — share, paste, browse, open, copy out, rename on the sharer, remove access, quit the sharer. Record what was checked; list the Mac-only items (firewall prompt, FSEvents watching, sleep / wake, real relays) as unverified.

- [ ] **Step 5: Commit** — `feat: show shared folders and refresh shares live`
