# Versions in the Space preview — design

Date: 2026-11-09 · Status: proposed, waiting for the human's review

## Problem

Two kinds of item keep a history, and you can't see it in the Space preview:

- **A file in a Keep Versions folder.** Each save is recorded under
  `.versions/`. To see an older one you go through Versions… and then open a
  read-only copy in another app.
- **An Arbesk asset.** Each save in the Studio is a new manifest that points
  at the one before it (`prev_asset_manifest_cid`). File Browser shows only
  the latest.

## Goal

When an item has more than one version, Space lets you step through its
versions in place, each shown with the viewer the file already uses (3D
model, image or other thumbnail, text).

The preview is for looking only. Open still opens the current file, and
restoring stays in Versions….

Out of scope:
- restoring, replacing or opening an older version from the preview;
- comparing two versions side by side;
- Arbesk versions in Versions… or Properties;
- deleted files (Versions… → Deleted Files keeps them).

## Decisions

**D1. The version control.** It goes in the preview's header, next to the
name:

```
chair.gltf      ‹  Version 7 of 12 ▾  ›        ✕
                9 Nov 2026, 14:05
```

- `‹` steps to the older version and `›` to the newer one. Each is disabled
  at its end of the history.
- The label is a button that opens a popover list (`openPopover`) with one
  row per version, newest first: "Version N" and its date. The current one
  is marked "Current". Choosing a row shows that version.
- The preview opens on the current version. With one version, or none, the
  header is as it is now.
- The buttons are reached with Tab and labelled for screen readers ("Older
  Version", "Newer Version"). The arrow keys are left alone: they turn a 3D
  model.
- Each step shows "Loading…" (#228) until the version shows. A step made
  while one is loading drops the earlier one (the preview's `seq`, as when
  moving to another file).
- The control takes the theme's header-button and popover styles; nothing new
  to draw.

**D2. Numbering.** "Version N of M" counts from the oldest, which is 1.

- Keep Versions: M is the number of versions with content in the list that
  was read (D3), and a version's number is its position in that list, oldest
  first. Deletion markers are left out. `version_count` isn't used for M,
  because it counts deletion markers too.
- Arbesk: M is the length of the chain that could be read (D4), and N is the
  position in that chain. The manifest's own `version` field isn't used,
  because older saves may not carry it.

**D3. Keep Versions files.**
- The list comes from `list_versions`, newest first, 1,000 at most, as
  Versions… reads it. When `version_count` says there are more, the popover
  says "Showing the newest 1,000 versions." under the list, and numbering
  starts at the oldest one shown.
- The current version is the file itself, shown as now.
- An older version is materialized with `open_version` as the read-only
  copy that Versions… → Open already makes (in the app cache, pruned after a
  week). The preview shows that copy through the same path as any file
  (`previewEntry` on the copy's path, under the original's name, so the
  file kind is picked from the original's name).
- Copies are made only for the versions you step to, never for the whole
  list.
- A `.gltf` that names separate buffers or textures reads them from the
  original file's folder, as they are now: only the `.gltf` itself is
  versioned.
- The control appears only inside a folder that keeps versions
  (`insideVersionedRoot`), for a regular file whose list holds more than one
  version with content.

**D4. Arbesk assets.**
- New backend command `arbesk_versions(path) -> [{ id, timestamp }]`. It
  follows `prev_asset_manifest_cid` from the asset's manifest, up to 50
  manifests (the Studio's `walkManifestChain` limit), and answers oldest
  first.
  - `id` is the manifest's CID.
  - `timestamp` is the manifest's `timestamp` in milliseconds, or null.
- A manifest in the chain that can't be fetched ends the chain there: the
  versions read so far still show.
- `arbesk_scene(path, version)` takes an optional manifest CID and resolves
  the scene from it instead of the asset's current manifest.
  - The CID must be in that asset's chain; any other one is refused with
    "This version is not in the asset.", so a path can't be used to read
    arbitrary content.
- `arbesk_part(path, version, key, limit)` checks the key against the scene
  of that version (kept per path and version, as `Scenes` keeps them now).
- Parts stay keyed by the source's CID, so a model that didn't change
  between versions is fetched and built once.
- The control appears for an Arbesk asset whose chain has more than one
  manifest. The mock gets a versioned asset.

**D5. Errors.**

| When | What the user sees |
|---|---|
| The version list can't be read | the preview as now, with no control |
| One version can't be made or read | that version says "No preview available.", and the control still works |
| An Arbesk chain breaks partway | the versions up to the break |
| A version over the size limit | "This file is too large to preview." for that version |

## Testing

- **Rust** (fake chain and gateway):
  - `arbesk_versions` gives the chain oldest first;
  - it stops at 50 and at a manifest that can't be fetched;
  - a scene resolves from an older manifest;
  - a CID outside the chain is refused;
  - a part is checked against its version's scene.
- **Frontend** (the mock gets an Arbesk asset with three saves, each a
  different model, and a versioned local text file and image):
  - Space shows "Version 3 of 3" on the current version;
  - `‹` shows the older one, and its content changes;
  - the popover jumps to any version;
  - each end button is disabled at its end;
  - a version that fails says so and the control still works;
  - a quick second step wins;
  - a file with one version has no control;
  - Open still opens the current file.
- **Screenshots** of the control, in light and dark.
- **Real data:** in the real app, step through an Arbesk asset that has
  several saves, and through a file in a Keep Versions folder.

## Files

- `src-tauri/src/arbesk/service.rs`, `arbesk/mod.rs`, `lib.rs`:
  `arbesk_versions`, and the `version` argument of `arbesk_scene` and
  `arbesk_part`.
- `src/api.ts`, `src/main.ts`: the preview's version control, and loading a
  version through the existing `previewEntry` paths.
- `src/styles.css`, `index.html`: the header control.
- `dev/mock-tauri/core.ts`: Arbesk versions, versioned mock files.
- `docs/arbesk.md`, README (feature list), AGENTS.md (the layout table, if a
  module is added).
