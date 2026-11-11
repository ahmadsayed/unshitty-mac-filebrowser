# Arbesk performance and parallel downloads

Status: proposed (2026-11-11)

Every Arbesk read goes to an IPFS gateway or the chain's RPC, at 100 ms to
several seconds each. An audit of the integration on `main` at 3bf68dd found
the places where those reads wait for each other with no need, and where the
app has no shared limit on how many run at once. This plan fixes them in six
phases, one PR each, in this order.

The numbers below assume 0.3 s per read. Line numbers are from 3bf68dd.

## Rules for every phase

- **No change to what is read, only to when.** Every fetch stays pinned to
  the same CID it is today: the preview's files stay pinned to the save on
  screen (F14 of the representations design), copy-out stays pinned to the
  CIDs the dialog showed, and a write still compares and swaps under the
  collection hold. Parallel reads never reorder what the user sees: results
  are placed by index.
- **One limit for the whole app.** No phase adds a private pool or raises a
  private cap; widening goes through the phase 2 limiter, so two features
  never multiply each other.
- **Cancel and progress keep working.** A parallel path listens to the same
  stop flag and reports the same progress row as the serial one did. A
  closed preview or a cancelled copy stops its queued reads.
- **Memory stays bounded.** Parallel builds share a byte budget, so N parts
  in flight never hold N times the 256 MiB preview or 512 MiB build cap.
- **Tests:** each phase has tests that fail on `main`: ordering, cancel,
  the limit actually holding (a fake gateway counts the peak in flight),
  and a failure of one read in a batch.

## Phase 1: the model starts before the version history

**Now.** `showPreview` (`src/main.ts:4925-4931`) waits for
`loadPreviewVersions` before it asks for the scene, so the first view of an
asset waits for the whole history walk: one gateway read per save, one after
another (`service.rs` `walk`, up to 50). 20 saves: about 6 s before the
first model byte is asked for; 50 saves: about 15 s. Each preview also reads
`tokenURI` up to three times, and `chain()` re-reads up to 50 cached
manifests to check a version it could match on the first.

**Change.**
- The scene starts at once from the newest save's manifest CID: the
  backend resolves it (one `tokenURI` + the collection manifest) and returns
  it with the scene (`arbesk_scene(path, null)` answers which CID it used).
  The version walk runs alongside and fills the version control when it ends.
- The files switcher (`loadPreviewFiles`) is pinned to the same CID the scene
  used, never to the walk's newest. If the walk's newest differs (a save
  landed meanwhile), the version control shows it and the preview stays on
  the save it is showing; nothing swaps under the user.
- `chain()` stops at the matching CID (newest first), and a version equal to
  the head just read skips the walk.
- When the head can't be read, the preview falls back to the asset as it is
  now with no switcher, as today.

**Gain.** Time to first model request: K + 2 serial reads become 3.

**As built.**
- `arbesk_scene` answers `{ cid, parts }` (`scene::SceneAt`, which derefs to
  `Scene`): `cid` is the save the scene was resolved from, the version asked
  for or, without one, the head `open_asset` read (`service::scene_at`).
  `Client::scene` keeps the scene under that CID only, the key the parts are
  asked by (a part asked for without a version resolves the newest scene
  again; the preview never asks so), so a first view takes one of the
  `KEPT_SCENES` slots, not two.
- `chain()` became `find_save(read, head, version)`: newest first, stopping
  at the match, within the newest `MAX_VERSIONS` as `versions` lists them;
  `scene_at` and `manifest_of` (so `arbesk_representations` and
  `arbesk_file`) skip it when the version is the head they just read. The
  same saves are accepted and refused as before. `walk` lost its
  `read_before` switch: `versions`, its only caller, always reads the save
  past `MAX_VERSIONS`.
- `showPreview` hands an Arbesk model to `previewArbeskFirst`: the scene is
  asked at once (before three.js loads), its `cid` pins the parts and
  `loadPreviewFiles` (now given a CID), and the parts are fetched by that
  `cid` only (never by path), and `loadArbeskVersions` draws the
  control when the walk ends and the scene has answered, at the shown save.
  The control belongs to the open preview (`previewOpened`), so it still
  comes if one of the save's files is shown by then.
- A failed scene (the head can't be read): *No preview available.*, no
  switcher; the control, if the saves read, at the newest, as before. A
  shown save the walk doesn't hold (the walk read the head before that save
  landed, the scene after) makes the saves be read once more, which then
  hold it; only if they still don't is there no control rather than a
  wrong *Version N of M*.
- Mock: `arbesk_scene` answers its `cid`; `__mockArbesk.saveAgain` lands a
  save from elsewhere; the delay switch for `arbesk_versions` is the
  existing `__mockCalls.hold`/`release`. Tests:
  `tests/arbesk-preview-head.test.ts`, the Rust `finding_a_save_*`,
  `a_version_reads_the_chain_only_as_far_as_itself`,
  `the_newest_scene_names_its_save` and
  `the_newest_scene_names_its_save_and_is_kept_by_it`.

## Phase 2: one limiter for Arbesk reads, and wider parts

**Now.** No single place caps Arbesk gateway reads. Listings run 8
(`service.rs:25`), one asset's parts 2 (`compose.rs:22`), thumbnails 4 in
the frontend (`src/thumbnails.ts:15`), the copy-out plan 4
(`src/main.ts:3324`). `ipfs::in_parallel` spawns scoped threads per call, so
the caps add up (about 16 in flight at a realistic peak) and nothing can be
widened safely. Inside one glTF or 3MF, buffers and textures go 2 at a time
in fixed pairs: a slow part idles its partner's slot.

**Change.**
- A `Limiter` in the arbesk crate: one per `Client`, 8 gateway reads in
  flight across every feature (listings, parts, thumbnails, files, versions),
  with a byte budget for parts held in memory. RPC calls get their own
  smaller limit (4). Waiting for a permit listens to the stop flag.
- Every read inside `in_parallel` takes a permit from it, one per read
  (not one per item: an item's work makes several reads, and a worker
  holding a permit while its next read waits for another could leave every
  permit waiting); `in_parallel` keeps only its threads.
- `Parts::fetch` becomes a sliding window through the limiter instead of
  fixed pairs of 2; parts are still assembled in order. The build's byte
  budget is reserved before a part is fetched (its size is known from the
  composite's `byteLength`, else the `_arbesk.bytes` asset-core records on
  every part, where present), so the cap can't be overshot.
- A single-flight map: two callers asking for the same CID at once share one
  read.

**Gain.** 12 parts: 6 rounds become 2; 40 parts: 20 become 5. Total reads in
flight never pass 8, whatever runs at once.

**As built.**
- `limiter.rs`: `Limiter` (one per `Client`, `Client::limiter`) with
  `READS` = 8 gateway permits and `RPCS` = 4 chain RPC permits; a wait
  for a permit hears the operation's stop every `TICK` (and a stopped
  operation starts no read). The limits travel with the `Http` a read is
  handed: `Client` wraps its client in `limiter::Limited`, which answers
  the new `Http::limits`; `Gateway::get` (manifests, parts, thumbnails) and
  `Gateway::raw` (an asset's other files) take a read permit, `chain`'s
  `eth_call` an RPC permit. The stop is the operation's `Progress` for a
  build, an export, a file and a part; a thumbnail's view stop, which may
  only be asked on its own thread, through `limiter::bridged`; listings,
  scenes, versions, Properties and file lists never stop. Writes (save,
  attach, detach, create, share, attach's planning) keep the bare client:
  unlimited, as before, so a save under its collection hold never queues
  behind a listing.
- Single-flight is `Limiter::shared`, keyed by the gateway URL, for the
  whole-body reads only: each caller gets its own copy, a caller whose
  shared read was cancelled by its leader's stop reads again for itself,
  and the answer is not kept once the read lands. A streamed download of
  an asset's other file is never shared (its progress and cancel are its
  caller's; one file's opens already take turns in `Builds`).
- `in_parallel` keeps `READS` threads for a listing's rows; the listing's
  `PARALLEL` is gone.
- `Parts::build` runs up to `READS` workers beside the assembler for the
  whole build (`compose::Window`). A part of known size starts while what
  landed, what is on its way and it fit the cap; alone it starts anyway and
  is charged when it lands, as before, so a size that understates is still
  caught and the verdict is the one charging every part after it lands
  gives. A part of unknown size (an older source that records none) goes
  as before: in pairs (`PARALLEL_PARTS` = 2) charged when both land, never
  beside a sized one. Any failure (a read, the budget, a cancel) fails the
  build with its words; reads under way finish, workers start nothing more.
- Not changed: what is read and where results land, the frontend's caps
  (thumbnails 4, the copy-out plan 4).
- Tests: `limiter`'s own (the cap, a cancel while waiting, shared reads,
  their failures and cancels, `bridged`), `ipfs`'
  `callers_of_one_cid_at_once_share_one_read`, `compose`'s
  `a_slow_part_does_not_idle_the_others`,
  `parts_in_flight_never_overshoot_the_budget`,
  `one_failed_part_among_many_fails_the_build`,
  `a_part_size_comes_from_byte_length_or_its_recorded_bytes` and
  `a_build_waiting_for_a_permit_stops_on_a_cancel`, and the client's
  `every_read_shares_one_limit` (a listing, a build and four thumbnails:
  12 reads at once without the limiter, at most 8 with it),
  `a_cancel_while_waiting_for_a_permit_returns_at_once` and
  `one_unreachable_read_still_fails_the_listing`.

## Phase 3: a scene's models load together

**Now.** A scene made of several stored models (or linked assets) resolves
depth first on one thread (`scene.rs`, `service.rs:757-796`: one source read
per model and one `tokenURI` + manifests per link), then the frontend loads
parts strictly one after another (`loadArbeskParts`, `src/main.ts:5101-5121`:
`for … await arbeskPart`). 10 models × 4 parts: about 20 serial rounds.

**Change.**
- `resolve` fetches each level's distinct sources and links together,
  through the limiter, and places them in manifest order.
- The frontend runs 3-4 `arbeskPart` calls at once and places results by
  index (`scene.parts` order and placements unchanged); three.js parsing
  stays serial. `MODEL_PREVIEW_LIMIT` is reserved per call in flight so it
  can't be overshot. One failed part still leaves the others.
- `arbesk_part` gets a cancel ticket (as thumbnails have), so a closed or
  stepped preview stops queued and running fetches.

**Gain.** 10 × 4 parts: about 20 rounds become 5; 30 single-part models:
30 become 4.

## Phase 4: fewer chain reads, one name list per paste

**Now.** Every per-asset action re-reads `tokenURI` (never cached): a scene
with 50 linked assets pays 50 serial chain reads every preview, and the
copy-out plan one per asset. A paste into a collection runs `pathExists`
per file (`src/main.ts:3578`), and `Client::exists` is a whole listing
(`lib.rs:562-570`): about three listings of the collection per pasted file.
The copy-out plan asks `arbesk_representations` for every asset before
anything starts (`src/main.ts:3310-3353`), even when the listing already
says no asset has other files.

**Change.**
- A short memo of `tokenURI` → collection manifest CID per (contract, token):
  shared by the calls of one operation, kept a few seconds across calls,
  cleared by this app's own writes and by Reload. A write never trusts it:
  the compare-and-swap re-reads under the collection hold as today.
- Paste into a collection takes the collection's names once per batch (from
  the view when it is the folder on screen, else one listing) and adds each
  landed name to that set; `exists` gets a name-only path that skips entries.
- The copy-out plan skips the read for an asset whose listing summary says it
  has no other files, and builds the several-asset dialog from the
  summaries' formats; full lists are read only for the assets that have
  files, after the choice.

**Gain.** Copy-out of 200 assets with no files: about 15 s of planning
becomes none. Paste of 200 files into 5,000 assets: 200 listings become 1.

## Phase 5: copy several assets out at once

**Now.** Copy-out runs one asset at a time (`transferBatch`,
`src/main.ts:3558`, one batch at a time app-wide), and an asset's other
files one file at a time. 20 assets × 6 parts: about 100 serial round trips.

**Change.**
- The backend builds (downloads and assembles) up to 4 assets ahead of the
  one being placed, through the limiter, into the app cache. Placement
  (`engine.rs` / `placement.rs`) stays one at a time and in order, with the
  same never-overwrite, Keep Both and Trash rules: only the download moves
  ahead, never the write to the user's folder.
- Representations copied with their assets are prefetched the same way,
  pinned to the CIDs the dialog showed.
- One progress row as today, counting bytes downloaded; Cancel stops the
  builds ahead and the placement. A built copy that was never placed is
  left in the cache (pruned as today), never in the target folder.

**Gain.** 20 assets × 6 parts: about 100 round trips become 20 (30 s → 6 s).

## Phase 6: large parts download on a timeout that fits their size

**Now.** Model parts are read with `Gateway::bytes` on the 30-second client
(`ipfs.rs:396`, `http.rs:207-218`): a buffer that takes longer than 30 s to
arrive fails the whole build, so a 40 MB part needs a steady 11 Mbit/s, and
every retry starts again. The fetch can't be cancelled mid-read.

**Change.**
- Parts are read with `get_streamed` and a timeout scaled to the expected
  size (the composite's `byteLength`, else a generous default) plus a stall
  timeout, as `Gateway::raw` already does for representations.
- The read checks the stop flag between chunks, so Cancel and a closed
  preview stop it.
- A part over the preview or build cap is refused when its size is known,
  before it is downloaded.

**Gain.** Large models preview and copy on ordinary connections; a cancelled
preview stops downloading at once.

## Not in this plan

From the same audit, for later: a disk cache of parts by CID (preview then
export downloads twice), LRU eviction with slack for the manifest cache,
streaming rows of a cold 2,000-asset listing, a listing cache for focus /
Back / tab switch, version steps that reuse an unchanged model, thumbnails
that cache the original and don't drop cancelled downloads, moving the
`.gltf` check of Paste Into Asset off the main thread, a size cap on
`cache/arbesk`, placing thousands of scene clones in slices.
