# Other files of an Arbesk asset (representations) — design

Date: 2026-11-10 · Status: done (all five phases built; owner asked for
full automation: every decision below is taken; nothing waits on an answer). Builds on Arbesk's asset representations design (in the Arbesk repository,
`docs/superpowers/specs/2026-11-10-asset-representations-design.md`, R1–R8,
merged in Arbesk #139) and on this app's
[provider crates](2026-11-10-provider-crates-design.md),
[copy-in](2026-11-09-arbesk-copy-in-design.md) and
[preview versions](2026-11-09-preview-versions-design.md) designs.

## Goal

A design has many files: the Blender `.blend` it is edited in, a 3MF and an
STL to slice, the GLB the Studio shows, the G-code a slicer made for one
printer. Arbesk now keeps them **on the asset itself**: an asset manifest
has a `representations` list (Arbesk R1), each entry a file with a role, a
format and what it was made from. File Browser shows **one file per asset**,
as today, and lets the user reach the others where a task needs them:

- the view says an asset has other files, and when one of them is out of
  date (made from an earlier model);
- **Open With** a slicer asks which file (3MF, G-code or STL); Open With
  Blender opens the `.blend`; plain **Open** still opens the model;
- **Copy** an asset out asks which files to copy (the model, all of them, or
  the ones chosen);
- **paste or drop** a file from this computer onto an asset adds it to the
  asset (a new version, as Arbesk R4 says); **Properties** lists the files
  and removes one;
- **Space** switches between the model and its other files.

Arbesk-native, as the owner prefers: the files live in Arbesk, in the
manifest, never as sidecar files beside the asset in the listing.

Out of scope here, each its own later change: **Print** (sending the G-code
to a printer) and a **G-code viewer**. F17 says how Print plugs in.

## What Arbesk stores (from its code)

- `packages/asset-core/src/manifest/schema.ts:100-187`: the entry schema.
  `id` `[a-z0-9-]{1,64}`, never `primary` (the asset's own model,
  `scene.nodes[0].source`, never in the list); `role` one of `source`,
  `mesh`, `view`, `machine`, `other`; `format` `[a-z0-9]{1,16}` (the
  lower-case extension); `name` 1–255 (UTF-16 units, Zod's `string.max`);
  `cid`; `bytes` (decoded size, an integer, at most 256 MiB); `encoding`
  `raw` or `brotli`; optional `derived_from {ref, cid, transform_matrix?}`;
  optional `tool`, `profile`, `stats` (at most 32 keys, string or number
  values); `created_at` (a number, milliseconds: `z.number()`). Entries are `passthrough` (a newer client's keys are
  kept). At most 32 entries, ids unique.
- `packages/asset-core/src/manifest/representations.ts`:
  `listRepresentations` (:68) is tolerant: an entry that doesn't parse is
  left out, the rest are shown. `currentCidOf` (:82): for `primary`,
  `scene.nodes[0].source.cid` when it is a non-empty string, else null; for
  another id, the `cid` of the **first valid** entry with that id
  (`listRepresentations(…).find`), so a ref to an entry this client can't
  parse reads as gone. `primaryTransformOf` (:97): null without
  a node 0, its `transform_matrix` (16 values, `map(Number)`) or the
  identity. `isStale` (:132): no `derived_from` → never stale; the ref's
  current CID differs → stale; for `primary` with a recorded
  `transform_matrix`, a different matrix → stale. `hasRoomFor` (:146)
  counts the **stored** entries, unparseable ones included, and matches ids
  on them (`entry?.id`). `withRepresentation` (:161) replaces by id in place,
  matching **stored** ids too, and keeps entries it can't parse;
  `withoutRepresentation` (:181) drops the key when the list empties.
  `manifestSchema` refuses a list with a duplicate id (`superRefine`,
  schema.ts:155), and its top-level `version` is `string | number` (:170). `representationId` (:203) is `source` for a source file, else
  the format (then printer and material when a profile names them).
  `defaultRole` (:54) and `encodingFor` (:59) are the format tables.
- `representation-codec.ts`: text formats are stored brotli-framed
  (`ARB\x01` + a brotli stream, quality 5), the rest raw; decoding follows
  the entry's `encoding` and **never sniffs** a raw file.
- `packages/besk/src/representations.ts`: `besk attach` checks everything
  before uploading (size :182, room :193, the entry as it will be recorded
  :204), warns before a `source` upload (:47, R7), uploads, then writes the
  next asset version (`nextVersion`, :107: `versionOf + 1`,
  `prev_asset_manifest_cid`, timestamp; `versionOf`, :92, is the top-level
  `version` when it is a JS number, else 1) and replaces the collection
  entry **only while it still names the CID it built from**
  (`replaceAssetEntry`, `catalog.ts:177-200`, error code `CONFLICT`). An
  entry `besk upload` stored as a bare composite is wrapped first
  (`asAssetManifest`, :101, any manifest without a `scene.nodes` array) by
  `compositeWrapper` (:80): `{type: "asset", name, asset_id, version,
  scene: {nodes: [{node_id: "node_1", transform_matrix: identity, source:
  {cid, format, path}}]}}`, `path` `composite.gltf` / `composite.3mf.json`,
  `format` from `detectFormat`, `name` the composite's `name` else the
  listed name, `version` `versionOf(composite)`. A child_ref asset has a
  `scene.nodes` array, so it is attached to as it is. `derived_from`
  defaults to `primary` for `machine` and `mesh`, nothing for the rest, and
  nothing (silently) when the model has no source CID (:166-180), as on a
  child_ref asset. `nextModelEntry` (:337) carries the previous version's
  list into a new model's wrapper only when it is a non-empty array; the
  Studio's `carryRepresentations` (`manifest-builder.ts:751`) copies any
  array and deletes the key otherwise.

## Where this app stands

- **Listing reads every asset manifest already.** `Reader::asset_entry`
  (`crates/arbesk/src/service.rs:744-755`) fetches the asset's manifest
  (through the manifest cache) to name the row and find its thumbnail;
  `entry` (:833-843) keeps only name, path and date.
- **`Entry`** is `places::Entry` (`crates/places/src/entry.rs:8-22`), whose
  only provider-specific key is Drive's `cloud`; Arbesk's own `Entry`
  (`crates/arbesk/src/lib.rs:63-71`) becomes one in `place_entry`
  (`provider.rs:122-133`).
- **Opening an asset** opens Arbesk's Studio in the browser
  (`provider.rs:169`); Space gets a built `.glb`/`.3mf` (`service.rs:253`);
  Copy out builds the asset's own file (`service.rs:268`, `Transfer::export`).
  Open With… is offered for local items only
  (`src/main.ts:2542-2552`) and `open_with` takes a local path
  (`src-tauri/src/lib.rs:579-582`); the chooser lists every application
  (`platform::applications`, `platform.rs:144`) and knows no MIME types.
- **Gateway reads sniff.** `Gateway::bytes` (`ipfs.rs:352`) runs `unpack`
  (:36-58), which unpacks anything that starts with the brotli frame or the
  gzip magic: right for parts, wrong for a raw representation.
- **Writes**: `write::add` (`write.rs:409`) uploads, then under the
  collection's lock (`CollectionLocks`, :169-280) re-reads the settled
  collection (:582) and relays. **Replace builds the new version from
  scratch** (`put_asset` :602 → `asset_manifest` :85-117, its own shape:
  node `type: source_asset`, a node `name`, `post_processor`, `path` per
  format): today it would drop an asset's representations, against Arbesk
  R3. F15 fixes that first. Its version is `old["version"].as_u64()` + 1,
  else 2, which differs from besk for a float or negative number.
- **Menus**: an Arbesk item's context menu is `arbeskItemMenu`
  (`main.ts:2251`, via `ITEM_MENUS.arbesk`): Open, Copy and, for a
  collection, Share Collection…. It never calls `previewAndOpenWithItems`
  (:2544), `propertiesMenuItem` (:2863) or the Paste / Paste Into Folder
  items (:2669).
- **Drops**: `DropTarget` is `{ dir, el }` and `dropTargetAt` (:3691) takes
  an item only when it is a folder; `dropOp` (:3721) sends every Arbesk
  target through `arbeskPasteRefusal`, and `performDrop` (:3767) always
  runs `runTransfer` (copy-in for Arbesk). Drops from other apps
  (`onDragDropEvent`, :4021) use the same `dropTargetAt`.
- **Clipboard**: `{ paths, dirs, mode }` (:3133). A paste names each target
  `basename(from)` and makes it safe (`safeLocalName`) only for Drive
  sources (`transferBatch`, :3417).
- **Properties** refuses every Arbesk path (`lib.rs:754`); the frontend's
  dialog (`showProperties`, `main.ts:2975`) skips the local-only rows only
  for Drive (`isCloud`), and its type line is `fileTypeDescription(entry)`,
  not a backend field. `places::FileProperties` (`info.rs:23`) has no
  version; `places::ext::Properties` (`provider.rs:452`) is an empty slot.
  `PROPERTIES_ITEM` (`item-actions.ts:139`) is one cell for every Arbesk
  path (account, collection, asset).
- **Open With**: `AppChoice.id` is the `.app` path on macOS and the
  `.desktop` **file path** on Linux (`platform.rs:137`, read by hand), not
  a GIO desktop id. The chooser launches on a row click or Open inside
  `dialog.run` and stays open on failure (`open-with.ts:170`);
  `hooks.opened` adds the opened path to Recent.
- **Saves**: `places::Version` is `{ id, timestamp }` (`records.rs:14`);
  the walk (`service.rs:369` `chain`) stops at `MAX_VERSIONS` or at the
  first manifest it can't read.
- **Emblems**: `icons.ts:278-317` decides which emblems an icon carries
  (link bottom-left, versions bottom-right; a thumbnail carries only the
  link); `.emblem` and `.emblem-versions` in `styles.css:644-668`.
- **Capabilities**: one row per kind (`providers.rs:207` `KINDS`,
  `tests/capabilities.json`, `src/capabilities.ts`); refusals are
  `places::Operation` (`provider.rs:32-42`), `can_copy_in` is derived from
  them (`providers.rs:168-190`).

## Decisions

### Reading

**F1. One Rust module owns the format: `crates/arbesk/src/representations.rs`.**
It mirrors asset-core's helpers on `serde_json::Value`, so a manifest is
never re-serialised through a struct that would drop keys:

- `list(manifest) -> Vec<Rep>`: the valid entries in stored order, each
  checked exactly as `representationSchema` checks it (the rules above,
  `name` counted in UTF-16 units, `bytes` an integer-valued number,
  `derived_from.ref` `primary` or an id). Tolerant like
  `listRepresentations`: a bad entry is left out, never an error. `Rep`
  keeps the parsed fields; the entry's raw `Value` is never rebuilt from it.
- `stored_count(manifest)` and `stored_ids(manifest)`: the stored entries,
  valid or not, and the `id` of each that has a string one (`hasRoomFor`'s
  `entry?.id`). **Every id decision** (F10's clash check, Keep Both's free
  id, add or replace in F11) uses `stored_ids`, never `list`.
- `current_cid_of(manifest, ref)`, `primary_transform_of(manifest)`,
  `is_stale(manifest, rep)`: exactly `currentCidOf`, `primaryTransformOf`
  and `isStale`. The primary is `/scene/nodes/0/source/cid` as a non-empty
  string, **not** `compose::source_cid` (`compose.rs:60`), whose fallbacks
  for older manifests would answer "not stale" where asset-core says stale.
  Another ref is looked up **only among `list`** (the valid entries) and
  the first match wins, as asset-core does, so a ref to an entry this
  client can't parse is stale.
  The matrix applies ECMAScript's `ToNumber` to each JSON value: `null` 0;
  `false` 0, `true` 1; a number as is; a string by `StringToNumber` (trim
  JS white space and line terminators; empty → 0; `0x`/`0X`, `0o`/`0O`,
  `0b`/`0B` followed by digits of that base, unsigned; `Infinity`,
  `+Infinity`, `-Infinity`; otherwise a whole decimal literal with optional
  sign, fraction and exponent, as `1`, `-.5`, `5.`, `1e3`; anything else,
  `inf`, `nan`, `1_000` included, NaN), so Rust's `str::parse::<f64>` is
  not used; an array → `ToNumber` of its `join(",")` (each element
  stringified as JS does, `null` as empty), so `[]` is 0, `[7]` and
  `["7"]` 7, `[1,2]` NaN; an object → NaN. Comparison is `f64` equality, as
  `===` is (NaN never equals, `-0` equals `0`).
- `version_of(manifest)`: besk's `versionOf`: the top-level `version` when
  it is a JSON number (any form, read with `as_f64`: `3`, `3.0`, `-1`),
  else 1 (a string, missing). The next version is `version_of + 1`,
  written as an integer when it is one (`4`, not `4.0`). Used by F11 and
  by `put_asset`'s Replace (F15), so every write from this app agrees with
  besk.
- `derived_from_of(manifest, ref)`, `default_role(format)`,
  `encoding_for(format)`, `representation_id(role, format, profile)`: the
  same tables as `representations.ts:36-56` and `:203`.
- `decode(bytes, rep, cap)` and `encode(bytes, format)`: by the entry's
  `encoding`, never sniffing: `raw` as is; `brotli` must start with the
  `ARB\x01` frame (else *Arbesk sent a damaged file.*), unpacked with the
  `brotli` crate and stopped past the cap, as `unpack` does. Encoding uses
  quality 5, window 22 (what `compressAuto` with the `binary` hint writes,
  as `decompose.rs:278` already matches for parts).
- `Gateway::raw(cid, cap)` (`ipfs.rs`) fetches without unpacking, for
  representations only; parts keep `bytes`.

**Staleness is tested against Arbesk's own cases.** A fixture
`crates/arbesk/tests/fixtures/representations-stale.json` holds asset-core's
`isStale` and `listRepresentations` cases (copied from its tests, with the
Arbesk commit they came from), and both a Rust test here and nothing else
read it; when asset-core's rules change, the fixture is re-copied in the
same PR as the Rust change. It also holds node matrices whose values need
`ToNumber` (`""`, `"  "`, `"0x10"`, `"Infinity"`, `"inf"`, `"nan"`,
`[]`, `[7]`, `["7"]`, `[1,2]`, `{}`, `true`, `null`), each with the answer
asset-core's `isStale` gives (generated by running it, not by hand), and a
ref to an unparseable entry (stale).

**F2. The listing carries a summary, from the manifest it already reads.**
`asset_entry` (`service.rs:744`) computes, from the manifest in hand, a
`places::Representations { count, stale, formats }`: the number of valid
entries, how many are stale, and their distinct formats in stored order. No
extra request per item: the manifest is the one the row's name comes from.
The summary comes from **any asset manifest's** list, a `child_ref` asset's
included (`compose::is_child_ref` only stops building it, `service.rs:307`;
besk attaches to it and the Studio carries its list): its entries derived
from `primary` read stale, exactly as `isStale` says, since
`current_cid_of(primary)` is null there. An asset with no list (or none
valid) has no summary, and so has a bare composite entry (`besk upload`'s),
which has no `representations` key.

`places::Entry` gains `representations: Option<Representations>`, serialised
only when present (`skip_serializing_if`), like `cloud` (D8 of the provider
design: existing JSON is unchanged for every other entry). It is named for
what it is, not for Arbesk, so another place with "other files of one item"
can fill it; today only Arbesk does, and `place_entry` copies it from
Arbesk's `Entry`, which gains the same field. The frontend's `Entry`
(`src/api.ts:5-25`) gains `representations?: { count: number; stale:
number; formats: string[] }`. At most 32 short strings per row (`formats`
stops at `MAX_REPRESENTATIONS` however long a list another client stored):
a collection of 1,000 assets with full lists adds well under 200 KB to its
listing. Staleness is worked out in one pass per manifest (the model's CID
and placement, and an id-to-CID map that keeps the first valid entry of
each id, as `currentCidOf` finds it), never one list per entry.

**F3. The full list is one command, from the cache.**
`arbesk_representations(path, version?) -> RepresentationInfo[]` reads the
asset manifest (the current one, or the save `version` names, a manifest
CID from `arbesk_versions`) through the manifest cache, so after a listing
it costs one `tokenURI` read (the collection's current CID; for a `version`
none) and no manifest request. Each item: `id`, `role`, `format`, `name`, `bytes`,
`stale`, `derived_from` (the ref only), `tool`, `profile`, `stats`,
`created_at`, and (from phase 3) `path`, the file's F4 path pinned to its
CID (F4), which the frontend puts on the clipboard and fetches, so it never
builds a path itself. The model is not in the list; the frontend shows it as the
first row from the entry itself. It is an extension of Arbesk's provider,
`ext::Representations` (D4 of the provider design: what only one place has
is an extension), with the methods F5, F10 and F12 add.

**F4. A representation has a path: one segment below its asset.**
`arbesk://<address>/<token>~<name>/<asset>~<name>.<ext>/<rep-id>~<rep name>`.
`location.rs` gains `ArbeskPath::File { address, token_id, asset_id, id,
pin }` (`pin`: the CID of a pinned path, below);
`is_folder` stays false for it and for the asset (an asset is still a file:
listing it is refused as today, `service.rs:48`). The id decides, the name
is for display (as everywhere in `location.rs`). Ids are `[a-z0-9-]`, so
nothing needs encoding. The segment always has its `~`, as the backend
writes it, so a fourth segment without one (`…/Box.gltf/more`) stays a
malformed path, as it was before this design. The name segment is the entry's `name` **made safe
first** (F5's `file_name` rules: no `/` or NUL, never empty, `.` or `..`;
the frontend also applies `safeLocalName` when it names a local target),
then encoded with `NAME_SET`, so every path a paste sees already names one
plain file: a representation named `parts/Bracket.gcode` or `..` can never
decode into a target outside the chosen folder.

Such a path means the **current** version's entry with that id. A path
may also **pin** the entry's CID: `<rep-id>@<cid>~<name>` (CIDs are
alphanumeric, ids never hold `@`). A pinned path means "that entry while
its CID is still `<cid>`": a fetch or copy of it whose current entry has
another CID, or none, is refused with *“Bracket.gcode” changed in Arbesk
since it was copied.* An unpinned path whose entry is gone says *This file
is no longer in the asset.* Everything that hands a representation to a later
step pins it (the Files… page's Copy, F9's sources), so a Studio save or
`besk attach` between Copy and Paste can't swap in a different file. These
paths are what the clipboard, the copy engine, a conflict dialog and an
operation's progress see. Nothing lists them, so they never reach stars,
bookmarks, Recent or tabs.

In the frontend, `paths.ts` gains `isArbeskFile(path)` (four segments, the
fourth with a `~` and, decoded, a valid id and optional pin before it, as
the backend's `file_id` parses it: `[a-z0-9-]{1,64}` but `primary`, then
`@` and letters and digits;
`isArbeskAsset` stays exactly three), with rows in `tests/paths.test.ts`;
`placeOf` answers `arbesk` for it. `locations.ts`' `arbeskRow` returns a new
capabilities row **`arbesk-file`** for it (a row in
`tests/locations.test.ts`), so it never falls through to the account's row
(whose `can_copy_out` is false). The row lands in phase 1 with the paths, in
`KINDS`, `tests/capabilities.json` and `capabilities.ts` together, since
`isCapabilityTable` requires every row: an asset's booleans but
`has_thumbnails` false (and `can_attach` false, F13). The mock's
`arbeskIds` (`core.ts:1695`) also returns the file id and the pin, refusing
what the backend's `file_id` refuses, and the mock's CIDs are letters and
digits, one per asset and file. Phase 1 refused an F4 source in
`place::copy` (*Copying an asset's other files out comes next.*) until
F9's copy landed in phase 3, which routes it out as an asset.

**F5. Fetching a file decodes it into the app cache, as an opened build.**
`fetch(<file path>, Fetch::Open | Fetch::Preview)` and the extension's
`file(path, version?, id, cx)` read the entry, fetch its CID with
`Gateway::raw` (streamed with `Http::get_streamed` on the slow client,
given `transfer_timeout` of the entry's `bytes` as an upload of that size
is: four minutes plus 64 KiB/s; encoded cap: 256 MiB + 1 MiB for a `brotli` entry, which
covers the frame and brotli's worst-case growth of incompressible input, a
few bytes per meta-block; 256 MiB for a `raw` one; *This file is too large
to open.* past it too), decode it (F1; decoded
cap exactly 256 MiB, *This file is too large to open.* past it), check
that the decoded length equals the entry's `bytes` (else *Arbesk sent a
damaged file.*) and save it under
`cache_dir/arbesk/files/<cid>/<name>` (a `cid` that isn't letters and
digits is refused with *Arbesk sent a damaged file.* before it names a
folder) with the `.downloaded` mark
(`places::mark`), through `build_file` (`service.rs:452`): an unchanged
copy is reused; one someone edited is kept, and a fresh copy goes beside
it. Keyed by the file's CID, so two versions sharing a G-code fetch it
once. `downloads::DIRS` already holds `arbesk` (`downloads.rs:11`): pruning
after a week deletes unchanged copies and sends edited ones to the Trash,
as for Drive downloads. Progress and Cancel work as for an opened Drive
file (`open_target`, `lib.rs:520`): the provider reports progress for
these fetches chunk by chunk as they arrive (the total is the decoded
`bytes`; what decoding adds is counted at the end), and a cancel stops the
download between chunks, or within a moment while the gateway sends
nothing (the read runs on a thread of its own, and the caller looks at the
cancel every 200 ms). A second open of the same file waits for the first
(one build per folder), and a cancel stops that wait too. The file name is the entry's `name` made safe with
`file_name`'s rules (`service.rs:847`: no `/` or NUL, never empty, `.`
or `..`), then cut to 200 bytes at a character boundary, keeping its
extension (a valid 255-unit name may take 765 bytes of UTF-8; the `.part`
and the mark beside it need the rest of the 255 a file system takes).

Edits to an opened copy stay in the cache, as for Drive and SSH files: File
Browser uploads nothing back. To keep them, the user saves elsewhere and
attaches the file (F10).

### Showing

**F6. One emblem, "has other files", with an out-of-date dot.** Drawn from
scratch as a 16 px symbolic glyph, `emblem-files`: two sheets, the back one
offset up and right, the front one with a folded corner (Adwaita's style:
2 px strokes, round caps, `currentColor`). It is not the versions stack
(`emblem-versions`, three flat layers) and not a paperclip (which reads as
mail). `Emblems` (`icons.ts:280`) gains `files: "none" | "fresh" |
"stale"`; `itemIcon` (:306) adds `<span class="emblem emblem-files">` after
the shared emblem, at the bottom-right, where `.emblem-versions` sits (an
Arbesk asset never keeps versions, so they never meet). Unlike the
versions badge it shows **on thumbnails too** (an Arbesk asset usually has
one), as the link emblem does. `stale` adds the class `stale`: a 6 px dot
at the emblem's top-right, in a new token `--warning` (GNOME theme:
libadwaita's warning colours, `#9c6e03` light and `#f8e45c` dark; macOS
theme: system yellow, `#b25000` light and `#ffd60a` dark), ringed with
`--view-bg` so it reads on any picture. Its `title` (the tooltip, and the
accessible description) says *Has 3 other files* or *Has 3 other files, 1
out of date* (`1 other file` in the singular).

`iconHtml` (`main.ts:1651`) fills `files` from `entry.representations`: no
request, no state, so a row the virtualized view renders, recycles or
redraws (`patchIcon`) always draws the same markup from its entry. The
grid, list and compact views share the markup (`styles.css` sizes the
emblem by the icon, as it does the others). Search results, Recent and
Starred never hold Arbesk assets, so nothing else changes.

**F7. Properties for an Arbesk asset, with a Files… page.** Properties
stops refusing an Arbesk **asset**. Backend: the empty `places::ext::Properties`
slot (`provider.rs:452`) gains `properties(path) -> FileProperties`, which
Arbesk's provider fills; `file_properties` routes `Kind::Arbesk` through it
for an asset, while `properties_refusal` (`lib.rs:751`) keeps *Properties
aren't available for Arbesk yet.* for an account or a collection.
`places::FileProperties` gains `version: Option<u32>`
(`skip_serializing_if`, so every other place's JSON is unchanged); Arbesk
answers the name, the collection as `parent`, `modified` from the
manifest's `timestamp`, `version` = `version_of` (F1) when it is a whole number from 0 up that fits a `u32` (else none, and no Version row), and
zeros and empty strings for the rest. The dialog's Parent folder row
shows the collection by name (`pathName` of `parent`), its `arbesk://`
address only in the row's tooltip: an address means nothing to read, as
Copy Location's absence already says (`parent` itself stays the path, as
Drive's does).

Frontend (`showProperties`, `main.ts:2975`): the `isCloud(info.path)` test
becomes a rule from `locations.ts`, `hasLocalFileFacts(kind)`, a
`Record<LocationKind, boolean>` (true for a folder and the Trash, false for
Drive, Arbesk and the rest): false skips Accessed, Created, Link target,
Permissions, Group and Others, as `isCloud` does today. Size is shown only
where `knowsSizes(kind)` (`locations.ts:129`; Drive yes, Arbesk no), and
Owner only when non-empty (Arbesk sends none). Drive's dialog is
unchanged. A **Version** row shows `version` when present. The
type line stays `fileTypeDescription(entry)`, which gains an Arbesk branch:
*Arbesk asset (glTF)* / *(3MF)* from the listed extension. Permissions… is
already hidden for remote paths. `tests/arbesk-properties.test.ts` asserts
the rows that must be absent.

`item-actions.ts`: `PROPERTIES_ITEM` is one cell per `LocationKind`, and
`arbesk` covers the account, collection, asset and F4 file, so one cell
can't tell them apart. The `properties` rule instead reads `here.paths` the
way `copyOut` does: for Arbesk, the item's `capabilityRow` (`arbesk-asset`
allowed, the rest QUIET); every other kind keeps its cell. A row in
`tests/item-actions.test.ts` for each Arbesk row.

The menu that changes is **`arbeskItemMenu`** (`main.ts:2251`): for a
single asset it gains, in GNOME's order, Open, **Open With…** (phase 3),
Copy, **Paste Into Asset** (phase 4), a separator, then **Properties**
(phase 2, `propertiesMenuItem`); a collection's and an account's menus
stay as they are. The "an Arbesk collection" rows of
`tests/support/item-actions-matrix.ts` (whose `keys.properties` is
"nothing" today: Ctrl+I on an asset now opens Properties) change in each
phase that changes them.

When the asset has other files, the dialog gets a **Files…** button beside
Close, as Permissions… is for a local file (the app's Properties has no
tabs, and neither has GNOME Files 50's; its sub-pages are pushed from
rows, which this app does with a second dialog). **Files of “Box.gltf”**
lists, in a boxed list:

- the model first: its icon, its listed name, *Model*;
- each representation in stored order: the icon of its file kind
  (`fileKind`), its name, and a subtitle joined with `·`: the format's
  label (*G-code*, *Blender file*, *STL*, *3MF*… — besk's `FORMAT_LABELS`,
  `representations.ts:365`, plus the format in capitals for others), its
  size (`formatSize(bytes)`), the role's words (*Source*, *Mesh*, *For
  viewing*, *For the printer*, *Other*), the tool (*PrusaSlicer 2.9.2*),
  the profile (*Original Prusa MK4, PETG, 0.2 mm layers, 0.4 mm nozzle*:
  besk's `printer`, `material`, `layer_mm`, `nozzle_mm`; other keys left
  out) and stats (*2 h 3 min* from `print_time_s`, *38.5 g* from
  `filament_g`; other keys left out, since asset-core leaves `stats`
  free-form and besk names no keys) when present;
- a stale row carries an *Out of date* tag in `--warning`, whose tooltip
  says what changed: *Made from an earlier version of the model* (ref
  `primary`) or *Made from an earlier “source”* (another ref), or *Made
  from a file File Browser can't find in this asset* (the ref is gone, or
  names an entry this client can't parse: true either way).

Each row has a menu button (`openMenu`): **Open With…** (F8), **Copy**
(F9; puts the file's **pinned** F4 path on the clipboard, F4) and, from
phase 4, **Remove…** (F12). The model's row offers Open With… and Copy.
Below the list, a dim line: *Paste or drop files onto the asset to add
them.* (phase 4). The dialog is built with `openDialog` and closes with
Close. After an attach or a remove (F11) an open Files… dialog is drawn
again from the manifest the write returns.

### Opening and copying

**F8. Open With asks which file when the app opens more than one.** Open
With… is offered for a single Arbesk **asset**, as an item of
`arbeskItemMenu` (F7). Open With… is not a `SharedAction` today (its only
rule is the local check inside `previewAndOpenWithItems`, `main.ts:2544`),
so `item-actions.ts` gains **`openWith`**: local items as today (a single
item not on a remote path), plus, for Arbesk, the item's `capabilityRow`
read from `here.paths` (`arbesk-asset` allowed; account, collection and
other remote places QUIET). `previewAndOpenWithItems` and `showOpenWith`'s
callers ask it (`refuse("openWith", hereFor(paths))`), and
`tests/item-actions.test.ts` gains its cells. The chooser (`open-with.ts`)
is unchanged until the user picks an app (a row click or Open); then, for
an Arbesk asset, `Chooser.launch` (`open-with.ts:170`) calls a new optional
hook, **`OpenWithHooks.launch(app): Promise<void>`**, instead of
`openWith`. The hook **closes the chooser first** (no busy mode, no retry
in it), then:

1. `app_formats(app, formats) -> string[]`, a new command: which of the
   asset's formats (the model's `gltf`/`3mf` and `entry.representations
   .formats`) the application declares it opens. Linux: GIO, already a
   dependency (`Cargo.toml:96`): each extension's content type
   (`gio::content_type_guess` on `x.<ext>`) and
   `gio::AppInfo::all_for_type`, which honours `MimeType=`, the MIME cache
   and subclasses (a text editor that opens `text/plain` opens G-code).
   `AppChoice.id` is the `.desktop` file path, not a desktop id, so each
   result is downcast to `gio::DesktopAppInfo` and its `filename()`,
   canonicalised, is compared with the canonicalised `AppChoice.id`; a
   result with no file name never matches. GLib (2.80 checked) learns
   which launcher takes which type from each applications folder's
   `mimeinfo.cache` (what `update-desktop-database` writes), not from the
   `.desktop` files' own `MimeType=` lines. macOS, as built: NSWorkspace's
   `URLsForApplicationsToOpenURL:` on an empty sample `x.<ext>` file in
   the temp folder (macOS 12 and later, checked with `respondsToSelector:`;
   on an older macOS the answer is empty, so the model opens), compared
   with the app's bundle path: LaunchServices decides by the extension as
   `UTType` would, with no new crate (`objc2-uniform-type-identifiers`).
   It type-checks for `aarch64-apple-darwin` but is unverified until run
   on a Mac. Answered from the OS, never from a table of our own. A format
   that isn't `[a-z0-9]{1,16}` is never asked about.
2. The candidates are the representations whose format the app opens,
   plus the model when its format is one. None (an app that declares
   nothing we know): the **model** opens with it, as GNOME Files hands any
   file to the app the user chose. One: that file opens. Several: the
   chooser closes and a **choice dialog** opens: heading *Open Which File
   With PrusaSlicer?*, a boxed list of radio rows (icon, name, the F7
   subtitle, *Out of date* tags), the first file that is not out of date
   selected (the model counts as never out of date), responses Cancel and
   Open (suggested). Enter opens, Escape cancels.
3. The chosen file is fetched (the model with `Transfer::export`'s build,
   `service.rs:268`, a representation with F5 through its pinned F4 path;
   both by one command, `arbesk_export(path, op)`, Arbesk's
   `Transfer::export`, which answers an F4 path with the F5 fetch)
   as a `startOperation` row, *Opening “Bracket.gcode”* (`opTitle`'s
   wording), with progress and
   Cancel, then `openWith(<cached file>, app)`. A failure (fetch or launch)
   is a toast, *Could not open “Bracket.gcode”: <reason>.*; a cancel says
   nothing. `hooks.opened` is **not** called with the cached path, so an
   app-cache file never lands in Recent.

Started from the Files… page or the preview's file page (F14), the file is
already chosen: step 2 is skipped. The preview's file page fetches with
`ext::Representations::file(path, version, id)` with the version on
screen, never a current-version F4 path (F14). The Files… page shows the
current save, so (as built in phase 3) its Open With… fetches the file's
**pinned** F4 path through `arbesk_export`: a file whose entry changed
since the page was drawn is refused, never swapped. Its model row hands
over the model.

Plain **Open** (double-click, Enter, the preview's Open) still opens the
asset in Arbesk's Studio (`provider.rs:169`): the default stays what it
is today. There is no "Open With <default app>" item to add: the app has
none for any file.

**F9. Copy out asks which files.** Copy (and a drag) still puts the
**asset** paths on the clipboard. A paste only has `{ paths, dirs, mode }`,
and by then the user has usually left the collection, so the entries'
summaries are gone. (An asset never leaves its window, F16, so a drop
from another window can't carry one: the question is asked for pastes and
drags in the window.)
`runTransfer`, when its sources hold Arbesk assets and the target is
this computer or Google Drive (the routes Copy out has today), first calls
`arbesk_representations(path)` for each asset (F3: one `tokenURI` read
each, manifests from the cache, at most 4 at a time), under the paste's
operation row (its existing *Preparing…*). When at least one asset has other
files, a dialog asks before anything is copied:

- One asset: heading *Copy Which Files of “Box.gltf”?*, a boxed list of
  check rows: the model (checked) and each representation (unchecked),
  with the F7 subtitle; an **All Files** check row above them (checked when
  all are, mixed otherwise) toggles them all. Responses Cancel and Copy
  (suggested, insensitive while nothing is checked).
- Several assets: heading *Copy Which Files?*, one check row per kind
  instead of per file: All Files above them as for one asset, *Models*
  (checked), then one row per format present across the assets (*G-code
  (3 assets)*, *STL (2 assets)*). The choice applies to each asset.

A failed read of an asset's list is a toast naming that asset (*Could not
copy “Box.gltf”: …*) and copies nothing. A cancel of the paste's row while
the lists are read starts no further read. The Files… page's Copy of the model row puts the
asset on the clipboard, so its paste asks too, the model already chosen.

The paste's sources become the chosen asset paths (the model) and
**pinned** F4 paths (each chosen representation, with the CID the dialog
was built from), in that order per asset; the engine copies them as it
copies anything (`Transfer::export` builds or fetches each into the cache,
the copy places it).

**Target names.** `transferBatch` takes each target name from
`basename(from)` (`main.ts:3417`) and makes it safe only for Drive
sources. It now applies `safeLocalName` to Arbesk sources too (an F4 name
is already safe, F4, so this is the second guard), and
`runTransfer`/`transferBatch` take an optional `names: Map<source,
targetName>`. The F9 dialog fills it: names repeated **within** one paste
(a `Box.stl` from two assets, or a representation named like its model)
get the app's Keep Both naming (`Box (copy).stl`, as `freeCopyName` makes it,
checked against the other names of the paste as well as the folder; names
within the paste are compared ignoring case, as a macOS volume does), and
the clash check uses the mapped name. Keep Both on a clash, too, skips every
name the paste plans to write, so a later file of the same paste is never
asked about. **Name clashes still ask Replace /
Skip / Keep Both** for each file already in the target folder, so the
dialog only asks about files that were there before. An asset without
other files copies as today, with no dialog. Cancel copies nothing.

`places::Transfer::export` (`provider.rs`) answers an F4 path with the F5
fetch (refusing a pinned path whose CID moved, F4); the route table
(`place::copy`) and `copyOutRefusal` treat an F4 path as an asset, through
its `arbesk-file` row (F4).

### Adding and removing

**F10. Paste Into Asset, or a drop on an asset, adds files as a new
version.** Sources: regular files on this computer, at most 256 MB each
(Arbesk R7), never folders. Two ways in, the same work:

- **Paste Into Asset** in `arbeskItemMenu` (F7) for a single asset,
  sensitive when the clipboard holds copied files. Plain Paste in a
  collection keeps today's meaning (model files become assets), and the
  `paste` rule is unchanged (it still refuses an asset target with *Paste
  into a collection.*).
- A **drop on an asset** in the view: the asset highlights as a folder does
  under a drag, and the drop adds. A drop on the empty space of a
  collection keeps today's meaning. This touches each drop function:
  `DropTarget` gains `kind: "dir" | "attach"` (`{ dir, el }` today,
  `main.ts:3685`); `dropTargetAt` (:3691) also accepts a non-folder item
  whose `capabilitiesOf(capabilityRow("arbesk", path)).can_attach` holds,
  as an `attach` target; `dropOp` (:3721) returns a new `"attach"` op for
  such a target when the `attach` rule allows it, before its Arbesk branch;
  `performDrop` (:3767) runs the attach flow (the confirmation below, then
  `arbesk_attach`) instead of `runTransfer` for it; and the drop from other
  apps (`onDragDropEvent`, :4021) takes the same branch.

**The rule.** `item-actions.ts` gains **`attach`** (a `SharedAction`), read
from `here.paths` as `copyOut` is: the target's `can_attach` (F13) and every
source a local regular file, with the toasts below. (As built: the
clipboard holds `{ paths, dirs }` and no link facts, so the rule takes
every source on this computer that isn't one of `dirs`, `locations.ts`'
`attachRefusal`; the backend's plan refuses anything but a regular file,
links followed, with the same words.)
Paste Into Asset and the drop both call `refuse("attach", hereFor(…))`, and
`tests/item-actions.test.ts` gains its cells.

Refused with one toast, before anything starts: Cut or a forced-move drag
(*Arbesk is read only.*, as copy-in says), folders or files from Drive,
shares or SSH servers (*Only files on this computer can be added to an
asset.*), a file with no extension or one that isn't `[a-z0-9]{1,16}`
(*“README” has no file type to record.*), a `.gltf` whose buffers or images
name anything but a `data:` URI, beside it, above its folder or an absolute
`file:` URI (*“Box.gltf” uses files beside it. Add a .glb instead.*, read
with `models.ts`' `gltfExternalUris`, since only the one file would be
stored; as built, a `.gltf` whose bytes aren't JSON is refused too:
*“Box.gltf” couldn't be read as glTF. Add a .glb instead.*, while one that
can't be read at all goes on to the plan, which names why: too large,
gone or not allowed), and a
file over 256 MB (*“Print.gcode” is larger than 256 MB.*). As built, an
asset whose `representations` is there but isn't a list (a newer schema's,
or a broken write) is refused before anything is asked or uploaded, by the
plan, an attach and a remove alike, rather than overwritten or dropped:
*“Box.gltf” has files File Browser can't show.* Room is checked **after**
the ids are settled (below).

**What each file records**, as `besk attach` records it (Arbesk R6): format
= the lower-case extension; role = `default_role(format)` (a 3MF is a
`mesh`, as `defaultRole` says); `derived_from` = `primary` with the
model's CID and matrix for `machine` and `mesh`, none for the rest, and
none (silently) when the model has no source CID (a child_ref asset);
`name` = the file's name; `bytes` = its size; `encoding` =
`encoding_for(format)`; `created_at` = now. For a `.gcode` the first 4 KB
and last 64 KB are read, best effort, for what PrusaSlicer, OrcaSlicer,
SuperSlicer, Bambu Studio and Cura write there: `tool` (*generated by
PrusaSlicer 2.9.2*), `profile` (`printer`, `material`, `layer_mm`,
`nozzle_mm`) and `stats` (`print_time_s`, `filament_g`); a value that
doesn't parse is left out. The id is `representation_id(role, format,
profile)`, the same slug `besk attach` makes
(`gcode-original-prusa-mk4-petg`), so the two clients name the same file
the same way.

**Ids are settled for the whole paste before anything is uploaded**, against
the **stored** ids (`stored_ids`, F1: valid entries and ones this client
can't parse alike, as `hasRoomFor` and `withRepresentation` match them),
never against what the Files… page shows:

- **An id an unparseable entry holds** is refused, never replaced: *This
  asset already has a file “<id>” that File Browser can't show.* (a newer
  client's file would otherwise be overwritten with no Replace dialog).
  The rest of the paste goes on.
- **An id a valid entry holds** asks, in its own dialog (not `askConflict`,
  whose heading and body speak of a name in a folder): an `openDialog`
  with heading *Replace “<existing name>” in “Box.gltf”?*, a body with the
  existing file's F7 subtitle and *The new file is “<new name>”.*,
  responses Skip, Keep Both and Replace, Enter = Keep Both, and the same
  *Apply this action to all* check-row when several clash. Replace swaps
  the entry (the old file stays in the asset's earlier version, R4).
- **Keep Both**, and **two files of one paste with the same id** (two
  `.stl` files, two G-code files with no recognised header), take the
  first free `<id>-2`, `<id>-3`… (the base cut so the whole stays within
  64 characters), free meaning not among the stored ids **nor** the ids
  already given to earlier files of the paste. Within the paste this is
  automatic, with no dialog: neither file was there before.
- When every file is skipped or refused, **no version is written** and
  nothing is relayed; a toast says *Nothing was added to “Box.gltf”.*
  (libadwaita shows one toast at a time: as built, a paste's refusals and
  this sentence share one toast.)

Then room: stored entries + files that add an entry (new ids, Keep Both)
must be at most 32; Replace adds none. Past it: *An asset holds at most 32
files.* F11 step 1 checks ids and room again against the manifest it reads.

**A confirmation, every time** (copy-in's D2, extended): heading *Add to
“Box.gltf”?*, body *Files in Arbesk are public: anyone with their address
can download them, and they can't be removed from IPFS.* When a file is a
`source`, a second paragraph (Arbesk R7): *“Bracket.blend” is a source
file. It can hold more than the model, such as other scenes and embedded
images.* Responses Cancel (default) and Add.

**One version for one paste or drop.** All files of one paste are added in
one new asset version (`version_of + 1`, F1; `prev_asset_manifest_cid` =
the CID it was built from, `timestamp` now), so a drop of three files is
one save, one relay call (Arbesk's relay allows 30 a minute) and one
history step (*Added G-code, STL*). It runs as one operation, *Adding 2
files to “Box.gltf”*, with progress over the uploaded bytes and Cancel
until the collection manifest is uploaded, as copy-in does
(`write.rs:402-407`).

**F11. The write: everything kept, compare-and-swap, nothing half done.**
`write::attach(…)` (in a new `crates/arbesk/src/attach.rs`, sharing
`write.rs`' `Uploader`, `CollectionLocks`, `settled_collection`,
`editor_proof`, `relay`, `next_collection` and wording):

1. Before any upload: the session (*Sign in to Arbesk again to add files
   to it.* opens Sign In Again and carries on, as copy-in does with its
   own words), the editor proof (*You can't add to this collection.*), the
   base: with a `Reader::open_fresh`, **take the collection's hold**
   (`CollectionLocks::take`), read the collection through
   `settled_collection` (so this app's own last write to it, still
   settling, shows first), note the asset's manifest CID, and **let the
   hold go** before uploading (an asset gone from the collection here
already: *“Box.gltf” is no longer in the collection.*, its listed name
taken from its path); the manifest itself; every F10 check, ids
   and room included, against it; and each entry as it will be recorded
   checked by F1's validator with a placeholder CID (`checkEntry`,
   `besk/representations.ts:204`). A failure here uploads nothing.
2. Upload each file, encoded (F1); a failed upload names that file
   (*Couldn't upload “Box.stl” to IPFS: …*), the manifests' the asset's.
   A failed or cancelled upload stops
   here: **the asset and the collection are unchanged**; what was uploaded
   stays on IPFS unreferenced, as for copy-in.
3. Build the next manifest **from the manifest read in step 1, as JSON**:
   every key kept (scene, thumbnail, metadata, comments, keys this app
   doesn't know), `representations` changed only by adding or replacing
   the new entries by **stored** id (entries it can't parse stay as they
   are, in place, as `withRepresentation` keeps them),
   `version_of + 1`, prev link, timestamp. A bare composite entry (no
   `scene.nodes` array) is first wrapped exactly as besk's
   `compositeWrapper` does, field for field (F1's "What Arbesk stores"):
   `type: "asset"`, `name` = the composite's `name` else the listed name,
   `asset_id` (as built: with no `name` of its own, the listed name
   without its extension, where besk takes the name the user typed),
   `version` = `version_of(composite)`, one node `node_id:
   "node_1"` with the identity `transform_matrix` and `source {cid,
   format, path}` (`path` for gltf and 3mf), the format from
   `compose::format_of` (glTF, or 3MF for `composite-3mf`, as
   `detectFormat`); then the next version of that, prev = the composite's
   CID. Not `write.rs`' `asset_manifest`, whose node shape differs. A test
   checks the wrapper equals besk's for the same composite (a fixture made
   by `scripts/make-arbesk-fixtures.ts`). Upload it.
4. Under the collection's lock, read the settled collection (`write.rs:582`)
   and **compare-and-swap**: the collection's entry for the asset must
   still be the CID read in step 1. If it moved (a Studio save, `besk`,
   another window), it is a conflict (below); if the asset is gone,
   *“Box.gltf” is no longer in the collection.* Else write the next
   collection manifest with that one entry changed (`next_collection`),
   upload it and relay, with the outcomes copy-in has (*Arbesk is still
   saving “Box.gltf”…*, *Arbesk couldn't save “Box.gltf”.*, the relay's
   502 words), and **`held.remember(current, next, &outcome)`** as
   `save` does, so the next write from this app (an attach, a remove, a
   copy-in) waits for this one to show.

**A conflict** is retried once by itself, from the new tip: steps 1, 3 and
4 again, reusing the file CIDs step 2 uploaded (content-addressed: the same
bytes have the same CID, so nothing is uploaded twice), with
`derived_from` recomputed from the new tip and the ids settled again (one
clashing with an unparseable entry is refused). As built, a new clash with
a valid entry is not asked again: a command can't ask mid-write, so a clash
nobody answered is Keep Both, which overwrites nothing, and Replace
replaces only while the entry still holds the file the user agreed to
replace (the plan's CID), else Keep Both too. It keeps whatever the other save changed. A second conflict stops
with *“Box.gltf” changed while the files were being added. Nothing was
changed; try again.* (the `CONFLICT` of `catalog.ts:184`). The asset's model
and every other representation are never dropped by an attach: a test
asserts the next manifest equals the previous one but for the keys an
attach owns.

**After a write** (attach or remove): the frontend reloads the collection,
as copy-in does after its write (which already waits on the settled
collection, so the new manifest CID is the one listed and the emblem
doesn't flicker back), patches the asset's icon (`patchIcon`) from the new
entry's summary, and redraws an open Files… dialog from the manifest the
command returns. `tests/arbesk-attach.test.ts` checks the emblem's count
changes after an attach.

**F12. Remove a file from Properties.** **Remove…** on a representation's
row (F7) asks: heading *Remove “Bracket.gcode” From “Box.gltf”?*, body
*Arbesk keeps it in the asset's earlier versions.*, responses Cancel and
Remove (destructive). The dialog uses `openDialog`'s `busy` option: Remove
returns the `arbesk_detach` promise from `onResponse`, Cancel stays live
(`liveWhileBusy`) until the collection manifest is uploaded, and a failure
is shown in the dialog in words, as AGENTS.md asks of a dialog whose
response waits on the backend. It writes the next version without that
entry, matched by stored id (`withoutRepresentation`: the key dropped when
the list empties), and only while that entry still holds the file the row
showed (the CID its path is pinned to, F4), on the first read and on a
conflict's retry alike: an entry gone is *This file is no longer in the
asset.*, one holding another file (or one File Browser can't show, or
an id such an entry also holds, beside the file shown, since every entry
of the id goes) is
*This file was replaced in the asset since it was shown. Nothing was
removed.*, as Replace replaces only the file the user agreed to; with F11's steps 1, 3 and 4 (nothing to upload; the
same hold, settle and `remember`). Files derived from the removed one
become out of date, which the Files… dialog shows when it is redrawn. The
model can't be removed. Removing needs the session (*Sign in to Arbesk
again to remove files from it.*, which marks the account as needing a
sign-in, as an attach does).

**F13. Capabilities: `can_attach`.** `places::Operation` gains `Attach`;
`providers::capability_table` derives `can_attach` from it as it derives
`can_copy_in` (`providers.rs:168-190`): true only for the `arbesk-asset`
row. Every other row refuses it with its own read-only or local-only
words; an Arbesk account, collection or file with *Add files to an asset.*
(as built, `places::ADD_TO_AN_ASSET`), and so does a folder on this
computer, which has no read-only or local-only words of its own.
The column is added to `tests/capabilities.json`, `capabilities.ts`'
`PlaceCapabilities` and built-in table, and the mock's answer. The
frontend's `attach` rule (F10) reads it.

### Previewing

**F14. Space switches between the model and its other files.** When the
shown version has other files, the preview's header gets a menu button
beside the version control (`preview-versions.ts`), labelled with what is
shown (*Model*, *STL*, *G-code*…, with the out-of-date dot when it is).
Its menu (`openMenu`) lists the model and each file of **that version**
(`arbesk_representations(path, version)`), with the F7 subtitle; the
version on screen decides the list, since each version keeps its own
(Arbesk R4). Choosing:

- a format the viewer draws (`models.ts`' `modelFormat`: STL, 3MF, GLB,
  a single-file glTF) fetches it (`ext::Representations::file(path,
  version, id)`, F5) and previews it with `preview_bytes`, as a local
  model;
- anything else (a `.blend`, G-code until F17's viewer) shows the
  preview's file page: the file's icon, name, F7 subtitle and *Out of
  date* tag, and an Open With… button (F8, with that file chosen).

Everything the switcher and the file page fetch goes through
`ext::Representations::file(path, version, id)` with the **shown
version's** manifest CID, never through a current-version F4 path, which
may name a different file once the id was replaced since. The file page's
Open With… (F8 step 3) does the same. A test replaces an id across two
versions and checks the earlier version's G-code is the one opened.

Stepping to another version shows its model again. The preview's Open
button still opens the asset in the Studio. The version menu's rows gain
the change a save made to the list (*Added G-code (Original Prusa MK4,
PETG)*, besk's `representationChanges` wording, `representations.ts:371`),
computed by comparing each manifest with the one before it. Arbesk's
`places::Version` gains an optional `changes: Vec<String>`
(`skip_serializing_if` empty, so Keep Versions' JSON is unchanged) and
`ArbeskVersion` in `api.ts` an optional `changes?: string[]`. Change text is
shown only for a version whose predecessor manifest was read: the walk
(`service.rs:369` `chain`) stops at `MAX_VERSIONS` or at the first manifest
it can't read, so for the oldest version shown, the walk reads its
`prev_asset_manifest_cid` manifest too when it has one (one extra cached
read) and compares with that; if it has none (the first version) its files
read as added; if that read fails it shows no change rather than claiming
*Added …* for every file.

As built (phase 5):

- The fetch is a command of its own, **`arbesk_file(path, version, id,
  op)`**, over `ext::Representations::file`, its `version` required (a
  manifest CID from `arbesk_versions`), so nothing in the preview can
  fetch the current save by mistake; the file page's Open With… passes
  the save on to F8's step 3 (`openAssetWith`'s `version`), which fetches
  through it instead of `arbesk_export`.
- The switcher is asked for only when the listing's summary says the
  asset has files now (no request for an asset without), and for every
  version stepped to (an older save may have files the newest dropped).
  When the asset's saves can't be read, there is no save CID to fetch
  from, so there is no switcher.
- The menu's model row is titled with the asset's listed name over
  *Model*, as the Files… page's is; the shown row is marked as the
  version menu marks its own. The button is a header menu button
  (`aria-haspopup`): Tab reaches it, Space or Enter opens the menu, the
  arrow keys move in it; Space in the view still closes the preview, and
  Space or Escape on a 3D model and Escape on any header button keep
  their meaning. Drawn again after a pick, it keeps the keyboard.
- A glTF representation whose buffers or images name anything but a
  `data:` URI (F10 refuses adding one; another client may have) can't be
  drawn from one file, so it gets the file page.
- A fetch the preview moved on from is not cancelled: it finishes into the
  app cache and is dropped, as a scene's parts are. A cancel sent after a
  command ended would be remembered for an operation id that never comes
  back (`progress::Registry::cancel`), and `operations_running` would then
  hold up Restart.
- A version's changes are joined with ` · ` as one dim second line under
  its row (`MenuItem.subtitle`, new in `ui.ts`), cut with an ellipsis
  rather than widening the list. The wording is besk's `changeText`,
  checked case for case against besk's own `representationChanges` (the
  `changes` cases of `representations-stale.json`, from the same
  generator, Arbesk commit `54e1659`), including besk's `Map` reading of a
  duplicate id. Adding `changes` to `places::Version` changes its struct
  literal, so the one test that builds it (`records.rs`) names
  `changes: vec![]`; its JSON assertions are unchanged.

### Data safety

**F15. Every write this app makes to an asset carries the list forward.**
First, before anything else ships: copy-in's **Replace** (`put_asset`,
`write.rs:602`) copies the replaced manifest's `representations` value
**verbatim** into the new version **when it is a non-empty JSON array**,
as `besk`'s `nextModelEntry` does; otherwise (no key, `[]`, `null`, an
object or any other non-array) the key is left out, never written as
`null` or as a non-array, which `manifestSchema` would refuse. (The Studio's
`carryRepresentations` keeps an empty array; leaving it out is equivalent,
since no key and `[]` both mean no files.) They become out of date against
the new model, which is the point (Arbesk R3). Without this, replacing a
model from File Browser would silently drop its G-code and `.blend` from
the newest version. The same commit makes Replace's version `version_of +
1` (F1) instead of `as_u64().map_or(2, …)`, so a float (`3.0`) or negative
version can't step history back to 2 and the result agrees with besk.

The rules, for every phase:

- **Never lose data.** An attach or remove never drops the model or another
  representation; writes start from the stored JSON, not a parsed copy.
- **A failure leaves the asset unchanged.** Nothing is written to the
  collection until every upload has succeeded; a conflict, a cancel, a
  sign-in failure or a refused relay leaves the collection as it was.
- **Compare-and-swap** on the collection entry for every write that
  builds on an asset's previous version (attach, remove, and Replace, which
  already re-finds the entry under the lock, `write.rs:552-560`).
- **Nothing is read as data that wasn't decoded by its entry's `encoding`**
  (F1); a raw file is never unpacked.
- **Opened copies are pruned as downloads are** (F5): edited ones go to the
  Trash, never deleted.
- **Copy out writes only to the target**, never to Arbesk.

### Later

**F16. What this design doesn't do.** Edit a representation's role,
profile or `derived_from` (attach chooses them; `besk attach` can set
them); re-slice; a Studio-like editor for the list; copy Arbesk to Arbesk;
drag files to other apps (assets stay inside File Browser, as today).

**F17. How Print would plug in.** A **Print…** item for an asset with a
`machine` representation (G-code, binary G-code, or a sliced 3MF): a
dialog lists the asset's printer files (F7's rows, out-of-date ones tagged
and confirmed with *Print Anyway*) and the printers this computer knows,
then fetches the chosen file with F5 and sends it. Printers would be a
provider-like registry in a new crate (`printers`): OctoPrint, Moonraker
(Klipper) and PrusaLink over HTTP, found with mDNS
(`_octoprint._tcp`, `_moonraker._tcp`, `_prusa-link._tcp`) or typed in,
their API keys kept with the keychain store Drive and Arbesk use, set up
from a **Printers** row in Online Accounts. The profile's `printer` picks
the default printer when a known one matches. A **G-code viewer** joins
the preview as a format `models.ts` names and `model-viewer.ts` draws
(toolpaths as line segments by layer, a layer slider), loaded only when
shown, as three.js is; F14's switcher then previews G-code instead of the
file page.

## Mock

`dev/mock-tauri/core.ts`'s `MockAsset` (:1644-1655) gains `representations`:
entries as Arbesk stores them (the mock's filesystem keeps sizes, not
bytes, so a fetched file is a sized node whose preview bytes come from its
name, as for every mock file). The fixtures
gain: *Bracket* in Shapes with a `.blend` (source), an STL (fresh), a G-code
for an Original Prusa MK4 in PETG (stale: derived from an earlier model
CID), a 3MF and one invalid entry (left out, but counted for room); and the
existing *Box* glTF with a single STL. (Phase 1, where nothing visible may
change, can't add an asset: every mock collection's listing is pinned by
existing tests. It gives the existing *Cube* in Untitled Collection, saved
three times, Bracket's set, its G-code made from its second save; *Bracket*
itself joins Shapes in phase 2, whose view changes on purpose and updates
those tests.) The mock serves:

- listing summaries (F2) computed by a TypeScript copy of the stale rule
  over the mock manifest;
- `arbesk_representations`, `app_formats` (a fixed table per mock app:
  *PrusaSlicer* opens `3mf`, `gcode`, `stl`; *Blender* opens `blend`,
  `gltf`, `glb`, `stl`; *Text Editor* opens `gcode`; others nothing),
  fetching F4 paths (pinned and not) through `open_target` and (from the
  phase that adds it, 3 or 5) the representation file command, each fetch **writing the file under a mock
  cache folder** in the fake filesystem (`…/arbesk/files/<cid>/<name>`, as
  the backend does), so `open_with` (`mockOpenWith` looks the path
  up) and `preview_bytes` find it; `arbesk_copy` of F4 paths;
  `arbesk_attach` and `arbesk_detach` (each a new mock version:
  version + 1, prev, the session check, a `conflictNext` switch that fails
  the next write with the CONFLICT words and changes nothing,
  `failUploadNext` likewise); `arbesk_versions` with `changes` (as built: each save keeps its own list,
`MockAsset.earlier` by save number, which a write fills before it adds a
save; Bracket's first save had the source and the G-code, its second
added the STL and the 3MF), `arbesk_file`;
  `file_properties` for an asset (with `version`); and the capabilities
  table with `can_attach` and the `arbesk-file` row. `arbeskIds` returns
  the file id and pin of an F4 path. *PrusaSlicer*, *Blender* and *Text
  Editor*, which the `app_formats` table above refers to, are the mock's
  `MOCK_ARBESK_APPS`, which the tests and screenshot scenes add with
  `__mockApps.set`: the default `apps.list` stays as it was, since existing
  Open With tests pin it. Phase 3 also adds `arbesk_export` (the model
  built under `…/mock-arbesk/builds/`, a file as `open_target` fetches it)
  and the test hooks `__mockArbesk.addAsset` and `changeFile`.

For phase 2's view test the mock also has a test hook,
`__mockArbesk.addManyAssets(count)`, which adds a collection *Many* of
`count` assets (every third without files, one fresh, one out of date),
so the 2,000-asset scroll test needs no fixture in every listing.
Bracket is saved twice (its G-code made from the first save), and the
mock's `file_properties` gives an asset the number of its saves as its
version.

Each new mock command gets its case in `tests/mock-core.test.ts`.

## Testing

**Rust (`crates/arbesk`).**

- `representations.rs`: the shared stale fixture (F1, with its
  `ToNumber` cases and a ref to an unparseable entry); `list` keeps valid
  entries and drops each kind of invalid one (id, `primary`, role, format,
  a 256-unit name counted in UTF-16, fractional or over-cap `bytes`,
  encoding, detail maps of 33 keys); `stored_count` and `stored_ids` count
  invalid ones; `version_of` on a string, a float (`3.0` → next 4), a
  negative number and a missing version; `decode` never unpacks a raw file
  starting with `ARB\x01` or the gzip magic, refuses an unframed `brotli`
  entry, stops past the decoded cap, refuses a length other than `bytes`;
  encode round trip; `representation_id` matches besk's slugs.
- `service.rs`: a listing's summaries (count, stale, formats), with the
  fake HTTP counting requests: the same number as without representations;
  a bare composite has none; a `child_ref` asset with a list gets its
  summary (its `primary`-derived entries stale); the oldest shown version
  compares with its predecessor, and shows no change when that can't be
  read.
- `location.rs`: F4 paths parse and format, pinned and not, a pending
  name, a malformed id, a name `a/b.gcode` or `..` made safe in the
  segment.
- `provider.rs`: `fetch` and `export` of an F4 path decode into
  `arbesk/files/<cid>/<name>` with the mark, reuse an unchanged copy, keep
  an edited one; capabilities of each path kind (`can_attach`).
- `write.rs` / `attach.rs`: Replace keeps `representations` verbatim (F15)
  and leaves the key out for an older manifest with none, with
  `representations: []` and with a non-array value; Replace's version from
  a string and from a float version; attach writes `version_of + 1` with
  the prev link, keeps every other key and entry (including an unparseable
  one, in place), replaces by stored id, adds several files in one
  version; a new id equal to an unparseable entry's is refused and that
  entry is unchanged; two STLs in one paste get `stl` and `stl-2`; room
  counted after ids (a paste that only replaces fits a full asset); every
  refusal happens before the first upload (the fake counts uploads); a
  failed upload, a cancel and a second CONFLICT leave the collection
  manifest and the token URI unchanged; a first CONFLICT is retried with
  no new file upload; two attaches in a row from this app don't conflict
  (the second waits for the first to settle); a bare composite is wrapped
  as besk's `compositeWrapper` wraps it; G-code header parsing for
  PrusaSlicer and Cura samples; detach, and the key dropped when the list
  empties.
- `providers.rs`: the capability table with the new column and row, against
  `tests/capabilities.json`.
- App: `app_formats` on Linux with a temporary `XDG_DATA_DIRS` holding
  `.desktop` files, a MIME database and each folder's `mimeinfo.cache`,
  run in a fresh copy of the test binary (GIO reads its folders once per
  process), including a launcher under a flatpak
  `exports/share/applications` folder and one whose desktop id differs
  from its file name (both matched by file path); macOS's half behind
  `cfg` with a test that Text Edit opens `txt`; `file_properties` for an
  Arbesk asset (with `version`) and the refusal for a collection.

**Frontend (vitest, the mock).** `tests/arbesk-representations-view.test.ts`
(emblem and tooltip in grid and list views, the stale dot, emblem on a
thumbnail, markup unchanged after scrolling a virtualized list of 2,000
assets back and forth), `tests/arbesk-properties.test.ts` (asset
Properties from the asset menu and Ctrl+I: the Version row, the type line,
and no Size, Accessed, Created, Permissions, Group, Others or Owner rows;
Files… rows and subtitles, out-of-date tags and their tooltips;
collections still refused), `tests/arbesk-open-with.test.ts` (Open With…
in an asset's menu; no candidates → the model; one → it opens; several →
the choice dialog, its default, Cancel; started by a row click and by
Open, the chooser closes first; the cached file goes to `open_with` and
not to Recent; the operation row's progress and Cancel; a failure's
toast), `tests/arbesk-copy-out-files.test.ts` (one asset, several assets,
All Files' mixed state, Copy insensitive with nothing checked; copy,
navigate to Home, paste still asks; a representation named `a/b.gcode`
lands as one file in the target; two assets each with `Box.stl` get two
names; Replace / Skip / Keep Both against existing files; a pinned file
changed since Copy is refused; no dialog without representations),
`tests/arbesk-attach.test.ts` (Paste Into Asset and a drop on an asset; a
drop on a collection's empty space still copies in; each refusal's toast;
the confirmation with and without the source paragraph; the id clash
dialog's heading and Keep Both; a drop of two STLs records two ids; every
file skipped writes nothing; CONFLICT's error with the asset unchanged; a
failed upload; Sign In Again with the add words then carrying on; the
emblem's count changes after an attach; Remove… busy until done, its
confirmation and its sign-in words) and
`tests/representations.test.ts` (phase 2: the format labels, the
subtitle's parts, the out-of-date reasons, the emblem's state and tooltip
from a summary) and
`tests/preview-representations.test.ts` (the switcher per version, an STL
previewed, a `.blend` on the file page, the earlier version's file opened
after its id was replaced, the version menu's change text and none for an
oldest version whose predecessor can't be read: as built, that last case
is the backend's, in `service.rs`, since the mock has no unreadable save;
the frontend checks a save sent without `changes` shows none). `tests/paths.test.ts`
(`isArbeskFile`), `tests/locations.test.ts` (`arbesk-file`,
`hasLocalFileFacts`), `tests/item-actions.test.ts` (`openWith`, `attach`,
`properties` by Arbesk row), `tests/capabilities.test.ts`,
`tests/item-actions-matrix-network.test.ts` and
`tests/support/item-actions-matrix.ts` gain the new column, rows and
actions.

The CRAP gate applies to each phase as to any change.

## Phases

Each phase is one PR, with its docs (`docs/arbesk.md`; AGENTS.md's layout
rows and Arbesk notes where a module is added) and its tests, and leaves
the app working.

1. **Backend read, and Replace keeps the list.** F15's Replace fix (first
   commit, on its own), F1 (`representations.rs`, `Gateway::raw`), F2 (the
   summary on `places::Entry` and the frontend type), F3
   (`ext::Representations`, `arbesk_representations`), F4 (paths, pinned
   paths, `isArbeskFile`, the `arbesk-file` capabilities row in `KINDS`,
   `tests/capabilities.json` and `capabilities.ts`), F5 (fetching into the
   cache). Mock fields and commands, `mock-core` cases. Nothing visible
   changes but Replace no longer dropping files.
2. **View and Properties.** F6 (emblem, `--warning` in both themes), F7
   (`ext::Properties`, `FileProperties.version`, `hasLocalFileFacts`, the
   `properties` rule by Arbesk row, Properties in `arbeskItemMenu`, the
   Files… page read only: Open With… and Copy rows come in phase 3,
   Remove… in phase 4).
3. **Open With and copy-out.** F8 (`app_formats`, the `openWith` rule,
   `OpenWithHooks.launch`, the choice dialog, Open With… in
   `arbeskItemMenu`), F9 (the copy dialog, `names` in `runTransfer`,
   `export` of F4 paths), the Files… rows' Open With… and Copy.
4. **Adding and removing.** F10–F13 (`attach.rs`, `Operation::Attach` and
   `can_attach`, the `attach` rule, Paste Into Asset in `arbeskItemMenu`,
   drops on assets, the dialogs, Remove…).
5. **Preview switcher.** F14, and the version menu's change text
   (`Version.changes`).

## Risks

- **Everything added is public.** Mitigated by the confirmation on every
  paste or drop, the source paragraph, and refusing silently nothing.
- **A write between our read and the relay.** The collection's token takes
  a whole new manifest (Arbesk R8: no contract change), so a save by
  another client that lands after our settled read and before our relay is
  overwritten in the collection (its asset manifest stays on IPFS and in
  its own history). The window is the collection upload plus the relay,
  the same as for copy-in, the Studio and `besk`; the compare-and-swap
  closes everything before it. A true fix needs a compare-and-swap in the
  contract, which is Arbesk's to add.
- **A wrong "out of date".** The Studio's first save of a generated model
  decomposes it to a new CID, so files derived from it read out of date
  (Arbesk R2's known false positive, the safe direction). The tooltip says
  what changed, not that the file is wrong.
- **Drift from asset-core.** One fixture of Arbesk's own cases (F1), copied
  with its commit; a manifest key File Browser doesn't know is never
  dropped because writes start from JSON.
- **Large files.** A 256 MB G-code is held in memory while it is encoded
  and uploaded, as copy-in holds a 100 MB model; the decoded cap stops a
  brotli bomb.
- **Apps that declare nothing.** `app_formats` trusts what the OS says an
  app opens; an app that declares nothing gets the model (F8), and the
  Files… page's Open With… reaches any file directly.

## Open questions

None blocking. Two defaults taken that the owner may want to revisit:
the emblem's drawing (two sheets, F6), and **Paste Into Asset** as the
menu wording (mirroring Paste Into Folder, F10).

## Files

New: `src-tauri/crates/arbesk/src/representations.rs`, `attach.rs`,
`crates/arbesk/tests/fixtures/representations-stale.json`,
`src/representations.ts` (pure rules: format labels, subtitles, Open With
candidates, copy choices, unique names), `src/representation-dialogs.ts`
(the Files…, choice, copy, add and remove dialogs), the test files above.
Changed: `crates/places/src/entry.rs` (`Representations`),
`crates/places/src/provider.rs` (`Operation::Attach`, the new
`ext::Representations`, `ext::Properties::properties`),
`crates/arbesk/src/{lib,service,location,provider,write,ipfs}.rs`,
`src-tauri/src/{lib,providers,platform}.rs` (commands, `app_formats`,
Properties), `src-tauri/Cargo.toml` (as built: only the `NSURL` and
`NSArray` features of `objc2-foundation` named, no new crate),
`crates/places/src/{info,records}.rs` (`FileProperties.version`,
`Version.changes`),
`src/{api,icons,main,open-with,item-actions,locations,paths,capabilities,preview-versions,ui}.ts`
(as built: `ui.ts` for a menu row's subtitle), `index.html` (the switcher's
place in the preview's header), `tests/preview-representations.test.ts`,
`tests/support/item-actions-matrix.ts`,
`src/styles.css`, `src/themes/{gnome,macos}.css`, `dev/mock-tauri/core.ts`,
`dev/mock-tauri/models.ts`, `tests/capabilities.json`,
`tests/mock-core.test.ts`, `docs/arbesk.md`, AGENTS.md,
`docs/architecture.md` (the extension and the entry's new key).
