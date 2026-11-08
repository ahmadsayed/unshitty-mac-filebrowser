# Arbesk in Network — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Connect an Arbesk account with email + emailed code and browse it read-only from Network: collections as folders, assets as `.gltf`/`.3mf` files, Space previews them, double-click opens them.

**Architecture:** A new backend module `src-tauri/src/arbesk/` rebuilds Arbesk's read path in plain Rust (config, indexer, `tokenURI` over JSON-RPC, IPFS gateway, compose) behind an `Http` trait so tests run without a network. A new place kind (`arbesk://`) is a row in each existing table (`place.rs`, `paths.ts`, `locations.ts`, `item-actions.ts`, `providers.ts`); sign-in reuses Drive's loopback wait.

**Tech Stack:** Rust (`reqwest` blocking, `serde_json`, `brotli`, `flate2`, `zip`, `url`), TypeScript without a framework, Vitest + jsdom against `dev/mock-tauri/`.

**Spec:** `docs/superpowers/specs/2026-11-08-arbesk-network-design.md` (D1–D9 below refer to it).

## Global Constraints

- Arbesk's repository is not changed; origin `https://promptscad.com`, chain id `84532`, both stored per account.
- Read only: every write refused in `place.rs` and `item-actions.ts`; copy out refused with "Copying from Arbesk comes next."
- No credential stored: after the D2 check, `DELETE {origin}/api/v1/sessions`; `arbesk_accounts.json` holds `{ id, email, address, origin, chain_id }` only.
- Paths: `arbesk://<address>`, `/<tokenId>~<encoded name>`, `/<assetID>~<encoded name>.<gltf|3mf>`; `<address>` lower-case; names encoded with Drive's encoder; the backend reads ids only.
- Unknown format message, exact: "This asset uses an Arbesk format File Browser can't read yet."
- Sign-in check failure, exact: "Sign-in failed: Arbesk did not confirm this account."; 502/503: "Arbesk could not confirm this account. Try again later."
- Untitled collection label: "Untitled Collection"; an untitled asset uses its asset id.
- At most 8 requests at a time; manifests cached on disk by CID, capped at 64 MB, oldest dropped first.
- Plain functions in the module; only `lib.rs` touches Tauri. No `std::fs::rename` outside the modules `placement.rs` allows (the build-to-cache rename uses `placement::rename_new`).
- No new crates: `brotli`, `flate2`, `zip`, `reqwest`, `url` are already in `Cargo.lock`; add `brotli` as a direct dependency at the version the lock has (9.0.0) and run cargo-deny 0.20.2.
- GNOME wording, `openDialog`/`showToast`, tokens only; every new mock command has a case in `tests/mock-core.test.ts`.
- CRAP gate: no function over 30; dispatch by table.

## Review Focus

1. **A callback that isn't Arbesk's** (a web page hits the loopback port with any `token`/`address`): sign-in must fail, never save an account. Task 6 tests a wrong address, a wrong email, a 401 and a 503.
2. **Names that break paths** (`/`, `~`, `%`, emoji, empty, two assets with one name): every asset lists and opens by id. Task 2 tests these segments; Task 5 lists two same-named assets.
3. **A gateway that fails half-way through building an asset**: no partial file in the cache, and the next open retries. Task 5 tests a fake gateway that fails the second part.
4. **A collection whose manifest can't be read or an unknown `arbesk_format`**: the folder still lists the rest; opening the odd asset gives D9's message. Tasks 4 and 5 test both.
5. **Network flapping while Network is open**: a failed account listing keeps Network's rows (as `refreshAccounts` does for Drive). Task 8 tests a failing `arbesk_accounts`.

---

### Task 1: Share Drive's loopback wait

**Files:**
- Create: `src-tauri/src/cloud/loopback.rs`
- Modify: `src-tauri/src/cloud/oauth.rs` (`finish`, `read_request`, `respond`, `REQUEST_TIMEOUT` move out), `src-tauri/src/cloud/mod.rs` (`pub mod loopback;`)

**Interfaces:**
- Produces: `pub fn wait_for_callback<T>(listener: &TcpListener, timeout: Duration, cancelled: impl Fn() -> bool, accept: impl FnMut(&Url) -> Option<T>, done_text: &str) -> Result<T, Waited>` where `pub enum Waited { Cancelled, TimedOut, Io(String) }`. `accept` gets each request's URL (`http://127.0.0.1<target>`); `None` answers 404 and keeps waiting; `Some` answers 200 with `done_text` and returns.
- Google's `finish` keeps its signature and messages, built on it.

- [ ] **Step 1: Write the failing tests** in `loopback.rs`: `ignores_favicon_then_returns_the_callback` (a GET `/favicon.ico` then `/?token=t`; `accept` returns `url.query_pairs()` token; result `Ok("t")`, the first response is 404), `stops_when_cancelled` (`cancelled` true → `Err(Waited::Cancelled)`), `times_out` (50 ms → `Err(Waited::TimedOut)`), `answers_with_done_text` (the 200 body equals `done_text`). Reuse the request helpers already in `oauth.rs`'s tests.
- [ ] **Step 2: Run** `cd src-tauri && cargo test cloud::loopback` — expect compile errors (module missing).
- [ ] **Step 3: Implement** `wait_for_callback` by moving `finish`'s accept loop, `read_request`, `respond` and `REQUEST_TIMEOUT` into `loopback.rs`; rewrite `oauth::finish` to call it, mapping `Waited::Cancelled` → "Sign-in was cancelled.", `TimedOut` → "Sign-in timed out. Try again.", `Io(e)` → `format!("Google sign-in failed: {e}")`; Google's `parse_callback`/state check/exchange stay in `oauth.rs` inside `accept`.
- [ ] **Step 4: Run** `cd src-tauri && cargo test cloud::` — all new and existing oauth tests pass.
- [ ] **Step 5: Commit** `refactor: share the browser sign-in's loopback wait`

### Task 2: The `arbesk://` path and the backend place kind

**Files:**
- Create: `src-tauri/src/arbesk/mod.rs` (module root, `pub mod location;` and later modules), `src-tauri/src/arbesk/location.rs`
- Modify: `src-tauri/src/lib.rs` (`mod arbesk;`), `src-tauri/src/place.rs` (`Kind::Arbesk` and its arms + table test), `src-tauri/src/cloud/location.rs` (`encode_name`/`decode_name` become `pub(crate)`)

**Interfaces:**
- Produces: `pub const PREFIX: &str = "arbesk://";` `pub fn is_arbesk(path: &str) -> bool;` `pub enum ArbeskPath { Account { address: String }, Collection { address: String, token_id: String }, Asset { address: String, token_id: String, asset_id: String } }` with `pub fn parse(path: &str) -> Result<ArbeskPath, String>`; `pub fn collection_path(address, token_id, name: &str) -> String`; `pub fn asset_path(collection_path: &str, asset_id, name: &str, ext: &str) -> String`.
- `place::Kind::Arbesk`; `place::local` refuses with "This action is not available for Arbesk."; `place::writable` refuses with "Arbesk is read only."; `place::transfer` refuses either side through `writable`.

- [ ] **Step 1: Write the failing tests** in `location.rs`: round trips for each level; `parse("arbesk://0xAbC")` lower-cases to `0xabc`; a name with `/`, `~`, `%`, `é`, emoji round-trips through `asset_path` → `parse` giving the same ids; an empty name segment (`/<id>~`) parses; a non-hex address, an empty token id, a fourth segment, and a malformed `%` escape are `Err`. In `place.rs`'s table test add rows: `arbesk://0xabc/1~A` → `Kind::Arbesk`, `local` Err, `writable` Err, `transfer(local, arbesk)` Err, `transfer(arbesk, local)` Err.
- [ ] **Step 2: Run** `cd src-tauri && cargo test arbesk::location place::` — fails to compile.
- [ ] **Step 3: Implement.** Segment = `<id>~<encode_name(name)>`; the id is everything before the first `~` (ids are digits for tokens, Arbesk asset ids contain no `~`; refuse one that is empty). The asset's display extension is part of the encoded name. Add the `Kind::Arbesk` arms; `kind()` checks `arbesk::location::is_arbesk` first.
- [ ] **Step 4: Run** `cd src-tauri && cargo test` — passes (existing `lib.rs` matches on `Kind` now need an `Arbesk` arm: give `list_dir`, `download_to_open`, `properties`/folder size the refusal "This action is not available for Arbesk." for now; Task 5 replaces the first two).
- [ ] **Step 5: Commit** `feat: add the arbesk:// place kind to the backend`

### Task 3: Reading Arbesk: config, indexer, tokenURI, gateway

**Files:**
- Create: `src-tauri/src/arbesk/http.rs`, `src-tauri/src/arbesk/chain.rs`, `src-tauri/src/arbesk/ipfs.rs`
- Modify: `src-tauri/Cargo.toml` (`brotli = "9"` direct, default features)

**Interfaces:**
- Produces: `pub trait Http: Send + Sync { fn get(&self, url: &str) -> Result<Vec<u8>, HttpError>; fn post_json(&self, url: &str, body: &serde_json::Value, session: Option<&str>) -> Result<Vec<u8>, HttpError>; fn delete(&self, url: &str, session: Option<&str>) -> Result<(), HttpError>; }`, `pub struct HttpError { pub status: Option<u16>, pub message: String }`, `pub struct ReqwestHttp` (30 s timeout, `Authorization: Session <token>` when given).
- `pub struct Site { pub origin: String, pub chain_id: u64 }`; `pub struct ChainConfig { pub contract: String, pub rpc_url: String, pub gateway: String }`; `pub fn config(http: &dyn Http, site: &Site) -> Result<ChainConfig, String>` (`networkConfigs[chain_id]` with `contractAddress`, `rpcUrl`; `ipfsGatewayUrl` top-level, trailing `/` kept off).
- `pub fn collection_ids(http, site, address) -> Result<Vec<String>, String>` (owned then shared from `/api/v1/indexer/{owned,shared}?address=&chainId=`, keys `owned`/`shared`, de-duplicated in order).
- `pub fn token_uri(http, cfg: &ChainConfig, token_id: &str) -> Result<String, String>`; `fn encode_token_uri_call(token_id: &str) -> Result<String, String>`; `fn decode_abi_string(hex: &str) -> Result<String, String>`; a returned `ipfs://<cid>` is reduced to `<cid>`.
- `pub fn unpack(bytes: Vec<u8>) -> Result<Vec<u8>, String>` (brotli frame `41 52 42 01` → brotli-decompress the rest; gzip `1f 8b` → gunzip; else as is). `pub struct Gateway<'a> { http: &'a dyn Http, base: String, cache: ManifestCache }` with `pub fn bytes(&self, cid: &str) -> Result<Vec<u8>, String>` (unpacked) and `pub fn json(&self, cid: &str) -> Result<serde_json::Value, String>` (cached on disk); `pub struct ManifestCache { dir: PathBuf, cap: u64 }` with `get(cid)`/`put(cid, bytes)`, cap 64 MB, oldest by mtime dropped first; a CID that isn't `[A-Za-z0-9]+` is refused before any request.

- [ ] **Step 1: Write the failing tests** with a `FakeHttp` (a `HashMap<String, Result<Vec<u8>, HttpError>>` plus a log of requests): `encode_token_uri_call("7") == "0xc87b56dd" + 63 zeros + "7"` (64 hex digits of the uint256); a token id that isn't decimal is Err; `decode_abi_string` of the encoding of `"ipfs://bafyX"` (offset `0x20`, length, padded bytes) returns it; a truncated answer is Err; `token_uri` POSTs `{"jsonrpc":"2.0","id":1,"method":"eth_call","params":[{"to":contract,"data":…},"latest"]}` and maps a JSON-RPC `error` to Err; `collection_ids` merges `{"owned":["1","2"]}` and `{"shared":["2","3"]}` to `["1","2","3"]`; `unpack` of a brotli frame, a gzip stream and raw bytes; `Gateway::json` asks once for the same CID across two calls and reads the second from `ManifestCache`; the cache drops the oldest file when a put exceeds the cap; `json("../x")` is Err with no request logged.
- [ ] **Step 2: Run** `cd src-tauri && cargo test arbesk::` — fails to compile.
- [ ] **Step 3: Implement** the signatures above. The ABI string: bytes 0..32 = offset (must be 32), 32..64 = length, then `length` bytes as UTF-8.
- [ ] **Step 4: Run** `cd src-tauri && cargo test arbesk::` — passes. Then `cargo deny --locked --manifest-path src-tauri/Cargo.toml --config deny.toml check` — no new finding.
- [ ] **Step 5: Commit** `feat: read Arbesk's config, collections and IPFS content`

### Task 4: Building the file (compose glTF and 3MF)

**Files:**
- Create: `src-tauri/src/arbesk/compose.rs`, `src-tauri/tests/fixtures/arbesk/` (see Step 1)

**Interfaces:**
- Consumes: `Gateway::bytes`, `Gateway::json` (Task 3).
- Produces: `pub enum Format { Gltf, ThreeMf }` with `pub fn ext(self) -> &'static str` (`"gltf"`, `"3mf"`); `pub fn format_of(source: &serde_json::Value) -> Result<Format, String>` (no `arbesk_format` → Gltf, `"composite-3mf"` → ThreeMf, anything else → D9's message); `pub fn source_cid(manifest: &Value, cid: &str) -> String` (asset-core's `resolveCompositeSourceCid`: `scene.nodes[0].source.cid` when the manifest has no `buffers`, `meshes` or `arbesk_format`, else `cid`); `pub fn is_child_ref(manifest: &Value) -> bool` (any `scene.nodes[*].child_ref`); `pub fn compose(gateway: &Gateway, source: &Value) -> Result<Vec<u8>, String>`.
- glTF: clone, drop `_arbesk` from every `buffers[i]`/`images[i]`; an `ipfs://<cid>` uri becomes `data:application/octet-stream;base64,…` (buffers) or `data:<mimeType or image/png>;base64,…` (images, `bufferView` removed); a `data:` uri stays; anything else → D9's message; output `serde_json::to_vec`.
- 3MF: requires string `contentTypes`, `rootRels`, `model`; entries `[Content_Types].xml`, `_rels/.rels`, `modelPath` (default `3D/3dmodel.model`), its rels at `<dir>/_rels/<file>.rels` when `modelRels` is a string, then each `parts[path].cid` via `Gateway::bytes`; zipped with deflate.

- [ ] **Step 1: Make the reference fixture.** In `~/Projects/arbesk`, with the compiled probe or `./besk`, download one small glTF and one 3MF asset the human names (ask them for the asset names and run `besk download`); save beside each its source composite JSON and every CID's raw bytes it names, as `tests/fixtures/arbesk/<case>/{source.json, cids/<cid>, expected.<ext>}`. If the human can't provide assets, build the composites by hand from `dev/mock-tauri/models.ts`'s box and say so in the PR (the comparison with besk then moves to the `needs-verification` issue).
- [ ] **Step 2: Write the failing tests:** `gltf_matches_besk` (compose the fixture through a fake gateway serving `cids/`; parse both JSONs; equal); `threemf_matches_besk` (same zip entry names and bytes per entry); `unknown_format_is_refused` (`arbesk_format: "composite-example"` → D9's message); `foreign_uri_is_refused` (`https://…` buffer → D9's message); `missing_model_is_refused` (3MF without `model` → Err); `source_cid_follows_a_wrapper`; `child_ref_is_detected`.
- [ ] **Step 3: Run** `cd src-tauri && cargo test arbesk::compose` — fails.
- [ ] **Step 4: Implement** the signatures above.
- [ ] **Step 5: Run** `cd src-tauri && cargo test arbesk::compose` — passes.
- [ ] **Step 6: Commit** `feat: build Arbesk assets as glTF and 3MF files`

### Task 5: Listing and opening (backend service + commands)

**Files:**
- Create: `src-tauri/src/arbesk/service.rs`
- Modify: `src-tauri/src/arbesk/mod.rs` (`ArbeskState`), `src-tauri/src/lib.rs` (`Kind::Arbesk` arms in `list_dir`, `download_to_open`, `preview_bytes`, `path_info`; `.manage(ArbeskState::new(...))`)

**Interfaces:**
- Consumes: Tasks 2–4; `accounts::Accounts` arrives in Task 6, so this task takes the account as `Site` + address resolved by a closure.
- Produces: `pub fn list(http: &dyn Http, site: &Site, path: &ArbeskPath, cache_dir: &Path) -> Result<Vec<fs_ops::Entry>, String>`: Account → one dir entry per collection (`name` = manifest `name` or "Untitled Collection", path `collection_path`); Collection → one file per asset (`name` = asset manifest `name` or asset id, plus `.` + `Format::ext`; a manifest that fails to read is left out; a child_ref asset is listed as `.gltf`); `size: 0`, `modified: 0`, `cloud: None`. Requests at most 8 at a time (a scoped thread pool over the ids).
- `pub fn build(http, site, path: &ArbeskPath, cache_dir: &Path, progress: &Progress) -> Result<PathBuf, String>`: an Asset only; resolves the asset manifest CID through the collection; returns `<cache>/arbesk/<manifest cid>/<display name>` if it exists, else composes into `<…>/.<name>.part`, then `placement::rename_new` into place, then `downloads::mark`; a failure removes the `.part` file. A child_ref asset → D9's message.
- `lib.rs`: `list_dir` → `arbesk::service::list` via `blocking`; `download_to_open` → `OpenTarget::File { path: build(...) }`; `preview_bytes` → `build` then `fs_ops::preview_bytes(path, limit)`; `path_info` → `Some(PathInfo { is_dir, device: 0 })` from the parsed level.

- [ ] **Step 1: Write the failing tests** in `service.rs` against `FakeHttp` serving a config, an indexer answer of two tokens, their `eth_call`s, manifests and parts: `account_lists_collections` (two dirs, untitled one named "Untitled Collection"); `collection_lists_assets` (a glTF and a 3MF with the right extensions; two assets both named "Box" both listed with different paths); `unreadable_manifest_is_left_out`; `build_reuses_the_cache` (second `build` logs no gateway request); `build_failure_leaves_no_part` (the gateway fails the second part; no file under `<cache>/arbesk/`; a following `build` with a healthy gateway succeeds); `build_refuses_child_ref`.
- [ ] **Step 2: Run** `cd src-tauri && cargo test arbesk::service` — fails.
- [ ] **Step 3: Implement** `list`, `build` and the `lib.rs` arms. Add `arbesk::service` to the `placement.rs` test's rename callers only if it calls `std::fs::rename` (it must not: use `rename_new`).
- [ ] **Step 4: Run** `cd src-tauri && cargo test` — passes.
- [ ] **Step 5: Commit** `feat: list and open Arbesk collections and assets`

### Task 6: Sign-in and the account store

**Files:**
- Create: `src-tauri/src/arbesk/accounts.rs`, `src-tauri/src/arbesk/signin.rs`
- Modify: `src-tauri/src/arbesk/mod.rs` (`ArbeskState { accounts: Mutex<Accounts>, flows: Mutex<HashMap<String, Arc<Flow>>>, http: Arc<dyn Http>, cache_dir }`), `src-tauri/src/lib.rs` (commands below, registered; Task 5's closure now reads `Accounts`)

**Interfaces:**
- Consumes: `cloud::loopback::wait_for_callback` (Task 1), `Http` (Task 3).
- Produces: `#[derive(Serialize, Deserialize, Clone)] pub struct ArbeskAccount { pub id: String, pub email: String, pub address: String, pub origin: String, pub chain_id: u64, pub root_path: String }`; `Accounts::load(path) / items() / get_by_address(&str) / upsert(account) / remove(id)` saved atomically to `arbesk_accounts.json` like `cloud::accounts`.
- `pub struct Flow { pub id: String, pub email: String, pub listener: TcpListener, pub sign_in_url: String, created: Instant, cancelled: AtomicBool }`; `pub fn start(site: &Site, email: &str) -> Result<Flow, String>` (email trimmed and lower-cased, must contain `@`; URL `{origin}/api/v1/cli-auth?email=<enc>&port=<port>`); `pub fn finish(flow: &Flow, http: &dyn Http, site: &Site, timeout: Duration) -> Result<(String /*email*/, String /*address*/), String>`: waits with done text "You can close this window and return to File Browser.", accepts a request with a `token` query; then checks D2: callback email == flow email (case-insensitive), `POST {origin}/api/v1/users/resolve-email {"email"}` with the session returns `exists: true` and `address` equal (case-insensitive) to the callback's; then `DELETE {origin}/api/v1/sessions` (its failure is ignored); returns the lower-cased address.
- Commands (Tauri, names exact): `arbesk_accounts() -> Vec<ArbeskAccount>`, `arbesk_connect_start(email: String) -> { flow_id, sign_in_url }`, `arbesk_connect_finish(flow_id: String) -> ArbeskAccount` (async, `blocking`; an address already connected keeps its id), `arbesk_connect_cancel(flow_id: String)`, `arbesk_disconnect(account: String)`. Timeout 5 min, flows older than that dropped on start (as `drive_connect_start` does).

- [ ] **Step 1: Write the failing tests** in `signin.rs` (callbacks sent to the flow's port from a thread, `FakeHttp` for the backend): `confirms_and_ends_the_session` (resolve-email answers the same address in other case → `Ok` with lower-case address; the log has the DELETE with the session); `wrong_email_fails` and `wrong_address_fails` → "Sign-in failed: Arbesk did not confirm this account."; `unauthorised_fails` (401) → same message; `unavailable_fails` (503) → "Arbesk could not confirm this account. Try again later."; `no_token_keeps_waiting` (`/favicon.ico` then a good callback → Ok); `cancel` → "Sign-in was cancelled."; `start` refuses `"not-an-email"`. In `accounts.rs`: save/load round trip; upsert by address keeps the id; the file has no `token` key.
- [ ] **Step 2: Run** `cd src-tauri && cargo test arbesk::` — fails.
- [ ] **Step 3: Implement** the above and register the commands.
- [ ] **Step 4: Run** `cd src-tauri && cargo test` — passes.
- [ ] **Step 5: Commit** `feat: sign in to Arbesk with email and code through the browser`

### Task 7: The frontend's tables: paths, kinds, actions, provider

**Files:**
- Modify: `src/paths.ts` (`ARBESK_PREFIX = "arbesk://"`, `isArbeskPath`, `Place` += `"arbesk"`, `placeOf`, `SCHEMES.arbesk`, `isRemotePath`, `arbeskAccountKey`), `src/locations.ts` (`LocationKind` += `"arbesk"`, `PLACE_KINDS`, `isReadOnlyKind`, `copyOutRefusalOf`, `knowsSizes`, `previewsRemotely`), `src/item-actions.ts` (rows for `"arbesk"`), `src/providers.ts` (`ProviderId` += `"arbesk"`, row, order, `fromArbesk`), `src/api.ts` (`ArbeskAccount` type), `dev/mock-tauri/core.ts` (`NOT_LOCAL.arbesk` = "This action is not available for Arbesk."; other `Record<Place,…>` the compiler names)
- Test: `tests/paths.test.ts`, `tests/locations.test.ts`, `tests/item-actions.test.ts`, `tests/item-actions-matrix.test.ts`, `tests/providers.test.ts`

**Interfaces:**
- Produces: `SCHEMES.arbesk` mirrors `drive`'s row: name = `cloudSegmentName` of the last segment, parent = up one segment with the account root (`arbesk://<address>`) as its own top, child = `joinCloudPath` (a pending `~<name>` segment; nothing creates one, as Arbesk is read only), segments via `remoteSegments(path, "arbesk", ARBESK_PREFIX, { root: key => ARBESK_PREFIX + key, rootName: key => key, segment: cloudSegmentName })`. In `main.ts`, `arbeskCrumbs(loc)` mirrors `driveCrumbs`: the account alone is one crumb "Arbesk" (icon `network-workgroup`), then one crumb per collection name.
- `knowsSizes(kind: LocationKind): boolean` (false for `"arbesk"`), `previewsRemotely(kind: LocationKind): boolean` (true only for `"arbesk"`), `copyOutRefusalOf` → "Copying from Arbesk comes next." when the first kind is `"arbesk"`.
- `PROVIDERS.arbesk = { label: "Arbesk", icon: "arbesk", owns: isArbeskPath, accountKey: arbeskAccountKey }`; `PROVIDER_ORDER = ["gdrive", "arbesk"]`; `fromArbesk(a: ArbeskAccount): Account` → `{ provider: "arbesk", id: a.id, email: a.email, rootPath: a.root_path }`.

- [ ] **Step 1: Write the failing tests:** `paths.test.ts` rows for `arbesk://0xabc`, `…/12~My%20Set`, `…/12~My%20Set/a1~Box.gltf` (placeOf, pathName "My Set" / "Box.gltf", parentPath, pathSegments); `locations.test.ts` rows: kind `"arbesk"`, read only, not local, no recursive search, no versions, no new items, `knowsSizes` false, `previewsRemotely` true only for it; `item-actions.test.ts`: every write and copy refused in `"arbesk"` with the matrix's wording; `item-actions-matrix.test.ts`: the `"arbesk"` row equals the `"sftp"` row except the copy refusal text; `providers.test.ts`: `providerOf("arbesk://0xabc/1~A")` is `"arbesk"`, `accountOf` matches by address, order.
- [ ] **Step 2: Run** `npx vitest run tests/paths.test.ts tests/locations.test.ts tests/item-actions.test.ts tests/item-actions-matrix.test.ts tests/providers.test.ts` — fails.
- [ ] **Step 3: Implement** the rows; let `npx tsc --noEmit` name every `Record<Place|LocationKind|ProviderId, …>` that needs one (main.ts `UNNAMED` and menu layouts get the SSH server's entries; `PROVIDER_ACTIONS.arbesk` is stubbed with `connect: () => {}` until Task 8).
- [ ] **Step 4: Run** the same vitest command and `npx tsc --noEmit` — pass.
- [ ] **Step 5: Commit** `feat: add Arbesk to the frontend's place and provider tables`

### Task 8: Connect, Network row, icon, mock

**Files:**
- Modify: `src/api.ts` (`arbeskAccounts`, `arbeskConnectStart(email)`, `arbeskConnectFinish(flowId)`, `arbeskConnectCancel(flowId)`, `arbeskDisconnect(account)`), `src/main.ts` (`refreshAccounts` merges both providers, `PROVIDER_ACTIONS.arbesk`, `connectArbesk(reconnect?: Account)`, `confirmDisconnectArbesk(account)`), `src/icons.ts` + `src/icons-macos.ts` (`arbesk` symbolic: an outlined cube, drawn from scratch, 16×16, `currentColor`), `dev/mock-tauri/core.ts` (`ARBESK_COMMANDS` + a fake account tree: collection "Shapes" with `Box.gltf` and `Box.3mf`, an untitled collection with one asset, `list_dir`/`open_target`/`preview_bytes` arms returning `sampleModelBytes`)
- Test: `tests/arbesk.test.ts` (new), `tests/mock-core.test.ts`, `tests/online-accounts.test.ts`

**Interfaces:**
- Consumes: Task 6's commands, Task 7's rows.
- Produces: the Connect dialog: heading "Connect to Arbesk", body "Arbesk emails you a code. Enter it in your browser to sign in.", an email entry (label "Email"), responses Cancel / "Open Browser" (suggested, disabled until the entry has an `@`), `busy` with Cancel live; while waiting the status reads "Waiting for Arbesk…"; on success toast "Connected to <email>" and navigate to the account root. Disconnect: heading "Disconnect Arbesk?", body "“<email>” is removed from File Browser. Your collections on Arbesk are not changed.", Cancel / "Disconnect" (destructive), default Cancel. `signIn(account)` opens Connect with the email filled in.
- `refreshAccounts`: Drive and Arbesk asked together; each provider's failed answer keeps that provider's known accounts.

- [ ] **Step 1: Write the failing tests** in `tests/arbesk.test.ts`: Connect from Online Accounts → type email → Open Browser → the mock finishes → Network lists "Arbesk"; opening it lists "Shapes" and "Untitled Collection"; opening "Shapes" lists `Box.gltf` and `Box.3mf` with empty size and date cells, and the path bar's crumbs read "Arbesk", "Shapes"; the context menu offers no Rename, Move to Trash, Cut or Copy; Ctrl+C toasts "Copying from Arbesk comes next."; `arbesk_accounts` failing on a later refresh keeps the row (Review Focus 5); Disconnect removes the row. In `mock-core.test.ts` one case per new command. In `online-accounts.test.ts`, "Connect an Account" lists Google Drive then Arbesk.
- [ ] **Step 2: Run** `npx vitest run tests/arbesk.test.ts tests/mock-core.test.ts tests/online-accounts.test.ts` — fails.
- [ ] **Step 3: Implement** the above; `sizeText` and the status bar ask `knowsSizes(locationKind(path, trashPath))` and leave the size out when false.
- [ ] **Step 4: Run** the same command and `npx tsc --noEmit` — pass. Then `npm run dev:mock`, connect, browse; screenshots `node scripts/mock-screenshot.mjs <arbesk collection path> after.png` and `--dark`, and `--theme gnome`; look at them.
- [ ] **Step 5: Commit** `feat: connect and browse Arbesk accounts from Network`

### Task 9: Space previews Arbesk assets

**Files:**
- Modify: `src/main.ts` (`showPreview`)
- Test: `tests/arbesk.test.ts`, `tests/model-preview.test.ts`

**Interfaces:**
- Consumes: `previewsRemotely` (Task 7), the mock's `preview_bytes` arm (Task 8), the backend arm (Task 5).

- [ ] **Step 1: Write the failing tests:** Space on `Box.gltf` and on `Box.3mf` in the mock Arbesk collection shows the model view (the same assertion `model-preview.test.ts` uses for local files); Space on a Drive and an SSH file still shows "Preview isn't available for remote items."; an Arbesk asset over `MODEL_PREVIEW_LIMIT` shows "This file is too large to preview."
- [ ] **Step 2: Run** `npx vitest run tests/arbesk.test.ts tests/model-preview.test.ts` — fails.
- [ ] **Step 3: Implement:** the remote check becomes `isRemotePath(entry.path) && !previewsRemotely(locationKind(entry.path, trashPath))`.
- [ ] **Step 4: Run** the same command — passes. Then the full checks: `npx tsc --noEmit`, `npm run build`, `npm test`, `cd src-tauri && cargo test`, `npm run crap`.
- [ ] **Step 5: Commit** `feat: preview Arbesk models with Space`

### Task 10: Docs, real app, PR

**Files:**
- Modify: `README.md` (features: Arbesk in Network, read only; "Not supported yet": writing to Arbesk, copying from it), `docs/architecture.md` (+ diagram: `arbesk/` beside `cloud/`), `AGENTS.md` (layout rows for `src-tauri/src/arbesk/` and the new place kind in `place.rs`'s row), `docs/arbesk.md` (new: what connecting does, what is stored, read only)
- Delete: nothing in the main checkout; tell the human the untracked draft `docs/superpowers/plans/2026-11-08-arbesk-network.md` there is superseded by this plan.

- [ ] **Step 1:** Write the docs; check every AGENTS.md rule this touches (data safety: nothing written remotely, the cache's built files go through `rename_new` and `downloads.rs`).
- [ ] **Step 2: Real app.** `npm run tauri dev`; ask the human to connect with their email and code, open a collection, Space a glTF and a 3MF, open one. Record what was tried and by whom.
- [ ] **Step 3:** Open a `needs-verification` issue: the macOS browser hand-off, a real sign-in on each OS, besk-equality on the human's own assets if Task 4 used hand-made fixtures.
- [ ] **Step 4:** Commit `docs: describe Arbesk in Network`, push, `scripts/pr-screenshots.sh before.png after.png`, open the PR with the template (`Refs #<verification issue>`), post the self-review comment, hand over.
