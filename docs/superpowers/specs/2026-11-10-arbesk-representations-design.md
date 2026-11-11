# Other files of an Arbesk asset (representations) — design

Date: 2026-11-10 · Status: proposed (owner asked for full automation: every
decision below is taken; nothing waits on an answer). Builds on Arbesk's asset representations design (in the Arbesk repository,
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
  values); `created_at`. Entries are `passthrough` (a newer client's keys are
  kept). At most 32 entries, ids unique.
- `packages/asset-core/src/manifest/representations.ts`:
  `listRepresentations` (:68) is tolerant: an entry that doesn't parse is
  left out, the rest are shown. `currentCidOf` (:82): for `primary`,
  `scene.nodes[0].source.cid` when it is a non-empty string, else null; for
  another id, that entry's `cid`. `primaryTransformOf` (:97): null without
  a node 0, its `transform_matrix` (16 values, `map(Number)`) or the
  identity. `isStale` (:132): no `derived_from` → never stale; the ref's
  current CID differs → stale; for `primary` with a recorded
  `transform_matrix`, a different matrix → stale. `hasRoomFor` (:146)
  counts the **stored** entries, unparseable ones included.
  `withRepresentation` (:161) replaces by id in place and keeps entries it
  can't parse; `withoutRepresentation` (:181) drops the key when the list
  empties. `representationId` (:203) is `source` for a source file, else
  the format (then printer and material when a profile names them).
  `defaultRole` (:54) and `encodingFor` (:59) are the format tables.
- `representation-codec.ts`: text formats are stored brotli-framed
  (`ARB\x01` + a brotli stream, quality 5), the rest raw; decoding follows
  the entry's `encoding` and **never sniffs** a raw file.
- `packages/besk/src/representations.ts`: `besk attach` checks everything
  before uploading (size :182, room :193, the entry as it will be recorded
  :204), warns before a `source` upload (:47, R7), uploads, then writes the
  next asset version (version + 1, `prev_asset_manifest_cid`, timestamp:
  :107) and replaces the collection entry **only while it still names the
  CID it built from** (`replaceAssetEntry`, `catalog.ts:177-200`, error code
  `CONFLICT`). An entry `besk upload` stored as a bare composite is wrapped
  in an asset manifest first (:101). `derived_from` defaults to `primary`
  for `machine` and `mesh`, nothing for the rest, and nothing (silently)
  when the model has no source CID (:166-180).

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
  scratch** (`put_asset` :602 → `asset_manifest` :85-117): today it would
  drop an asset's representations, against Arbesk R3. F15 fixes that
  first.
- **Properties** refuses every Arbesk path (`lib.rs:754`).
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
- `stored_count(manifest)`: the stored entries, valid or not (`hasRoomFor`).
- `current_cid_of(manifest, ref)`, `primary_transform_of(manifest)`,
  `is_stale(manifest, rep)`: exactly `currentCidOf`, `primaryTransformOf`
  and `isStale`. The primary is `/scene/nodes/0/source/cid` as a non-empty
  string, **not** `compose::source_cid` (`compose.rs:60`), whose fallbacks
  for older manifests would answer "not stale" where asset-core says stale.
  The matrix applies JavaScript's `Number` to each value (a number as is,
  a numeric string parsed, `null` and `false` 0, `true` 1, anything else
  NaN, which never equals); comparison is `f64` equality, as `===` is.
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
same PR as the Rust change.

**F2. The listing carries a summary, from the manifest it already reads.**
`asset_entry` (`service.rs:744`) computes, from the manifest in hand, a
`places::Representations { count, stale, formats }`: the number of valid
entries, how many are stale, and their distinct formats in stored order. No
extra request per item: the manifest is the one the row's name comes from.
An asset with no list (or none valid) has no summary. A bare composite
entry (`besk upload`'s) and a live `child_ref` asset have no list.

`places::Entry` gains `representations: Option<Representations>`, serialised
only when present (`skip_serializing_if`), like `cloud` (D8 of the provider
design: existing JSON is unchanged for every other entry). It is named for
what it is, not for Arbesk, so another place with "other files of one item"
can fill it; today only Arbesk does, and `place_entry` copies it from
Arbesk's `Entry`, which gains the same field. The frontend's `Entry`
(`src/api.ts:5-25`) gains `representations?: { count: number; stale:
number; formats: string[] }`. At most 32 short strings per row: a
collection of 1,000 assets with full lists adds well under 200 KB to its
listing.

**F3. The full list is one command, from the cache.**
`arbesk_representations(path, version?) -> RepresentationInfo[]` reads the
asset manifest (the current one, or the save `version` names, a manifest
CID from `arbesk_versions`) through the manifest cache, so after a listing
it costs no request. Each item: `id`, `role`, `format`, `name`, `bytes`,
`stale`, `derived_from` (the ref only), `tool`, `profile`, `stats`,
`created_at`. The model is not in the list; the frontend shows it as the
first row from the entry itself. It is an extension of Arbesk's provider,
`ext::Representations` (D4 of the provider design: what only one place has
is an extension), with the methods F5, F10 and F12 add.

**F4. A representation has a path: one segment below its asset.**
`arbesk://<address>/<token>~<name>/<asset>~<name>.<ext>/<rep-id>~<rep name>`.
`location.rs` gains `ArbeskPath::File { address, token_id, asset_id, id }`;
`is_folder` stays false for it and for the asset (an asset is still a file:
listing it is refused as today, `service.rs:48`). The id decides, the name
is for display (as everywhere in `location.rs`). Ids are `[a-z0-9-]`, so
nothing needs encoding; the name is encoded with `NAME_SET`. Such a path
always means the **current** version's entry with that id; it is what the
clipboard, the copy engine, a conflict dialog and an operation's progress
see. Nothing lists these paths, so they never reach stars, bookmarks,
Recent or tabs. `paths.ts`' Arbesk segments parse them (`placeOf` →
`arbesk`), `locations.ts` treats them as Arbesk files.

**F5. Fetching a file decodes it into the app cache, as an opened build.**
`fetch(<file path>, Fetch::Open | Fetch::Preview)` and the extension's
`file(path, version?, id, cx)` read the entry, fetch its CID with
`Gateway::raw` (cap 256 MiB + the frame, encoded), decode it (F1; decoded
cap 256 MiB, *This file is too large to open.* past it) and save it under
`cache_dir/arbesk/files/<cid>/<name>` with the `.downloaded` mark
(`places::mark`), through `build_file` (`service.rs:452`): an unchanged
copy is reused; one someone edited is kept, and a fresh copy goes beside
it. Keyed by the file's CID, so two versions sharing a G-code fetch it
once. `downloads::DIRS` already holds `arbesk` (`downloads.rs:11`): pruning
after a week deletes unchanged copies and sends edited ones to the Trash,
as for Drive downloads. Progress and Cancel work as for an opened Drive
file (`open_target`, `lib.rs:520`): the provider reports progress for
these fetches. The file name is the entry's `name` made safe with
`file_name`'s rules (`service.rs:847`: no `/` or NUL, never empty, `.`
or `..`).

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
stops refusing an Arbesk **asset** (`lib.rs:754`; an account or a collection
keeps *Properties aren't available for Arbesk yet.*), through
`ext::Representations::properties(path)` answering the dialog's
`FileProperties` from the manifest: name, *Arbesk asset* as the type (the
model's format in brackets: *Arbesk asset (glTF)*), the collection as
*Parent folder*, *Modified* from the manifest's `timestamp`, *Version* N
(`version`), and no size, permissions, owner or access dates (Arbesk has
none, so those rows are skipped as Drive skips its local-only ones). `PROPERTIES_ITEM` in
`item-actions.ts:139` allows `arbesk` items that are assets.

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
  the profile (*Original Prusa MK4, PETG, 0.2 mm layers, 0.4 mm nozzle*)
  and stats (*2 h 3 min*, *38.5 g*) when present;
- a stale row carries an *Out of date* tag in `--warning`, whose tooltip
  says what changed: *Made from an earlier version of the model* (ref
  `primary`) or *Made from an earlier “source”* (another ref), or *Made
  from a file this asset no longer has* (the ref is gone).

Each row has a menu button (`openMenu`): **Open With…** (F8), **Copy**
(F9; puts the file's F4 path on the clipboard) and, from phase 4,
**Remove…** (F12). The model's row offers Open With… and Copy. Below the
list, a dim line: *Paste or drop files onto the asset to add them.*
(phase 4). The dialog is built with `openDialog` and closes with Close.

### Opening and copying

**F8. Open With asks which file when the app opens more than one.** Open
With… is offered for a single Arbesk **asset** (the rule moves from
`previewAndOpenWithItems`' local check, `main.ts:2543`, to `item-actions.ts`:
local items as today, plus Arbesk assets; collections, accounts and other
remote places stay disabled). The chooser (`open-with.ts`) is unchanged
until the user picks an app and presses Open; then, for an Arbesk asset,
`launch` (`open-with.ts:170`) hands over to a hook instead of `openWith`:

1. `app_formats(app, formats) -> string[]`, a new command: which of the
   asset's formats (the model's `gltf`/`3mf` and `entry.representations
   .formats`) the application declares it opens. Linux: GIO, already a
   dependency (`Cargo.toml:96`): each extension's content type
   (`gio::content_type_guess` on `x.<ext>`) and
   `gio::AppInfo::all_for_type`, which honours `MimeType=`, the MIME cache
   and subclasses (a text editor that opens `text/plain` opens G-code).
   macOS: `UTType` from the extension and NSWorkspace's
   `URLsForApplicationsToOpenContentType:` (macOS 12 and later; on an
   older macOS the answer is empty, so the model opens), compared with the
   app's bundle path. Answered from the OS, never from a
   table of our own.
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
   `service.rs:268`, a representation with F5) as one operation with
   progress and Cancel (*Opening “Bracket.gcode”…*), then
   `openWith(<cached file>, app)`.

Plain **Open** (double-click, Enter, the preview's Open) still opens the
asset in Arbesk's Studio (`provider.rs:169`): the default stays what it
is today. There is no "Open With <default app>" item to add: the app has
none for any file.

**F9. Copy out asks which files.** Copy (and a drag) still puts the
**asset** paths on the clipboard. When a paste or a drop copies Arbesk
assets out (to this computer or Google Drive, the routes Copy out has
today) and at least one of them has other files (its entry's summary), a
dialog asks before anything is copied:

- One asset: heading *Copy Which Files of “Box.gltf”?*, a boxed list of
  check rows: the model (checked) and each representation (unchecked),
  with the F7 subtitle; an **All Files** check row above them (checked when
  all are, mixed otherwise) toggles them all. Responses Cancel and Copy
  (suggested, insensitive while nothing is checked).
- Several assets: heading *Copy Which Files?*, one check row per kind
  instead of per file: *Models* (checked), then one row per format present
  across the assets (*G-code (3 assets)*, *STL (2 assets)*), and All Files.
  The choice applies to each asset.

The paste's sources become the chosen asset paths (the model) and F4 paths
(each chosen representation), in that order per asset; the engine copies
them as it copies anything (`Transfer::export` builds or fetches each into
the cache, the copy places it). **Name clashes still ask Replace / Skip /
Keep Both** for each file already in the target folder. Names repeated
**within** one paste (a `Box.stl` from two assets, or a representation
named like its model) are made unique first with the app's Keep Both
naming, so the dialog only asks about files that were there before. An
asset without other files copies as today, with no dialog. Cancel copies
nothing.

`places::Transfer::export` (`provider.rs`) answers an F4 path with the F5
fetch; the route table (`place::copy`) and `copyOutRefusal` treat an F4
path as an asset. A capabilities row **`arbesk-file`** joins `KINDS`,
`tests/capabilities.json` and `capabilities.ts`: an asset's booleans but
`has_thumbnails` false (and `can_attach` false, F10).

### Adding and removing

**F10. Paste Into Asset, or a drop on an asset, adds files as a new
version.** Sources: regular files on this computer, at most 256 MB each
(Arbesk R7), never folders. Two ways in, the same work:

- **Paste Into Asset** in an asset's context menu (beside Paste, as Paste
  Into Folder is for a folder, `main.ts:2669`), sensitive when the
  clipboard holds copied files. Plain Paste in a collection keeps today's
  meaning (model files become assets).
- A **drop on an asset** in the view: the asset highlights as a folder does
  under a drag, and the drop adds. A drop on the empty space of a
  collection keeps today's meaning. `dropOp` (`main.ts:3721`) learns an
  "attach" op for an item target whose capabilities say `can_attach`.

Refused with one toast, before anything starts: Cut or a forced-move drag
(*Arbesk is read only.*, as copy-in says), folders or files from Drive,
shares or SSH servers (*Only files on this computer can be added to an
asset.*), a file with no extension or one that isn't `[a-z0-9]{1,16}`
(*“README” has no file type to record.*), a `.gltf` whose buffers or images
point at files beside it (*“Box.gltf” uses files beside it. Add a .glb
instead.*, read with `models.ts`' `companionPath`, since only the one file
would be stored), a file over 256 MB (*“Print.gcode” is larger than
256 MB.*), and more files than the asset has room for (*An asset holds at
most 32 files.*, counting stored entries as `hasRoomFor` does, F1).

**What each file records**, as `besk attach` records it (Arbesk R6): format
= the lower-case extension; role = `default_role(format)` (a 3MF is a
`mesh`, as `defaultRole` says); `derived_from` = `primary` with the
model's CID and matrix for `machine` and `mesh`, none for the rest, and
none (silently) when the model has no source CID; `name` = the file's name;
`bytes` = its size; `encoding` = `encoding_for(format)`; `created_at` =
now. For a `.gcode` the first 4 KB and last 64 KB are read, best effort,
for what PrusaSlicer, OrcaSlicer, SuperSlicer, Bambu Studio and Cura write
there: `tool` (*generated by PrusaSlicer 2.9.2*), `profile` (`printer`,
`material`, `layer_mm`, `nozzle_mm`) and `stats` (`print_time_s`,
`filament_g`); a value that doesn't parse is left out. The id is
`representation_id(role, format, profile)`, the same slug `besk attach`
makes (`gcode-original-prusa-mk4-petg`), so the two clients name the same
file the same way.

**An id the asset already has asks, like a name clash**: *Replace
“Bracket.gcode”?* with the existing file's subtitle, responses Skip, Keep
Both and Replace (the app's conflict dialog, `main.ts:3181`, with "Apply
to all" when several clash). Replace swaps the entry (the old file stays
in the asset's earlier version, R4); Keep Both records the new file under
the first free `<id>-2`, `<id>-3`… (kept under 64 characters).

**A confirmation, every time** (copy-in's D2, extended): heading *Add to
“Box.gltf”?*, body *Files in Arbesk are public: anyone with their address
can download them, and they can't be removed from IPFS.* When a file is a
`source`, a second paragraph (Arbesk R7): *“Bracket.blend” is a source
file. It can hold more than the model, such as other scenes and embedded
images.* Responses Cancel (default) and Add.

**One version for one paste or drop.** All files of one paste are added in
one new asset version (version + 1, `prev_asset_manifest_cid` = the CID it
was built from, `timestamp` now), so a drop of three files is one save,
one relay call (Arbesk's relay allows 30 a minute) and one history step
(*Added G-code, STL*). It runs as one operation, *Adding 2 files to
“Box.gltf”*, with progress over the uploaded bytes and Cancel until the
collection manifest is uploaded, as copy-in does (`write.rs:402-407`).

**F11. The write: everything kept, compare-and-swap, nothing half done.**
`write::attach(…)` (in a new `crates/arbesk/src/attach.rs`, sharing
`write.rs`' `Uploader`, `CollectionLocks`, `settled_collection`,
`editor_proof`, `relay`, `next_collection` and wording):

1. Before any upload: the session (*Sign in to Arbesk again to copy files
   into it.* opens Sign In Again and carries on, as copy-in does), the
   editor proof (*You can't add to this collection.*), the asset's current
   manifest CID from a fresh collection read (`Reader::open_fresh`), the
   manifest itself, every F10 check, and each entry as it will be recorded
   checked by F1's validator with a placeholder CID (`checkEntry`,
   `besk/representations.ts:204`). A failure here uploads nothing.
2. Upload each file, encoded (F1). A failed or cancelled upload stops
   here: **the asset and the collection are unchanged**; what was uploaded
   stays on IPFS unreferenced, as for copy-in.
3. Build the next manifest **from the manifest read in step 1, as JSON**:
   every key kept (scene, thumbnail, metadata, comments, keys this app
   doesn't know), `representations` changed only by adding or replacing
   the new entries (entries it can't parse stay as they are, in place, as
   `withRepresentation` keeps them), version + 1, prev link, timestamp. A
   bare composite entry is first wrapped in an asset manifest whose model
   is that composite, as `asAssetManifest` does (`besk/representations.ts:101`).
   Upload it.
4. Under the collection's lock, read the settled collection (`write.rs:582`)
   and **compare-and-swap**: the collection's entry for the asset must
   still be the CID read in step 1. If it moved (a Studio save, `besk`,
   another window), stop with *“Box.gltf” changed while the files were
   being added. Nothing was changed; try again.* (the `CONFLICT` of
   `catalog.ts:184`); if the asset is gone, *“Box.gltf” is no longer in the
   collection.* Else write the next collection manifest with that one entry
   changed (`next_collection`), upload it and relay, with the outcomes
   copy-in has (*Arbesk is still saving “Box.gltf”…*, *Arbesk couldn't
   save “Box.gltf”.*, the relay's 502 words).

A retry after a conflict starts again from step 1 on the new tip, so it
keeps whatever the other save changed. The asset's model and every other
representation are never dropped by an attach: a test asserts the next
manifest equals the previous one but for the keys an attach owns.

**F12. Remove a file from Properties.** **Remove…** on a representation's
row (F7) asks: heading *Remove “Bracket.gcode” From “Box.gltf”?*, body
*Arbesk keeps it in the asset's earlier versions.*, responses Cancel and
Remove (destructive). It writes the next version without that entry
(`withoutRepresentation`: the key dropped when the list empties), with
F11's steps 1, 3 and 4 (nothing to upload). Files derived from the removed
one become out of date, which the dialog shows when it reopens. The
model can't be removed. Removing needs the session, as adding does.

**F13. Capabilities: `can_attach`.** `places::Operation` gains `Attach`;
`providers::capability_table` derives `can_attach` from it as it derives
`can_copy_in` (`providers.rs:168-190`): true only for the `arbesk-asset`
row. Every other row refuses it with its own read-only or local-only
words; an Arbesk account, collection or file with *Add files to an asset.*.
The column is added to `tests/capabilities.json`, `capabilities.ts`'
`PlaceCapabilities` and built-in table, and the mock's answer.

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
  a single-file glTF) fetches it (`ext::Representations::file`, F5) and
  previews it with `preview_bytes`, as a local model;
- anything else (a `.blend`, G-code until F17's viewer) shows the
  preview's file page: the file's icon, name, F7 subtitle and *Out of
  date* tag, and an Open With… button (F8, with that file chosen).

Stepping to another version shows its model again. The preview's Open
button still opens the asset in the Studio. The version menu's rows gain
the change a save made to the list (*Added G-code (Original Prusa MK4,
PETG)*, besk's `representationChanges` wording, `representations.ts:371`),
computed by comparing each manifest with the one before, which the version
walk already reads (`service.rs:368`).

### Data safety

**F15. Every write this app makes to an asset carries the list forward.**
First, before anything else ships: copy-in's **Replace** (`put_asset`,
`write.rs:602`) copies the replaced manifest's `representations` value
**verbatim** into the new version (as `besk`'s `nextModelEntry` does). They
become out of date against the new model, which is the point (Arbesk R3).
Without this, replacing a model from File Browser would silently drop its
G-code and `.blend` from the newest version.

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
entries as Arbesk stores them, plus each file's text or bytes. The fixtures
gain: *Bracket* in Shapes with a `.blend` (source), an STL (fresh), a G-code
for an Original Prusa MK4 in PETG (stale: derived from an earlier model
CID), a 3MF and one invalid entry (left out, but counted for room); and the
existing *Box* glTF with a single STL. The mock serves:

- listing summaries (F2) computed by a TypeScript copy of the stale rule
  over the mock manifest;
- `arbesk_representations`, `app_formats` (a fixed table per mock app:
  *PrusaSlicer* opens `3mf`, `gcode`, `stl`; *Blender* opens `blend`,
  `gltf`, `glb`, `stl`; *Text Editor* opens `gcode`; others nothing),
  fetching F4 paths through `open_target` and the representation file
  command, `arbesk_copy` of F4 paths, `arbesk_attach` and `arbesk_detach`
  (each a new mock version: version + 1, prev, the session check, a
  `conflictNext` switch that fails the next write with the CONFLICT words
  and changes nothing, `failUploadNext` likewise), and the capabilities
  table with `can_attach` and the `arbesk-file` row.

Each new mock command gets its case in `tests/mock-core.test.ts`.

## Testing

**Rust (`crates/arbesk`).**

- `representations.rs`: the shared stale fixture (F1); `list` keeps valid
  entries and drops each kind of invalid one (id, `primary`, role, format,
  a 256-unit name counted in UTF-16, fractional or over-cap `bytes`,
  encoding, detail maps of 33 keys); `stored_count` counts invalid ones;
  `decode` never unpacks a raw file starting with `ARB\x01` or the gzip
  magic, refuses an unframed `brotli` entry, stops past the cap; encode
  round trip; `representation_id` matches besk's slugs.
- `service.rs`: a listing's summaries (count, stale, formats), with the
  fake HTTP counting requests: the same number as without representations;
  a bare composite and a `child_ref` asset have none.
- `location.rs`: F4 paths parse and format, a pending name, a malformed
  id.
- `provider.rs`: `fetch` and `export` of an F4 path decode into
  `arbesk/files/<cid>/<name>` with the mark, reuse an unchanged copy, keep
  an edited one; capabilities of each path kind (`can_attach`).
- `write.rs` / `attach.rs`: Replace keeps `representations` verbatim (F15);
  attach writes version + 1 with the prev link, keeps every other key and
  entry (including an unparseable one, in place), replaces by id, adds
  several files in one version; every refusal happens before the first
  upload (the fake counts uploads); a failed upload, a cancel and a
  CONFLICT leave the collection manifest and the token URI unchanged; a
  bare composite is wrapped; G-code header parsing for PrusaSlicer and Cura
  samples; detach, and the key dropped when the list empties.
- `providers.rs`: the capability table with the new column and row, against
  `tests/capabilities.json`.
- App: `app_formats` on Linux with a temporary `XDG_DATA_DIRS` holding
  `.desktop` files and a MIME cache; macOS's half behind `cfg` with a test
  that Text Edit opens `txt`.

**Frontend (vitest, the mock).** `tests/arbesk-representations-view.test.ts`
(emblem and tooltip in grid and list views, the stale dot, emblem on a
thumbnail, markup unchanged after scrolling a virtualized list of 2,000
assets back and forth), `tests/arbesk-properties.test.ts` (asset
Properties rows, Files… rows and subtitles, out-of-date tags and their
tooltips; collections still refused), `tests/arbesk-open-with.test.ts`
(no candidates → the model; one → it opens; several → the choice dialog,
its default, Cancel; the cached file goes to `open_with`; progress and
Cancel of the fetch), `tests/arbesk-copy-out-files.test.ts` (one asset,
several assets, All Files' mixed state, Copy insensitive with nothing
checked, names made unique within a paste, Replace / Skip / Keep Both
against existing files, no dialog without representations),
`tests/arbesk-attach.test.ts` (Paste Into Asset and a drop, each refusal's
toast, the confirmation with and without the source paragraph, the id
clash dialog, CONFLICT's error with the asset unchanged, a failed upload,
Sign In Again then carrying on, Remove… and its confirmation) and
`tests/preview-representations.test.ts` (the switcher per version, an STL
previewed, a `.blend` on the file page, the version menu's change text).
`tests/capabilities.test.ts` and `tests/item-actions-matrix-network.test.ts`
gain the new column, row and actions.

The CRAP gate applies to each phase as to any change.

## Phases

Each phase is one PR, with its docs (`docs/arbesk.md`; AGENTS.md's layout
rows and Arbesk notes where a module is added) and its tests, and leaves
the app working.

1. **Backend read, and Replace keeps the list.** F15's Replace fix (first
   commit, on its own), F1 (`representations.rs`, `Gateway::raw`), F2 (the
   summary on `places::Entry` and the frontend type), F3
   (`ext::Representations`, `arbesk_representations`), F4 (paths), F5
   (fetching into the cache). Mock fields and commands, `mock-core` cases.
   Nothing visible changes but Replace no longer dropping files.
2. **View and Properties.** F6 (emblem, `--warning` in both themes), F7
   (asset Properties and Files…, read only: Open With… and Copy rows come
   in phase 3, Remove… in phase 4).
3. **Open With and copy-out.** F8 (`app_formats`, the hook in
   `open-with.ts`, the choice dialog, Open With… for Arbesk assets), F9
   (the copy dialog, `export` of F4 paths, the `arbesk-file` capabilities
   row), the Files… rows' Open With… and Copy.
4. **Adding and removing.** F10–F13 (`attach.rs`, `Operation::Attach` and
   `can_attach`, Paste Into Asset, drops on assets, the dialogs, Remove…).
5. **Preview switcher.** F14, and the version menu's change text.

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
`crates/places/src/provider.rs` (`Operation::Attach`, the extension slot),
`crates/arbesk/src/{lib,service,location,provider,write,ipfs}.rs`,
`src-tauri/src/{lib,providers,platform}.rs` (commands, `app_formats`,
Properties), `src-tauri/Cargo.toml` (the macOS `UTType` binding,
`objc2-uniform-type-identifiers`, checked with cargo-deny),
`src/{api,icons,main,open-with,item-actions,locations,paths,capabilities,preview-versions}.ts`,
`src/styles.css`, `src/themes/{gnome,macos}.css`, `dev/mock-tauri/core.ts`,
`dev/mock-tauri/models.ts`, `tests/capabilities.json`,
`tests/mock-core.test.ts`, `docs/arbesk.md`, AGENTS.md,
`docs/architecture.md` (the extension and the entry's new key).
