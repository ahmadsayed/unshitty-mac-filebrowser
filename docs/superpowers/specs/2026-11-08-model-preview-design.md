# 3D Model Preview — Design

**Date:** 2026-11-08
**Status:** implemented (branch `feat/model-preview`)
**Related:** `AGENTS.md` (rules every change must keep)

## Intent

Space previews 3D models the way it previews text and pictures: glTF and
GLB, STL and 3MF, shown in the preview panel to turn with the pointer and
zoom with the wheel. The user asked for the preview only, with no
thumbnails yet.

## Decisions

- **D1. three.js, loaded on demand.** Its loaders read all four formats
  (glTF and GLB, ASCII and binary STL, 3MF with its own unzip and the
  webview's XML parser), so the app parses nothing itself. Alternatives
  weighed: OGL (smallest, about 29 KB, but STL and 3MF parsers of our own
  and a basic glTF loader) and Babylon.js (heavier, no 3MF; Babylon Lite is
  WebGPU-only, which WebKitGTK can't be relied on for). three.js is the
  app's first runtime dependency beyond Tauri's own; the viewer is a
  separate chunk (about 680 kB, 170 kB gzipped) that loads only when a
  model is previewed, so start-up doesn't change.
- **D2. The bytes come from the backend.** `preview_bytes` reads a local
  file whole, behind `place::local`, and refuses one over the limit the
  preview asks (256 MiB; the backend caps any request at 512 MiB) with
  "This file is too large to preview." Raw bytes travel as an IPC response,
  as thumbnails do.
- **D3. A .gltf's companion files are read the same way.** Its buffers' and
  images' URIs name them; only paths in the model's folder or below are
  read (no scheme, no absolute path, no `..`, no backslash), and nothing
  else is fetched: the loader gets blob URLs for what was read and an empty
  data URI for anything else. The model and its companions share the
  256 MiB.
- **D4. What it looks like.** A transparent canvas on the preview's
  background, the model centred and framed from the front-right and a little
  above, a room environment and a key light beside the eye, neutral tone
  mapping so light surfaces don't burn out. STL and 3MF are Z-up and are
  stood up; a file that names no material (STL, a plain 3MF, a glTF
  primitive without one) is drawn in a mid grey that reads on light and
  dark; a model's own colours stay. It renders only when the view changes.
- **D5. Failing quietly.** Without WebGL, for a file three.js can't read, or
  for one that holds nothing, the preview says "No preview available."; a
  preview closed while its model loads never draws it. Closing the preview
  disposes the renderer, geometries, materials and textures.
- **D6. Keys.** The canvas takes focus when dragged, so Escape and Space on
  it close the preview as they do from the view.
- **D7. Turning without dragging.** An on-screen bar over the model, in the
  translucent toast style, has **Rotate Left** and **Rotate Right** (a
  quarter turn about the vertical, the model's front swinging that way, as
  GNOME Image Viewer names its rotate buttons) and **Reset View** (the
  model and camera as first shown). On the focused model, the arrow keys
  turn it in 15° steps the way a drag would (Left and Right about the
  vertical, Up and Down about the view's horizontal axis) and Home resets;
  they don't move the folder's selection. Turns animate over 200 ms, at
  once when the user prefers reduced motion. The model area shrinks to the
  room the preview panel has, so the bar is never scrolled out of sight.
- **D8. Dragging turns the model, like an object in the hand.** The camera
  never orbits; it only moves in and out along one line, and every turn
  (drag, buttons, keys) turns the model, so they all agree and Reset View
  undoes them all. A drag turns at once, the front following the pointer:
  across the whole canvas is a full turn, left and right about the
  vertical (the model stays upright), up and down about the view's
  horizontal axis, with no limit, so the underside can be seen. three.js's
  OrbitControls was dropped: it orbited the camera with the tilt stopped at
  the poles, which felt unlike the buttons.
- **D9. Zoom: the wheel, a pinch, + and -.** A pinch arrives three ways and
  each zooms: two touch points, a trackpad's Ctrl/⌘ + wheel (Chromium,
  WebKitGTK; a finer rate than a wheel notch) and WebKit's `gesture*`
  events on a Mac (the wheel waits while one runs). Each is kept from also
  zooming the page. The camera stays between 0.3 and 12 times the model's
  size away. `src/model-gestures.ts` holds this, without three.js.

- **D10. Draco-compressed glTF (added 2026-11-08).** Blender's exporter
  writes `KHR_draco_mesh_compression` when compression is on, and seven of
  the eight glTF test models the user had required it. three.js ships
  Google's Draco decoder (Apache-2.0); the glTF-only WebAssembly build
  (192 kB plus a 58 kB wrapper) is set on one shared `DRACOLoader`, made on
  first use with at most two workers, so later previews reuse it. Vite
  bundles the decoder files from `DRACOLoader`'s own `new URL(…,
  import.meta.url)` references; that also copies the general-purpose build
  (about 1 MB) into the app, unused. Licences: `src/assets/licenses/`.
- **D11. A glTF's own images.** GLTFLoader turns an image stored in the file
  into a `blob:` URL itself; those pass, as data URIs do, while any other
  URL the file names that wasn't read for it stays empty
  (`resourceUrl`). Before this, textures stored in GLB files were dropped.

- **D12. Where it turns (added 2026-11-08).** The user found the turn
  felt off its pivot. Measured in Chromium with their models, the pivot was
  the centre of the meshes' stored bounds, which sat up to about 4% of the
  height above the visible middle (Howdy drew 8 px low at every angle), and
  a scene's box centre can be far from the object you look at
  (`intro.gltf`'s flower among scattered tiles). Now the model turns about
  the middle of its visible meshes' own vertices (`shownBounds`, hidden
  meshes left out, sampled to at most 400,000 vertices: 7 ms for 4 million
  triangles). A double-click on the model picks a new pivot and glides it to
  the centre (`recentred`): climbing from the mesh hit, the largest group
  still under half the model's size is the object clicked (in
  `intro.gltf`, the Suka figure of many meshes) and its middle is the
  pivot, so it spins in place; when even the mesh is that large (a floor),
  the point hit is the pivot, as in Sketchfab (`pickedPoint`). Turning the
  surface point instead made the figure circle it by about 20 px. A
  double-click on empty space goes back to the middle, and Reset View puts
  back the pivot, the turn and the zoom.

## Not in this change

- Thumbnails (3MF usually carries one; glTF and STL would need rendering).
- Playing animations: a scene shows its rest pose.
- meshopt-compressed glTF and KTX2 textures: they need other decoders.
- Remote items (Drive, shares, SSH) keep "Preview isn't available for
  remote items."
