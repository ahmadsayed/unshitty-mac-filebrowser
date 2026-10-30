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
layouts); `recent.rs` branches for its source and `file_icons.rs` for whose
icons it asks (NSWorkspace on macOS, GIO and the icon theme on Linux).
Everything else must stay platform-neutral.

Design intent and decisions live in `docs/superpowers/specs/` and the task plan
in `docs/superpowers/plans/`. Read the spec before changing behaviour.

## Layout

| Path | What it holds |
| --- | --- |
| `src/main.ts` | App state, navigation, path bar, search, virtualized grid/list views, selection, menus, clipboard + paste, drag and drop, bookmarks, keyboard shortcuts, sidebar, init |
| `src/ui.ts` | libadwaita-style widgets: popovers, popover menus, dialogs, toasts |
| `src/icons.ts` | Icon sprite (symbolic `sym-*`, full-colour `fc-*`), drawn for the theme; file-kind detection |
| `src/icons-macos.ts` | The macOS theme's drawings under the same symbol ids: folders, file pages, line icons |
| `src/thumbnails.ts` | Thumbnail request queue (4 concurrent) and bounded blob-URL cache |
| `src/native-icons.ts` | Icons from the OS: ids in one batch per render, one image per type and size; single icons for Open With, Properties and the preview |
| `src/format.ts` | GNOME-style size and date strings |
| `src/api.ts` | Typed wrappers for every Tauri command |
| `src/styles.css` | All styling: layout, views and widgets, reading every look-related value from theme tokens |
| `src/themes/gnome.css`, `src/themes/macos.css` | The two themes' tokens (colours, font, sizes, radii, glass), light + dark; macOS also has a few scoped rules |
| `src/theme.ts` | The platform's theme (macOS on a Mac, GNOME elsewhere) and the per-theme layout sizes the virtualized views use |
| `src/qr-share.ts` | Share with QR Code…: the menu item and the dialog (code, link, progress, downloads) |
| `src/updates.ts` | Check for Updates dialogs: offer, download progress, restart, failures |
| `src-tauri/src/fs_ops.rs` | Listing, item counts, mkdir, rename, copy/move/trash, path info |
| `src-tauri/src/sftp.rs` | Read-only SSH/SFTP connections for Network: agent/key/password auth, known-host checks, listing and downloads |
| `src-tauri/src/share/file_share.rs`, `file_link.rs` | Share with QR Code: hashing, the separate iroh endpoint and serving (`file_share`); the ticket, file list and `filebrowser/file/2` wire format (the page still reads version 1 links), compiled into the phone page too (`file_link`) |
| `web/file-get/` | The page a phone opens from the QR code: Rust → WebAssembly (iroh in the browser), `build.sh`, `serve.sh` (test from this computer), published by `.github/workflows/file-page.yml`; `static/get/preview.js` (file types and sizes, the image preview, the rows for several files) is tested in `tests/file-page.test.ts` |
| `src-tauri/src/properties.rs` | Properties dialog data: size, dates, permissions, symlink targets, recursive folder size |
| `src-tauri/src/platform.rs` | Platform differences: volume roots per OS, `%XX` decoding, `file://` URIs, XDG data dir |
| `src-tauri/src/thumbnails.rs` | Quick Look (`qlmanage`) thumbnails on macOS with an `image`-crate fallback everywhere, disk cache |
| `src-tauri/src/file_icons.rs` | File and folder icons from the OS: Finder's (NSWorkspace) on macOS, the desktop's icon theme (GIO + freedesktop lookup) on Linux |
| `src-tauri/src/trash_ops.rs` | Trash + locate (for Undo), restore, delete permanently, empty; macOS `~/.Trash` + Finder "Put Back" in `.DS_Store`, Linux `Trash/{files,info}` + `.trashinfo`, `trash_log.json` |
| `src-tauri/src/updates.rs` | Check for Updates: how this copy was installed (only the AppImage and the Mac app replace themselves), failure kinds, one install at a time; `lib.rs` wraps `tauri-plugin-updater` |
| `src-tauri/src/downloads.rs` | Copies of opened SFTP/Drive/share files in the app cache: marks them as downloaded, prunes them after a week (edited copies go to the Trash) |
| `src-tauri/src/recent.rs` | Recent: Spotlight (`mdfind`) on macOS, `recently-used.xbel` on Linux, merged with `recent.json` |
| `src-tauri/src/config_store.rs` | `pins.json` / `starred.json` / `flags.json` in the app config dir, Full Disk Access check |
| `src-tauri/src/versions/` | Keep Versions: `.versions/` ownership, SQLite index, content-addressed zstd objects, capture/watch/restore |
| `src-tauri/src/lib.rs` | Command registration, commands that need `AppHandle`, startup |
| `dev/mock-tauri/` | In-memory fake backend for browser previews and the frontend tests |
| `tests/` | Vitest frontend tests: the real app in jsdom against `dev/mock-tauri/` (`tests/app.ts` boots and drives it) |
| `scripts/crap-gate.mjs` | CRAP gate run by `.githooks/pre-commit` |
| `scripts/session-start.sh` | Where the project stands, from fresh data: main vs GitHub, worktrees, merged branches, drafts, issues |
| `scripts/new-task.sh`, `scripts/end-task.sh` | A worktree per task in `.worktrees/<name>`; finalized (worktree and branches deleted) once its PR is merged |
| `scripts/mock-screenshot.mjs`, `scripts/pr-screenshots.sh` | PR screenshots: capture the mock preview, upload to the `captures` branch |
| `.github/pull_request_template.md` | The PR body every task follows |
| `HUMANS.md` | The human's side of the workflow: review, merge, test, release |

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
  `trash_ops.rs`, `downloads.rs`, `file_icons.rs` and `versions/` into a scratch crate depending on `serde`,
  `serde_json`, `trash`, `image` (same features as `src-tauri/Cargo.toml`),
  `rusqlite` (`default-features = false`, features `bundled` + `backup`), `zstd`,
  `blake3`, `notify` and `similar` (and `gio` on Linux, `objc2`, `objc2-foundation`,
  `objc2-app-kit` and `xattr` on macOS, with the features `src-tauri/Cargo.toml`
  lists), plus dev-dependency `tempfile`, strip the
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
- Pruning the app cache deletes only what the app made and nobody changed
  (thumbnails, opened version copies, unchanged downloads); a downloaded copy
  that was edited goes to the Trash (`downloads.rs`).
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

**Look like the theme, behave like GNOME Files 50.**
- Two themes, picked by the platform, with no choice in the app: the GNOME
  theme follows GNOME Files 50 on Linux, the macOS theme follows macOS 26
  Tahoe on a Mac. Only the look differs: wording, behaviour, shortcuts, dialog
  button order and default responses are GNOME's in both.
- Use the tokens (`--accent-bg`, `--view-bg`, `--hover`, `--radius-button`,
  …), not hard-coded colours. A new look-related value is a token with a value
  in each theme (`src/themes/`), and every colour needs a dark-mode value.
  Sizes the views compute in JavaScript go in `LAYOUT_SIZES` (`src/theme.ts`).
- The macOS theme is an original imitation: no Apple fonts, icons or other
  assets (the system font is used through `-apple-system`). File and folder
  icons the OS draws at run time (`file_icons.rs`) are not assets: nothing is
  copied into the repository or the bundle.
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

## Life of a task

**Nothing lands on `main` that the human didn't merge.** Every change, docs
and release bumps included, goes through a pull request. `main` is protected
on GitHub (a PR is required, admins included; no force push or deletion), so
a direct push is refused. Agents never merge unless the human says so in the
conversation; GitHub can't enforce that part, since agents use the human's
`gh` account.

0. **Start from fresh data.** At the start of a session, and before answering
   any question about the project's state (what's left, issues, PRs, what
   was merged, what a release contains), run `scripts/session-start.sh`.
   Never answer from memory or from a checkout that may be behind; if the
   fetch fails, say the data may be stale. Act on what it reports: finalize
   merged tasks (step 8), mention open PRs, drafts and branches that can go.
1. **Work in a worktree of your own**, never in the main checkout: it may
   hold the human's uncommitted work. `scripts/new-task.sh <kind> <name>`
   (`feat`, `fix`, `perf`, `refactor`, `docs`, `chore`) creates
   `.worktrees/<name>` on branch `<kind>/<name>` from `origin/main`, with
   `node_modules` linked. Look for an open issue that covers the task.
2. **Test first**, then make the change in small commits, then run the
   checks under [Commands](#commands). For a bug, reproduce it before fixing
   and say how.
3. **Test the real thing, not only the mock.** Mock and unit tests don't run
   the packaged app, its environment or the host's programs (the 0.5.1
   AppImage couldn't open files: host `gio` loaded its bundled GLib). When a
   change touches packaging, launching other programs, the filesystem or the
   platform, run the real app (`npm run tauri dev`, or the built bundle) or
   ask the human to, saying exactly what to try and what they should see.
4. **Take screenshots** of anything visible: before (from `main`) and after,
   with `npm run dev:mock` running: `node scripts/mock-screenshot.mjs <path>
   <file.png>` (`--dark` for dark mode, `--theme gnome|macos`, `--scene
   grid|list|menu|dialog`; the clock is fixed, so `cmp` can prove two
   captures identical). Look at them before using them.
5. **Open the PR** ([Pull requests](#pull-requests)): push the branch,
   upload the screenshots, fill in the template.
6. **Review your own PR** and post the review as a comment, then hand over
   to the human: the link, the verdict, what to look at or test. Stop there.
7. **The human reviews and merges.** If they ask for changes, commit them on
   the same branch, push, and update the review comment.
8. **Finalize after the merge**, from the main checkout:
   `scripts/end-task.sh <name>`. It checks the work is on `origin/main` (or
   in a merged PR, for a squash merge), deletes the worktree, the local
   branch and the branch on GitHub, and fast-forwards the main checkout if
   it is clean. Then reply on the issue if the merge didn't close it, and
   tell the human it's done.

**Record what isn't verified.** Work that can't be checked here (macOS from
Linux and the reverse, a real launch, real network shares) is merged as
done-but-unverified: the PR's "Not verified" section says so, and an open
issue labelled `needs-verification` keeps a checklist line per point, so it
isn't lost in a chat. Remove the label when every point is checked.

Run `gh`, `git` and the scripts as plain commands, without `cd …&&`, `;` or
variables around them: permission rules match whole commands.

## Pull requests

- **Title:** conventional and short, like a commit subject
  (`fix: open files from the Linux AppImage`).
- **Body:** follow `.github/pull_request_template.md`, readable in two
  minutes. Write it to a file outside the repository (the scratchpad) and
  run `gh pr create --base main --title "…" --body-file <file>`.
- **Issues:** one line each at the top: `Closes #<n>` when the PR resolves
  all of it, `Refs #<n>` when a part or a verification stays open, `No
  issue` otherwise.
- **Screenshots:** `scripts/pr-screenshots.sh before.png after.png` stores
  them on the `captures` branch (never `main`), in a folder named after the
  task branch, and prints the Markdown to paste. Light and dark when colours
  change; "No visible change" otherwise.
- **How it was checked:** tick only what really ran, with the test counts.
  "Real app" names what was tried and by whom, or why it wasn't needed.
- **Self-review comment**, posted with `gh pr comment <n> --body-file <file>`
  once the PR is open:

  ```markdown
  ## Review

  **Verdict: ✅ ready to merge** (or ⚠️ needs a decision / ❌ blocked)

  | Check | Result |
  |---|---|
  | Mergeable with main, no conflict | ✅ |
  | Checks run locally (tsc, build, npm test, cargo test, CRAP gate) | ✅ N + M passed |
  | Mock preview and screenshots inspected | ✅ / no UI change |
  | Real app, where packaging or other programs are involved | ✅ … / not needed: … |
  | Rules of this file (data safety, huge folders, GNOME look, security) | ✅ |
  | Docs (README, AGENTS.md, HUMANS.md) in sync | ✅ |

  ### Findings
  - Fixed: …
  - For the human to decide: … (options A / B)
  ```

  Fix what you find before posting; anything you can't settle is ⚠️ with
  options, never improvised.

## Releases

Release only when the human asks, and only after something significant.

1. `scripts/session-start.sh`: `main` is what you mean to ship, no open PR
   or worktree holds work meant for it, no draft from an earlier attempt
   lingers.
2. In a `chore` task (`release-X.Y.Z`), bump the version in `package.json`,
   `src-tauri/Cargo.toml`, `src-tauri/Cargo.lock` (the `filebrowser` package
   only), `src-tauri/tauri.conf.json` and `docs/linux-appimage.md`, and open
   the PR `chore: release X.Y.Z` listing what's new. The human merges it.
3. Tag `vX.Y.Z` (annotated) on the merged commit of `origin/main` and push
   the tag. The Release workflow
   checks the tag matches the version, runs the tests, builds macOS and Linux
   and creates a **draft** release.
4. **Smoke-test the draft before publishing.** Download its package for this
   machine (verify the size and SHA-256 against the release's digest), install
   it the way users do (`scripts/install-linux-appimage.sh` for the
   AppImage), quit any running copy, and in the installed app: open a folder,
   open a file with its default app and with Open With, search, and show
   thumbnails of a photo folder. Ask the human to do the same on the other
   platform, or note it on a `needs-verification` issue.
5. Publish (`gh release edit vX.Y.Z --draft=false`) only when the human says
   so. The release must contain `latest.json`; without it Check for Updates
   reports "Could not check for updates" to everyone. A draft can be fixed
   by deleting it and moving the tag to the fix; a **published** tag never
   moves: ship a new patch version instead.
6. **Right after publishing, check the update path** (the updater only sees
   published releases): in the previously installed version, Check for
   Updates must offer the new one, install it and restart into it. Ask the
   human to do the same on the other platform. If it fails, the next patch
   release has to fix it, so say so at once.

**The updater signing key is the human's.** Its private half and password
live only in the human's backup and in the `TAURI_SIGNING_PRIVATE_KEY` /
`TAURI_SIGNING_PRIVATE_KEY_PASSWORD` secrets. Agents never generate, read,
print, copy or replace it; the public key in `tauri.conf.json` changes only
when the human says so (installed copies would reject every update signed
with a new key).

## Licensing

The project is GPL-3.0-or-later (`LICENSE`). GNOME Files is the design
reference only:
- Don't copy code, icons or other assets from Nautilus, libadwaita or the
  Adwaita icon theme. Draw icons from scratch in the Adwaita style. Showing
  the icons the user's own OS provides at run time (Finder's, the installed
  icon theme) is fine: they are read from the system, never bundled.
- Bundled third-party assets need their license next to them (see
  `src/assets/fonts/AdwaitaSans-LICENSE.txt`) and a line in the README credits.
- Keep the README's independence notice and trademark section intact. The
  project must never suggest it is affiliated with or endorsed by GNOME, Apple
  or anyone else.
