# Arbesk Responsiveness Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make Arbesk folders and model previews feel much closer to Google Drive by removing avoidable gateway requests, cache rescans and repeated chain reads, without changing what Arbesk stores or how it looks.

**Architecture:** Five read-path changes, all in `src-tauri/crates/arbesk`:

1. `ipfs::FileCache` replaces the per-write directory rescan in `ManifestCache` and becomes the disk cache for Arbesk thumbnails.
2. A listing trusts the source format the asset manifest already records (`scene.nodes[].source.format`, then `metadata.computed.format`) instead of downloading the source manifest only to read its extension. The write path (`claim`) keeps the source-derived name.
3. `chain::ChainCache` keeps the deployment config for read paths. It does **not** cache `tokenURI`; the network design's D6 still holds, and a cached CID would be a correctness risk for Replace and Reload.
4. `compose` fetches a model's parts in small parallel batches, on demand, after the source has been validated.
5. Arbesk thumbnails are kept on disk by content-addressed CID and size.

The frontend, the mock, the Tauri command list and the UI do not change. Files, CIDs and manifests written to Arbesk are unchanged.

**Tech Stack:** Rust (`arbesk` crate, the existing `Http` seam, `std::thread::scope` through `ipfs::in_parallel`), no new dependencies.

**Spec:** none; this is a performance follow-up to `docs/superpowers/specs/2026-11-08-arbesk-network-design.md`, `...-arbesk-copy-in-design.md` and `...-arbesk-scenes-design.md`. Their behaviour decisions still hold.

## Plan Review

A read-only plan judge reviewed the first version and found these issues; the plan below incorporates every fix:

- **No `tokenURI` cache.** A cache keyed without the contract could return another collection's manifest for a link, and a 30 s stale value would let `path_exists` skip the Replace prompt before a fresh write. Spec D6 says `tokenURI` is asked on every listing and Reload. Only the deployment config is cached.
- **`source.format` comes first, not `metadata.computed.format`.** `computed` is not top-level and is derived from `source.format`; it is a second candidate only.
- **The write path is untouched.** `write::claim` keeps deriving names from the source manifest, so a lying manifest format cannot change which asset a paste replaces.
- **FileCache totals are process-wide per directory.** A per-instance total would let several Readers each grow the same cache. A damaged entry is removed before a re-fetch, so it is repaired rather than fetched forever.
- **Compose validates before fetching.** Unknown formats, missing 3MF fields and climbing part paths are refused with zero part requests. Parts are prefetched only a batch ahead as the assembler asks for them, so peak memory stays near today's.
- **Tests were made deterministic:** `File::set_modified` instead of sleeps, a barrier with a timeout for parallel fetches, and a warm cache before the paste invalidation test.

## Global Constraints

- **No change to Arbesk's data.** No new manifests, no changed compression, no changed CIDs, no changed wording.
- **No new runtime dependency.** Use `std::sync`, `HashMap`, the existing `in_parallel` and the existing cache/rename helpers.
- **Reads may use the config cache; writes may not.** `write.rs`, `create.rs` and `share.rs` use `Reader::open_fresh`; reads may use `ChainCache`.
- **Only the extension fast path may trust a manifest's own format claim:** `gltf` → `.gltf`, `3mf` → `.3mf`. Any other value (`glb`, unknown) falls back to reading the source, so source-only GLB and unknown formats keep today's result.
- **Thumbnails are content-addressed:** cache key = thumbnail CID + pixel size; bytes never change. Keep them in `cache_dir/arbesk-thumbnails`, outside `cache_dir/arbesk`, which `downloads::prune` treats as downloaded files and would send to the Trash.
- **Cap every cache:** 64 MB for manifests, 64 MB for Arbesk thumbnails.
- **No UI change:** no new visible text or layout. The mock does not exercise the Arbesk crate, so screenshots only prove the app still renders; real-account timing stays in #224.
- Every commit passes the pre-commit CRAP gate; never `--no-verify`.

## Review Focus

1. **A manifest's format claim agrees with the source.** A listing shows `Box.3mf` from the manifest's `3mf` claim without fetching the source; an older manifest without the claim still reads its source; a `glb` claim still falls back; a mixed/link scene keeps today's extension.
2. **The manifest cache still enforces its cap** across several `FileCache` instances and repeated puts, without a rescan per write; a damaged entry is repaired.
3. **The config cache is only a read shortcut.** Two listings share one `/api/v1/config`; a write is fresh; the lock is never held across the HTTP call.
4. **A part failure or a part over the build cap is still refused with the same wording**, and unknown formats / bad 3MF paths are refused before any part is fetched. Duplicate CIDs are charged per reference.
5. **A thumbnail survives a fake gateway that then stops answering**, a damaged cache entry falls back and is repaired, and a cancel still returns nothing even with a warm cache.

---

### Task 1: A bounded disk file cache, and no rescan per manifest

**Files:**
- Modify: `src-tauri/crates/arbesk/src/ipfs.rs`

**Interfaces:**
- Produces:
  - `pub struct FileCache` with `pub fn new(dir: PathBuf, cap: u64) -> Self`, `pub fn get(&self, key: &str) -> Option<Vec<u8>>`, `pub fn put(&self, key: &str, bytes: &[u8])`, `pub fn remove(&self, key: &str)`.
  - Keys are ASCII letters, digits, `.`, `_` and `-`, and are refused when empty, `.` or `..`; anything else is ignored like a bad CID.
  - `put` writes beside the target under a unique `.part` name (process-wide `AtomicU64` counter plus pid) and renames it in with `crate::cache::rename_new`; an existing entry wins. A damaged entry is removed by the reader before it re-fetches (see `Gateway::json` and the thumbnail code).
  - A process-wide `Mutex<HashMap<PathBuf, u64>>` tracks each directory's total bytes. It is scanned once per directory per process, then kept in step by `put` and reset by a trim. `trim` runs only when the total exceeds `cap`.
  - `trim` keeps the entry just written, deletes the oldest files first, ignores directories, removes `.part` files older than 10 minutes, and returns the remaining total.
  - `get` refuses (and ignores) a file larger than the cache cap, so a corrupt giant file cannot be read into memory.
- `ManifestCache` keeps its `new/get/put` API and delegates to a `FileCache`, still validating CIDs with `checked_cid`; it gains `remove(cid)`.
- `Gateway::json` removes the cache entry when the bytes do not parse, so the re-fetched manifest is stored again. `ManifestCache::put` still trims after a write.

- [ ] **Step 1: Write the failing tests** in `ipfs.rs`:
  - `a_file_cache_keeps_a_file_and_refuses_a_bad_key` — put/get round-trip; `""`, `.`, `..`, `../x`, `a/b`, `a b` and `bafy?x` return `None` and write nothing.
  - `two_caches_on_one_directory_keep_the_cap` — two `FileCache`s over one temp dir, a cap that holds three small files; write five through each; the files left on disk total at most the cap (plus the one just written).
  - `the_cache_drops_its_oldest_entries_past_its_cap` — the existing manifest test, still passing, using `File::set_modified` to give distinct times (no sleeps).
  - `a_cached_file_is_never_half_written` — one writer puts a large value in a loop while a reader gets it repeatedly; every answer is `None` or one of the two whole values.
  - `a_stale_part_file_is_removed_but_a_live_one_is_kept` — an old `.part` is gone after a trim; a just-created one stays.
  - `a_damaged_manifest_is_repaired` — put junk at a CID, list, then read the file back and assert it parses.
  - Keep `json_is_read_once_then_from_the_cache`, `a_damaged_cache_entry_is_fetched_again` and `a_manifest_over_its_cap_is_refused_and_not_cached` green.
- [ ] **Step 2: Run** `cd .worktrees/arbesk-responsiveness/src-tauri && cargo test -p arbesk ipfs::` — Expected: FAIL (no `FileCache`).
- [ ] **Step 3: Implement** `FileCache` and move `ManifestCache` onto it. Keep the doc comments explaining that the cache is only a shortcut.
- [ ] **Step 4: Run** `cargo test -p arbesk ipfs::` — Expected: PASS.
- [ ] **Step 5: Commit** `perf: stop rescanning the Arbesk manifest cache on every write`.

### Task 2: List an asset by the format its manifest already names

**Files:**
- Modify: `src-tauri/crates/arbesk/src/service.rs`
- Test: same file (`mod tests`)

**Interfaces:**
- Produces (private to `service.rs`):
  - `fn listed_format(manifest: &Value) -> Option<Format>` — first the first source node's `source.format`, then `metadata.computed.format`; only `"gltf"` → `Format::Gltf` and `"3mf"` → `Format::ThreeMf`; anything else `None`.
  - `Reader::asset_entry`/`name_of` uses `listed_format(manifest)`; only when it is `None` does it read `source_cid`'s manifest as today.
  - `Reader::listed_name` (used by `write::claim`) is **not** changed: it keeps reading the source manifest, so a paste's Replace decision is source-derived as today.
- A live link (`is_child_ref`) still wins first and shows `.gltf`; a mixed scene (a source plus a link, e.g. `bafyTable`) also shows `.gltf`, unchanged.
- Consequence to state in the PR: an asset whose manifest names its format now appears even when its source manifest is missing or temporarily unreachable (today a 404/410 source drops the row and a 502 source fails the listing). Build/export report the source error when the row is opened. This is intentional: one broken asset no longer hides the collection.
- Test helper: add `testing::put_asset_with_format(cid, name, source, format)`; do not change the shared `world()` fixtures, so existing request-count tests stay stable.

- [ ] **Step 1: Write the failing tests** in `service.rs`:
  - `a_listed_format_avoids_the_source_manifest` — a local world copy with an asset whose node says `source: { "cid": "bafyMissingSource", "format": "3mf" }` and no source CID in the world; listing Shapes shows `Box.3mf`; the request log has no `GET` ending in `bafyMissingSource`.
  - `a_listed_gltf_needs_no_source_fetch` — same with `"format": "gltf"`; the row is `Box.gltf`.
  - `an_old_manifest_without_a_format_still_reads_its_source` — the existing `bafySrcOdd` case still shows `Odd` with no extension; a node with no `source.format` and a 3MF source still shows `.3mf`.
  - `a_glb_claim_falls_back_rather_than_guessing` — node `format: "glb"`, source `bafySrcOdd` (an unknown composite) → `Box` with no extension, not `Box.gltf`.
  - `a_mixed_scene_and_a_link_keep_their_extension` — `bafyTable` stays `.gltf`, `bafyA3` stays `.gltf`.
  - `the_listed_extension_is_the_one_export_builds` — for the glTF and 3MF fixtures, the fast-path extension equals the one `format_of` gives the source.
  - Keep the other listing tests green.
- [ ] **Step 2: Run** `cargo test -p arbesk service::` — Expected: FAIL on the new tests.
- [ ] **Step 3: Implement** `listed_format` and use it only in the listing name path.
- [ ] **Step 4: Run** `cargo test -p arbesk service::` — Expected: PASS.
- [ ] **Step 5: Commit** `perf: list Arbesk assets without fetching their source manifest`.

### Task 3: Keep the deployment config for reads

**Files:**
- Modify: `src-tauri/crates/arbesk/src/chain.rs` (cache type), `service.rs` (`Reader` and read functions), `lib.rs` (`Client` holds it), `write.rs`, `create.rs`, `share.rs` (open fresh).
- Test: `chain.rs`, `service.rs`.

**Interfaces:**
- Produces in `chain.rs`:
  - `pub struct ChainCache` (`Default`) with `pub fn config(&self, http: &dyn Http, site: &Site) -> Result<ChainConfig, String>`.
  - Config entries live 5 minutes (`Duration::from_secs(300)`), keyed by `origin + chain_id`; a miss or an expired entry calls `config`. Only `Ok` answers are kept. The lock is dropped before the HTTP call; duplicate concurrent misses are harmless.
  - `#[cfg(test)] fn with_ttl(ttl: Duration) -> Self` so a TTL test can use `Duration::ZERO` without sleeping.
  - The map is swept of expired entries when it grows past 64.
- Produces in `service.rs`:
  - `Reader::open(http, site, cache_dir, cache: Option<&ChainCache>) -> Result<Self, String>` and `Reader::open_fresh(http, site, cache_dir)` (no config cache, same manifest disk cache).
  - `list`, `thumbnail`, `build`, `export`, `scene`, `versions` and `part` take `chains: &ChainCache` as their last argument before the `stop` closure for `thumbnail`: `(http, site, path, size, cache_dir, thumbs, chains, stop)`.
- Produces in `lib.rs`: `Client` gains `chains: chain::ChainCache`; every read passes `&self.chains`.
- `write.rs`, `create.rs` and `share.rs` call `Reader::open_fresh`; their behaviour is unchanged.
- `tokenURI` is **not** cached. Spec D6's “asked again on every listing and Reload” stays true, and Replace's `path_exists` keeps seeing the live collection.

- [ ] **Step 1: Write the failing tests:**
  - `chain.rs`: `the_config_is_read_once` (two `cache.config` calls, one `GET /api/v1/config`); `an_expired_config_is_read_again` with `with_ttl(Duration::ZERO)`.
  - `service.rs`: `a_listing_uses_the_cached_config` — list Shapes twice with one `ChainCache`; one `/api/v1/config` request (today: two).
- [ ] **Step 2: Run** `cargo test -p arbesk` — Expected: FAIL/compile errors from the new arguments.
- [ ] **Step 3: Implement**, then update every test call site to pass `&ChainCache::default()`; use `Reader::open_fresh` in write/create/share.
- [ ] **Step 4: Run** `cargo test -p arbesk` — Expected: PASS (all crate tests).
- [ ] **Step 5: Commit** `perf: keep the Arbesk deployment config for reads`.

### Task 4: Fetch a model's parts a batch at a time

**Files:**
- Modify: `src-tauri/crates/arbesk/src/compose.rs`
- Test: same file (`mod tests`)

**Interfaces:**
- Produces (private):
  - `fn part_cids(source: &Value) -> Vec<(String, usize)>` — unique CIDs and how many times the source references each: glTF `buffers[].uri` / `images[].uri` with `ipfs://`, 3MF `parts.*.cid`. Data URIs are not included.
  - `fn validate_source(source: &Value) -> Result<Format, String>` — `format_of` first; for glTF, every buffer/image URI must be `ipfs://` or a `data:…;base64,…` URI; for 3MF, the required `contentTypes`/`rootRels`/`model` strings and every `modelPath`/part path are checked with `part_path` before any network call. This keeps today's error order.
  - `Parts` keeps `remaining: RefCell<HashMap<String, usize>>` (references left), `queue: RefCell<VecDeque<String>>` (unique CIDs in source order), `fetched: RefCell<HashMap<String, Vec<u8>>>` and `left: Cell<u64>`.
  - `Parts::prefetch(&self, source: &Value)` fills `remaining`/`queue` after validation; it fetches nothing.
  - `Parts::fetch(&self, cid)`:
    - returns a prefetched part, charging no budget again; it removes the bytes at the last reference and clones only while an earlier reference still needs them;
    - when the CID is not fetched, takes it plus up to `PARALLEL_PARTS - 1` (3) next queued CIDs and fetches them with `in_parallel` (limit 4); charges `len * references` per CID; refuses with `TOO_LARGE_TO_BUILD` once the batch takes the budget below zero. This bounds over-fetch to one batch and lets the bytes be released as the assembly consumes them.
    - falls back to a single on-demand fetch for a CID the source did not declare, charging it once (so a caller can never silently skip a part).
- `compose_within` and `compose_preview_within` call `validate_source` first, then `prefetch`, then the existing assemblers. The output bytes must be identical to today's.
- `3MF` assembly keeps its existing path checks; validation moved ahead of the part loop, not removed.

- [ ] **Step 1: Write the failing tests:**
  - `parts_are_fetched_in_parallel` — a fake gateway with two parts that waits on a `Condvar` (with a 1 s timeout) for a second request while recording the peak; `compose_preview` sees peak `2` and both buffers in the GLB. A sequential implementation fails after the timeout instead of hanging.
  - `an_unknown_format_is_refused_without_fetching_parts` — `bafySrcOdd` / an unknown marker; the error is `UNKNOWN_FORMAT` and the fake logged zero part requests.
  - `a_bad_3mf_path_is_refused_without_fetching_parts` — a climbing `modelPath` or part path; `UNKNOWN_FORMAT`, zero requests.
  - `a_part_that_cannot_be_fetched_is_still_the_same_error` — one missing part; the existing storage error and no partial GLB.
  - `a_build_over_its_cap_is_refused_after_one_batch` — eight parts, a cap that fits two; the error is `TOO_LARGE_TO_BUILD`, and at most `PARALLEL_PARTS` part requests were made.
  - `a_duplicate_part_is_charged_for_every_reference` — a cap that fits one copy but not two; `TOO_LARGE_TO_BUILD`.
  - The existing compose cases (`gltf_matches_arbesk`, `a_gltf_previews_as_a_glb_holding_what_arbesk_builds`, `a_3mf_previews_as_itself`, the buffer/image tests) stay equivalent.
- [ ] **Step 2: Run** `cargo test -p arbesk compose::` — Expected: FAIL on the new tests.
- [ ] **Step 3: Implement** validation, prefetch and the on-demand batches; keep the assemblers' order.
- [ ] **Step 4: Run** `cargo test -p arbesk compose::` — Expected: PASS.
- [ ] **Step 5: Commit** `perf: fetch an Arbesk model's parts in parallel preview batches`.

### Task 5: Keep Arbesk thumbnails on disk

**Files:**
- Modify: `src-tauri/crates/arbesk/src/service.rs` (the thumbnail path and `Thumbnails`), `src-tauri/crates/arbesk/src/lib.rs` (`Client` constructs it)
- Test: same files

**Interfaces:**
- Produces in `service.rs`:
  - `Thumbnails` gains `disk: Option<FileCache>`; `Thumbnails::new(cache_dir: PathBuf)` uses `cache_dir/arbesk-thumbnails` with a 64 MB cap; `Default` keeps `None` for unit tests that do not need disk.
  - `Thumbnails::disk()` returns the cache for the thumbnail path; `clear()` still clears only the in-memory map (the disk cache is content-addressed and safe).
  - `service::thumbnail` checks `{cid}-{size}.png` before `gateway.image` and stores the PNG after a successful decode. A disk hit is validated as a PNG at most `size` pixels; a bad entry is removed and the gateway is asked. `stop()` is checked after a disk hit and before writing, so a cancelled request returns nothing even with a warm cache.
- `lib.rs`: `Client::with` constructs `Thumbnails::new(cache_dir.clone())`.
- `downloads::DIRS` stays unchanged; add a comment/test that `arbesk-thumbnails` (and `arbesk-manifests`) must not be added there, or stale thumbnails would go to the Trash.

- [ ] **Step 1: Write the failing tests** in `service.rs`:
  - `a_thumbnail_is_read_from_disk_the_second_time` — list Shapes (records the thumbnail CID), call `thumbnail` once with `Thumbnails::new(cache.path())`, assert the PNG file exists at `arbesk-thumbnails/<cid>-<size>.png`, then swap in a fake that answers every gateway request with `not_found`; call again and get the same bytes with no image request.
  - `a_damaged_thumbnail_cache_entry_falls_back_to_the_gateway` — corrupt the file after the first call; the second returns the real PNG and exactly one image request; the file is valid again afterwards.
  - `a_cancelled_thumbnail_with_a_warm_cache_returns_none` — `stop` returns true; the call returns `None` and reads no gateway.
  - Keep `a_listed_asset_shows_its_own_thumbnail_after_one_request`, `an_asset_saved_without_a_thumbnail_has_none_and_asks_nobody` and the other thumbnail tests green (they may pass `Thumbnails::default()` and skip disk).
- [ ] **Step 2: Run** `cargo test -p arbesk service::thumbnail` — Expected: FAIL on the new tests.
- [ ] **Step 3: Implement** the cache lookup/store around the existing image decode and validation.
- [ ] **Step 4: Run** `cargo test -p arbesk service::` — Expected: PASS.
- [ ] **Step 5: Commit** `perf: cache Arbesk thumbnails on disk`.

### Task 6: Documentation and full verification

**Files:**
- Modify: `AGENTS.md` (the `ipfs.rs`, `service.rs`, `chain.rs` rows: the disk file cache, the listing fast path, the config-only read cache)
- Modify: `docs/architecture.md` only if its Arbesk flow describes the reads this changes.
- Add a short `## Performance` note to `docs/arbesk.md`? Only if it helps users understand why a first visit is slower; do not promise numbers.

- [ ] **Step 1: Update the docs** with the cache lifetimes (config 5 min, manifests/thumbnails 64 MB) and the “reads may cache the config, writes are fresh; `tokenURI` is never cached” rule. Say plainly that the first visit to a collection still fetches one manifest per asset and that a copy-in still waits for the relay.
- [ ] **Step 2: Run** `npx tsc --noEmit`, `npm run build`, `npm test`.
- [ ] **Step 3: Run** `cd src-tauri && cargo test`.
- [ ] **Step 4: Run** `npm run crap` (the pre-commit hook does the language-specific version on commit).
- [ ] **Step 5: Look at the mock preview** (`npm run dev:mock`) and confirm no visible change; take before/after screenshots for the PR anyway (`node scripts/mock-screenshot.mjs ...`), and say they do not exercise the Arbesk crate.
- [ ] **Step 6: Commit** `docs: describe the Arbesk read caches and their limits`.

## Out of Scope (follow-ups, not this PR)

- Caching `tokenURI`. It needs a spec change (D6) and a decision about Replace/Reload staleness; the plan review rejected the first version's 30 s cache.
- Parallel IPFS uploads or changing brotli quality. A copy-in still waits for the relay's on-chain receipt; a single large GLB is one upload anyway.
- Progressive listing (showing rows before every manifest is resolved). It needs a streaming command and frontend work.
- An in-memory parsed-manifest cache. The manifest disk cache already avoids network on revisits; this can come later if large collections still parse too slowly.
- Changing the 8-way listing concurrency. Measure first; a public IPFS gateway may throttle.

## Verification

- `cargo test -p arbesk` — every crate test, including the new ones.
- `cd src-tauri && cargo test` — the app crate and the crate together.
- `npx tsc --noEmit`, `npm run build`, `npm test` — the frontend is untouched but the command surface must stay green.
- `npm run crap` — no new function over the CRAP threshold.
- Real-account verification remains in #224 (`needs-verification`): the timings above cannot be reproduced against a real Arbesk deployment from this environment. The PR's “Not verified” section must say what to time on Linux and macOS.
