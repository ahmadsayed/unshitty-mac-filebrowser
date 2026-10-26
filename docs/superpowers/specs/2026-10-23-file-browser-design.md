# File Browser — Design

**Date:** 2026-10-23
**Status:** Draft, pending approval

## Intent and the why

The user finds macOS Finder "seriously unusable." They want a personal file
browser modeled on GNOME Nautilus — the look and feel they are comfortable
with on Linux. This is for personal use on the user's own Mac.

The explicit north star is **Nautilus**: its sidebar + main-pane layout, its
navigation model, and its straightforward file operations. The app does **not**
need to look native to macOS — it needs to feel like Nautilus and get out of
the user's way.

Success looks like: the user reaches for this app instead of Finder for
everyday file navigation and management, and it doesn't frustrate them the way
Finder does.

## Goals

A standalone desktop application for macOS that provides:

1. **Layout** — a left sidebar and a main content pane, Nautilus-style.
   - Sidebar: standard places (Home, Desktop, Documents, Downloads) plus
     user-pinned folders.
   - Main pane: contents of the current folder.
2. **Two view modes** — icon view and list view, toggleable.
   - List view columns: name, size, modified date.
3. **Navigation**
   - Double-click a folder to enter it.
   - Back / forward / up buttons.
   - A clickable breadcrumb path bar.
   - **Cmd+L / Ctrl+L** toggles the breadcrumb into an editable, focused path
     field pre-filled with the current path; Enter navigates to the typed path,
     Esc reverts to the breadcrumb. (Added 2026-10-23 at the user's request —
     matches GNOME Files behavior.)
4. **File operations**
   - Copy, cut, paste, rename, delete-to-Trash, create new folder.
   - Available via right-click context menu and keyboard shortcuts.
5. **Open files** in their default application (double-click a file).
6. **Show hidden files** toggle (dotfiles).

## Non-goals (v1)

These are deliberately out of scope for the first version. They are not
rejected forever — just not in v1.

- Tabs and split panes.
- Search across the disk / recursive search.
- File previews and thumbnail generation.
- Network drives, remote/SMB/cloud mounts.
- Archive handling (zip/unzip in-app).
- Syncing Finder tags, labels, or custom metadata.

## Anti-goals (what would make this a failure)

- Being slow to open or slow to list a large folder. The whole point is that
  Finder frustrates the user; a sluggish replacement fails even if
  feature-complete.
- Destroying data. Delete must go to Trash (recoverable), never a hard
  `rm`. File operations must not silently clobber or lose files.
- Requiring the user to fight macOS permission prompts constantly for normal
  home-directory browsing.

## Constraints

- Target platform: macOS (user is on macOS 27.0.1, Apple Silicon implied by
  Homebrew `/opt/homebrew`).
- Must be a standalone launchable application, not a browser tab and not a
  dev-server-only web app.
- Personal use — single user, local filesystem. No multi-user, no sandbagging
  for untrusted input beyond basic safety.

## The how — decisions made (my calls)

The user explicitly delegated all tooling/look-and-feel decisions ("it does
not matter, look feel whatever easier", "standalone but no need to be
native"). The following are therefore **my calls**, not things the user
chose:

- **Framework: Tauri 2.x** (Rust backend + web frontend). Rationale: produces a
  real standalone `.app` bundle, small and fast to launch, far lighter than
  Electron. Rust + Node are already installed on the machine (cargo 1.90,
  node 26.7). Filesystem operations run in Rust for speed and safety.
  - Alternative considered — **Electron**: heavier, slower to launch, larger
    bundle. Rejected because launch speed is an anti-goal concern.
  - Alternative considered — **pure web app**: not standalone. Rejected by the
    user's standalone requirement.
  - Alternative considered — **native Swift/AppKit**: slowest to build, and
    "native" was explicitly not required. Rejected for delivery speed.
- **Frontend: plain HTML/CSS/TypeScript with Vite**, no heavy UI framework.
  Rationale: the UI is a sidebar + grid/list; a framework adds setup cost
  without much benefit at this scope. Keeps the bundle small and the build
  simple. (A lightweight reactive layer may be added if state management gets
  awkward — builder's discretion.)
- **Delete-to-Trash**: use a maintained Rust crate (e.g. `trash`) rather than
  shelling out, so deletes are recoverable via macOS Trash.
- **Open-in-default-app**: use macOS `open`; on Linux use `gio open` so the
  desktop's MIME default is respected rather than a browser fallback.
- **Styling**: approximate the look of **modern GNOME Files (GNOME 48–51)**
  with **libadwaita** styling, NOT classic Nautilus. (Revised 2026-10-23 after
  the user clarified the target and approved a mockup — see
  "Visual target (revised)" below.) Pixel-exact fidelity is not required, but
  the distinctive modern-Files elements are: Adwaita blue folder icons
  (darker tab + lighter face + symbolic glyph inside), a flat rounded-rectangle
  sidebar selection (not a pill or full-width bar), a sidebar header row
  ("search / Files / menu"), a centered pill breadcrumb in the headerbar with a
  home glyph, the grid/list view toggle with a dropdown chevron, and a light,
  airy layout with generous icon spacing. Dark mode follows the Adwaita dark
  palette.

### Visual target (revised 2026-10-23)

The user corrected the original "Nautilus" framing: the target is **modern
GNOME Files (GNOME 50/51) with libadwaita**, confirmed against a real
screenshot and an approved HTML mockup. Key specifics:

- **Folder icons:** Adwaita style — a darker-blue folder tab over a lighter-blue
  face, with a symbolic monochrome glyph centered on known folders (Downloads =
  down-arrow, Music = note, Pictures = image, Videos = video, Public = share).
- **Sidebar:** top header row with a search icon, the word "Files", and a
  hamburger menu. Two sections split by a divider: places
  (Home / Recent / Starred / Network / Trash) then bookmarks
  (Documents / Music / Pictures / Videos / Downloads) then mounts. Active row
  uses a flat rounded-rectangle highlight with the accent-colored icon.
- **Headerbar:** back / forward, a centered pill breadcrumb (home glyph +
  segments), then `⋮` menu, new-folder button, and the grid/list toggle with a
  dropdown chevron.
- **Palette (Adwaita, from the GNOME HIG):** accent blue `#3584e4`; light bg
  `#ffffff`/`#f6f6f4`, borders `~#e4e4e4`/`#deddda`, text `#2e3436`/`#241f31`;
  dark bg `~#1d1d1f`/`#242426`, dark accent `~#78aeed`.
- **Font:** Cantarell where available, falling back to the system UI font.
- **Path bar — Cmd+L / Ctrl+L** (new requirement, see Goals): the breadcrumb
  toggles into a focused, pre-filled editable path field; Enter navigates, Esc
  reverts to breadcrumb.
- **Icons**: use a simple icon set (folder/file/known-types). Full per-file
  thumbnails are a non-goal; generic type icons are sufficient for v1.

## What's left to the builder

- Exact project scaffolding and file layout within the Tauri conventions.
- Precise Rust command surface (the set of `#[tauri::command]` functions) for
  listing directories, file ops, etc.
- How directory contents are modeled and passed to the frontend (one listing
  command returning name, path, is_dir, size, modified, is_hidden per entry is
  the expected shape).
- Keyboard shortcut specifics (sensible Nautilus-like defaults: Cmd+C/X/V,
  Enter/F2 to rename, Cmd+Delete to Trash, Cmd+Shift+. for hidden files, etc.).
- CSS details and the specific icon set.
- How pinned sidebar folders are persisted (a small local config/JSON file is
  the expected approach).
- Error presentation (how a failed op — permission denied, name collision —
  is surfaced to the user).

## Resolved decisions (from user + builder check)

These were surfaced by the builder check and resolved before finalizing.

### File access (user's decision)
The app will rely on a **one-time "Full Disk Access"** grant in System
Settings → Privacy & Security. After the user enables it once, the app
browses the whole filesystem with no further TCC prompts. This fits the
anti-goal of "don't fight permission prompts constantly."
- Implication: the build can be an unsigned/ad-hoc local Tauri app (personal
  use, no Developer ID / notarization needed). Onboarding should include a
  one-time instruction screen or note telling the user to grant Full Disk
  Access to the app, and ideally a deep link to the right System Settings pane.
- Implication: Tauri FS access goes through Rust `std::fs` (not the sandboxed
  scoped-FS plugin), since Full Disk Access makes the whole tree reachable.

### Paste conflict policy (user's decision)
On paste into a folder containing a same-named entry, the app **asks the
user**: Replace / Skip / Keep both (Nautilus behavior). Never silently
overwrites. This directly serves the "never clobber or lose files" anti-goal.
- Implication: copy/move Rust commands take a conflict-resolution parameter and
  surface conflicts back to the UI for a dialog; the UI has a conflict dialog.

### Large-folder performance (my call)
Target: a folder with ~10k entries should feel responsive (list/grid visible
in well under a second). To get there without a UI framework:
- The Rust listing command returns all entries in one call (name, path,
  is_dir, size, modified, is_hidden) — a single `read_dir` + `symlink_metadata`
  pass is fast in Rust.
- The frontend uses **virtualized rendering** (render only visible rows/cells,
  recycle on scroll) so thousands of entries don't create thousands of DOM
  nodes. This is the one place I'll accept a small amount of custom JS
  complexity. If a folder is enormous, this keeps scrolling smooth.
- No pagination or streaming in v1 — virtualization on a one-shot listing is
  sufficient for personal use and much simpler.

### Copy/move semantics (my call)
- Copy and move operate recursively on directory trees.
- Move within the same volume uses `rename` (atomic, fast); move across volumes
  falls back to copy-then-delete.
- "Cut" is a true move-on-paste: the cut highlight clears only after a
  successful paste; Esc cancels a pending cut. Cut-then-paste across volumes is
  a move (copy+delete), consistent with within-volume cut.
