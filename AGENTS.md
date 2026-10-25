# AGENTS.md

Guidance for AI coding agents working in this repository.

## What this is

A file browser for **macOS and Linux** that imitates **GNOME Files 50**
(Nautilus + libadwaita). Tauri 2 app: Rust backend for every filesystem
operation, plain TypeScript + Vite frontend (no UI framework). Personal hobby
project, GPL-3.0-or-later; it contains no GNOME code and must not copy any (see
"Licensing" below).

Platform differences are confined to `src-tauri/src/platform.rs` (volumes,
percent-decoding, XDG paths) and `src-tauri/src/trash_ops.rs` (the two Trash
layouts); `recent.rs` branches for its source. Everything else must stay
platform-neutral.

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
| `src-tauri/src/sftp.rs` | Read-only SSH/SFTP connections for Network: agent/key/password auth, known-host checks, listing and downloads |
| `src-tauri/src/properties.rs` | Properties dialog data: size, dates, permissions, symlink targets, recursive folder size |
| `src-tauri/src/platform.rs` | Platform differences: volume roots per OS, `%XX` decoding, `file://` URIs, XDG data dir |
| `src-tauri/src/thumbnails.rs` | Quick Look (`qlmanage`) thumbnails on macOS with an `image`-crate fallback everywhere, disk cache |
| `src-tauri/src/trash_ops.rs` | Trash + locate (for Undo), restore, delete permanently, empty; macOS `~/.Trash` + Finder "Put Back" in `.DS_Store`, Linux `Trash/{files,info}` + `.trashinfo`, `trash_log.json` |
| `src-tauri/src/recent.rs` | Recent: Spotlight (`mdfind`) on macOS, `recently-used.xbel` on Linux, merged with `recent.json` |
| `src-tauri/src/config_store.rs` | `pins.json` / `starred.json` / `flags.json` in the app config dir, Full Disk Access check |
| `src-tauri/src/versions/` | Keep Versions: `.versions/` ownership, SQLite index, content-addressed zstd objects, capture/watch/restore |
| `src-tauri/src/lib.rs` | Command registration, commands that need `AppHandle`, startup |
| `dev/mock-tauri/` | In-memory fake backend for browser previews and the frontend tests |
| `tests/` | Vitest frontend tests: the real app in jsdom against `dev/mock-tauri/` (`tests/app.ts` boots and drives it) |
| `scripts/crap-gate.mjs` | CRAP gate run by `.githooks/pre-commit` |

## Commands

```sh
npm install                 # see "npm registry" below
npm run tauri dev           # run the app
npm run tauri build         # macOS: bundle/macos/*.app; Linux: bundle/{deb,rpm,appimage}
npx tsc --noEmit            # type-check the frontend
npm run build               # tsc + vite build
npm run dev:mock            # UI in a normal browser with a fake filesystem (port 1420)
cd src-tauri && cargo test  # backend tests
npm test                    # frontend tests (Vitest + jsdom)
npm run crap                # CRAP gate (also runs as the pre-commit hook)
npm run crap:baseline       # accept the current scores into .crap-baseline.json
```

Before finishing a change: `npx tsc --noEmit`, `npm run build`,
`npm test` and `cargo test` must pass. For UI changes, also look at the result in
`npm run dev:mock`.

**CRAP gate.** `npm install` sets `core.hooksPath` to `.githooks`, whose
`pre-commit` runs `scripts/crap-gate.mjs` for the languages a commit touches.
It fails when a function's CRAP score is above 30 and is new or worse than in
`.crap-baseline.json` (currently empty: nothing is above 30). Both languages
are scored with real coverage, so their tests must pass: Rust through
`cargo llvm-cov` + `cargo-crap` (install with
`cargo install --locked cargo-llvm-cov cargo-crap` and
`rustup component add llvm-tools-preview`), TypeScript through Vitest's
coverage handed to `fallow`. CRAP can never be below a function's cyclomatic
complexity, so a function over 30 needs splitting (lookup tables work well for
key and command dispatch), not just tests. Fix a failure by testing or
simplifying the function; update the baseline only when a score is accepted
on purpose, and say so in the commit message. Don't bypass the hook with
`--no-verify`.

**Frontend tests** boot the real app (`tests/app.ts`) in jsdom with
`dev/mock-tauri/` as the backend; each test file gets a fresh app and fake
filesystem. `tests/setup.ts` stubs what jsdom lacks (layout sizes,
`ResizeObserver`, `elementFromPoint`, the clipboard, a Mac user agent).
Drive the UI with events (`key`, `mouse`, `goTo`, `respond`) and assert on
the DOM. Code that adds a mock command needs a case in
`tests/mock-core.test.ts`.

## Environment gotchas

- **The Tauri crate needs GTK3 + WebKitGTK 4.1 headers on Linux** (they come with
  the OS; see README). If the host has no root access to install them, build in a
  rootless container instead — the image needs the same packages:
  ```sh
  podman build -t tauri-linux -f dev/tauri-linux.Containerfile   # gtk3-devel, webkit2gtk4.1-devel, libsoup3-devel, rust, cargo
  podman run --rm -v "$PWD:/src:z" -v tauri-cargo:/cargo -w /src/src-tauri \
           tauri-linux cargo test
  ```
  `npm run build` must run first, because `generate_context!` reads `../dist`.
- **Backend modules must still be testable without Tauri.** Copy `fs_ops.rs`,
  `config_store.rs`, `platform.rs`, `properties.rs`, `thumbnails.rs`, `recent.rs`,
  `trash_ops.rs` and `versions/` into a scratch crate depending on `serde`,
  `serde_json`, `trash`, `image` (same features as `src-tauri/Cargo.toml`),
  `rusqlite` (`default-features = false`, features `bundled` + `backup`), `zstd`,
  `blake3`, `notify` and `similar`, plus dev-dependency `tempfile`, strip the
  `#[tauri::command]` lines, and run `cargo test`. Keep new backend logic in
  plain functions; only `lib.rs` may use `AppHandle` and other Tauri types.
- **macOS-only behaviour can't be verified from Linux**: window chrome, traffic
  lights, Quick Look thumbnails, Spotlight results in Recent, where Finder puts
  trashed items and its `.DS_Store` "Put Back" records, Full Disk Access,
  opening files. Say so explicitly when reporting work that touches them. The
  mirror image also holds: Linux-side behaviour (`.trashinfo`, `recently-used.xbel`,
  udisks2 mount paths) is unverified on a Mac.
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
- Deleting always goes to the Trash (`trash` crate), never `remove_*`. The only
  exceptions are Delete Permanently and Empty Trash in `trash_ops.rs`: they act
  only inside a Trash folder (refused elsewhere, with tests) and only after a
  confirmation dialog whose default response is Cancel.
- Restoring never overwrites: an existing item at the original path is an error.
- Anything that touches `.versions/` must prove ownership (the `meta.json` + DB
  `meta` marker, D14) before registering, clearing or trashing it. Object files
  are content-addressed: a full frame and a delta frame of the same bytes share
  a name, so the `objects` row decides the encoding and a referenced object is
  never rewritten.
- Anything that touches the Trash must work for both layouts (macOS `~/.Trash`
  and freedesktop `Trash/{files,info}`); a change there needs a test on each.
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
- Commands that touch many files or wait on other programs are `async fn`
  commands in `lib.rs` that run their work through `blocking(…)` (Tauri's
  blocking-thread pool). Don't use `#[tauri::command(async)]` on a plain
  function: that runs it on the few shared async workers, and a slow copy or a
  queue of thumbnails then stalls every other command.
- The thumbnail cache in `src/thumbnails.ts` is capped by bytes (64 MB), not
  just by count: photo thumbnails are ~120 KB at 256 px and ~450 KB at 512 px.

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
