# Arbesk scenes in the Space preview — design

Date: 2026-11-09 · Status: approved; D3 and D5 settled after the final review

## Problem

An Arbesk asset is a scene, not always one model. Its manifest lists
`scene.nodes`, each with a `transform_matrix` and either a `source` (a stored
glTF or 3MF) or a `child_ref` (a link to another asset, in the same collection
or another). Example: the asset `asset_1785058266797` of collection "scaling"
is a table (node 0, raised 1.27 and scaled ×4) with two links to the
collection's `chair`, one each side, turned ∓90°.

Today the file browser previews only the first node's source, and refuses any
asset containing a link ("This asset uses an Arbesk format File Browser can't
read yet."). Five of the account's assets are refused that way: table, world,
new-contract, puppy and cave.

## Goal

Space on an Arbesk asset shows its whole scene as the Studio places it: every
node's model at its matrix, links followed, colour and scale applied. Assets
of one model look as they do now.

Out of scope: writing scenes, animations across parts, links to another chain
(skipped, see D4), the `.gltf` a later copy-out gives (unchanged: still the
first node's source, as `besk download` gives).

## Decisions

**D1. Rust resolves the scene; the preview places the parts (approach B).**
The backend reads the manifest, follows links and multiplies matrices, then
answers with a flat list of parts. Each distinct stored source is built once
as the preview already builds one (`compose_preview`: GLB for glTF, the 3MF
for 3MF), so two chairs share one download. The frontend loads each part's
bytes once and places a copy per placement with three.js. Nothing merges glTF
documents, so no index is renumbered.

**D2. The scene command.** `arbesk_scene(path) -> ArbeskScene`:

```
ArbeskScene { parts: [ { key, format: "glb" | "3mf",
                         placements: [ { matrix: [f64; 16],      // column-major, right-handed (D3)
                                         color: Option<String>,  // "#rrggbb", tints every mesh
                                         mesh_colors: { name: "#rrggbb" },
                                         scale: [f64; 3] } ] } ] }
```

`key` is the source's CID (letters and digits only). The bytes come from
`arbesk_part(path, key, limit)`, which builds or reuses
`<cache>/arbesk/<source cid>/part.<glb|3mf>` the way `build` does now (staged
`.part`, `rename_new`, marked, reused while unchanged, the size caps), and
refuses a key the asset's scene doesn't hold. `preview_bytes` for an Arbesk
path is no longer used by the preview.

**D3. Coordinates.** The Studio applies `transform_matrix` (column-major) to
a Babylon anchor in Babylon's default left-handed space, and Babylon's glTF
loader puts each model under a root that mirrors X (`rotation` 180° about Y,
`scaling` (1, 1, −1)). three.js is right-handed like glTF, so a placement's
matrix is `X · M · X` with `X = diag(−1, 1, 1, 1)` and `M` the product of the
matrices from the asset down to the node. A model alone (identity) stays as it
looks now; the scene's layout is mirrored the way the Studio shows it. This
is checked by eye against the Studio on the table scene before merging.

Each model is the right way round, as its glTF says. The Studio today shows
every model with a `post_processor.scale` mirrored: its `applyScale` replaces
that root's (1, 1, −1) instead of multiplying it (ahmadsayed/arbesk#138). The
human chose to keep the right handedness here rather than copy the mirror;
the two agree once the Studio is fixed.

As the Studio's `centerImportedAsset` does, the middle of each model's box
sits on its place, and the node's scale applies after that move. A 3MF part
is drawn in metres from the unit it declares, as the Studio's 3MF reader
does, and a manifest saved before 2026-10-17 06:19:37 UTC has its 3MF
placements in millimetres (the Studio's `normalizeLegacy3mfPlacements`).

**D4. Links.** A `child_ref { collection { chainId, contractAddress,
tokenId }, assetID }` is followed by asking the chain for that token's
manifest (`tokenURI` on its `contractAddress`, else the deployment's
contract), then that asset's manifest, as the Studio does; a link to `"self"`
is in the opened asset's own collection. Each link is asked once per scene,
and a scene stops at 2,000 copies. As in the Studio,
at most 5 levels deep, and a manifest already being resolved on the way down
is not entered again. A link to another chain, one that can't be resolved,
or one too deep is left out: the rest of the scene still shows. A scene with
no part at all says "No preview available."

**D5. Post-processor.** A node's `post_processor.color` tints every mesh of
its model (`meshOverrides[<mesh name>].color` first), as the Studio's
`applyColor` does; `post_processor.scale` multiplies the model's scale. A
tinted copy gets its own materials, so the other copies of a part keep
theirs.

**D6. Listing.** An asset whose manifest has more than one node, or a link,
is listed with the extension of its first source's format (`.gltf` / `.3mf`)
or `.gltf` when it has only links, so Space opens the model preview. Today
such assets list as `.gltf` (a link) or without an extension. The asset's
thumbnail is unchanged.

**D7. The preview.** For an Arbesk asset, `previewModelEntry` asks
`arbesk_scene`, then each part's bytes (`arbesk_part`), loads each part once
(`loadModel`, GLB recognised by its bytes as now), and builds one group:
for each placement, a clone of the part's object with the matrix, colour and
scale. The group goes to `showModel` as a single model, so turning, zooming,
the double-click pick and Reset View work on the whole scene. "Loading…"
covers the wait (#228). One part failing to build or load is left out; all of
them failing says "No preview available."

## Errors

| When | What the user sees |
|---|---|
| A link to another chain, unresolvable or past 5 levels | the scene without that part |
| Every part fails | "No preview available." |
| A part over the size cap | that part left out; all of them over: "This file is too large to preview." |
| The chain or storage unreachable while resolving | "No preview available." (and the folder's own listing error, as now) |

## Testing

- **Rust** (fake chain and gateway): a scene of a source and two links to one
  asset gives one part with three placements and the right matrices; nested
  links multiply; `X·M·X` on known matrices (a translation, a 90° turn about
  Y); a cycle and a 6th level are left out; another chain is left out; colour,
  mesh colours and scale carried; `arbesk_part` refuses a key outside the
  scene and reuses its file; listing gives the scene a `.gltf` name.
- **Frontend** (mock with a scene asset: one model, two linked copies, one
  tinted): Space builds one group with three placed copies, loads the shared
  part once, tints only the tinted copy, survives one failing part, and says
  "No preview available." when all fail.
- **Real data:** the five refused assets and a single-model one resolve and
  build (temporary test, not committed); the table scene is compared by eye
  with the Studio in the real app.

## Files

- `src-tauri/src/arbesk/scene.rs` (new): resolving a manifest into parts.
- `src-tauri/src/arbesk/service.rs`: `scene`, `part` (the source build split
  out of `build`), listing names (D6).
- `src-tauri/src/arbesk/mod.rs`, `src-tauri/src/lib.rs`: `arbesk_scene`,
  `arbesk_part`.
- `src/api.ts`, `src/main.ts` (`previewModelEntry`), `src/model-viewer.ts`
  (`placeScene`: the group of placed copies), `dev/mock-tauri/core.ts`.
- `docs/arbesk.md`, README, `docs/superpowers/specs/2026-11-08-arbesk-network-design.md` (D9: links now preview).
