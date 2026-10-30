# Native file and folder icons

**Date:** 2026-10-30

**Status:** Implemented. Linux checked in the real app; macOS
type-checked only, not run on a Mac.

**Amends:** [macOS icons](2026-10-30-macos-icons-design.md): the drawings
there stay, as the fallback.

## Goal

Files and folders show the icons the user's own system shows: Finder's on a
Mac, the desktop's icon theme on Linux. A drawing can't give an app its real
icon, a folder its custom icon, or a document the icon of the app that opens
it; the OS can. This also follows a Linux user's chosen theme (Papirus,
Breeze…) instead of imposing Adwaita.

Out of scope here, as a later step: the system accent colour and the
sidebar's translucent material, and SF Symbols or theme symbolic icons for
the sidebar and header bar.

## Rules

- **Nothing bundled.** Icons are read from the running system and never
  copied into the repository or the app bundle, so the licensing rule
  ("no Apple or Adwaita assets") still holds. AGENTS.md says so explicitly.
- **The app's drawings stay as the fallback:** while an icon loads, for
  remote places (Drive, shares, SFTP, Network), when the OS has no icon, and
  in the mock preview and tests.
- **The app's badges stay:** the symlink emblem, the versions badge, the
  Network emblem on shared folders. Thumbnails still win over icons.
- **Huge folders stay fast:** only rendered items are asked, ids come in one
  batch per render, and files of one type share one image.

## How it works

**Backend (`src-tauri/src/file_icons.rs`).** Two commands:

- `icon_ids(paths)` gives each path an icon id, or null. Cheap: no image
  work. Ids are registered, and only registered ids can be drawn.
- `icon_image(id, size)` draws an id about `size` pixels square: PNG on
  macOS, the theme's SVG or PNG file on Linux. Only PNG and SVG files are
  served.

**macOS.** Files share one icon per extension (`type:pdf`), drawn by
`NSWorkspace.iconForFileType`; extensionless files are `public.data` or
`public.unix-executable`, plain folders `public.folder`. These get their own
icon, drawn by `NSWorkspace.iconForFile`:

- packages (apps and any folder with an extension);
- the folders Finder marks specially: home and its children, the disk root
  and its children, and volumes;
- items with a custom icon (the `kHasCustomIcon` flag in
  `com.apple.FinderInfo`).

The image is drawn into an `NSBitmapImageRep` of the requested pixel size, so
macOS picks its best representation, then encoded as PNG. Everything runs on
the blocking pool; none of these AppKit calls is main-thread-only.

**Linux.**

- GIO's `standard::icon` names the icon: by content type, plus special
  folders and custom icons, as GNOME Files does.
- A freedesktop icon theme lookup finds the closest-sized file: the theme,
  the themes it inherits from, then (as GTK does) Adwaita and gnome, then
  hicolor, then loose pixmaps. GTK's Adwaita fallback matters: Ubuntu's GTK
  settings name Yaru even where only Adwaita is installed.
- The theme name comes from GTK's settings, read on the main thread at
  start-up, since GTK objects can't be used from command threads.
- Ids are the icon names (`names:application-pdf,x-office-document`).

**Frontend (`src/native-icons.ts`).**

- Each render asks for the rendered items' unknown paths in one batch.
- Each id's image is loaded once per size; sizes step through 32, 64, 128,
  256 and 512 px at screen resolution.
- After a zoom, the image at the old size is shown until the new one arrives.
- Types already loaded are drawn as soon as a new folder's ids arrive.
- The path → id map is cleared past 20,000 entries. Images are few (one per
  type and size), so they are never evicted.

**Dialogs (#70).** Open With, the Properties header and the Space preview's
icon (folders, files without a preview) ask `nativeIcon(key, size)` for their
one icon and swap it in when it arrives, keeping any badge.

- An Open With choice is an app, not a file. Its key is `appIconKey(app)`
  and its id comes from `app_icon_ids`.
- On Linux the app is a `.desktop` file. GIO's icon for that file is the
  generic "desktop file" one, so the backend reads the app's `Icon=` line
  through `DesktopAppInfo` instead.
- On macOS the app is its `.app` bundle, which already gets its own icon per
  path.
- The preview's icon sits in a fixed 192 px box. The app's drawing used to
  take whatever size the SVG defaulted to, about 220 px, which an OS icon
  wouldn't have matched.

## Verification

- **Backend tests.** The theme lookup is tested against a fixture theme:
  sizes, scalable ranges, name order, inheritance, hicolor, pixmaps,
  inheritance loops and odd names. Also tested:
  - the macOS id rules, and the FinderInfo flag;
  - that GIO names a PDF and a folder;
  - that only registered ids draw.

  A one-off run against this machine's Adwaita theme found SVGs for PDF,
  text, archive, audio, image, Rust source, ODT, a folder and the home
  folder.
- **The real Linux app** (debug build, Xvfb, a demo home folder) shows the
  theme's icons, special folders included. With `gtk-icon-theme-name` set to
  Humanity in GTK's settings, it shows Humanity's, so the name really comes
  from the desktop.
- **Empty files**, like in GNOME Files, are `application/x-zerosize` (GIO
  sniffs content) and get the theme's generic icon. Where the theme has none,
  they get the app's drawing by extension.
- **The macOS code** type-checks (`cargo check --target aarch64-apple-darwin`)
  in a scratch crate with the same dependencies; it has not run.
- **App tests** (`tests/native-icons.test.ts`) use a mock that can serve
  icons:
  - the OS icon is drawn, one image per type;
  - badges stay;
  - nothing is re-asked for a folder seen before;
  - apps get their own icon;
  - a zoom keeps showing the old size;
  - thumbnails win;
  - Network doesn't ask;
  - Open With shows each app's own icon;
  - Properties keeps the link badge;
  - the preview of a folder shows its icon.
- **Not verified:** the macOS drawing on a Mac (sharpness, app and custom
  folder icons, speed in a big folder), and a real Linux desktop session
  (GNOME or KDE, switching the theme while the app runs needs a restart).
