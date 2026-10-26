# File Browser

A file browser for **macOS and Linux** that looks and works like **GNOME Files 50**
(Nautilus with libadwaita), for people who find Finder hard to live with. The
interface is HTML, CSS and TypeScript in a system webview; every filesystem
operation goes through Rust.

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

- GNOME Files 50 look: libadwaita colours (light + dark), Adwaita Sans, Adwaita folder icons, header bar, popover menus, dialogs and toasts
- Grid and list views; sort by name, date, size or type; hidden files toggle; icon size with ⌘+/⌘−, Ctrl+wheel or pinch
- ⌘L to type a path; breadcrumb path bar; tabs (⌘T/⌘W); ⌘N new window; type to search file names in the current folder
- Copy, cut, paste with **Replace / Skip / Keep Both** on name clashes — never silently overwrites
- Drag and drop between folders, sidebar entries and the path bar; drag out to Finder; ⌥ to copy, ⌘ to move
- Thumbnails for images (HEIC included), videos, PDFs and documents via Quick Look (macOS); toggle in the menu
- Rename, new folder, Move to Trash — each with an **Undo** toast (⌘Z)
- Batch rename: find/replace, prefix, suffix with live preview and clash check
- Extract archives (zip, tar, tar.gz/bz2/xz/zst and single-file compressed files); Compress to zip/tar.gz/tar.xz
- Properties (⌘I): size, dates, permissions, symlink target; change read/write/execute bits
- Trash: restore from Trash (including Finder's “Put Back”), Delete Permanently and Empty Trash — both after confirmation
- Recent (Spotlight on macOS), Starred, Bookmarks (⌘D), mounted volumes in the sidebar
- **Network (Experimental):** SSH/SFTP browsing (read-only); **Folder Sharing (Experimental):** share a folder via `share://` link
- **Google Drive (Experimental):** browse, copy and move files between Drive and local folders — [setup guide](docs/google-drive.md)
- **Keep Versions (Experimental):** per-folder version history with diff, restore and deleted-file recovery — local only, not a backup

Press ⌘? for the full keyboard shortcut list.

## Download

Ready-made builds are on the
[Releases page](https://github.com/ahmadsayed/unshitty-mac-filebrowser/releases):

- **macOS:** the `.dmg` (one app for Apple Silicon and Intel). The app isn't
  notarized, so after copying it to Applications run
  `xattr -dr com.apple.quarantine "/Applications/File Browser.app"` once, or
  right-click it and choose Open.
- **Linux:** the `.deb` (Debian, Ubuntu), the `.rpm` (Fedora) or the
  `.AppImage` (any distribution). For the AppImage, use the
  [GNOME launcher install steps](docs/linux-appimage.md) so it appears in
  application search; the script is
  [`scripts/install-linux-appimage.sh`](scripts/install-linux-appimage.sh).

### Linux GNOME desktop

The AppImage includes WebKitGTK and GTK, so it is about 90–120 MB but works
without installing the runtime dependencies. To install it for your user and
make **File Browser** appear in GNOME search:

```sh
# Download the 0.4.0 AppImage from the Releases page, then:
chmod +x scripts/install-linux-appimage.sh
scripts/install-linux-appimage.sh ~/Downloads/File.Browser_0.4.0_amd64.AppImage
```

The script installs the AppImage under `~/.local/bin`, extracts its icon, and
writes `~/.local/share/applications/io.github.ahmadsayed.filebrowser.desktop`
with the name **File Browser**, file-manager keywords and the correct
`StartupWMClass`. It then refreshes the desktop and icon caches.

Open GNOME Activities and search for **File Browser**. If it does not appear
yet, log out and back in once. Full manual steps and the FUSE fallback are in
[docs/linux-appimage.md](docs/linux-appimage.md).

To use Google Drive, follow [docs/google-drive.md](docs/google-drive.md).

Releases are built by `.github/workflows/release.yml`: pushing a `v*` tag
that matches the app version runs the tests, then drafts a release with the
packages; publishing it is a manual step.

## Build and run

Requires Rust and Node.js.

```sh
npm install
npm run tauri dev      # run in development
npm run tauri build    # macOS: bundle/macos/File Browser.app
                       # Linux: bundle/deb, bundle/rpm and bundle/appimage
```

Google Drive needs no build setup. The app ships no Google client: each user
creates a Desktop OAuth client in their own Google Cloud project and adds its
JSON file in the app. See [docs/google-drive.md](docs/google-drive.md).

### Linux

Tauri has no bundled browser engine: on Linux it uses WebKitGTK 4.1, which is
the GTK3 generation of WebKit. Install the development headers first:

```sh
# Fedora
sudo dnf install gtk3-devel webkit2gtk4.1-devel javascriptcoregtk4.1-devel \
                 libsoup3-devel openssl-devel libxdo-devel librsvg2-devel

# Debian / Ubuntu
sudo apt install libwebkit2gtk-4.1-dev libgtk-3-dev libsoup-3.0-dev \
                 libssl-dev libxdo-dev librsvg2-dev
```

What differs from macOS is handled in `src-tauri/src/platform.rs` and
`src-tauri/src/trash_ops.rs`:

| | macOS | Linux |
| --- | --- | --- |
| Trash | `~/.Trash` | `~/.local/share/Trash` (`files/` + `info/`) |
| Put Back / restore records | Finder's `.DS_Store` | `.trashinfo` sidecars |
| Recent | Spotlight (`mdfind`) | `recently-used.xbel` |
| Mounted volumes | `/Volumes` | `/run/media`, `/media`, `/mnt` |
| Full Disk Access | checked | not applicable |

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
  safety checks, settings files, thumbnails, Recent, Trash, volumes). On a host
  without the GTK/WebKit headers, run the same command through the rootless
  container in `dev/tauri-linux.Containerfile`.
- Thumbnails are cached in the app cache directory (`~/Library/Caches/…` on
  macOS, `~/.cache/…` on Linux); entries unused for 60 days are removed at
  startup.
- Design notes and the implementation plan live in `docs/superpowers/`.

## Not supported yet

- Transfers run one at a time; no pause/resume. Undo covers only the last Move to Trash.
- SFTP is read-only and session-only (no upload, rename, delete or saved servers). No SMB/WebDAV.
- Folder Sharing: copying files out of a share and live refresh are not yet implemented.
- Search matches file names only (not content); results capped at 1,000.
- Preview covers text and thumbnails — no video or audio playback.
- Open With lists apps but cannot change the system default handler.
- No Nautilus extensions; no 7z, rar, deb, rpm or pkg archive support.
- Google Drive: no Shared Drives, no "Shared with me", no Drive Trash view, no Drive-wide search. Google Docs/Sheets/Slides can't be exported to a local folder. Edits to a locally-opened Drive snapshot are not uploaded.
- Keep Versions: history is local only (not a backup); changes made while the app is closed may be missed; one instance per versioned folder; no "folder at time T" view.
- Linux: thumbnails are images only (no Quick Look); Recent reads `recently-used.xbel` only; window uses the desktop's native title bar.

## Credits and thanks

All credit for the design goes to the people behind GNOME. This project only
tries to bring their work to other desktops.

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
