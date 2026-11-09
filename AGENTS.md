# AGENTS.md

Guidance for AI coding agents working in this repository.

## What this is

A file browser for **macOS and Linux** that imitates **GNOME Files 50**
(Nautilus + libadwaita). Tauri 2 app: Rust backend for every filesystem
operation, plain TypeScript + Vite frontend (no UI framework). Personal hobby
project, GPL-3.0-or-later; it contains no GNOME code and must not copy any (see
"Licensing" below).

Platform differences are confined to `src-tauri/src/platform.rs` (volumes,
percent-decoding, XDG paths, the rename that refuses a taken name) and
`src-tauri/src/trash_ops.rs` (the two Trash layouts); `recent.rs` branches
for its source and `file_icons.rs` for whose icons it asks (NSWorkspace on
macOS, GIO and the icon theme on Linux).
Everything else must stay platform-neutral.

Design intent and decisions live in `docs/superpowers/specs/` and the task plan
in `docs/superpowers/plans/`. Read the spec before changing behaviour.
[docs/architecture.md](docs/architecture.md) shows how the modules fit
together and where each kind of change lands; keep it and its diagram in
step when a module's role changes.

## Layout

| Path | What it holds |
| --- | --- |
| `src/main.ts` | App state, navigation, path bar, search, virtualized grid/list views, selection, menus, clipboard + paste, drag and drop, bookmarks, keyboard shortcuts, sidebar, init |
| `src/ui.ts` | libadwaita-style widgets: popovers, popover menus, dialogs, toasts; a dialog's busy mode (one action in flight, what stays live, the status line, failures in words, the buttons put back) and `cleanError` |
| `src/icons.ts` | Icon sprite (symbolic `sym-*`, full-colour `fc-*`), drawn for the theme; file-kind detection; `itemIcon`, every item icon's markup: its picture (drawing, OS icon, thumbnail, glyph) and which emblems (link, versions, shared) go on it, in what order |
| `src/icons-macos.ts` | The macOS theme's drawings under the same symbol ids: folders, file pages, line icons |
| `src/thumbnails.ts` | Thumbnail request queue (4 concurrent) and bounded blob-URL cache |
| `src/item-icons.ts` | Which picture an item shows (thumbnail, the SSH server's and each online account provider's glyph in Network, OS icon, drawing), the shown size, which places have OS icons, the first-screenful wait before a folder shows, the macOS folder icons asked again per listing and when recounted; the dialogs' icons |
| `src/native-icons.ts` | The OS icons' cache: ids in one batch, one image per type and size, rechecking cached ids; `item-icons.ts` decides what to ask for and when |
| `src/format.ts` | GNOME-style size and date strings |
| `src/api.ts` | Typed wrappers for every Tauri command |
| `src/styles.css` | All styling: layout, views and widgets, reading every look-related value from theme tokens |
| `src/themes/gnome.css`, `src/themes/macos.css` | The two themes' tokens (colours, font, sizes, radii, glass), light + dark; macOS also has a few scoped rules |
| `src/theme.ts` | The platform's theme (macOS on a Mac, GNOME elsewhere) and the per-theme layout sizes the virtualized views use |
| `src/qr-share.ts` | Send with QR Code…: the menu item and the dialog (code, link, progress, downloads) |
| `src/models.ts` | 3D models in the Space preview without three.js: which files are models (`modelFormat`), and which companion files a .gltf may load, only in its folder or below (`companionPath`, `gltfCompanions`) |
| `src/model-gestures.ts` | Turning and zooming the 3D preview with a pointer, without three.js: a drag turns, the wheel and a pinch zoom (two touch points, Ctrl/⌘ + wheel from a trackpad, WebKit's `gesture*` events on a Mac), none zooming the page, and a double-click picks a point (`attachGestures`, `wheelZoom`) |
| `src/model-viewer.ts` | The 3D preview, loaded on demand in its own chunk: three.js reads glTF/GLB/STL/3MF (`loadModel`, no WebGL needed; Draco-compressed glTF through one shared decoder, `dracoLoader`; a glTF fetches only its companions and its own data and blob URLs, `resourceUrl`; STL and 3MF stood up from Z-up; files without a material drawn grey) and draws it to turn and zoom (`showModel`, released by `dispose`); the model turns about the middle of its visible vertices (`shownBounds`) or what is double-clicked (`pickedPoint`: the small object clicked, or the point on a large one; `recentred`) and the camera only moves in and out; Rotate Left, Rotate Right and Reset View over the model, the arrow keys, Home, + and - on it (`modelControls`, `keyAction`, `turned`, `dragTurns`, `zoomed`) |
| `src/preview-versions.ts` | Versions in the Space preview: a Keep Versions history (without deletion markers) or an Arbesk asset's saves as one list, oldest first (`fromLocal`, `fromArbeskSaves`), and the header control that steps through it (`drawVersionControl`: Older Version, "Version N of M" over its date opening the list, Newer Version); `main.ts` shows the picked version through `previewEntry`'s `PreviewAt` |
| `src/preview-actions.ts` | Which actions the Space preview offers (Open, Rename…, Move to Trash, Open Item Location) for where the item is, from `item-actions.ts`'s rules plus the preview's kept exceptions |
| `src/selection.ts` | The view's selection, cursor and anchor, and the user's intent counter a paste checks before selecting what it made (#50) |
| `src/path-refs.ts` | What remembers paths (stars, bookmarks, the clipboard, the background tabs' folders and every tab's Back and Forward) kept pointing at moved, renamed and trashed items: `moved(pairs)` and `gone(paths)`, one rule per holder, each holder replaced (and saved) at most once per report, a paste reporting each item as it lands; `followPath`, the prefix rewrite for what is inside a moved folder |
| `src/latest-save.ts` | `LatestSave`: one file's saves (`starred.json`, `pins.json`) one write at a time, each reading the state as it starts, so the last write is the latest state; saves asked for meanwhile share the next write, and each learns whether it was written |
| `src/search.ts` | The search without the DOM: `SearchSession` (open or not, the text, the walk after a 160 ms pause, at most 1000 results, late answers dropped, what a tab saves), `arrangeSearch` / `searchSections` (what the view shows and its "In This Folder" / "In Subfolders" headers), and `fold`, tested against the backend's `fs_ops::fold` with `tests/fold-cases.json` |
| `src/locations.ts` | Which kind of place a path is (`locationKind`: Recent, Starred, Network, the Trash, a folder in it, a share, an SSH server, Arbesk, Drive, a folder), in one precedence built on `paths.ts`'s `placeOf`, and what each kind allows: read only, local, recursive search, versions, new items, its own thumbnails; the copy-out refusal's wording; what a paste into Arbesk accepts (`arbeskPasteRefusal`, `isArbeskModelFile`, `arbeskTargetName`); which items are local files (`isLocalFile`, `isLocalRegularFile`) |
| `src/item-actions.ts` | Whether an action several surfaces reach (cut, copy, paste, rename, trash, delete permanently, star, bookmark, properties, open location) may run in a kind of place, and the toast that says why not: a `Record<LocationKind, …>` per action, or a rule over `locations.ts`'s kind predicates (cut, copy, paste, open location; paste into Arbesk has its own rule); differences between surfaces kept on purpose are marked "Kept:" |
| `src/paths.ts` | Each kind of path's layout (`gdrive://`, `share://`, `sftp://`, `arbesk://`, local) as one row: its kind (`placeOf`), name, parent, child join (encoded as the backend decodes it) and crumbs; a collection (`isArbeskCollection`); containment (`isUnder`, `isWithin`: not interchangeable), which paths a window may open, share links, what the path bar resolves typed text to |
| `src/folder-summaries.ts` | Folder item counts and version badges for the view: batches of 400, skips remote, known and in-flight folders, drops late answers, forgets counts on a new location or Show Hidden setting, asks again after a reload for those the view asks about (showing the old ones until then); says which answers were asked for one by one (`recounted`) |
| `src/listing-failure.ts` | What a folder that couldn't be listed turns into, one table row per kind and phase (first view, navigate, reload, path entry): share status page, Connect to Server, go to the parent, toast with Grant Access, error page; a Drive failure that lost its credential also carries the answer from `drive-auth.ts`, which opens the sign-in dialog |
| `src/providers.ts` | Online account providers, one row each (`PROVIDERS`: label, icon, which paths are its own, the account key in a path), the accounts in one shape whatever their provider (`Account`), which account a path belongs to (`accountOf`); what a provider does is `PROVIDER_ACTIONS` in `main.ts`, a `Record<ProviderId, …>` |
| `src/online-accounts.ts` | Online Accounts, as GNOME Settings' panel: "Your Accounts" and "Connect an Account" as boxed lists of rows, and each account's dialog (Sign In Again when its last request needed one, Disconnect…, the provider's links); built from `providers.ts` |
| `src/drive-auth.ts` | Which failed Google Drive request needs a credential, and which: the client JSON (`client`) or a new sign-in (`sign-in`); the wording is tested against the backend's messages |
| `src/open-with.ts` | Open With…: the searchable application chooser (filter, keyboard selection, launch and retry) |
| `src/updates.ts` | Check for Updates dialogs: offer, download progress, restart, failures |
| `src/drag-out.ts` | Dragging files out of the window: when the pointer has left it, and the system drag (our GTK command on Linux, the drag plugin on macOS) |
| `src-tauri/src/fs_ops.rs` | Listing, item counts, mkdir, rename, copy/move/trash, path info |
| `src-tauri/src/place.rs` | The backend twin of `placeOf`: a path's kind (`Local`/`Drive`/`Share`/`Sftp`/`Arbesk`), a local path or why it isn't one (remote, malformed or relative), the read-only refusals (`writable`), where a copy, move or rename runs (`transfer`; `copy` also takes an Arbesk asset out, built first, and `Route::IntoArbesk` puts a local model file in), and answering a batch for its local paths only (`local_only`) |
| `src-tauri/src/placement.rs` | How a copy, move or Drive download reaches its local path: the refusals and their wording (`Refusal`), the `.`/`..` guard, staging names, Replace (stage, trash the old item through `ReplaceTrash`, rename in, or take it back out if that fails), the rename that never replaces (`rename_new`) and what a failure undoes (`Target::place`) |
| `src-tauri/src/sftp.rs` | Read-only SSH/SFTP connections for Network: agent/key/password auth, known-host checks, listing and downloads |
| `src-tauri/src/arbesk/` | Arbesk accounts in Network, read plus copying local `.gltf`/`.glb`/`.3mf` files into a collection: `location.rs` (`arbesk://<address>/<token>~<name>/<asset>~<name>.<ext>`, ids decide; `parse_target` for a paste's destination), `chain.rs` (the backend's config and token index, `tokenURI` by hand-written `eth_call`), `ipfs.rs` (gateway reads, brotli/gzip unpacking, manifests cached by CID), `compose.rs` (a stored source back to `.gltf`/`.3mf`, unknown formats refused), `service.rs` (listing; building an asset into the app cache, reused only while unchanged: the preview's file, `build`, or the asset's own `.gltf`/`.3mf` that a copy takes out, `export`; the thumbnail an asset was saved with, its place remembered by the listing; an asset's saves through `prev_asset_manifest_cid`, the newest 50, and the scene of one of them, `versions`/`scene`), `scene.rs` (a manifest's nodes and links resolved into parts and placements), `signin.rs` (`besk login`'s email + code flow through the shared loopback wait, the callback confirmed with Arbesk, the session handed to `session.rs`), `session.rs` (the sign-in's session, 24 h, in the OS keychain with a 0600 file fallback; ended on Disconnect), `accounts.rs` (`arbesk_accounts.json`: email, address, deployment; no credential), `decompose.rs` (asset-core's decompose ported: GLB textures split, parts brotli-framed with `_arbesk` murmur3-128 metadata, 3MF parts raw), `write.rs` (uploads, manifests, the relay's collection update, one write at a time per collection, the wait after a save that may still land), `testing.rs` (the fake Arbesk the write tests share, test-only), `http.rs` (the `Http` seam tests fake, multipart, the slow relay POST); tested against `asset-core`'s own output (`tests/fixtures/arbesk/`, made by `scripts/make-arbesk-fixtures.ts`) |
| `src-tauri/src/share/file_share.rs`, `file_link.rs` | Send with QR Code: hashing, the separate iroh endpoint and serving (`file_share`); the ticket, file list and `filebrowser/file/2` wire format (the page still reads version 1 links), compiled into the phone page too (`file_link`) |
| `web/file-get/` | The page a phone opens from the QR code: Rust → WebAssembly (iroh in the browser), `build.sh`, `serve.sh` (test from this computer), published by `.github/workflows/file-page.yml`; `static/get/preview.js` (file types and sizes, the image preview, the rows for several files) is tested in `tests/file-page.test.ts` |
| `src-tauri/src/properties.rs` | Properties dialog data: size, dates, permissions, symlink targets, recursive folder size |
| `src-tauri/src/platform.rs` | Platform differences: volume roots per OS, `%XX` decoding, `file://` URIs, XDG data dir, the rename that refuses a taken name (`rename_exclusive`: `renameat2` on Linux, `renamex_np` on macOS) |
| `src-tauri/src/thumbnails.rs` | Thumbnails: the desktop's own on Linux (the shared `~/.cache/thumbnails`, read only, used while its URI and modification time match), Quick Look (`qlmanage`) on macOS, an `image`-crate fallback everywhere; disk cache (decoded photos as small JPEGs, transparent ones as PNG), pruned by age and kept under 100 MB; `png_from_bytes` for a picture a place hands over (Arbesk's) |
| `src-tauri/src/file_icons.rs` | File and folder icons from the OS: Finder's (NSWorkspace) on macOS, the desktop's icon theme (GIO + freedesktop lookup) on Linux |
| `src-tauri/src/trash_ops.rs` | `Trash` (one in `lib.rs`'s state): `trash_log.json` and its lock, where an item came from (sidecar, log, Put Back), send (trash one item and log it), untrash (for a Replace that failed after trashing the old item, forgetting its entry), restore, delete permanently, empty, what is inside the Trash; Move to Trash's batches (`trash_many`, `trash_routed`) as free functions; macOS `~/.Trash` and `<volume>/.Trashes/<uid>` + Finder "Put Back" in each one's own `.DS_Store`, Linux `Trash/{files,info}` and `<volume>/.Trash-<uid>` + `.trashinfo`; the layout is a value (`Layout`), so tests lay out either on any host |
| `src-tauri/src/updates.rs` | Check for Updates: how this copy was installed (only the AppImage and the Mac app replace themselves), failure kinds, one install at a time; `lib.rs` wraps `tauri-plugin-updater` |
| `src-tauri/src/downloads.rs` | Copies of opened SFTP/Drive/share files and built Arbesk assets in the app cache: marks them as downloaded, prunes them after a week (edited copies go to the Trash) |
| `src-tauri/src/recent.rs` | Recent: Spotlight (`mdfind`) on macOS, `recently-used.xbel` on Linux, merged with `recent.json` |
| `src-tauri/src/config_store.rs` | `pins.json` / `starred.json` / `flags.json` in the app config dir, Full Disk Access check |
| `src-tauri/src/versions/` | Keep Versions: `.versions/` ownership, SQLite index, content-addressed zstd objects, capture/watch/restore |
| `src-tauri/src/lib.rs` | Command registration, commands that need `AppHandle`, startup |
| `dev/mock-tauri/` | In-memory fake backend for browser previews and the frontend tests |
| `tests/` | Vitest frontend tests: the real app in jsdom against `dev/mock-tauri/` (`tests/app.ts` boots and drives it) |
| `scripts/crap-gate.mjs` | CRAP gate run by `.githooks/pre-commit` |
| `scripts/check-lockfile.mjs` | Refuses a `package-lock.json` that could install anything but the public npm registry's own tarballs; run by the Release workflow before `npm ci` |
| `.github/dependabot.yml` | Weekly update PRs for npm, both Cargo projects and the actions, with a 7-day cooldown (30 for majors); each one is reviewed like any PR |
| `scripts/session-start.sh` | Where the project stands, from fresh data: main vs GitHub, worktrees, merged branches, drafts, issues |
| `scripts/github.sh` | The scripts' GitHub queries, all through the REST API (`gh api`): cloud sessions can't reach GraphQL |
| `scripts/new-task.sh`, `scripts/end-task.sh` | A worktree per task in `.worktrees/<name>`; finalized (worktree and branches deleted) once its PR is merged |
| `scripts/mock-screenshot.mjs`, `scripts/pr-screenshots.sh` | PR screenshots: capture the mock preview, upload to the `captures` branch |
| `scripts/sign-updater.sh` | Signs the updater packages with minisign in the Release workflow's `sign` job, the only job that gets the key; checked as the updater checks it by `src-tauri/tests/release_signing.rs` |
| `scripts/check-macos-libraries.sh` | Refuses a macOS build that loads a library from outside macOS (0.7.5 loaded the runner's Homebrew liblzma and couldn't open); run by the Release workflow after the macOS build |
| `.github/pull_request_template.md` | The PR body every task follows |
| `HUMANS.md` | The human's side of the workflow: review, merge, test, release |
| `deny.toml` | cargo-deny for both Rust projects: RustSec advisories, licences compatible with GPL-3.0-or-later, crates.io only; run by the Release workflow's `rust-deps` job |

## Commands

```sh
npm ci                      # exactly package-lock.json; see "npm registry" below
npm run tauri dev           # run the app
npm run tauri build         # macOS: bundle/macos/*.app; Linux: bundle/{deb,rpm,appimage}
npx tsc --noEmit            # type-check the frontend
npm run build               # tsc + vite build
npm run dev:mock            # UI in a normal browser with a fake filesystem (port 1420)
cd src-tauri && cargo test  # backend tests
npm test                    # frontend tests (Vitest + jsdom)
npx vitest run tests/<file>.test.ts  # only the frontend tests a change touches
npx vitest                  # the same, in watch mode
npm run crap                # CRAP gate (also runs as the pre-commit hook)
npm run crap:ts             # CRAP gate for TypeScript only: Vitest coverage + fallow
npm run crap:baseline       # accept the current scores into .crap-baseline.json
```

Before finishing a change: `npx tsc --noEmit` and `npm run build` must pass,
plus `npm test` for a frontend change and `cargo test` for a backend one (the
pre-commit hook runs the full suite of each language a commit touches). For UI
changes, also look at the result in `npm run dev:mock`.

### TypeScript-only changes

While a change touches only `src/`, `tests/`, `dev/` or `scripts/`, keep the
loop fast:

- type-check with `npx tsc --noEmit`;
- run the test files it touches, not the whole suite:
  `npx vitest run tests/drive.test.ts tests/listing-failure-table.test.ts`
  (drop `run` to watch them);
- score the gate for TypeScript only with `npm run crap:ts`. That still runs
  every frontend test with coverage, because fallow needs the coverage, but it
  skips the Rust build, tests and coverage; `npm run crap` does both halves.
  The pre-commit hook already picks the languages from the staged files
  (`--staged`), so a TypeScript-only commit never runs Rust.

`cargo test` is not needed when no file under `src-tauri/` changed. The
pre-commit hook runs the whole frontend suite with coverage on every
TypeScript commit, so the PR has that result even though the loop used only
the related files.

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

The hook's `--staged` picks the languages itself. To score one language by
hand, `npm run crap:ts` and `node scripts/crap-gate.mjs --rust` skip the
other half.

**Frontend tests** boot the real app (`tests/app.ts`) in jsdom with
`dev/mock-tauri/` as the backend; each test file gets a fresh app and fake
filesystem. `tests/setup.ts` stubs what jsdom lacks (layout sizes,
`ResizeObserver`, `elementFromPoint`, the clipboard, a Mac user agent).
Drive the UI with events (`key`, `mouse`, `goTo`, `respond`) and assert on
the DOM. Code that adds a mock command needs a case in
`tests/mock-core.test.ts`.

**Rust dependencies** are checked against `deny.toml` before every release
(the `rust-deps` job), and a new advisory stops it. When you add or update a
crate, run cargo-deny 0.20.2 (`cargo install --locked cargo-deny@0.20.2`) on
the project you changed:
`cargo deny --locked --manifest-path src-tauri/Cargo.toml --config deny.toml check`.
Fix a finding by updating the crate; ignore an advisory only with the reason
it doesn't apply here, in `deny.toml`, and say so in the PR. A licence that
isn't on the allow-list needs the human's decision (the app is
GPL-3.0-or-later).

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
  `placement.rs`, `config_store.rs`, `platform.rs`, `properties.rs`, `thumbnails.rs`, `recent.rs`,
  `trash_ops.rs`, `downloads.rs`, `file_icons.rs` and `versions/` into a scratch crate depending on `serde`,
  `serde_json`, `trash`, `image` (same features as `src-tauri/Cargo.toml`),
  `rusqlite` (`default-features = false`, features `bundled` + `backup`), `zstd`,
  `blake3`, `notify` and `similar` (and `libc` on Linux and macOS, `gio` on
  Linux, `objc2`, `objc2-foundation`, `objc2-app-kit` and `xattr` on macOS,
  with the features `src-tauri/Cargo.toml` lists), plus dev-dependency
  `tempfile`, strip the `#[tauri::command]` lines, and run `cargo test`.
  Keep new backend logic in plain functions; only `lib.rs` may use
  `AppHandle` and other Tauri types.
- **macOS-only behaviour can't be verified from Linux**: window chrome, traffic
  lights, Quick Look thumbnails, Spotlight results in Recent, where Finder puts
  trashed items and its `.DS_Store` "Put Back" records, Full Disk Access,
  opening files. Say so explicitly when reporting work that touches them. The
  mirror image also holds: Linux-side behaviour (`.trashinfo`, `recently-used.xbel`,
  udisks2 mount paths) is unverified on a Mac.
- **npm registry and the lockfile**: `package-lock.json` is committed and must
  resolve only to `https://registry.npmjs.org/` (an earlier lockfile pinned
  every package to a private corporate registry). The Release workflow runs
  `scripts/check-lockfile.mjs`, which refuses any other host, an entry whose
  tarball isn't its own package's, a missing sha512, or an `.npmrc` that sets
  another registry or credentials, then installs with
  `npm ci --ignore-scripts`. Never add registry URLs or `.npmrc` entries
  pointing at private registries.
  - Install with `npm ci` in the main checkout. **Never run `npm ci` or
    `npm install` inside a worktree**: its `node_modules` is a link to the
    main checkout's, and npm would wipe or rewrite that.
  - Change dependencies in a task (or take Dependabot's PR) and regenerate the
    lock in a scratch copy of the tree, against the public registry and only
    with versions at least a week old:
    `npx -y npm@11.21.1 install --package-lock-only --ignore-scripts --registry=https://registry.npmjs.org/ --min-release-age=7`.
    Copy the lock back, run `node scripts/check-lockfile.mjs`, and check the
    diff touches only what you meant to change.
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
  Commands ask `trash_ops::Trash` (`send`, `untrash`, `record`, `info`,
  `restore`, `delete_permanently`, `empty`, `listing_root`, `contains`)
  rather than reading `trash_log.json` or building Trash paths in `lib.rs`.
  Anything the app trashes outside Move to Trash (Replace's old item, a
  Drive move's source, a DMG install's old app, `.versions/`, an edited
  download, a replaced version) goes through `Trash::send` (`send_to_trash`
  or `AppTrash` in `lib.rs`; modules take it as a parameter), which logs
  where it came from. Only `trash_ops.rs` calls the `trash` crate; a test
  fails on any other caller.
- Replacing an existing item stages the new one under a temporary name, sends
  the old one to the Trash, then renames into place. Local copy, move and
  Drive downloads do this only through `placement.rs` (`Target::place`);
  don't build the sequence again at a call site.
- Placing renames with `placement::rename_new`, never `std::fs::rename`:
  on Unix that replaces a file that took the name after it was checked free.
  Rename and Batch Rename use `placement::rename_in_place`, which also
  changes only the case on a disk that ignores it. A test in `placement.rs`
  lists the modules that may call `std::fs::rename` (the app's own saves
  and caches).
- A new file a copy makes is staged too and appears only whole; a new
  folder fills in place (`Stage::by_kind`) and counts as a partial copy to
  clean up only once the fill has created it, so a failure never removes
  what took the name after the check.
- A move that replaces nothing is one `rename_new` straight onto the target
  (`Target::free_path`): the user's own item never waits under a staging
  name, where a crash would leave it hidden. Only its copy across volumes
  is staged, like any copy.
- If Replace's last rename fails once the old item is in the Trash, the old
  item comes back out (`placement::ReplaceTrash::take_back`; the app's is
  `Trash::untrash`, which also forgets its log entry; as Drive untrashes)
  unless something took its place; then it stays in the Trash, logged, and
  the error says where. Histories the Trash step closed come back with it:
  a Drive download's Replace goes through `versions::state::VersionedTrash`,
  which reopens and registers them again.
- A cross-volume move deletes the source only after the copy fully succeeded.
- Copy/move refuse: vanished source, same path, destination inside the source,
  existing target without `overwrite`, replacing a folder with its own child,
  a target whose last component is empty, `.` or `..`. Their wording lives in
  `placement::Refusal`, and the mock's `checkTransfer` makes the same ones in
  the same order. Add a test for every new refusal.
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
  `ui.ts` rather than new ad-hoc overlays. A dialog whose response waits on
  the backend uses `openDialog`'s `busy` option (return the promise from
  `onResponse`, or `dialog.run()` for work started elsewhere) rather than
  its own busy flag: it ignores a second Enter, disables the rest except
  `liveWhileBusy`, and puts back what validation wants.
- Shortcuts accept ⌘ on macOS and Ctrl elsewhere (`isMac ? metaKey : ctrlKey`).

**Security.** Escape every file name or path inserted with `innerHTML` via
`escapeHtml`. Icon markup comes from fixed templates only.

## Conventions

- Match the surrounding code: section banners (`// ----`), short doc comments
  on non-obvious functions, no framework, no new runtime dependencies without a
  good reason.
- Persist small UI preferences in `prefs` (localStorage, wrapped in try/catch);
  persist durable settings through `config_store.rs`.
- Ask `src/locations.ts` (or `isRemotePath`) what a location allows rather
  than combining `isCloud`, `isSharePath`, `isSftpPath` and the Trash checks
  at the call site; a new kind of location or rule is a row in
  `tests/locations.test.ts`. Code that picks something per kind of place (a
  menu, a rule) is a `Record<LocationKind, …>`, so a new kind can't be
  forgotten.
- An action that menus, shortcuts, the preview, the row star or a drag can
  all reach asks `src/item-actions.ts` inside the action function itself
  (`refuse(action, hereFor(paths))` in `main.ts`), not at each surface; a
  changed rule is a cell in its table and in `tests/item-actions.test.ts`.
  Menus keep their own per-place layouts and may leave an allowed action
  out. `tests/item-actions-matrix.test.ts` pins what every place offers
  through every surface: update it when that changes on purpose.
- Ask `src/paths.ts` how a path breaks up (`pathName`, `parentPath`,
  `childPath`, `pathSegments`) and what holds it, rather than slicing a
  scheme's prefix or writing `p === dir || p.startsWith(dir + "/")` at the
  call site; a new scheme is a row in its `SCHEMES` table and in
  `tests/paths.test.ts`. Containment has two helpers that differ only when
  `dir` is `/` or ends in a slash, so don't swap one for the other:
  `isUnder` ignores a trailing slash on `dir`, and `/` holds everything (a
  versioned folder, a folder that received items); `isWithin` takes `dir`
  as written, so `/` holds only itself (Home, which is `/` when HOME=/,
  the Trash, volumes).
  Never rewrite a path the backend produced: stars, pins and history store
  them as they are, and only `src/path-refs.ts` changes them, to follow a move.
  The one exception is Home: `homeFolder` trims `$HOME`'s trailing slash once
  at start-up, keeping `/` for HOME=/ (#170).
- Code that moves, renames or trashes items reports it to `src/path-refs.ts`
  (`pathRefs.moved(pairs)` with the paths the backend reported, as soon as
  it moved them and only for what really moved; `pathRefs.gone(paths)` after a Move
  to Trash) instead of updating stars, bookmarks, the clipboard or tabs by
  hand. Something new that remembers paths is a holder there, with its rule
  tested in `tests/path-refs.test.ts`.
- In `lib.rs`, ask `place.rs` which kind a path is and whether it may be
  written, rather than chaining `is_cloud`, `is_share` and `is_sftp`: a
  command that works on this computer only takes its path from
  `place::local` (or `local_write`), a write command checks
  `place::writable` first (copy, move and rename: `place::transfer`), and a
  batch with nothing to say about remote paths answers through
  `place::local_only`; give a new command the gate too. Keep each
  command's local arm a single call into its module; a new kind or refusal
  is a row in `place.rs`'s table test.
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
  In a cloud session, where `gh pr …` (GraphQL) fails, use REST:
  `gh api 'repos/{owner}/{repo}/pulls' -f base=main -f head=<branch> -f title="…" -F body=@<file>`.
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
  (or `gh api 'repos/{owner}/{repo}/issues/<n>/comments' -F body=@<file>`)
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
   `package-lock.json` (its two root entries), `src-tauri/Cargo.toml`,
   `src-tauri/Cargo.lock` (the `filebrowser` package only),
   `src-tauri/tauri.conf.json` and `docs/linux-appimage.md`, and open the PR
   `chore: release X.Y.Z` listing what's new. The human merges it.
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
with a new key). Only the Release workflow's `sign` job may use the secrets:
never pass them to a job or step that installs or builds anything (npm
packages and crates' build scripts run there; one wrote the whole
environment into `target/`, RUSTSEC-2026-0418). Agents never approve the
`release` environment's deployment, through the web or the API, even though
they use the human's account: the human approves each release.

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
