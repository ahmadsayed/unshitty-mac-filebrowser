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

## Not in this change

- Thumbnails (3MF usually carries one; glTF and STL would need rendering).
- Compressed glTF (Draco, meshopt) and KTX2 textures: they need extra
  decoders.
- Remote items (Drive, shares, SSH) keep "Preview isn't available for
  remote items."
