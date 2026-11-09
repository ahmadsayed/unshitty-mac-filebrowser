# Versions in the Space preview Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Space lets you step through the versions of a Keep Versions file or
an Arbesk asset, each shown with the viewer the file already uses.

**Architecture:** The backend gains `arbesk_versions` (the manifest chain), and
a `version` argument on `arbesk_scene` and `arbesk_part`. Keep Versions needs
nothing new: `list_versions` and `open_version` exist. A new frontend module,
`src/preview-versions.ts`, turns either history into one list and draws the
header control. `main.ts` then shows a chosen version through the existing
`previewEntry` paths, reading from a different place (`PreviewAt`).

**Tech Stack:** Rust (Tauri 2 commands, serde_json), TypeScript (no
framework), Vitest + jsdom against `dev/mock-tauri/`.

**Spec:** `docs/superpowers/specs/2026-11-09-preview-versions-design.md`

## Global Constraints

- Look only: nothing in this plan writes to the user's files or to Arbesk.
  The only file made is `open_version`'s read-only copy in the app cache.
- Wording (exact):
  - "Version N of M"; "Current";
  - "Older Version" / "Newer Version" (button labels and tooltips);
  - "Showing the newest 1,000 versions.";
  - "This version is not in the asset.";
  - "No preview available."; "This file is too large to preview.".
- At most 50 Arbesk manifests in a chain (`MAX_VERSIONS = 50`), and 1,000
  local versions listed (`LOCAL_VERSION_LIMIT = 1000`).
- Numbering counts from the oldest version, which is 1.
- Use the theme tokens and existing widgets (`openMenu` with `radio` items),
  with no new colours. Escape names that go into `innerHTML` (better: use
  `textContent`).
- `rustfmt --edition 2024` only on the files you change (it follows `mod`
  declarations: put back any other file it touches). CRAP ≤ 30 on every
  commit; never use `--no-verify`.
- Every new mock command or argument gets a case in `tests/mock-core.test.ts`.

## Review Focus

1. **A step while a version is loading:** the later step wins, and nothing
   from the earlier one lands in the preview. Test: "a quick second step
   shows only the second version".
2. **An Arbesk version id from outside the asset's chain:** it is refused,
   and no gateway read happens for it. Test:
   `a_version_outside_the_chain_is_refused`.
3. **A local history whose newest row is a deletion marker, or with
   deletion markers in between:** markers are left out of the list and the
   numbering. Test: "deletion markers are not versions".
4. **Closing the preview, or moving to another file, during a version
   load:** nothing shows afterwards (`previewStale`). Covered by the test in
   point 1 with Escape.
5. **A version that fails while others work:** that one says "No preview
   available.", and the control stays usable. Test: "a version that fails
   says so; the others still show".

---

### Task 1: Arbesk versions in the backend

**Files:**
- Modify: `src-tauri/src/arbesk/service.rs` (`versions`, `scene` and `part` take a version, the chain walk, tests)
- Modify: `src-tauri/src/arbesk/mod.rs` (`ArbeskState::versions`, and `scene`/`part` with a version, the kept-scene key)
- Modify: `src-tauri/src/lib.rs` (`arbesk_versions` command and registration; `version: Option<String>` on `arbesk_scene` and `arbesk_part`)

**Interfaces:**
- Produces:
  - `service::Version { id: String, timestamp: Option<i64> }` (Serialize).
  - `pub const MAX_VERSIONS: usize = 50`.
  - `service::versions(http, site, path, cache_dir) -> Result<Vec<Version>, String>`, oldest first.
  - `service::scene(http, site, path, version: Option<&str>, cache_dir) -> Result<Scene, String>`.
  - Commands:
    - `arbesk_versions(path) -> Vec<Version>`;
    - `arbesk_scene(path, version: Option<String>)`;
    - `arbesk_part(path, version: Option<String>, key, limit)`.
  - The JSON is `{ id, timestamp }` per version.
- Consumes: `open_asset`, `Reader`, `resolve`, `Scenes` (from #230).

- [ ] **Step 1: Write the failing tests in `service.rs`.** Add a chain to the
  test world:
  - a collection token 5, "History", holding asset `h1` → `bafyH3`;
  - `bafyH3` (`timestamp` 3000, `prev_asset_manifest_cid` `bafyH2`) → `bafyH2` (2000, prev `bafyH1`) → `bafyH1` (1000, no prev);
  - each wraps its own source: `bafyH3` and `bafyH2` wrap `bafySrcGltf`, `bafyH1` wraps `bafySrc3mf`.

  Tests:
  - `an_assets_versions_are_its_manifest_chain_oldest_first`: `versions(.., h1 path, ..)` gives ids `["bafyH1","bafyH2","bafyH3"]` and timestamps `[Some(1000), Some(2000), Some(3000)]`.
  - `a_chain_stops_at_fifty`: 60 chained manifests → `len() == MAX_VERSIONS`, and the last id is the asset's own manifest (newest kept, oldest cut).
  - `a_chain_stops_where_a_manifest_cant_be_read`: `bafyH1` in `w.failing` → ids `["bafyH2","bafyH3"]`.
  - `an_older_version_resolves_its_own_scene`: `scene(.., Some("bafyH1"), ..)` gives one part keyed `bafySrc3mf` with format `"3mf"`; `scene(.., None, ..)` gives `bafySrcGltf`.
  - `a_version_outside_the_chain_is_refused`: `scene(.., Some("bafyA1"), ..)` → `Err("This version is not in the asset.")`, and the requests show no fetch of `bafyA1`'s sources.
  - Update the existing `scene(…)` and `table_scene` calls to pass `None`.

- [ ] **Step 2: Run them and watch them fail.**
  Run: `cd src-tauri && cargo test --lib arbesk::service 2>&1 | grep -E "^error|test result"`
  Expected: compile errors (`versions` is not found; `scene` takes 4 arguments).

- [ ] **Step 3: Implement the chain walk, `versions`, and `scene` with a version.**
  - `fn chain(reader: &Reader, cid: &str) -> Vec<(String, Value)>`: walk `prev_asset_manifest_cid` from `cid`, newest first, stop after `MAX_VERSIONS` or at a manifest that fails, then reverse.
  - `versions` maps it to `Version` (the `timestamp` read with `as_i64`).
  - `scene`: when `version` is `Some(v)`, `v` must be among the chain's ids, or the result is `Err("This version is not in the asset.")`; then `resolve(v, &reader)`.
  - Keep each function small for the CRAP gate.

- [ ] **Step 4: Run them and watch them pass.**
  Run: `cd src-tauri && cargo test --lib arbesk:: 2>&1 | grep -E "panicked|test result"`
  Expected: `test result: ok`.

- [ ] **Step 5: Wire up the state and the commands.**
  - `ArbeskState::versions(path)`.
  - `scene(path, version: Option<&str>)` and `part(path, version: Option<&str>, key, progress)` keep scenes under `format!("{path}\n{}", version.unwrap_or(""))`.
  - In `lib.rs`: the `arbesk_versions` command through `blocking`, registered next to `arbesk_scene`, and `version: Option<String>` passed through on the two commands.
  - Run: `cd src-tauri && cargo test 2>&1 | grep -E "^test result|FAILED"`. Expected: all ok.

- [ ] **Step 6: Commit** (`feat: read an Arbesk asset's versions and their scenes`).

### Task 2: The mock keeps versions

**Files:**
- Modify: `dev/mock-tauri/core.ts`
- Test: `tests/mock-core.test.ts`

**Interfaces:**
- Consumes: Task 1's command names and arguments.
- Produces, in the mock:
  - `arbesk_versions({ path })`: Cube (`asset_1` in Untitled Collection) has three versions, `[{ id: "cube-v1", timestamp }, { id: "cube-v2", … }, { id: "cube-v3", … }]`, oldest first. Every other asset has one, `[{ id: "<asset id>-v1", … }]`.
  - `arbesk_scene({ path, version })`: Cube's version `cube-vN` has N copies of `box-src` at x = 0, 50, 100. Version null is the newest. Any other id throws "This version is not in the asset.".
  - `arbesk_part({ path, version, key, limit })` checks the key against that version's scene.
  - `open_version` now makes the read-only copy as a file in the mock filesystem at `${OPENED}/<name>`, creating `OPENED` if needed, so `preview_text` and `thumbnail` can read it. It still returns that path.

- [ ] **Step 1: Write the failing cases** in `tests/mock-core.test.ts`:
  - Cube's versions are `cube-v1..3`, oldest first;
  - `arbesk_scene` for `cube-v2` gives 2 placements;
  - an unknown version throws the refusal;
  - `arbesk_part` with `version: "cube-v1"` refuses a key that isn't in that version (use a key only in the Table scene);
  - after `open_version` of `notes.txt` version 2, `preview_text` of the returned path starts with `Preview of /Users/me/.cache/opened-versions/`.
- [ ] **Step 2: Run** `npx vitest run tests/mock-core.test.ts`. Expected: the new cases FAIL.
- [ ] **Step 3: Implement** them in the mock.
- [ ] **Step 4: Run** the same command. Expected: PASS.
- [ ] **Step 5: Commit** (`test: the mock keeps Arbesk and local versions`).

### Task 3: One version list for both kinds (`src/preview-versions.ts`)

**Files:**
- Create: `src/preview-versions.ts`
- Modify: `src/api.ts` (`ArbeskVersion`, `arbeskVersions(path)`, and `version?: string | null` on `arbeskScene`/`arbeskPart`)
- Test: `tests/preview-versions.test.ts`

**Interfaces:**
- Produces:
  ```ts
  export interface PreviewVersion { id: string; date: number | null /* ms */; current: boolean }
  export const LOCAL_VERSION_LIMIT = 1000;
  export function fromLocal(rows: VersionInfo[]): PreviewVersion[];       // oldest first, no deletion markers
  export function fromArbesk(rows: ArbeskVersion[]): PreviewVersion[];    // oldest first, last is current
  export function versionTitle(index: number, total: number): string;     // "Version 3 of 12"
  export function versionMenu(versions: PreviewVersion[], shown: number, pick: (index: number) => void): MenuItem[];
  ```
  - `api.ts` gets `arbeskVersions(path): Promise<ArbeskVersion[]>`, `arbeskScene(path, version?: string | null)` and `arbeskPart(path, key, limit, version?: string | null)`.
  - The existing callers keep working (`version` defaults to null).
- `versionMenu` lists newest first. Each item is `{ label: "Version N", accel: <formatDate(date) or "">, radio: index === shown, action }`. The current one's label is "Version N — Current".

- [ ] **Step 1: Write the failing tests:**
  - "deletion markers are not versions": `fromLocal` of rows `[current id 3, id 2, deleted id 0, id 1]` (newest first, as the backend gives them) → ids `["1","2","3"]`, the last one current, dates in ms (`mtime * 1000`).
  - "Arbesk versions: the newest is current": `fromArbesk` keeps the order and marks the last one; a null timestamp gives a null date.
  - `versionTitle(2, 12)` is `"Version 3 of 12"`.
  - "the menu lists newest first and marks the shown one": for 3 versions with 1 shown, the labels are `["Version 3 — Current", "Version 2", "Version 1"]`, only "Version 2" has `radio: true`, and its action calls `pick(1)`.
- [ ] **Step 2: Run** `npx vitest run tests/preview-versions.test.ts`. Expected: FAIL (the module doesn't exist).
- [ ] **Step 3: Implement** the module and the `api.ts` changes.
- [ ] **Step 4: Run** it again, plus `npx tsc --noEmit`. Expected: PASS, and tsc is clean.
- [ ] **Step 5: Commit** (`feat: one list of versions for the preview`).

### Task 4: The preview's version control

**Files:**
- Modify: `index.html` (`<div class="preview-versions" id="preview-versions" hidden></div>` between `#preview-name` and `#preview-close`)
- Modify: `src/styles.css` (`.preview-versions`, using the header-button tokens)
- Modify: `src/main.ts`
- Test: `tests/preview-versions-ui.test.ts` (new; it boots the app as `model-preview.test.ts` does, with `showModel` faked)

**Interfaces:**
- Consumes: Task 3's module and API; the mock from Task 2.
- Produces in `main.ts`:
  - `interface PreviewAt { readPath?: string; arbeskVersion?: string | null }`.
  - `previewEntry(entry, seq, kind, at: PreviewAt = {})`. The text, thumbnail and model paths read from `at.readPath ?? entry.path`. A .gltf's companions still come from `entry.path`'s folder. An Arbesk scene uses `at.arbeskVersion`.
  - `resetPreviewBody(): number`: bumps `previewSeq`; clears the body, the object URL and `modelView`; keeps the layer, name and actions. `closePreview` uses it.
  - `loadPreviewVersions(entry, seq)`, called from `showPreview`, fills `#preview-versions` when there is more than one version. It holds:
    - a "‹" button (`aria-label`/title "Older Version", `sym("go-previous")`);
    - a label button: "Version N of M" over the date, which opens `openMenu(label, versionMenu(...))`;
    - a "›" button ("Newer Version", `sym("go-next")`);
    - the note "Showing the newest 1,000 versions." in the menu as a `{ heading }`, when `versionCount` is above the number of rows read.
  - `showPreviewVersion(entry, versions, index)`: `resetPreviewBody()`, then "Loading…" (`previewLoading`), then:
    - the current version: `previewEntry(entry, seq, kind)`;
    - a local version: `openVersion(entry.path, Number(id), restoreName(entry.path, date / 1000))`, then `previewEntry` with `{ readPath }`;
    - an Arbesk version: `previewEntry` with `{ arbeskVersion: id }`.

    A failure says "No preview available.". The control is redrawn for the new index.
  - Which history applies: `locationKind(entry.path) === "arbesk"` → `arbeskVersions`; a regular file `insideVersionedRoot(entry.path, versionedRootPath)` → `listVersions(path, LOCAL_VERSION_LIMIT, 0)`; nothing else.

- [ ] **Step 1: Write the failing tests:**
  - "an Arbesk asset with three saves shows Version 3 of 3": Space on `Cube.gltf` in Untitled Collection → `#preview-versions` is visible with "Version 3 of 3"; "Newer Version" is disabled; the shown scene has 3 copies.
  - "Older Version shows the older save": click "Older Version" → "Version 2 of 3", and the scene passed to `showModel` has 2 copies; both buttons are enabled.
  - "the list jumps to any version": click the label, choose "Version 1" → "Version 1 of 3", 1 copy, "Older Version" disabled.
  - "a quick second step shows only the second version": click "Older Version" twice without settling → `showModel`'s last scene has 1 copy, and it was called once after the first render.
  - "a local text file steps to its older version": in `/Users/me/Documents/Versioned`, Space on `notes.txt` → "Version 3 of 3" (the deletion marker isn't counted); "Older Version" → the `<pre>` text starts `Preview of /Users/me/.cache/opened-versions/`.
  - "a version that fails says so; the others still show": `failNext("open_version", …)` → that version says "No preview available."; "Newer Version" brings back the current text.
  - "one version, no control": Space on `image.png` in the versioned folder, and on a file outside it → `#preview-versions` is hidden.
  - "Open still opens the current file": on an older local version, the preview's Open opens `notes.txt`'s own path (check the mock's open call).
  - "Escape during a version load shows nothing afterwards": click "Older Version", then Escape before settling → the layer is hidden and `showModel` isn't called again.
- [ ] **Step 2: Run** `npx vitest run tests/preview-versions-ui.test.ts`. Expected: FAIL.
- [ ] **Step 3: Implement** the markup, styles and `main.ts` changes listed above.
- [ ] **Step 4: Run** `npx vitest run tests/preview-versions-ui.test.ts tests/model-preview.test.ts tests/preview-versions.test.ts tests/mock-core.test.ts` and `npx tsc --noEmit`. Expected: PASS, and tsc is clean.
- [ ] **Step 5: Commit** (`feat: step through versions in the Space preview`).

### Task 5: Docs, checks, screenshots

**Files:**
- Modify: `README.md` (feature list: versions in the preview), `docs/arbesk.md` (versions paragraph), `AGENTS.md` (a layout-table row for `src/preview-versions.ts`), `docs/architecture.md` if it lists the preview's modules.

- [ ] **Step 1: Update the docs.**
- [ ] **Step 2: Run the full checks:** `npx tsc --noEmit`, `npm run build`, `npm test`, and `cd src-tauri && cargo test`. Expected: all pass.
- [ ] **Step 3: Take screenshots** with `npm run dev:mock -- --port 1421` (Space on Cube.gltf at Version 2 of 3, and on notes.txt at an older version), light and dark, and look at them.
- [ ] **Step 4: Commit** (`docs: versions in the Space preview`).
