# File Browser

A file browser for macOS that looks and works like **GNOME Files 50** (Nautilus
with libadwaita), for people who find Finder hard to live with.

Built with Tauri 2: a Rust backend does all filesystem work, and a plain
TypeScript + Vite frontend draws the GNOME-style UI.

## Features

- GNOME 50 look: libadwaita colours (light and dark), Adwaita Sans, Adwaita-style
  folder and file icons, header bar with path bar, split view button, popover
  menus, dialogs, toasts and banners
- Grid and list views, zoom, sorting (A–Z, Z–A, modified, size, type), hidden
  files toggle
- Path bar with breadcrumbs; ⌘L to type a location; type anywhere to filter the
  current folder
- Copy, cut, paste with Replace / Skip / Keep Both on name clashes; replacing
  sends the old item to the Trash, never deletes it
- Rename, new folder, Move to Trash
- Open files in their default app (double-click or Enter); `.app` bundles launch,
  with "Show Package Contents" in the context menu
- Bookmarks: "Add to Bookmarks" (⌘D) pins folders to the sidebar
- Mounted volumes in the sidebar
- Large folders stay fast: only the visible items are rendered

Press ⌘? in the app for the full list of keyboard shortcuts.

## Build and run

Requires Rust and Node.js.

```sh
npm install
npm run tauri dev      # run in development
npm run tauri build    # produces src-tauri/target/release/bundle/macos/File Browser.app
```

### Full Disk Access

macOS hides some folders (Mail, Safari, other apps' data, parts of `~/Library`)
from apps without **Full Disk Access**. Until it's granted the app shows a banner
with a button that opens the right page in System Settings → Privacy & Security.
Turn on "File Browser" there and restart the app.

The app is signed ad hoc (no Apple Developer ID), so macOS ties the grant to that
exact build: after rebuilding, toggle the permission off and on again.

## Development

- `npm run dev:mock` opens the UI in a normal browser at http://localhost:1420
  with a fake in-memory filesystem (`dev/mock-tauri/`), handy for UI work without
  the Rust side. Type `/big` in the path bar for a 10,000-item folder.
- `cd src-tauri && cargo test` runs the backend tests (listing, copy/move/trash
  safety checks, settings files).
- Design notes and the implementation plan live in `docs/superpowers/`.

## Not supported yet

Recent, Starred and Network are placeholders; there is no Undo after Move to
Trash, no drag and drop, no tabs, and no disk-wide search.

## Credits

Adwaita Sans is © The Inter Project Authors, Renzhi Li and Jamie Gravendeel,
under the SIL Open Font License (`src/assets/fonts/AdwaitaSans-LICENSE.txt`).
The look follows the GNOME Human Interface Guidelines and libadwaita; icons are
original drawings in that style.
