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
  scenes (a part's own scene aside, below), versions, Properties and file lists never stop. Writes (save,
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
  build with its words; reads under way finish, and workers start nothing
  more past the failed part in source order (a cancel: nothing at all).
  Of several failed parts, the build tells the failure of the part the
  assembler asks for first (each part's failure is kept, and parts put
  back before the failed one are still fetched to their own verdicts), as
  when parts went in order; a worker that panics fails the build rather than keeping
  its slot (the panic still reaches the caller).
- A part of known size is read no further than that size
  (`Gateway::bytes_within` through `Http::get_within`, with the slack of
  `ipfs::within`): a source that understates a size can't start eight
  large reads at once. A part found larger than declared is put back,
  first, as a part of unknown size and fetched again in a pair, charged as
  it lands: the verdict is the one charging every part gives, and what is
  held never passes the cap by more than one part (or two of unknown size),
  as with fixed pairs.
- The shared byte budget: the limiter's `HELD` (512 MiB) counts the parts
  every build holds or has on its way. A part of known size waits for it
  unless its build has nothing on its way (so builds never wait on each
  other for ever); a part of unknown size is counted once it lands. A
  part's bytes go back when the assembler takes its last reference, or when
  the build ends. Each build still refuses past its own cap; a waiting
  build hears a freed budget at the next `TICK`.
- `Client::part` resolves the scene under the part's stop, so a Cancel is
  heard while its reads queue for permits, but never keeps a scene
  resolved once stopped: resolving leaves out what it can't read, and a
  read is only cancelled once the stop (which never goes back) says so, so
  such a scene could be missing parts.
- A shared read nobody else joined is handed back as it was read, never
  copied; only a read with followers is published for them to copy.
- Not changed: what is read and where results land, the frontend's caps
  (thumbnails 4, the copy-out plan 4).
- Tests: `limiter`'s own (the cap, a cancel while waiting, shared reads,
  their failures and cancels, a read nobody shares not copied, the shared
  byte budget, `bridged`), `ipfs`'
  `callers_of_one_cid_at_once_share_one_read`, `compose`'s
  `a_slow_part_does_not_idle_the_others`,
  `parts_in_flight_never_overshoot_the_budget`,
  `one_failed_part_among_many_fails_the_build`,
  `a_part_size_comes_from_byte_length_or_its_recorded_bytes`,
  `a_build_waiting_for_a_permit_stops_on_a_cancel`,
  `a_source_that_understates_its_sizes_cannot_overshoot_the_budget`,
  `a_worker_that_panics_does_not_hang_the_build`,
  `of_two_failed_parts_the_first_in_order_is_told`,
  `a_part_fetched_again_is_told_before_a_later_failure` and
  `builds_at_once_share_one_byte_budget`, and the client's
  `every_read_shares_one_limit` (a listing, a build and four thumbnails:
  12 reads at once without the limiter, at most 8 with it),
  `a_cancel_while_waiting_for_a_permit_returns_at_once`,
  `one_unreachable_read_still_fails_the_listing`,
  `chain_calls_never_pass_their_own_limit`,
  `a_file_download_waits_for_a_read_permit` and
  `a_cancelled_part_keeps_no_scene_missing_parts`.

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

**As built.**
- `scene::resolve_together` (what `service::scene_at` uses) reads first,
  then walks: `Known::read` takes the scene a level at a time from the
  asset's manifest, and asks each level's distinct source formats and
  links together through `in_parallel` (`READS` threads, a limiter
  permit per read); a link's job reads its `tokenURI` and collection, then
  the linked asset's manifest unless it is already read. A manifest is read
  at the shallowest level it is linked from, and its links are followed
  while that level is under `MAX_LINK_DEPTH`. Then the unchanged
  depth-first `resolve` walks those answers (`Known` is its `Links`: a
  failed read answers the walk's first ask with its failure and is read
  again at each later ask, as the serial walk reads again what failed (the
  gateway caches only what it read), so a read that fails once and then
  works leaves out no more than before; anything not read is asked then,
  as before), so the parts, their order, placements and omissions are the
  ones the serial walk gives. The one difference in what is read: a scene
  past the 2,000-placement cap may have the format of a source read that
  the walk, stopped by the cap, never asks for.
- Phase 2's rule holds as built: `Client::part` resolves under the part's
  stop and keeps no scene resolved once stopped.
- `arbesk_part` takes a `ticket` (`part_tickets::PartTickets`, one
  `Progress` per window and ticket) and `cancel_arbesk_parts(tickets)`
  cancels it: the fetch, waiting for a limiter permit or reading, stops and
  answers "Cancelled.". `arbesk_part` is async and registers its ticket
  only when the runtime first runs it, while the cancel runs at once, so a
  cancel can overtake its call: a cancel of a ticket not running is kept
  (the last 64) and that part starts cancelled. The frontend numbers its
  tickets from `Date.now()` at load, so a reload never reuses one a kept
  cancel names. `PartTickets` repeats `thumbnails::Requests`' shape rather
  than sharing a generic registry with it: thumbnails keep their own
  cancel behaviour (a cancel before start still reaches nothing there),
  which this phase leaves as it was; one registry for both is a follow-up.
- The frontend's window is `src/arbesk-parts.ts` (`fetchParts`,
  `PARTS_AT_ONCE` = 4). The limit is reserved per call in flight: a call
  gets its share of what the calls in flight and the parts landed but not
  yet parsed leave of `MODEL_PREVIEW_LIMIT`, split among the slots it can
  still fill. Parts are parsed one at a time in the scene's order, each
  part's bytes counted whether or not it parses, as before. A part refused
  as too large for a share smaller than what was left when its turn came
  is asked again alone once the calls in flight have landed, the parts
  after it let go (the backend keeps them built and answers them again
  from its cache): so what shows and *This file is too large to preview.*
  are what fetching one after another gives, and what is held and on its
  way never passes the limit. `loadArbeskParts` gives each call a ticket;
  `resetPreviewBody` (a closed or stepped preview) cancels the tickets
  under way, and calls still running once the budget ran out are cancelled
  too. A stale preview starts no more calls.
- Mock: `arbesk_part` honours its ticket (`cancel_arbesk_parts`);
  `__mockArbesk.addScene` adds a scene asset (a part may declare its size,
  `bytes`, refused past the limit), `holdParts`/`releasePart` hold the calls
  and answer them in any order, `heldParts` and `cancelledParts` show them.
- Tests: the Rust `a_scenes_reads_go_together_and_give_the_same_scene`,
  `reading_together_leaves_out_what_the_walk_leaves_out`,
  `a_scenes_sources_and_links_are_read_together` (the fake gateway's peak
  in flight above 1, the scene equal to a serial walk's) and
  `a_read_that_fails_once_is_read_again_as_the_walk_does`, and
  `part_tickets`' own (`a_cancel_before_its_part_starts_starts_it_cancelled`
  among them); the frontend's `tests/arbesk-parts.test.ts` (random
  scenes answered out of order against the one-at-a-time loop: same parts,
  same *too large*, the limit never passed) and
  `tests/arbesk-scene-parts.test.ts` (order and placements kept when calls
  answer out of order, the limit with calls in flight, a closed preview
  cancelling its calls, one failed part leaving the others), and the
  mock's case in `tests/mock-core.test.ts`.

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

**As built.**
- `memo.rs`: `TokenMemo`, one per `Client`, held by its `Limiter`
  (`Limiter::tokens`): the CID `tokenURI` answered per (RPC, contract,
  token), kept `KEPT` = 5 s (one operation's calls fit in it, and a save
  made in the Studio shows within seconds without a Reload), at most
  `MAX_TOKENS` = 256 (stale ones first, then the oldest, go). Callers of
  one token at once share one call (`Limiter::shared`, keyed with the
  memo's generation, so nobody after a clear joins a read from before
  it); a failure is never kept. Reads reach it through the `Limited`
  `Http` they are handed (`Reader::token_cid`, used by
  `collection_manifest` and a link's collection): writes, and attach's
  planning, read through the bare client and never see it, so
  `collection_manifest_with_cid`, what compare-and-swap uses, still reads
  `tokenURI` under the collection hold. Cleared (`Client::forget_tokens`)
  after every write returns, whatever its outcome (save, Replace, attach,
  detach, New Collection, and Share though it moves only the editor
  list), and before every listing: the backend can't tell Reload from
  navigation, and clearing more is always safe. A read that started
  before a clear answers its caller but is not kept. The preview's scene
  and its saves now usually read the same head; when they don't, phase
  1's rule holds (the preview pins the CID the scene answered).
- `Client::exists` is a names-only read (`service::lists_name`: the
  collection's names through `Reader::listed_names`, its `tokenURI` read
  afresh as the listing did; no rows, no thumbnail bookkeeping).
- A listing stands in for asking the backend only while it is current
  (`main.ts`: `wroteIn`, `windowReturned`, `unchangedSince`,
  `viewAnswersFor`). A count goes up with each write of this app into a
  folder (each item landed, `showLanded`; attach and remove, also when
  they fail, as a late Cancel may have written; rename; Move to Trash)
  and each time the window comes back (another app, or another window of
  this one, may have written anywhere; the view reloads then too), and
  with each drop from another app (`dropFromOtherApp`: a drop onto a
  background window does not focus it on macOS or under Mutter). The
  view's listing and the clipboard's summaries keep the count from when
  their listing started; a write into their collection since, or a
  return of the window, makes them unknown.
- `transferBatch` asks `takenNamesIn(destDir)` once per batch: for an
  Arbesk collection, the view's names when it is the folder on screen
  (listed without an error, and current), else one `list_dir`, with each
  name the paste lands added; the clash check and Keep Both's free name
  ask it. Elsewhere, and when that listing fails, `path_exists` as before.
  Names compare exactly, as `Client::exists` does. Each batch ends by
  reloading the view, so a paste queued behind another finds its names
  there; the count covers a reload that did not run or land first (one
  put off by a drag in the view, a navigation's listing that started
  before the landing).
- Copy keeps each copied asset's listing summary with the clipboard
  (`Clip.summaries`: `KeptSummaries`, the summaries of the assets the
  view's listing holds and that listing's count; a search result's is not
  kept). With every asset's summary known and current (`knownSummaries`:
  the folder on screen's first, else the clipboard's), `planArbeskCopy`
  asks `askFromSummaries`: no asset with files, no read and no question;
  one asset with files, its list read, then asked as before (the dialog
  shows each file and pins to its CID); several, the dialog built at once
  from the summaries' formats (`formatCounts`, the rows the full lists'
  `formatsOfFiles` give) while the lists of the assets with files are
  read, 4 at a time, and once the choice is made an asset it takes no
  format of (`choiceTakesFrom`) is no longer read; Cancel stops every
  read not yet started. A summary naming 32 formats (`mayOmitFormats`:
  the backend's summary stops at `MAX_REPRESENTATIONS`, a list another
  client stored may hold more; the mock's now stops there too) can't
  build the rows. Otherwise (`askFromLists`) every list is read first, as
  before.
- Correction to *Change* above: the lists are not read only after the
  choice. They start at once beside the dialog, so that a failed read or
  a cancelled row still ends the paste with the same words and no
  question, as the existing tests (`tests/arbesk-copy-out-files.test.ts`)
  pin: a failed read, or reads ending cancelled, withdraw the dialog
  (`chooseFormatsToCopy`'s `withdrawn`). What waits for the choice is the
  question itself and the reads not yet started. What is copied is still
  what the lists say now, pinned; every asset takes part in `copySources`
  (one not read with no files), so Models applies to each.
- No new command, so no new mock; the dialog looks as before (before and
  after screenshots of the one-asset dialog are identical).
- Tests: `memo`'s own (kept then read again, per contract and token, a
  clear, failures not kept, a read from before a clear not kept, one read
  for callers at once, the cap) and the client's
  `repeated_calls_for_one_collection_read_its_token_once`,
  `a_scenes_links_to_one_collection_read_its_token_once`,
  `a_listing_and_a_name_check_read_the_token_afresh` and
  `a_write_reads_the_token_itself_and_clears_the_memo`;
  `tests/arbesk-paste-names.test.ts` (no `path_exists` for a paste into a
  collection, one listing for a drop on one not on screen, Replace, Skip
  and Keep Both as before, a name landed earlier in the paste caught, and
  a paste made while the view hasn't caught up with an earlier one's
  names still asked);
  `tests/arbesk-copy-out-summaries.test.ts` (no read for assets without
  files, the question from the summaries while the lists are read,
  assets the choice takes nothing from not read, Models unchecked, the
  same rows from summaries as from lists, a file this app attached after
  the Copy still offered, one another app attached offered once the
  window is back, Cancel stopping the reads not started, a 33-format
  asset read first), cases in `tests/representations.test.ts` and the
  mock's 32-format summary in `tests/mock-core.test.ts`.

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

**As built.**
- Where the look-ahead lives: the frontend asks for each build ahead, the
  backend builds it. `arbesk_prefetch(path, ticket)` (`lib.rs`,
  `built_ahead`) runs `arbesk_exported`, the very `Transfer::export` the
  copy's own build (`copy_entry` → `exported_source`) runs on the same
  provider, into the app cache: the same cache keys
  (`cache/arbesk/<manifest CID>`, an other file's
  `cache/arbesk/files/<CID>`), the same limiter permits and byte budget,
  and `Builds` makes the copy's build of a source being built ahead wait
  for it and take its file (no part is read twice). `engine.rs`,
  `placement.rs` and `copy_entry` are untouched. A backend command fed the
  whole list would have had to learn from the copy which item it was
  placing, what it skipped and when it ended; the loop already knows.
- `src/arbesk-ahead.ts` (`buildsAhead`, `BUILDS_AHEAD` = 4): while
  `transferBatch` places item i, the items i+1 … i+4 not started yet are
  built ahead, if `builtBeforePlacing` says so (an Arbesk asset or one of
  its other files, copied, never moved, to this computer or Drive). The
  window counts every item, so a mixed batch never builds further ahead;
  an item the loop passes (Skip, a failure) is never built ahead later,
  and `placing(i)` cancels the builds of items before i still running (a
  Skip passes its item before its build ends). So at most 4 builds ahead
  run, plus the build of the item being placed, however many items a
  "Skip, apply to all" passes in a row, and `PartTickets`' 64 early
  cancels are never outrun.
  An asset's other files go by the paths the plan made, pinned to the
  CIDs the dialog showed.
- Placing is the loop as it was: one `copy_entry` at a time, in order, the
  clash question asked at the item's turn, Keep Both, Replace through the
  Trash, never overwriting. Nothing ahead writes outside the app cache.
- Cancel: runTransfer's cancel calls `batch.stopAhead` at once, and the
  batch's end stops what still runs (an item skipped): `cancel_arbesk_parts`
  with their tickets, which `arbesk_prefetch` registers in
  `part_tickets::PartTickets` (tickets from the preview's counter, so a
  cancel that overtakes the async call is kept). The batch does not wait
  for them to end: a read that can't be interrupted yet (phase 6) would
  hold the row. A build ahead is not a tracked operation, so it reports
  into no row and never keeps `restart_app` refusing.
- Correction to *Change* above: the progress row is today's. `measure`
  finds no size for an Arbesk source, so the row counts files as they are
  placed (*N of M files*); the builds ahead report into no row.
- A failed build ahead is told at its item's turn, never earlier, in
  today's words (*Could not copy “name”: …*). At the turn the copy waits
  for the item's build ahead if it still runs (`buildsAhead.turn`,
  `aheadFailure`): the copy's own build would have waited for it in
  `Builds` anyway. If it then fails, that failure is the copy's (the same
  `arbesk_exported`, the same words) and the copy does not build again:
  a slow failure (a gateway timeout) costs one wait, as on main, not the
  rest of the build ahead's plus a whole second one. A build ahead that
  failed before its turn is not told: the copy builds the item again, as
  it always did, so a failure that passed costs no more than today's
  single attempt. A cancelled build ahead, or a sign-in failure, leaves
  the item to the copy as before.
- A built copy never placed (a Skip, a Cancel, a later failure) stays in
  the app cache, marked and pruned as today.
- Mock: `arbesk_prefetch` builds a copy's source once (a second build
  waiting for the first, as `Builds`), honouring its ticket; `copy_entry`
  from Arbesk takes it or builds itself; `__mockArbesk.holdBuilds`,
  `releaseBuilds`, `failBuilds`, `failBuildOnce`, `builds`,
  `buildsRunning`, `buildsPeak`, `buildsAheadPeak`, `placements`, `placingPeak`,
  `resetBuilds`.
- `arbesk_prefetch` runs `under_ticket`: it registers the window's
  ticket in `PartTickets`, does nothing when its cancel came first, and
  hands the ticket's `Progress` to the build's `Cx`.
- Tests: the arbesk crate's
  `builds_ahead_overlap_and_the_copy_takes_what_they_built` (six assets,
  four ahead of the one placed: peak reads above 1 and within `READS`,
  each placement the file built ahead, no part read twice),
  `a_copy_waits_for_its_build_ahead_and_a_cancel_stops_the_wait` and
  `builds_ahead_share_the_byte_budget` (five builds under a one-part
  budget never have more than one part each on their way; more without):
  regression guards that pass on main too, since the backend's build
  ahead is the existing `Client::export` with no new code in the crate;
  the byte budget holds for a build ahead because `arbesk_prefetch` goes
  through `arbesk_exported`, the copy's own export on the app's one
  `Client` and limiter (no test drives the budget through the command:
  the app has no fake gateway). The app's
  `arbesk_commands_reach_arbesks_provider` (a build ahead in the copy's
  words) and `prefetch_ticket_tests` (a cancel before the start builds
  nothing; a cancel of the window's ticket stops a build reading within a
  tick, another window's or ticket's does not); `tests/arbesk-ahead.test.ts`
  (the window; seventy items skipped in a row never have more than 4 + 1
  builds running, each cancelled once; a turn takes a build still
  running's failure, not an earlier one's);
  `tests/arbesk-copy-ahead.test.ts` (six assets built five at once,
  placed one at a time in order, each built once; Cancel stops the builds
  ahead and lands nothing; a failing build ahead told only at its turn,
  in the copy's words; one failing once built again and landed; one still
  running at its turn told once, not built again; a Skip placing nothing;
  "Skip, apply to all" on twelve clashes stopping every build ahead, never
  more than 5 at once; other files built ahead by their pinned paths), and the
  mock's case in `tests/mock-core.test.ts`. No visible change: the dialogs
  and the row look as before.

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
