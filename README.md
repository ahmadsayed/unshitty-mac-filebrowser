# File Browser

A file browser for macOS that looks and works like **GNOME Files 50** (Nautilus
with libadwaita), for people who find Finder hard to live with.

> [!NOTE]
> **Independent hobby project.** This is a personal, non-commercial project
> made in spare time, for fun. It is **not affiliated with, sponsored by or
> endorsed by** the GNOME Project, the GNOME Foundation, the GNOME Files
> (Nautilus) developers, Apple Inc., or any employer, company or organisation.
> It contains no code from GNOME Files; it is a from-scratch look-alike.

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/screenshots/grid-dark.png">
  <img alt="File Browser showing the home folder in grid view" src="docs/screenshots/grid-light.png">
</picture>

## Screenshots

| Light | Dark |
| --- | --- |
| ![Grid view, light](docs/screenshots/grid-light.png) | ![Grid view, dark](docs/screenshots/grid-dark.png) |
| **List view** with sizes, item counts and GNOME-style dates | |
| ![List view, light](docs/screenshots/list-light.png) | ![List view, dark](docs/screenshots/list-dark.png) |
| **Thumbnails** for photos, videos, PDFs and documents | |
| ![Thumbnails, light](docs/screenshots/thumbnails-light.png) | ![Thumbnails, dark](docs/screenshots/thumbnails-dark.png) |
| **Paste with name clashes**: Replace, Skip or Keep Both | |
| ![Conflict dialog, light](docs/screenshots/conflict-light.png) | ![Conflict dialog, dark](docs/screenshots/conflict-dark.png) |
| **Drag and drop**, here copying two files to Music (⌥ held) | |
| ![Drag and drop, light](docs/screenshots/dragdrop-light.png) | ![Drag and drop, dark](docs/screenshots/dragdrop-dark.png) |
| **Context menus** in the libadwaita popover style | |
| ![Context menu, light](docs/screenshots/menu-light.png) | ![Context menu, dark](docs/screenshots/menu-dark.png) |

<sub>Screenshots are from the development preview (`npm run dev:mock`) with sample
files, so the photo thumbnails are placeholders.</sub>

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
- Drag and drop: drag items onto a folder, a sidebar entry or a path-bar
  segment; hovering a folder opens it. Same disk moves, another disk copies;
  hold ⌥ to copy or ⌘ to move. Dropping on Trash trashes. Files dragged in from
  Finder land in the folder under the pointer
- Thumbnails for images (HEIC too), videos, PDFs and office documents, made by
  macOS Quick Look and cached on disk; toggle with "Show Thumbnails" in the menu
- Rename, new folder, Move to Trash
- Open files in their default app (double-click or Enter); `.app` bundles launch,
  with "Show Package Contents" in the context menu
- Recent: files opened lately in any app (from Spotlight) or from here, newest
  first, with their location; "Open Item Location" and "Remove from Recent"
- Starred: star files and folders from the context menu or the star at the end
  of each list row; Starred lists them all, and stars follow renames and moves
  made in the app
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
  the Rust side. Type `/big` in the path bar for a 10,000-item folder; add `?demo`
  to the URL to hide the test-only folders.
- `cd src-tauri && cargo test` runs the backend tests (listing, copy/move/trash
  safety checks, settings files, thumbnails, Recent).
- Thumbnails are cached in `~/Library/Caches/io.github.ahmadsayed.filebrowser/thumbnails`;
  entries unused for 60 days are removed at startup.
- Design notes and the implementation plan live in `docs/superpowers/`.

## Not supported yet

Network is a placeholder; there is no Undo after Move to
Trash, no dragging files out of the app into Finder or other apps, no tabs,
and no disk-wide search.

## Credits and thanks

All credit for the design goes to the people behind GNOME. This project only
tries to bring their work to the Mac.

- **The GNOME Files (Nautilus) team and contributors.** The layout, behaviour
  and wording of this app (the sidebar, path bar, views, menus, dialogs and
  messages such as "Folder is Empty") follow GNOME Files. Thank you for the file
  manager this project is trying to imitate.
  [apps.gnome.org/Nautilus](https://apps.gnome.org/Nautilus/) ·
  [gitlab.gnome.org/GNOME/nautilus](https://gitlab.gnome.org/GNOME/nautilus)
- **The libadwaita developers and the GNOME design team.** The colours,
  spacing and widget styles follow libadwaita and the
  [GNOME Human Interface Guidelines](https://developer.gnome.org/hig/).
- **The Adwaita icon theme designers.** The icons here are original drawings
  made in the Adwaita style; no icon files were copied.
- **Adwaita Sans** by the Inter Project Authors, Renzhi Li, Jamie Gravendeel and
  Florian Müllner, bundled unmodified under the SIL Open Font License 1.1
  (`src/assets/fonts/AdwaitaSans-LICENSE.txt`).
- **[Tauri](https://tauri.app/)** and the Rust crates this app builds on,
  including [`trash`](https://crates.io/crates/trash) and
  [`image`](https://crates.io/crates/image).

## Trademarks

"GNOME" and the GNOME logo are trademarks of the GNOME Foundation. Apple, Mac,
macOS and Finder are trademarks of Apple Inc. These names are used only to
describe what this project is inspired by and runs on; no endorsement is
implied.

## License

Copyright © 2026 ahmadsayed.

This program is free software: you can redistribute it and/or modify it under
the terms of the **GNU General Public License, version 3 or (at your option) any
later version** (`GPL-3.0-or-later`), the same license GNOME Files uses. See
[`LICENSE`](LICENSE) for the full text.

This program is distributed in the hope that it will be useful, but WITHOUT ANY
WARRANTY; without even the implied warranty of MERCHANTABILITY or FITNESS FOR A
PARTICULAR PURPOSE.

### Third-party licenses

- The bundled Adwaita Sans font stays under the SIL Open Font License 1.1.
- Rust crates and npm packages are under MIT, Apache-2.0, BSD, ISC, Zlib,
  Unicode-3.0, Unlicense or MPL-2.0 licenses, all compatible with GPL-3.0.
- A built `.app` carries this project's license and the font license in
  `Contents/Resources/licenses/`. If you publish built binaries, also include
  the dependencies' notices (for example with
  [`cargo-about`](https://github.com/EmbarkStudios/cargo-about)).
