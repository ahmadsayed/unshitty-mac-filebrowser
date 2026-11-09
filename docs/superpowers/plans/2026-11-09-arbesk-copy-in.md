# Copying Files into Arbesk Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Copy/Paste and drops put local `.gltf`, `.glb` and `.3mf` files into an Arbesk collection as assets the Studio reads.

**Architecture:** The backend keeps the sign-in's Arbesk session in the keychain, splits a model file into IPFS parts (`arbesk/pack.rs`), uploads them through Pinata credentials Arbesk mints, writes the asset and collection manifests and asks Arbesk's relay to update the collection on chain (`arbesk/write.rs`). `copy_entry` routes a local source with an Arbesk target there (`place::copy`). The frontend allows the paste, confirms it, maps `.glb` to the listed `.gltf` name and signs in again when needed.

**Tech Stack:** Rust (Tauri 2, existing `Http` seam, `zip`, `base64`, `keyring`), TypeScript (no framework), Vitest + jsdom, the mock backend.

**Spec:** `docs/superpowers/specs/2026-11-09-arbesk-copy-in-design.md` (decisions D1–D11 are cited by number below).

## Global Constraints

- No change to Arbesk itself; no new crate (multipart is built by hand).
- Accepted: local regular files, extension `.gltf` / `.glb` / `.3mf` (case-insensitive), ≤ 100 MB (`MAX_UPLOAD = 100 * 1024 * 1024`, a `.gltf` counting its parts).
- Wording, exactly:
  - "Only .gltf, .glb and .3mf files on this computer can be copied to Arbesk."
  - "Paste into a collection."
  - "Arbesk is read only." (Cut, forced move)
  - Dialog: heading "Copy to Arbesk?", body "Files in Arbesk are public: anyone with their address can download them, and they can't be removed from IPFS.", responses Cancel (default) / Copy.
  - "You can't add to this collection." (403) · "Arbesk is busy. Try again in a minute." (429) · "Arbesk couldn't save the change. Sign in again to renew its permission." (relay 502) · "Sign in to Arbesk again to copy files into it." (no / expired session, 401) · "Arbesk is still saving “{name}”. It appears once the change is confirmed." (status null) · "Arbesk couldn't save “{name}”." (status false) · "This Arbesk server's storage isn't supported." (kubo-api) · "This isn't a 3MF file." · "“{uri}” is outside the model's folder." · "“{uri}” can't be found." · "This file is too large for Arbesk (100 MB at most)."
  - Status line during the relay: "Saving to Arbesk…".
- Relay HTTP timeout 4 minutes; a Pinata credential older than 45 s is replaced before use.
- Keychain entry: service `filebrowser`, user `arbesk:{account id}`; fallback file `{config_dir}/tokens/arbesk-{account id}.json` (0600).
- Asset manifest shape (D7.4) and collection update (D7.5) exactly as the spec writes them; `proof: []`, `assetScope` = `"0x"` + 64 zeros.
- Every commit passes the pre-commit CRAP gate; never `--no-verify`.

## Review Focus

1. A `.gltf` whose `uri` is percent-encoded, has `./`, or points at `../x.bin`: decoded and confined to the model's folder, never read outside it. → Task 3 test `gltf_parts_stay_in_its_folder`.
2. Two pastes into one collection at once (two windows): both assets end up in the collection. → Task 4 test `two_writes_to_one_collection_keep_both`.
3. A relay that answers 200 with `status: false` or `null`: never reported as success. → Task 4 test `relay_status_is_checked`.
4. A session that expired between the pre-check and the write (401 mid-batch): the paste signs in again and carries on, without duplicating the item that failed. → Task 7 test "an expired session signs in again and carries on".
5. Keep Both twice on `Box.glb`: the second becomes "Box (copy 2).gltf", not a clash with the `.glb` name. → Task 6 test "a .glb pastes as its listed .gltf name".

---

### Task 1: HTTP seam for uploads, and a shared fake Arbesk

**Files:**
- Modify: `src-tauri/src/arbesk/http.rs`
- Create: `src-tauri/src/arbesk/testing.rs` (`#[cfg(test)]`, declared in `arbesk/mod.rs`)
- Modify: `src-tauri/src/arbesk/service.rs` (its tests use `testing::World` and `testing::arbesk` instead of their own)

**Interfaces:**
- Produces, in `http.rs`:
  - `pub fn multipart_body(boundary: &str, file_name: &str, bytes: &[u8], fields: &[(&str, &str)]) -> Vec<u8>` (fields first, then the `file` part, `Content-Type: application/octet-stream`).
  - `Http::post_multipart(&self, url: &str, file_name: &str, bytes: &[u8], fields: &[(&str, &str)], session: Option<&str>) -> Result<Vec<u8>, HttpError>`.
  - `Http::post_json_slow(&self, url: &str, body: &Value, session: Option<&str>) -> Result<Vec<u8>, HttpError>` (4-minute timeout; `ReqwestHttp` gets a second client).
  - `fake::Request` gains `data: Option<Vec<u8>>` (the uploaded file) and `fields: Vec<(String, String)>`.
- Produces, in `testing.rs` (moved from `service.rs` tests, now `pub(crate)`): `World`, `world()`, `arbesk(Arc<Mutex<World>>) -> FakeHttp`, `site()`, `ADDRESS`, `ORIGIN`, `fixture()`, `shapes()`, plus:
  - `World.sessions: HashSet<String>` (tokens Arbesk accepts), `World.relay: Relay` (`Ok` / `Pending` / `Failed` / `Status(u16)`), `World.kubo: bool`, `World.minted: usize`.
  - Endpoints: `POST {ORIGIN}/api/v1/ipfs/upload-urls` (`{count}` → `{credentials:[{strategy:"presigned-put", url:"https://up.test/<n>", gateway, reusable:false}]}`, or `kubo-api` when `kubo`); `POST https://up.test/<n>` multipart (stores `data` under CID `bafyUp<n>`, answers `{"data":{"cid":…}}`; a URL used twice → 409); `POST {ORIGIN}/api/v1/wallet/relay` (checks the session and that `tokenId` is in `owned`, else 401 / 403; on `Ok` sets `tokens[tokenId] = newUri` and answers `{"receipt":{"transactionHash":"0x1","status":true}}`).

- [ ] **Step 1: Write the failing tests** in `http.rs`: `multipart_body_has_fields_then_the_file` (boundary lines, `name="network"` then `name="file"; filename="a.bin"`, the bytes, closing `--boundary--\r\n`). In `testing.rs`: `an_upload_is_served_by_the_gateway_and_a_relay_moves_the_token` (upload through a minted URL, fetch it back from `https://gw.test/ipfs/<cid>`, relay `updateUri` for token 1 → `token_uri` answers the new CID; relay with an unknown session → 401; token 4 → 403; a URL used twice → 409).
- [ ] **Step 2: Run them** — `cd src-tauri && cargo test --lib arbesk::` — Expected: compile errors (`multipart_body`, `post_multipart` missing).
- [ ] **Step 3: Implement** the trait methods (also on `FakeHttp`), `multipart_body`, move the world into `testing.rs` and add the endpoints.
- [ ] **Step 4: Run** `cd src-tauri && cargo test --lib arbesk::` — Expected: PASS, every existing `service.rs` test still green.
- [ ] **Step 5: Commit** `refactor: a shared fake Arbesk with uploads and the relay, and multipart over the Http seam`.

### Task 2: Keep the Arbesk session

**Files:**
- Create: `src-tauri/src/arbesk/session.rs`
- Modify: `src-tauri/src/arbesk/signin.rs`, `src-tauri/src/arbesk/mod.rs`, `src-tauri/src/cloud/token.rs` (only to expose its keyring/file helpers as `pub(crate)` if the store reuses them)

**Interfaces:**
- Produces:
  - `pub struct Session { pub token: String, pub expires_at_ms: i64 }` (serde).
  - `pub trait SessionStore: Send + Sync { fn load(&self, account: &str) -> Option<Session>; fn save(&self, account: &str, session: &Session) -> Result<(), String>; fn delete(&self, account: &str); }`
  - `pub struct KeyringSessions` (service `filebrowser`, user `arbesk:{id}`, fallback file per the constraints) and `#[cfg(test)] pub struct MemorySessions`.
  - `signin::finish(...) -> Result<SignedIn, String>` where `pub struct SignedIn { pub email: String, pub address: String, pub session: Session }`; it no longer ends the session. `expiresAt` missing or unparseable → now + 24 h.
  - `ArbeskState::with(..)` takes `Box<dyn SessionStore>`; `connect_finish` saves the session under the account id; `disconnect` sends `DELETE /api/v1/sessions` with it (errors ignored) and deletes it; `pub fn session_for(&self, path: &str) -> Result<String, String>` (the token, or "Sign in to Arbesk again to copy files into it." when missing or `expires_at_ms <= now + 60_000`); `pub fn signed_in(&self, path: &str) -> bool`; `pub fn forget_session(&self, path: &str)` (after a 401).

- [ ] **Step 1: Write the failing tests:** `signin`: `a_confirmed_sign_in_keeps_its_session` (callback `token=t&expiresAt=<ms>` → `SignedIn.session`, and no DELETE in the log); an unconfirmed one still ends the session it was handed (DELETE logged). `mod.rs`: `connecting_saves_the_session_and_disconnect_ends_it` (MemorySessions has it after `connect_finish`; `disconnect` logs `DELETE …/sessions` with `Session t` and the store is empty); `a_missing_or_expiring_session_asks_for_a_sign_in` (`session_for` errors with the exact wording for none, and for one expiring in 30 s).
- [ ] **Step 2: Run** `cargo test --lib arbesk::` — Expected: FAIL (types missing).
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** — Expected: PASS.
- [ ] **Step 5: Commit** `feat: keep the Arbesk session in the keychain for writes`.

### Task 3: Pack a model file into parts

**Files:**
- Create: `src-tauri/src/arbesk/pack.rs`
- Test fixtures: reuse `tests/fixtures/arbesk/glb-box` (its `expected.gltf`) and `3mf-box`; tests write a `.glb`, a split `.gltf` + `.bin` + `tex/a.png` into a tempdir.

**Interfaces:**
- Produces:
  - `pub struct Packed { pub format: Format, pub composite: Value, pub parts: Vec<Vec<u8>> }` — in `composite`, part `i` is referenced as `"ipfs://part:{i}"` (glTF `uri`) or `"part:{i}"` (3MF `parts.*.cid`).
  - `pub fn pack(file: &Path) -> Result<Packed, String>` (dispatch on extension; size check first, `MAX_UPLOAD`).
  - `pub fn fill(composite: &Value, cids: &[String]) -> Value` (replaces both placeholder forms).
  - `pub const MAX_UPLOAD: u64`.
- Consumes: `compose::Format`.

- [ ] **Step 1: Write the failing tests:**
  - `a_glb_becomes_its_json_and_one_buffer_part` (1 part = the BIN chunk; `buffers[0].uri == "ipfs://part:0"`; images keep `bufferView`).
  - `a_gltf_uploads_data_uris_and_files_beside_it` (data-URI buffer, `scene.bin`, `tex/a%20b.png` → 3 parts, `mimeType` kept or sniffed `image/png`).
  - `gltf_parts_stay_in_its_folder` (`../x.bin`, `/etc/x`, `http://…` → "“…” is outside the model's folder."; a missing `scene.bin` → "“scene.bin” can't be found.").
  - `a_3mf_becomes_arbesks_composite` (keys `arbesk_format:"composite-3mf"`, `modelPath`, `contentTypes`, `rootRels`, `modelRels`, `model` text; other entries as parts); `a_zip_without_a_model_is_not_a_3mf`.
  - `other_extensions_and_large_files_are_refused` (exact wordings).
  - `fill_puts_cids_in_place`.
- [ ] **Step 2: Run** `cargo test --lib arbesk::pack` — Expected: FAIL.
- [ ] **Step 3: Implement `pack`, `fill`.** GLB: header magic `glTF`, chunk `JSON` then optional `BIN\0`. 3MF: the root `.rels` `Target` names the model; missing → "This isn't a 3MF file.".
- [ ] **Step 4: Run** — Expected: PASS.
- [ ] **Step 5: Commit** `feat: pack glTF, GLB and 3MF files into Arbesk's parts`.

### Task 4: Write an asset into a collection

**Files:**
- Create: `src-tauri/src/arbesk/write.rs`
- Modify: `src-tauri/src/arbesk/service.rs` (make `Reader` usable from `write.rs`: `pub(crate)` `open`, `collection_manifest_with_cid(token) -> (String, Value)`), `src-tauri/src/progress.rs` (`Phase::Saving`, serialized `"saving"`).

**Interfaces:**
- Consumes: Task 1 `Http::post_multipart`, `post_json_slow`; Task 3 `pack`, `fill`, `Packed`.
- Produces:
  - `pub struct Target { pub address: String, pub token_id: String, pub name: String /* listed, e.g. "Box.gltf" */ }`
  - `pub fn asset_manifest(name: &str, asset_id: &str, version: u64, prev: Option<&str>, timestamp_ms: i64, source_cid: &str, format: Format) -> Value`
  - `pub fn next_collection(current: &Value, current_cid: &str, asset_id: &str, asset_cid: &str) -> Value`
  - `#[derive(Default)] pub struct CollectionLocks` with `fn hold(&self, address: &str, token_id: &str) -> impl Drop`.
  - `pub fn add(http: &dyn Http, site: &Site, session: &str, target: &Target, file: &Path, overwrite: bool, progress: &Progress, cache_dir: &Path, locks: &CollectionLocks) -> Result<String, String>` — returns the new asset's `arbesk://` path. A taken listed name without `overwrite` → "Target already exists: {name}". With `overwrite`: the first asset whose listed name equals `target.name` gets the new version (same id, `version` = old + 1 or 2 when missing, `prev_asset_manifest_cid` = its current CID).
  - Errors map per Global Constraints; a 401 anywhere returns exactly "Sign in to Arbesk again to copy files into it.".
- Progress: `set_total` = sum of part sizes + composite; `add_bytes` per upload; `set_phase(Phase::Saving)` before the relay; `progress.check()` before each upload and before uploading the collection manifest, never after.

- [ ] **Step 1: Write the failing tests** (all against `testing::world()`):
  - `a_glb_added_reads_back_as_the_same_model` — `add` a GLB built from `glb-box` into Shapes as `Crate.gltf`, then `service::export` of the returned path; `compose::testing::assert_same_model` against `glb-box/expected.gltf`; Shapes' listing now has `Crate.gltf`.
  - `a_split_gltf_and_a_3mf_read_back` — export equals the original's composed bytes (glTF: JSON equal to the source with data URIs; 3MF: same entries).
  - `a_new_asset_and_a_replace_build_their_manifests` — `asset_manifest` fields; Replace keeps `asset_id`, `version` 2, `prev_asset_manifest_cid` = old; the collection: `version` + 1, `prev_asset_manifest_cid` = the CID read, `name`/`type` kept.
  - `a_taken_name_without_overwrite_is_refused_before_any_upload` (no upload request logged).
  - `relay_status_is_checked` — `Pending` and `Failed` give the exact wordings.
  - `arbesk_refusals_are_said_in_words` — 401, 403, 429, relay 502, `kubo` → exact wordings.
  - `two_writes_to_one_collection_keep_both` — two threads `add` different files into Shapes; both asset ids are in the final collection.
  - `a_cancel_stops_before_the_collection_changes` — cancelled progress → error, `tokens[1]` unchanged, no relay request.
  - `credentials_are_minted_in_one_call` — one `upload-urls` request with `count` = parts + 3.
- [ ] **Step 2: Run** `cargo test --lib arbesk::write` — Expected: FAIL.
- [ ] **Step 3: Implement** per spec D7 (lock held from re-reading `tokenURI` through the relay's answer).
- [ ] **Step 4: Run** `cargo test --lib arbesk::` — Expected: PASS.
- [ ] **Step 5: Commit** `feat: add a model file to an Arbesk collection`.

### Task 5: Route copies into Arbesk through the backend

**Files:**
- Modify: `src-tauri/src/arbesk/location.rs`, `src-tauri/src/arbesk/mod.rs`, `src-tauri/src/place.rs`, `src-tauri/src/lib.rs`

**Interfaces:**
- Consumes: Task 2 `session_for`, `forget_session`, `signed_in`; Task 4 `add`, `Target`, `CollectionLocks`.
- Produces:
  - `location::parse_target(path: &str) -> Result<(String /*address*/, String /*token*/, String /*name*/), String>` for `arbesk://<addr>/<token>~<name>/~<encoded name>`.
  - `place::Route::IntoArbesk`, returned only by `place::copy` for a local source and an Arbesk target whose parent is a collection (account or Network → "Paste into a collection."; source not local → "Only .gltf, .glb and .3mf files on this computer can be copied to Arbesk."). `transfer` (move, rename) still refuses Arbesk targets ("Arbesk is read only.").
  - `ArbeskState::add(&self, to: &str, file: &Path, overwrite: bool, progress: &Progress) -> Result<String, String>` (looks up the session; a 401 answer calls `forget_session`); `ArbeskState::exists(&self, path: &str) -> bool` (a pending target: the collection lists that name).
  - `lib.rs`: `copy_routed` arm for `IntoArbesk` (blocking); `exists_at` Arbesk arm; command `arbesk_signed_in(path: String) -> bool`, registered.
- [ ] **Step 1: Write the failing tests:** `place.rs` table rows (local → collection target: `IntoArbesk`; → account: "Paste into a collection."; Drive/share source → the D1 wording; `transfer` local → collection still "Arbesk is read only."); `location.rs` `a_pending_name_in_a_collection_is_a_target` (and `~` in an account or asset path is refused); `mod.rs` `exists_knows_a_listed_name` and `a_401_forgets_the_session`.
- [ ] **Step 2: Run** `cargo test --lib` — Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** `cd src-tauri && cargo test` — Expected: PASS (all, including integration tests).
- [ ] **Step 5: Commit** `feat: copy_entry writes local model files into Arbesk`.

### Task 6: Frontend rules and names

**Files:**
- Modify: `src/locations.ts`, `src/paths.ts`, `src/item-actions.ts`, `src/main.ts` (`hereFor`, `paste`, `dropOp`, `transferBatch` target name)
- Test: `tests/locations.test.ts`, `tests/paths.test.ts`, `tests/item-actions.test.ts`, `tests/item-actions-matrix.test.ts`

**Interfaces:**
- Produces:
  - `paths.ts`: `isArbeskCollection(path: string): boolean` (two levels below the prefix).
  - `locations.ts`: `isArbeskModelFile(path: string): boolean` (local path, extension per D1); `arbeskPasteRefusal(dir: string, paths: readonly string[], moving: boolean): string | null` (order: moving → "Arbesk is read only."; dir not a collection → "Paste into a collection."; any path not a local model file → D1 wording; else null); `arbeskTargetName(name: string): string` (`*.glb` → `*.gltf`, case kept for the stem).
  - `item-actions.ts`: `Here` gains `dir?: string` and `moving?: boolean`; `paste` for kind `arbesk` returns `arbeskPasteRefusal(here.dir ?? "", here.paths ?? [], here.moving ?? false)`.
  - `main.ts`: `paste()` passes the clipboard's paths, `dir` and `moving` to `refuse("paste", …)`; `dropOp` allows `"copy"` into a collection when `arbeskPasteRefusal` is null (forced move → null); `transferBatch` names an Arbesk target `arbeskTargetName(name)`.
- [ ] **Step 1: Write the failing tests:** each `arbeskPasteRefusal` branch; `arbeskTargetName("Box.GLB") === "Box.gltf"`, `.gltf`/`.3mf` unchanged; `isArbeskCollection`; item-actions paste cases; matrix row "an Arbesk collection" key paste with a local `.txt` on the clipboard → the D1 toast.
- [ ] **Step 2: Run** `npx vitest run tests/locations.test.ts tests/paths.test.ts tests/item-actions.test.ts tests/item-actions-matrix.test.ts` — Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** the same — Expected: PASS; `npx tsc --noEmit` clean.
- [ ] **Step 5: Commit** `feat: allow pasting model files into an Arbesk collection`.

### Task 7: The paste flow, the mock and the UI tests

**Files:**
- Modify: `src/main.ts` (`runTransfer`, `connectArbesk`), `src/api.ts` (`arbeskSignedIn`), `src/operations.ts` (`phase: "saving"` → status "Saving to Arbesk…"), `dev/mock-tauri/core.ts`, `tests/mock-core.test.ts`, `tests/arbesk.test.ts`, `tests/operations.test.ts` (if it exists; else the status in `arbesk.test.ts`)

**Interfaces:**
- Consumes: Task 5 `arbesk_signed_in`, `copy_entry` routing; Task 6 helpers.
- Produces:
  - `connectArbesk(reconnect?: Account, opts?: { stay?: boolean }): Promise<boolean>` — resolves true once connected (no navigation when `stay`), false on Cancel.
  - `confirmArbeskCopy(): Promise<boolean>` — the D2 dialog, Cancel default.
  - `runTransfer` into Arbesk: confirm → `arbeskSignedIn(destDir)` or `connectArbesk(account, { stay: true })` → batch; an item failing with "Sign in to Arbesk again to copy files into it." opens `connectArbesk` once and retries that item once.
  - Mock: `copy_entry` local → Arbesk adds `{id: "asset_<n>", name, ext}` to the collection (Replace keeps the id and adds a version for `arbesk_versions`), with the backend's refusals; `__mockArbesk.signedIn` (default true) and `__mockArbesk.expireOnce()` for tests; `arbesk_signed_in` command.
- [ ] **Step 1: Write the failing tests** (`tests/arbesk.test.ts`): "Copy and Paste put model files into a collection after confirming" (dialog text, Cancel default leaves the collection unchanged, Copy adds `Box.gltf` from `Box.glb` and `Tray.3mf`); "a .glb pastes as its listed .gltf name" (Keep Both twice → `Box (copy).gltf`, `Box (copy 2).gltf`; Replace → one `Box.gltf` with two versions); "a paste with no session signs in first"; "an expired session signs in again and carries on" (expireOnce: the dialog opens once, both files land, no duplicate); "a drop on a collection copies"; "a text file or a folder is refused with the D1 toast". `tests/mock-core.test.ts`: copy_entry into Arbesk, its refusals, `arbesk_signed_in`.
- [ ] **Step 2: Run** `npx vitest run tests/arbesk.test.ts tests/mock-core.test.ts` — Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** `npm test` and `npx tsc --noEmit && npm run build` — Expected: all PASS.
- [ ] **Step 5: Commit** `feat: confirm, sign in and paste into Arbesk`.

### Task 8: Docs and verification list

**Files:**
- Modify: `README.md` (feature line; "Not supported yet": Arbesk read only → what still is), `docs/arbesk.md` (a "Copying into Arbesk" section: what, public data, Replace = version, session, errors), `AGENTS.md` (layout rows for `arbesk/` modules `session.rs`, `pack.rs`, `write.rs`, `testing.rs`; `place.rs` row), `docs/architecture.md` (Arbesk no longer read only)
- [ ] **Step 1:** Write the docs; check every wording against the Global Constraints.
- [ ] **Step 2:** `npx tsc --noEmit && npm run build` — Expected: clean.
- [ ] **Step 3: Commit** `docs: copying files into Arbesk`.
- [ ] **Step 4:** After the PR is open, add the D-testing real-app lines to #224.
