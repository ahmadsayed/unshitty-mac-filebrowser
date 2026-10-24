# AGENTS.md

Guidance for AI coding agents working in this repository.

## What this is

A macOS file browser that imitates **GNOME Files 50** (Nautilus + libadwaita).
Tauri 2 app: Rust backend for every filesystem operation, plain TypeScript +
Vite frontend (no UI framework). Personal hobby project, GPL-3.0-or-later; it
contains no GNOME code and must not copy any (see "Licensing" below).

Design intent and decisions live in `docs/superpowers/specs/` and the task plan
in `docs/superpowers/plans/`. Read the spec before changing behaviour.

## Layout

| Path | What it holds |
| --- | --- |
| `src/main.ts` | App state, navigation, path bar, search, virtualized grid/list views, selection, menus, clipboard + paste, drag and drop, bookmarks, keyboard shortcuts, sidebar, init |
| `src/ui.ts` | libadwaita-style widgets: popovers, popover menus, dialogs, toasts |
| `src/icons.ts` | Icon sprite (symbolic `sym-*`, full-colour `fc-*`), file-kind detection |
| `src/thumbnails.ts` | Thumbnail request queue (4 concurrent) and bounded blob-URL cache |
| `src/format.ts` | GNOME-style size and date strings |
| `src/api.ts` | Typed wrappers for every Tauri command |
| `src/styles.css` | All styling; libadwaita colour tokens at the top, light + dark |
| `src-tauri/src/fs_ops.rs` | Listing, item counts, mkdir, rename, copy/move/trash, path info |
| `src-tauri/src/thumbnails.rs` | Quick Look (`qlmanage`) thumbnails with an `image`-crate fallback, disk cache |
| `src-tauri/src/recent.rs` | Recent: Spotlight's last-used dates (`mdfind`) merged with `recent.json` |
| `src-tauri/src/config_store.rs` | `pins.json` / `flags.json` in the app config dir, Full Disk Access check |
| `src-tauri/src/lib.rs` | Command registration, commands that need `AppHandle`, startup |
| `dev/mock-tauri/` | In-memory fake backend for browser previews |

## Commands

```sh
npm install                 # see "npm registry" below
npm run tauri dev           # run the app (macOS)
npm run tauri build         # .app in src-tauri/target/release/bundle/macos/
npx tsc --noEmit            # type-check the frontend
npm run build               # tsc + vite build
npm run dev:mock            # UI in a normal browser with a fake filesystem (port 1420)
cd src-tauri && cargo test  # backend tests
```

Before finishing a change: `npx tsc --noEmit`, `npm run build` and
`cargo test` must pass. For UI changes, also look at the result in
`npm run dev:mock`.

## Environment gotchas

- **Linux / CI containers can't build the Tauri crate** (it needs WebKitGTK).
  The backend modules are written so they can be tested without Tauri: copy
  `fs_ops.rs`, `config_store.rs`, `thumbnails.rs` and `recent.rs` into a scratch crate that
  depends on `serde`, `serde_json`, `trash`, `image` (same features as
  `src-tauri/Cargo.toml`) and dev-depends on `tempfile`, strip the
  `#[tauri::command]` / `#[tauri::command(async)]` lines, and run `cargo test`.
  Keep new backend logic in plain functions so this keeps working; only
  `lib.rs` may use `AppHandle` and other Tauri types.
- **Nothing here can be verified on macOS from Linux**: window chrome, traffic
  lights, Quick Look thumbnails, Spotlight results in Recent, Full Disk Access,
  opening files. Say so
  explicitly when reporting work that touches them.
- **npm registry**: `package-lock.json` is git-ignored on purpose. An earlier
  lockfile pinned every package to a private corporate registry. Never commit a
  lockfile and never add registry URLs or `.npmrc` entries pointing at private
  registries. If `npm install` hangs, run
  `npm install --package-lock=false`.
- `Cargo.lock` *is* committed. When adding a crate, update it minimally (e.g.
  `cargo metadata --format-version 1 >/dev/null` in `src-tauri/`) and check the
  diff only adds entries.

## Mock preview

`npm run dev:mock` aliases `@tauri-apps/api/core`, `/path`, `/webview` and
`@tauri-apps/plugin-opener` to `dev/mock-tauri/` (see `vite.config.ts`).
**Every new Tauri command or plugin call needs a mock** in
`dev/mock-tauri/core.ts` (or a sibling), or the preview breaks.

- Fake home is `/Users/me`; `/big` has 10,000 entries for performance checks;
  `/System` fails with "Operation not permitted".
- `?demo` in the URL hides test-only folders (used for README screenshots).
- `window.__mockDrop(paths, x, y)` simulates files dropped from Finder.
- Playwright works for screenshots and scripted checks; Chromium is at
  `/opt/pw-browsers` in the cloud environment. Use a Mac user agent so the app
  takes its macOS code paths.

## Rules the code must keep

**Never lose data.** This is the project's main anti-goal.
- Deleting always goes to the Trash (`trash` crate), never `remove_*`.
- Replacing an existing item stages the new one under a temporary name, sends
  the old one to the Trash, then renames into place.
- A cross-volume move deletes the source only after the copy fully succeeded.
- Copy/move refuse: vanished source, same path, destination inside the source,
  existing target without `overwrite`, replacing a folder with its own child.
  Add a test for every new refusal.
- Name clashes always ask (Replace / Skip / Keep Both); Enter picks Keep Both.

**Stay fast with huge folders.**
- Views are virtualized: items are absolutely positioned slots computed by
  `computeLayout` / `itemPos`, and only `renderWindow`'s range is in the DOM.
  Never render all of `shown`, never measure items with `getBoundingClientRect`
  for hit-testing (rubber band and drop targets use layout maths), and use
  `scrollToIndex` instead of `scrollIntoView`.
- Folder item counts and thumbnails are fetched only for rendered rows.
- Commands that touch many files are `#[tauri::command(async)]` so they run off
  the main thread.

**Look and behave like GNOME Files 50.**
- Use the libadwaita tokens in `styles.css` (`--accent-bg`, `--view-bg`,
  `--hover`, …), not hard-coded colours; every colour needs a dark-mode value.
- Use GNOME's wording for labels, menus and messages ("Folder is Empty",
  "Move to Trash", "A folder with that name already exists.").
- Widgets: reuse `openMenu`, `openPopover`, `openDialog`, `showToast` from
  `ui.ts` rather than new ad-hoc overlays.
- Shortcuts accept ⌘ on macOS and Ctrl elsewhere (`isMac ? metaKey : ctrlKey`).

**Security.** Escape every file name or path inserted with `innerHTML` via
`escapeHtml`. Icon markup comes from fixed templates only.

## Conventions

- Match the surrounding code: section banners (`// ----`), short doc comments
  on non-obvious functions, no framework, no new runtime dependencies without a
  good reason.
- Persist small UI preferences in `prefs` (localStorage, wrapped in try/catch);
  persist durable settings through `config_store.rs`.
- Commit messages: conventional style (`feat:`, `fix:`, `docs:`, `chore:`),
  imperative subject, a body explaining what and why.
- Keep the README's feature list and "Not supported yet" section in sync with
  behaviour changes.

## Licensing

The project is GPL-3.0-or-later (`LICENSE`). GNOME Files is the design
reference only:
- Don't copy code, icons or other assets from Nautilus, libadwaita or the
  Adwaita icon theme. Draw icons from scratch in the Adwaita style.
- Bundled third-party assets need their license next to them (see
  `src/assets/fonts/AdwaitaSans-LICENSE.txt`) and a line in the README credits.
- Keep the README's independence notice and trademark section intact. The
  project must never suggest it is affiliated with or endorsed by GNOME, Apple
  or anyone else.
