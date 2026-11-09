# Arbesk Scenes in the Space Preview — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Space on an Arbesk asset shows its whole scene: every node's model at its matrix, links followed, colour and scale applied.

**Architecture:** Rust resolves an asset's manifest into distinct parts (one per stored source) with their placements (`arbesk_scene`), and builds each part's bytes on request (`arbesk_part`, the existing GLB/3MF preview build). The frontend loads each part once and places a clone per placement in one three.js group, shown by the existing viewer.

**Tech Stack:** Rust (`serde`, the `arbesk` module's `Http`, `Gateway`, `chain`), TypeScript, three.js, Vitest + the mock.

**Spec:** `docs/superpowers/specs/2026-11-09-arbesk-scenes-design.md` (D1–D7).

## Global Constraints

- Matrices are 16 numbers, column-major. A placement's matrix is `X · M · X`, `X = diag(−1, 1, 1, 1)`, `M` = product of matrices from the asset down to the node (parent first).
- Links: at most 5 levels deep (`MAX_LINK_DEPTH = 5`); a manifest CID already on the way down is not entered again; a link whose `collection.chainId` differs from the account's is left out; unresolvable links are left out.
- `post_processor.color` and `meshOverrides[name].color` are `"#rrggbb"`; `post_processor.scale` `{x,y,z}`, a missing axis is 1.
- Part keys are source CIDs (letters and digits only, as `ipfs::checked_cid`); `arbesk_part` refuses a key the asset's scene doesn't hold.
- Part files: `<cache>/arbesk/<source cid>/part.glb` or `part.3mf`, staged `.part` + `rename_new` + `downloads::mark`, reused while unchanged, `Builds` hold, size caps (`compose_preview`).
- Messages: none new. Every part failing → "No preview available."; all parts too large → "This file is too large to preview."
- Plain functions in `arbesk/`; only `lib.rs` touches Tauri; every new command has a mock and a `tests/mock-core.test.ts` case; CRAP gate ≤ 30.

## Review Focus

1. **A link back to the asset itself, or two assets linking each other**: the preview must still show what isn't looped, never hang. Task 1 tests a cycle.
2. **A scene whose first node is a link** (no `source` at node 0): it must preview the linked models. Task 1 tests it.
3. **The same source used by two different nodes with different colours**: one download, two differently tinted copies, the untinted copy keeps its own colour. Task 3 tests it.
4. **A part that fails to build (gateway down for one CID)**: the rest of the scene shows. Task 3 tests it in the app.
5. **A single-model asset** (most assets): it must look as it does today (identity placement, no tint). Tasks 1 and 3 test it.

---

### Task 1: Resolving a manifest into parts (Rust, no network)

**Files:**
- Create: `src-tauri/src/arbesk/scene.rs`
- Modify: `src-tauri/src/arbesk/mod.rs` (`pub mod scene;`)

**Interfaces:**
- Produces:
  - `pub type Matrix = [f64; 16];` `pub const IDENTITY: Matrix;` `pub fn multiply(a: &Matrix, b: &Matrix) -> Matrix` (column-major, `a·b`); `pub fn to_right_handed(m: &Matrix) -> Matrix` (`X·m·X`).
  - `#[derive(Serialize, Debug, Clone, PartialEq)] pub struct Placement { pub matrix: Vec<f64>, pub color: Option<String>, pub mesh_colors: BTreeMap<String, String>, pub scale: [f64; 3] }`
  - `#[derive(Serialize, Debug, Clone, PartialEq)] pub struct Part { pub key: String, pub format: &'static str /* "glb" | "3mf" */, pub placements: Vec<Placement> }`
  - `#[derive(Serialize, Debug, Clone, PartialEq, Default)] pub struct Scene { pub parts: Vec<Part> }` (parts in first-seen order)
  - `pub trait Links { fn manifest(&self, cid: &str) -> Result<Value, String>; fn linked(&self, child_ref: &Value) -> Result<String /* asset manifest cid */, String>; fn format(&self, source_cid: &str) -> Result<Format, String>; }`
  - `pub fn resolve(asset_cid: &str, links: &dyn Links) -> Scene`

- [ ] **Step 1: Write the failing tests** in `scene.rs`, with a fake `Links` over a `HashMap` of manifests, links (`assetID` → cid) and formats:
  - `multiply` of a translation by a 90° turn about Y, against known numbers; `to_right_handed` of a translation `(2, 1, 3)` gives `(-2, 1, 3)`, of a +90° turn about Y gives −90°; `to_right_handed(IDENTITY) == IDENTITY`.
  - `a_single_model_is_one_part_placed_as_is`: one node, source `bafyS`, no matrix → one part `key "bafyS"`, format `"glb"`, one placement with `IDENTITY`, `color None`, `scale [1,1,1]`.
  - `the_table_scene`: node 0 source `bafyT` with a matrix translating `(0, 1.27, 0)` scaled 4; nodes 1–2 `child_ref` to `chair` (cid `bafyC`, one node, source `bafyS`) at translations `(-2.57, 1.95, 0)` and `(2.08, 1.95, 0.03)` → two parts (`bafyT` with one placement, `bafyS` with two), each matrix `to_right_handed(parent · node)`.
  - `nested_links_multiply`: A links B at translation `(1,0,0)`, B links C at `(0,2,0)`, C has source `bafyS` → one placement translated `(-1, 2, 0)` after `X·M·X`.
  - `a_cycle_is_left_out`: A links B, B links A and has source `bafyS` → one part, one placement; returns.
  - `past_five_levels_is_left_out`: a chain of 7 links ending in a source → no part.
  - `an_unresolvable_link_or_unknown_source_is_left_out`: `linked` errors, or `format` errors → that node only is skipped.
  - `colour_mesh_colours_and_scale_are_carried`: `post_processor { color: "#ff0000", meshOverrides: { seat: { color: "#00ff00" } }, scale: { x: 2 } }` → `color Some("#ff0000")`, `mesh_colors {"seat": "#00ff00"}`, `scale [2, 1, 1]`.
  - `a_scene_starting_with_a_link`: node 0 is a link → its model's part is there.
- [ ] **Step 2: Run** `cd src-tauri && cargo test --lib arbesk::scene` — fails to compile.
- [ ] **Step 3: Implement** the interfaces. Depth counts link levels from the asset (0); a link at depth ≥ `MAX_LINK_DEPTH` is skipped. `seen` holds manifest CIDs on the current path (insert before descending, remove after). Format strings come from `Format::preview_ext()`.
- [ ] **Step 4: Run** the same command — passes.
- [ ] **Step 5: Commit** `feat: resolve an Arbesk asset's scene into parts and placements`

### Task 2: The scene and part commands (Rust)

**Files:**
- Modify: `src-tauri/src/arbesk/service.rs` (`scene`, `part`, `Reader` implements `Links`), `src-tauri/src/arbesk/mod.rs` (`ArbeskState::scene`, `ArbeskState::part`), `src-tauri/src/lib.rs` (`arbesk_scene`, `arbesk_part`, registered)

**Interfaces:**
- Consumes: Task 1.
- Produces:
  - `pub fn scene(http, site: &Site, path: &str, cache_dir: &Path) -> Result<Scene, String>`: asset paths only (`“…” is a folder, not a file.` otherwise).
  - `pub fn part(http, site, path: &str, key: &str, cache_dir: &Path, progress: &Progress, builds: &Builds) -> Result<PathBuf, String>`: refuses a key not in `scene(...)`'s parts with "This part is not in the asset."; builds `<cache>/arbesk/<key>/part.<glb|3mf>`.
  - `Reader::linked(child_ref)`: `chainId` ≠ `site.chain_id` → Err; `tokenURI(tokenId)` through `collection.contractAddress` when given, else the config's; then `assets[assetID]`.
  - Commands: `arbesk_scene(path) -> Scene` and `arbesk_part(path, key, limit) -> bytes (tauri::ipc::Response)`, both `async` through `blocking`; `limit` as `preview_bytes`'s.

- [ ] **Step 1: Write the failing tests** in `service.rs`'s tests (the fake Arbesk world gains the table scene: a table source from fixture `glb-box`, links to `Box` in Shapes): `the_table_asset_resolves_to_two_parts` (keys, placement counts); `a_part_is_built_once_and_reused` (second call fetches nothing); `a_key_outside_the_scene_is_refused`; `a_link_to_another_chain_is_left_out`; `a_link_resolves_through_its_own_contract` (the fake RPC answers only for the link's contract address); `the_table_asset_lists_as_gltf` (D6: a scene lists as `Table.gltf`, which today's naming already gives).
- [ ] **Step 2: Run** `cd src-tauri && cargo test --lib arbesk::` — fails.
- [ ] **Step 3: Implement** `scene`, `part` (split the source-building half out of `build`; `build` keeps working for `preview_bytes`), the state methods and commands.
- [ ] **Step 4: Run** `cd src-tauri && cargo test` — passes.
- [ ] **Step 5: Commit** `feat: answer an Arbesk asset's scene and build its parts`

### Task 3: The preview places the scene (frontend)

**Files:**
- Modify: `src/api.ts` (`ArbeskScene` types, `arbeskScene(path)`, `arbeskPart(path, key, limit)`), `src/model-viewer.ts` (`placeScene`), `src/main.ts` (`previewModelEntry`), `dev/mock-tauri/core.ts` (`arbesk_scene`, `arbesk_part`, a scene asset "Table" in Shapes: one `Box` part and one `Box.3mf`-like part, or two copies of the box, one tinted `#ff0000`)
- Test: `tests/model-viewer-scene.test.ts` (new), `tests/model-preview.test.ts`, `tests/mock-core.test.ts`

**Interfaces:**
- Consumes: Task 2's commands (shape of `Scene` as JSON: `{ parts: [{ key, format, placements: [{ matrix, color, mesh_colors, scale }] }] }`).
- Produces: `export function placeScene(parts: { object: Object3D; placements: ScenePlacement[] }[]): Group` in `model-viewer.ts`, `ScenePlacement = { matrix: number[]; color: string | null; mesh_colors: Record<string, string>; scale: [number, number, number] }`: one clone per placement (`object.clone(true)`), `matrix` applied (`matrixAutoUpdate` stays on: decompose into position/quaternion/scale, then multiply scale by `scale`), materials cloned before tinting (mesh name in `mesh_colors` first, else `color`).

- [ ] **Step 1: Write the failing tests:**
  - `tests/model-viewer-scene.test.ts`: two placements of one box (identity and a translation `(3,0,0)`) give a group of 2 children at x 0 and 3; the tinted copy's material colour is `#ff0000` and the untinted copy's isn't; `mesh_colors` beats `color` for that mesh; `scale [2,1,1]` doubles x.
  - `tests/model-preview.test.ts`: Space on the mock's `Table.gltf` shows the model view with `showModel` called once on a group of 3 placed copies, and `arbesk_part` asked once per distinct key; with `arbesk_part` failing for one key the other copies still show; failing for all says "No preview available."; a single-model asset (`Box.gltf`) still shows its model; parts adding up to more than `MODEL_PREVIEW_LIMIT` say "This file is too large to preview."
  - `tests/mock-core.test.ts`: `arbesk_scene` and `arbesk_part` cases (shape, refused key).
- [ ] **Step 2: Run** `npx vitest run tests/model-viewer-scene.test.ts tests/model-preview.test.ts tests/mock-core.test.ts` — fails.
- [ ] **Step 3: Implement.** In `previewModelEntry`, an Arbesk path (`locationKind(entry.path, trashPath) === "arbesk"`) goes through `arbeskScene` → `arbeskPart` per part (sequentially, total bytes ≤ `MODEL_PREVIEW_LIMIT`) → `loadModel` (format from the bytes: GLB by `isGlb`, else `"3mf"`) → `placeScene` → `showModel`. Other paths unchanged.
- [ ] **Step 4: Run** the same command, then `npx tsc --noEmit` — pass.
- [ ] **Step 5: Commit** `feat: show an Arbesk asset's whole scene in the Space preview`

### Task 4: Real data, docs, PR

**Files:**
- Modify: `docs/arbesk.md`, `README.md` (links now preview; limits), `docs/superpowers/specs/2026-11-08-arbesk-network-design.md` (D9: links preview through scenes)

- [ ] **Step 1:** Temporary ignored Rust test (not committed): `scene` + `part` for the account's assets, the five previously refused included; each part checked with the GLB structural check used for #229. Remove it.
- [ ] **Step 2:** Docs.
- [ ] **Step 3:** Full checks: `npx tsc --noEmit`, `npm run build`, `npm test`, `cd src-tauri && cargo test`; commit `docs: Arbesk scenes`.
- [ ] **Step 4:** Mock screenshot of the Table scene (light and dark), push, PR with the template, self-review comment; ask the human to compare the table scene with the Studio in the real app (D3), and add the line to #224.
