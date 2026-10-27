# File Browser

A file browser for **macOS and Linux** that works like **GNOME Files 50**, for
people who find Finder hard to live with. Rust does every filesystem operation;
the interface is plain TypeScript in a system webview.

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

More screenshots: [list view](docs/screenshots/list-light.png),
[thumbnails](docs/screenshots/thumbnails-light.png),
[name clashes](docs/screenshots/conflict-light.png),
[drag and drop](docs/screenshots/dragdrop-light.png),
[menus](docs/screenshots/menu-light.png).

## Why not Finder?

Every file manager, editor and IDE agrees on how files work. Finder disagrees.

| You want to… | Finder | Everyone else | Here |
| --- | --- | --- | --- |
| Cut and paste | ⌘X is greyed out. ⌘C, then **⌥⌘V**. Forget the ⌥, enjoy your copy. | Ctrl+X, Ctrl+V: Explorer, Nautilus, Dolphin, VS Code | ⌘X, ⌘V |
| Line up icons | Icons land anywhere, overlap, wander off-screen. View → Clean Up, per folder, forever. | A sorted grid: Explorer, Nautilus, Dolphin | A grid. Always. Folders first. |
| Type a location | No address bar. ⇧⌘G opens a floating box. | Ctrl+L: Nautilus, Dolphin, Explorer, Chrome, Firefox. Even Safari: ⌘L | ⌘L |
| Copy a path | Right-click, **hold ⌥**, "Copy as Pathname". A secret handshake. | "Copy as path" in Explorer, "Copy Path" in VS Code, the address bar in any browser | "Copy Location" in every menu |
| See where you are | Title shows the folder name only. Path bar is off by default, and at the **bottom**. | Address bar on top: Explorer, Nautilus, Dolphin, every web browser | Breadcrumbs on top, always |
| Rename | Enter. Yes, Enter. | F2: Explorer, Nautilus, Dolphin, VS Code | F2 |
| Open | ⌘O or ⌘↓, because Enter was taken. | Enter: Explorer, Nautilus, Dolphin, VS Code | Enter |
| Get Info | A tiny window off to the side. Select 20 files, get 20 windows. | One Properties dialog: Explorer, Nautilus, Dolphin | One dialog, right where you are |
| Search this folder | ⌘F searches the whole Mac. | The current folder: Explorer, Nautilus, Dolphin | Type, and the current folder is searched |
| Copy a big folder | "Preparing to copy…" "Estimating time remaining…" | Speed, size and time left: Explorer, Nautilus, Dolphin | "12 MB of 80 MB — 1 minute left" |
| Replace a folder | Gone. Not merged, not in the Trash. | Merge: Explorer merges, Nautilus offers Merge, Dolphin offers Write Into | Old one goes to the Trash |
| Read an error | "Error code -8058." Thanks. | A sentence: Nautilus, Dolphin | "A folder with that name already exists." |
| Make a new file | Can't. New Folder only. | New → Text Document in Explorer, New Document in Nautilus, New File in VS Code | New Document |
| Keep folders clean | `.DS_Store` in every folder, USB stick and Git repo. | Settings stay in the app: Nautilus, VS Code | Writes none |
| Ignore clutter | Recents, AirDrop, iCloud nags and seven Tag colours you never asked for. | Places and your bookmarks: Nautilus, Dolphin | Home, Recent, Starred, Network, Trash |
| Keep file history | Time Machine needs a drive, and document versions stay on that Mac. | Git, if it's code | Keep Versions: history lives in `.versions/` inside the folder, so it moves with the folder to any Mac or Linux machine |
| Share a folder | AirDrop: Apple devices only, in the same room. | Upload it to someone's cloud | A `share://` link over the open-source [Iroh](https://www.iroh.computer/) protocol: peer to peer, any network, Mac or Linux |
| Get a missing feature | Wait for "one more thing…" at a keynote, years after everyone else moved on. | Plugins, sometimes | Fork it, change it, build it. It's GPL-3.0 |

## Also in the box

- **Keep Versions (Experimental):** tick "Keep versions" when creating a folder
  and every saved state of its files is kept in `.versions/` inside it. Diff,
  restore, or bring back deleted files. Local only, **not a backup**.
- **Folder Sharing (Experimental):** share a folder read-only with a
  `share://` link, for anyone or one person. Received shares show up in Network.
- **Network (Experimental):** browse SSH/SFTP servers read-only.
- **Google Drive (Experimental):** browse, copy, move and trash in My Drive
  with your own OAuth client ([setup](docs/google-drive.md)).
- **Trash done right:** Undo (⌘Z), Restore (also for items trashed in Finder),
  and nothing is ever deleted without asking.
- Grid and list views, tabs, thumbnails, Space to preview, batch rename,
  extract and compress archives, Starred, Recent, bookmarks, a progress popover
  with Cancel, and fast 10,000-item folders.

Press ⌘? in the app for every shortcut.

## Download

Get a build from the
[Releases page](https://github.com/ahmadsayed/unshitty-mac-filebrowser/releases).

- **macOS:** the `.dmg` (Apple Silicon and Intel). It isn't notarized, so run
  `xattr -dr com.apple.quarantine "/Applications/File Browser.app"` once, or
  right-click the app and choose Open. Grant **Full Disk Access** in System
  Settings if you want to see every folder.
- **Linux:** the `.deb`, `.rpm` or `.AppImage`. To make the AppImage show up in
  GNOME search, see [docs/linux-appimage.md](docs/linux-appimage.md).

## Build it yourself

Fork it, change it, ship it. You need Rust and Node.js (and, on Linux, the
WebKitGTK 4.1 headers: `libwebkit2gtk-4.1-dev` or `webkit2gtk4.1-devel`).

```sh
npm install
npm run tauri dev      # run it
npm run tauri build    # package it
npm run dev:mock       # UI only, in a browser, with a fake filesystem
npm test && (cd src-tauri && cargo test)
```

Design notes live in `docs/superpowers/`; agent and contributor rules in
[AGENTS.md](AGENTS.md).

## Not supported yet

- Search matches names only, not file contents.
- SFTP and shares are read-only; no SMB or WebDAV.
- Preview has no video or audio playback.
- Google Drive: no Shared Drives, "Shared with me" or Drive-wide search.
- Keep Versions may miss changes made while the app is closed.
- Linux: image thumbnails only (no Quick Look).

## Credits and thanks

All credit for the design goes to the people behind GNOME. This project only
tries to bring their work to other desktops.

- **The GNOME Files (Nautilus) team**, whose layout, behaviour and wording this
  app follows. [apps.gnome.org/Nautilus](https://apps.gnome.org/Nautilus/)
- **The libadwaita developers and the GNOME design team**, for the colours,
  spacing, widgets and the
  [Human Interface Guidelines](https://developer.gnome.org/hig/).
- **The Adwaita icon theme designers.** The icons here are original drawings in
  the Adwaita style; no icon files were copied.
- **Adwaita Sans** by the Inter Project Authors, Renzhi Li, Jamie Gravendeel and
  Florian Müllner, bundled unmodified under the SIL Open Font License 1.1
  (`src/assets/fonts/AdwaitaSans-LICENSE.txt`).
- **[Tauri](https://tauri.app/)**, **[Iroh](https://www.iroh.computer/)** and the
  Rust crates this app builds on.

## Trademarks

"GNOME" and the GNOME logo are trademarks of the GNOME Foundation. Apple, Mac,
macOS and Finder are trademarks of Apple Inc. These names are used only to
describe what this project is inspired by and runs on; no endorsement is
implied.

## License

Copyright © 2026 ahmadsayed.

GPL-3.0-or-later, the same license GNOME Files uses; see [`LICENSE`](LICENSE).
Distributed WITHOUT ANY WARRANTY. The bundled font stays under the SIL Open
Font License 1.1; Rust and npm dependencies use GPL-compatible licenses (MIT,
Apache-2.0, BSD, ISC, Zlib, Unicode-3.0, Unlicense, MPL-2.0).
