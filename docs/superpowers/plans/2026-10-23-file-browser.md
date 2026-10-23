# File Browser Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A standalone macOS desktop app, modeled on GNOME Nautilus, for browsing and managing local files.

**Architecture:** Tauri 2.x app. A Rust backend exposes `#[tauri::command]` functions for all filesystem work (listing, copy/move/rename/trash/mkdir, conflict detection) using `std::fs` + the `trash` crate; the opener plugin opens files in their default app. A plain TypeScript + Vite frontend renders a Nautilus-style sidebar + main pane, with virtualized rendering for large folders. All FS mutation lives in Rust so the frontend never touches the disk directly.

**Tech Stack:** Tauri 2.x, Rust (cargo 1.90), `trash` crate, `@tauri-apps/plugin-opener`, TypeScript, Vite, plain HTML/CSS (no UI framework).

**Spec:** `docs/superpowers/specs/2026-10-23-file-browser-design.md`

## Global Constraints

- Target: macOS (Apple Silicon), unsigned/ad-hoc local build — no Developer ID or notarization.
- File access relies on a one-time **Full Disk Access** grant; FS access uses Rust `std::fs`, not the scoped-FS plugin.
- Delete MUST go to macOS Trash (recoverable) via the `trash` crate — never a hard `rm`.
- Paste conflicts MUST prompt the user (Replace / Skip / Keep both) — never silently overwrite.
- Copy/move are recursive over directory trees; move within a volume uses `rename`, across volumes falls back to copy-then-delete.
- No tabs, split panes, disk-wide search, thumbnails, network drives, or archive handling in v1.
- All filesystem paths are absolute. Commands return a `Result`; errors carry a human-readable message string.

## Review Focus

Inputs the spec implies but no happy-path test exercises, most likely to bite first:

1. **Permission-denied on a folder** (e.g. a system dir without access) — listing must surface a clear error, not crash or hang. → Task 2.
2. **Broken symlinks / entries whose metadata can't be read** — listing must skip or mark them, not abort the whole listing. → Task 2.
3. **Paste where source and destination are the same path, or dest is inside source** (recursive copy into itself) — must refuse, not loop infinitely. → Task 5.
4. **Rename/new-folder to a name that already exists** — must error clearly, not clobber. → Task 4.
5. **Deleting/operating on a path that vanished between listing and action** — must return a clean error, not panic. → Task 5.

---

## Task 1: Scaffold the Tauri app and verify it launches

**Files:**
- Create: whole Tauri project in repo root (`src-tauri/`, `src/`, `package.json`, `vite.config.ts`, `index.html`).
- Create: `src-tauri/entitlements.plist` note is deferred to Task 8.

**Interfaces:**
- Produces: a buildable Tauri app; a Rust `lib.rs` with an empty command registry the later tasks extend; a TS `src/main.ts` entry point.

- [ ] **Step 1:** Scaffold with `npm create tauri-app@latest` — name `filebrowser`, frontend `TypeScript`, package manager `npm`, template `Vanilla`, flavor `TypeScript`. Run it so files land in the repo root (scaffold into a temp subdir then move if the tool refuses a non-empty dir — the `docs/` dir is already present).
- [ ] **Step 2:** `npm install`, then add the opener plugin: `npm run tauri add opener`.
- [ ] **Step 3:** Add the `trash` crate to `src-tauri/Cargo.toml` dependencies.
- [ ] **Step 4:** Run `npm run tauri dev` and confirm the default window opens, then close it.
- [ ] **Step 5:** Commit.

```bash
git init && git add -A && git commit -m "chore: scaffold Tauri app with opener plugin and trash crate"
```

---

## Task 2: Directory listing command (Rust)

**Files:**
- Create: `src-tauri/src/fs_ops.rs` (filesystem command module).
- Modify: `src-tauri/src/lib.rs` (register command, declare module).
- Test: `src-tauri/src/fs_ops.rs` `#[cfg(test)]` module.

**Interfaces:**
- Produces:
  - `struct Entry { name: String, path: String, is_dir: bool, size: u64, modified: i64 (unix secs), is_hidden: bool, is_symlink: bool }` (Serde `Serialize`).
  - `#[tauri::command] fn list_dir(path: String, show_hidden: bool) -> Result<Vec<Entry>, String>`
  - A testable inner `fn read_entries(path: &Path, show_hidden: bool) -> Result<Vec<Entry>, String>` that the command wraps.

- [ ] **Step 1: Write failing tests** in `fs_ops.rs` against `read_entries`, using a `tempfile`-created dir:
  - lists a known file and subdir with correct `name`, `is_dir`, `size`;
  - a dotfile is excluded when `show_hidden=false` and included when `true`, with `is_hidden=true`;
  - a nonexistent path returns `Err`;
  - a broken symlink is returned (not skipped) with `is_symlink=true` and does not abort the listing.
- [ ] **Step 2:** Run `cargo test` in `src-tauri`; expect FAIL (function not defined). Add `tempfile` as a dev-dependency.
- [ ] **Step 3:** Implement `read_entries` (use `std::fs::read_dir`; per entry use `symlink_metadata` so broken symlinks don't error the whole call; `is_hidden` = name starts with `.`; `modified` from `metadata.modified()` → unix secs, defaulting to 0 if unavailable) and the thin `list_dir` command. Register `list_dir` in `lib.rs`.
- [ ] **Step 4:** Run `cargo test`; expect PASS.
- [ ] **Step 5:** Commit.

---

## Task 3: Frontend shell — sidebar, main pane, listing render

**Files:**
- Modify: `src/main.ts`, `index.html`, `src/styles.css` (Nautilus-style layout).
- Create: `src/api.ts` (typed wrappers around `invoke`).

**Interfaces:**
- Consumes: `list_dir` from Task 2.
- Produces:
  - `src/api.ts`: `type Entry = {...}` mirroring the Rust struct; `listDir(path: string, showHidden: boolean): Promise<Entry[]>`.
  - A module-level `currentPath` state and a `render(path)` function later tasks call.

- [ ] **Step 1:** Build `index.html` with a `.sidebar` (Home, Desktop, Documents, Downloads links, resolved from the home dir) and a `.main` pane containing a toolbar placeholder and a `.listing` container.
- [ ] **Step 2:** In `api.ts`, define `Entry` and `listDir` wrapping `invoke('list_dir', ...)`.
- [ ] **Step 3:** In `main.ts`, on load render the home directory: call `listDir`, create one row per entry (icon by `is_dir`/type, name, and in list mode size + modified). Clicking a sidebar place calls `render(thatPath)`. Double-clicking a dir row calls `render(entry.path)`.
- [ ] **Step 4:** Style to approximate Adwaita (light, flat, sidebar). Run `npm run tauri dev`; confirm the home folder lists and sidebar navigation works. (Manual visual check — no automated test for layout.)
- [ ] **Step 5:** Commit.

---

## Task 4: Navigation, view toggle, hidden-files toggle, new folder, rename

**Files:**
- Modify: `src/main.ts`, `index.html`, `src/styles.css`, `src/api.ts`.
- Modify: `src-tauri/src/fs_ops.rs`, `src-tauri/src/lib.rs`.
- Test: `fs_ops.rs` test module.

**Interfaces:**
- Consumes: Task 2/3.
- Produces:
  - Rust: `#[tauri::command] fn make_dir(path: String) -> Result<(), String>` (errors if exists), `#[tauri::command] fn rename_entry(from: String, to: String) -> Result<(), String>` (errors if `to` exists).
  - Frontend: history stack (`back`/`forward`/`up`), breadcrumb from `currentPath`, a path-entry input, icon/list view toggle, hidden-files toggle wired to `listDir`'s `showHidden`.

- [ ] **Step 1: Write failing Rust tests:** `make_dir` creates a dir and errors when it already exists; `rename_entry` renames and errors when the target name already exists.
- [ ] **Step 2:** `cargo test` → FAIL.
- [ ] **Step 3:** Implement `make_dir` (error if `path.exists()`) and `rename_entry` (error if `to` exists, else `fs::rename`); register both.
- [ ] **Step 4:** `cargo test` → PASS.
- [ ] **Step 5:** Frontend: add back/forward/up (history stack), clickable breadcrumb, path input (Enter jumps), view-mode toggle (icon/list), hidden toggle (Cmd+Shift+.), new-folder button → `make_dir` then re-render, rename (F2/Enter on selection) → `rename_entry` then re-render. Surface errors in a small banner/dialog.
- [ ] **Step 6:** `npm run tauri dev`; confirm each control works. Commit.

---

## Task 5: Copy / move / trash commands with conflict detection (Rust)

**Files:**
- Modify: `src-tauri/src/fs_ops.rs`, `src-tauri/src/lib.rs`.
- Test: `fs_ops.rs` test module.

**Interfaces:**
- Produces:
  - `#[tauri::command] fn path_exists(path: String) -> bool`
  - `#[tauri::command] fn copy_entry(from: String, to: String, overwrite: bool) -> Result<(), String>` — recursive for dirs; refuses when `to` exists and `!overwrite`; refuses when `from == to` or `to` is inside `from`.
  - `#[tauri::command] fn move_entry(from: String, to: String, overwrite: bool) -> Result<(), String>` — `rename` within volume, copy-then-delete across volumes; same refusals as copy.
  - `#[tauri::command] fn trash_entry(path: String) -> Result<(), String>` — uses the `trash` crate.
  - testable inner helpers `fn copy_recursive`, `fn is_subpath(child, ancestor) -> bool`.

- [ ] **Step 1: Write failing tests:** copy a file; copy a dir tree recursively; copy refuses when dest exists and `overwrite=false` but succeeds when `true`; `is_subpath` true when dest nested in source, and `copy_entry` refuses that case and the `from==to` case; `move_entry` moves a file and removes the source; `trash_entry` removes a temp file from its location (assert it no longer exists at the path — do not assert Trash internals); operating on a vanished path returns `Err`.
- [ ] **Step 2:** `cargo test` → FAIL.
- [ ] **Step 3:** Implement the commands and helpers; register all. Guard `is_subpath`/equality before any write.
- [ ] **Step 4:** `cargo test` → PASS.
- [ ] **Step 5:** Commit.

---

## Task 6: Clipboard UX — copy/cut/paste with conflict dialog (frontend)

**Files:**
- Modify: `src/main.ts`, `src/api.ts`, `index.html`, `src/styles.css`.

**Interfaces:**
- Consumes: Task 5 commands.
- Produces: a frontend clipboard model `{ paths: string[], mode: 'copy' | 'cut' }`; a conflict dialog returning `'replace' | 'skip' | 'keepboth'`.

- [ ] **Step 1:** Add `copyEntry`/`moveEntry`/`trashEntry`/`pathExists` wrappers to `api.ts`.
- [ ] **Step 2:** Implement selection + Cmd+C (copy) / Cmd+X (cut, with cut-highlight) / Esc (cancel cut).
- [ ] **Step 3:** Implement paste: for each clipboard path compute dest in `currentPath`; if `pathExists(dest)`, show the conflict dialog (Replace / Skip / Keep both). Replace → call with `overwrite=true`; Keep both → append ` (copy)`/` (copy N)` to the name; Skip → do nothing. Cut mode calls `moveEntry` and clears the clipboard only after all paths succeed; copy mode calls `copyEntry`. Re-render after.
- [ ] **Step 4:** Wire Cmd+Delete → `trashEntry` (with a confirm). Add a right-click context menu exposing Copy/Cut/Paste/Rename/Move to Trash/New Folder.
- [ ] **Step 5:** `npm run tauri dev`; verify copy, cut-move, paste-with-conflict for all three choices, and trash. Commit.

---

## Task 7: Open files in default app + pinned sidebar folders

**Files:**
- Modify: `src/main.ts`, `src/api.ts`, `src-tauri/src/lib.rs` (opener already added in Task 1).
- Create: pinned-folders persistence — read/write a JSON file in the app config dir via a Rust command.

**Interfaces:**
- Produces:
  - Rust: `#[tauri::command] fn load_pins() -> Vec<String>` and `#[tauri::command] fn save_pins(paths: Vec<String>) -> Result<(), String>` reading/writing `pins.json` in the Tauri app-config dir.
  - Frontend: double-click a *file* row → opener `open(path)`; a "Pin to sidebar" context action; the sidebar renders standard places plus loaded pins.

- [ ] **Step 1:** Implement `load_pins`/`save_pins` (JSON file in `app_config_dir`; missing file → empty list). Register them.
- [ ] **Step 2:** Frontend: double-clicking a file (not dir) calls the opener plugin's `open(entry.path)`.
- [ ] **Step 3:** Add "Pin to sidebar" / "Remove pin" context actions that update via `save_pins` and re-render the sidebar; load pins on startup via `load_pins`.
- [ ] **Step 4:** `npm run tauri dev`; confirm opening a file launches its default app, and pins persist across a restart. Commit.

---

## Task 8: Full Disk Access onboarding + production build

**Files:**
- Modify: `src/main.ts`, `index.html`, `src/styles.css` (onboarding note).
- Modify: `src-tauri/tauri.conf.json` (bundle/identifier/window title).

**Interfaces:**
- Consumes: everything prior.

- [ ] **Step 1:** Add a first-run note (dismissible, persisted-dismissed flag in `pins.json` or a sibling) explaining Full Disk Access, with a button that opens `x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles` via the opener.
- [ ] **Step 2:** Set app `identifier`, product name "File Browser", and default window size/title in `tauri.conf.json`.
- [ ] **Step 3:** Run `npm run tauri build`; confirm a `.app` is produced under `src-tauri/target/release/bundle/macos/`.
- [ ] **Step 4:** Launch the built `.app`, grant Full Disk Access once, and smoke-test: browse, open a file, copy/paste with a conflict, trash, pin. Commit.

---

## Self-Review Notes

- **Spec coverage:** layout/sidebar (T3,T7), icon+list views (T3,T4), navigation/breadcrumb/path-jump (T4), copy/cut/paste/rename/new-folder/trash (T4,T5,T6), open-in-default (T7), hidden toggle (T4), Full Disk Access onboarding (T8), paste-conflict prompt (T6), recursive/cross-volume copy-move (T5), virtualization — see note below.
- **Virtualization:** the spec's performance anti-goal is addressed by rendering rows on demand. Folded into T3's render function as the builder's implementation choice; if a measured 10k-entry folder stutters during T3's manual check, add windowed rendering before moving on.
- **Review Focus:** all five mapped to tasks 2/4/5 with explicit tests.
