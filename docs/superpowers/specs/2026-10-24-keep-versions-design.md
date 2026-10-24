# Keep Versions — Design

**Date:** 2026-10-24
**Status:** Draft, pending approval
**Related:** `2026-10-23-file-browser-design.md` (the app's overall design),
`AGENTS.md` (rules every change must keep)

## Intent and the why

The user wants some folders to keep every saved version of the files inside
them, so any earlier state of a file can be brought back. The idea comes from
the user's own project Arbesk (github.com/ahmadsayed/arbesk): content is
stored once under its hash, versions are immutable and chained to the one
before, and history can be browsed in time. **This feature is local only.**
No blockchain, no IPFS, no network, no sync.

A user opts in per folder when creating it: the New Folder dialog gets a
**Keep versions** checkbox marked *Experimental*. Only such folders are
tracked; the rest of the browser behaves exactly as today.

Success looks like:

- A file edited in a versioned folder, by any app, can be brought back to any
  earlier saved state captured while the file browser was running, and to
  its last state before launch.
- History costs far less disk than full copies, because each version stores
  only its changes (zstd deltas).
- Nothing about this slows down browsing other folders, and nothing in it can
  destroy a user's file.

GNOME Files has no version history, so this is the project's own feature.
Wording and widgets still follow libadwaita and GNOME conventions. Finder's
"Browse All Versions" is the closest prior art.

## Decisions

### Made by the user (2026-10-24)

1. **Opt-in at creation:** a "Keep versions" checkbox (labelled Experimental)
   in the New Folder dialog, below the name field.
2. **History lives inside the folder** in a hidden `.versions/` folder, so it
   follows the folder through renames, moves and copies.
3. **Capture while the app runs, plus a catch-up scan at launch.** No
   background helper. Saves made and overwritten while the app is closed are
   not kept; the launch scan records the state the app finds.
4. **v1 stores zstd deltas for every file type**, with a full copy every N
   versions. Splitting container formats (3MF, DOCX, ODT, glTF) into parts is
   a later spec.
5. **UI:** a **Versions…** dialog on a file (list, text diff preview, Restore
   Copy, Replace…) and a **Deleted Files…** dialog for a folder.
6. **Retention: forever.** Nothing is deleted automatically. Properties shows
   the history's size, and the user can clear it explicitly.
7. **Approach A:** our own store: SQLite index + zstd + file watcher, rather
   than an embedded git repository or JSON records.
8. **Compression: zstd**, not Brotli (Arbesk's choice), because zstd has
   first-class "patch from a reference" deltas and compresses much faster.

### Made by the designer (review these)

| # | Decision | Why |
|---|---|---|
| D1 | Hash: **BLAKE3** of the raw (uncompressed) content, hex-encoded. | Fast; hashing raw content keeps identity independent of how it is stored (Arbesk's `dedup.ts` does the same). |
| D2 | A versioned folder tracks **all subfolders recursively**. Creating a versioned folder inside one is not offered (checkbox disabled). A subtree that already has its own `.versions/` (moved in from elsewhere) is skipped by the outer folder. | One owner per file, no double tracking. |
| D3 | Only **regular files** are versioned. Symlinks, folders, sockets and the like are ignored. Empty folders leave no trace in history. | Keeps v1 small; folders are recreated on restore as needed. |
| D4 | History follows the **path**. A rename is recorded as a deletion of the old path plus a new file. | Rename detection is guesswork; can be added later without changing stored data. |
| D5 | Files over **1 GiB** are not versioned. Files over **128 MiB** are stored as full copies, never deltas. | Deltas need the base and the new file in memory; caps keep memory bounded. |
| D6 | Delta chains: a full copy when the chain would reach depth **20**, or when the delta is ≥ 90 % of the raw size. | Restoring never replays more than 19 deltas; whole-file rewrites don't pay delta cost. |
| D7 | **Clear History…** and **Stop Keeping Versions…** move `.versions/` to the **Trash**, not delete it. | "Never lose data" (AGENTS.md): deleting goes to the Trash. |
| D8 | **Replace…** first records the current file as a version, then follows the existing replace rule (stage, Trash the old, rename into place). | The current state is never lost, even if the watcher hadn't caught it yet. |
| D9 | The Versions… diff compares the **selected version with the current file**. | The question the user is answering is "should I restore this?". |
| D10 | `.versions` is **always hidden**: in listings, item counts, search and Recent, even with Show Hidden Files on. Folder size in Properties includes it, and a separate row shows the history's size. | It is an implementation detail; disk usage should still be honest. |
| D11 | Turning versions on for an **existing** folder is out of scope for v1. | User asked for the New Folder entry point; adding it to Properties later is easy. |
| D12 | Editor temp files are ignored (list below). | They would flood history with noise. |
| D13 | If creating `.versions/` fails after the folder was created, the folder is **kept** and an error says versions could not be turned on. | Never delete; an empty folder is harmless. |

## Non-goals (v1)

- Background capture while the app is closed.
- Splitting container formats (zip-based, glTF) into parts.
- The "folder at time T" slider (Arbesk scene clock).
- Pruning or thinning old versions; size limits.
- Turning versions on for existing folders.
- Rename/move detection inside a versioned folder.
- Versioning symlinks, folder metadata, permissions or extended attributes.
- Any network, sync, IPFS or blockchain feature.
- Image or document previews of old versions (text diff only).

## Architecture

Approach A. Backend logic lives in plain functions, testable without Tauri
(AGENTS.md); only `lib.rs` touches Tauri types.

### Backend: `src-tauri/src/versions/`

| File | Responsibility | Depends on |
|---|---|---|
| `mod.rs` | Public API used by `lib.rs`; constants (limits, ignore list); the folder registry (`versioned.json`) helpers. | the files below, `config_store` |
| `store.rs` | Object store in `.versions/objects/`: write a full or delta object atomically, read one back (following its chain), verify the BLAKE3 hash. Knows nothing about paths or versions. | `zstd`, `blake3` |
| `index.rs` | `.versions/index.sqlite`: open/create, schema and migrations, all queries. | `rusqlite` |
| `capture.rs` | Record one path (new version, unchanged, or deletion); the launch scan of a whole versioned folder. | `store`, `index` |
| `watch.rs` | One `notify` watcher per versioned folder; coalesces events per path and fires `capture` after a quiet period. The coalescer is a pure struct with an injected clock so it can be unit-tested. | `notify`, `capture` |
| `restore.rs` | Rebuild a version to bytes; Restore Copy; Replace; list deleted files; text diff for the dialog. | `store`, `index`, `trash`, `similar` |

Add to `src-tauri/Cargo.toml` (and update `Cargo.lock` minimally, per AGENTS.md):

- `rusqlite` with feature `bundled` (SQLite compiled in; no system library needed)
- `zstd`
- `blake3`
- `notify` (FSEvents on macOS, inotify on Linux)
- `similar` (line diff)

All are MIT/Apache/BSD-style licensed and compatible with GPL-3.0-or-later;
SQLite is public domain. They are crates, not bundled assets, so no license
file is needed next to them, but add them to the README credits if the README
lists dependencies.

### Wiring in `lib.rs`

- A `VersionsState` in Tauri managed state: a map from versioned-folder root
  to its open `rusqlite::Connection` (behind a `Mutex`) and its watcher
  handle. Every command that touches a folder's index goes through it, so one
  process never opens two connections to the same index.
- **Startup (`setup`)**: read `versioned.json` from the app config dir. For
  each root that still exists and contains `.versions/index.sqlite`: open it,
  start its watcher, then run the launch scan on a background thread (one
  folder at a time, so launch isn't slowed). Drop registry entries whose
  folder or `.versions/` is gone.
- **Commands** (below). Anything that hashes, reads file contents or walks a
  tree is an `async fn` running through `blocking(…)`, per AGENTS.md.

### Frontend

- `src/versions.ts` (new): the Versions… dialog, the diff view, the Deleted
  Files… dialog, the Properties rows. Keeps `main.ts` from growing further.
- `src/main.ts`: the checkbox in `promptNewFolder`, menu entries, calling
  `versioned_root` on navigation, hiding `.versions`.
- `src/api.ts`: typed wrappers for the new commands.
- `dev/mock-tauri/core.ts`: mocks for every new command (AGENTS.md requires
  it), with a fake versioned folder `/Users/me/Documents/Versioned` holding a
  text file with several versions, a binary file, and one deleted file.
- `src/styles.css`: only libadwaita tokens; every colour needs a dark value.

## On-disk layout

```
<folder>/
└── .versions/
    ├── index.sqlite          (+ -wal / -shm while open)
    ├── objects/
    │   └── 3f/
    │       └── 3fa9…e1       one stored object, named by its BLAKE3 hex hash
    └── tmp/                  staging for atomic object writes
```

Object files are raw zstd frames (zstd's own magic number makes them
self-identifying). Whether a frame is a full copy or a delta, and against
which base, is recorded in the index; the hash in the file name is checked
against the decoded content on every read.

### Schema (`index.sqlite`, version 1)

```sql
PRAGMA journal_mode = WAL;
PRAGMA foreign_keys = ON;

CREATE TABLE meta (
  key   TEXT PRIMARY KEY,
  value TEXT NOT NULL
);
-- rows: schema_version = '1', created_at = unix seconds, app = 'unshitty-mac-filebrowser'

CREATE TABLE objects (
  hash        TEXT PRIMARY KEY,                 -- BLAKE3 hex of the raw content
  kind        TEXT NOT NULL CHECK (kind IN ('full', 'delta')),
  base        TEXT REFERENCES objects(hash),    -- NULL for 'full'
  depth       INTEGER NOT NULL,                 -- 0 for 'full', base.depth + 1 for 'delta'
  size        INTEGER NOT NULL,                 -- raw size in bytes
  stored_size INTEGER NOT NULL,                 -- size of the object file
  codec       TEXT NOT NULL DEFAULT 'zstd'      -- room for other codecs later
);

CREATE TABLE versions (
  id          INTEGER PRIMARY KEY,              -- grows with time; the version id
  path        TEXT NOT NULL,                    -- relative to the folder root, '/' separators, NFC
  hash        TEXT REFERENCES objects(hash),    -- NULL = the file was deleted (tombstone)
  size        INTEGER NOT NULL,                 -- 0 for tombstones
  mtime       INTEGER NOT NULL,                 -- file's modification time, unix nanoseconds
  recorded_at INTEGER NOT NULL                  -- when captured, unix seconds
);
CREATE INDEX versions_by_path ON versions(path, id);
```

The "previous version" of a row is the row with the same `path` and the
next-lower `id`; no explicit `prev` column is needed.

Open with `busy_timeout = 5000`. `index.rs` refuses to open an index whose
`schema_version` is newer than it knows ("This folder's history was made by
a newer version of the app."). Migrations run inside one transaction.

## Data flow

### Creating a versioned folder

1. User ticks **Keep versions** in New Folder and presses Create.
2. Frontend calls `make_versioned_dir(path)`.
3. Backend: `fs_ops::make_dir`, then create `.versions/`, `objects/`, `tmp/`,
   the index with schema v1; add the root to `versioned.json`; open the
   connection and start the watcher.
4. If step 3 fails after the folder exists, return an error and keep the
   folder (D13). Message: "The folder was created, but versions could not be
   turned on: <reason>".

### Capturing one path (`capture::record(root, rel_path)`)

1. Skip if `rel_path` is inside `.versions/`, inside a subtree with its own
   `.versions/` (D2), or matches the ignore list (D12).
2. `symlink_metadata`:
   - Missing → if the latest row for the path is not a tombstone, insert a
     tombstone (`hash = NULL`). Done.
   - Not a regular file (D3) → done.
   - Larger than 1 GiB (D5) → done (log at debug level).
3. If the latest row has the same `size` and `mtime` → done (unchanged).
4. Read the file. Stat again afterwards; if size or mtime changed during the
   read, the file is still being written: re-queue it in the watcher and stop.
5. Hash with BLAKE3. Same hash as the latest row → only the mtime changed:
   update that row's `mtime` (the one field allowed to change after insert,
   so step 3 doesn't re-read the file every time) and insert nothing. Hash already in `objects` (another file or an earlier
   state has this content) → insert a `versions` row only.
6. Otherwise write a new object:
   - **Delta** if the path's latest object exists, its `depth` is < 19, and
     both it and the new content are ≤ 128 MiB: rebuild the base's raw bytes
     (`store::read`), compress the new content with zstd using the base as a
     reference prefix (zstd's `ZSTD_CCtx_refPrefix`; check the exact method
     name in the `zstd` crate version used). Set the window log large enough
     to cover base + new, at most 27, and the decoder's `window_log_max` to
     match. Level 19 for files ≤ 1 MiB, level 6 above.
   - If the delta is ≥ 90 % of the raw size, or no base qualifies: **full**
     copy, zstd level 19 (≤ 1 MiB) or 6, with long-distance matching on for
     files over 8 MiB.
7. Write the object to `tmp/`, `fsync` it, rename into `objects/xx/<hash>`.
   Then insert the `objects` row and the `versions` row **in one
   transaction**. A crash can leave an orphan object file (harmless, ignored),
   never a row pointing at a missing object.

### Launch scan (`capture::scan(root)`)

Walk the folder (skipping `.versions/` and nested versioned subtrees, not
following symlinks). For each regular file, compare `size`/`mtime` with its
latest row and call `record` when they differ or there is no row. Then, for
each path whose latest row is not a tombstone and that no longer exists,
insert a tombstone. Runs in a background thread; never blocks the UI.

### Watching (`watch.rs`)

- `notify::RecommendedWatcher`, recursive, on the root.
- Every event's paths go into a coalescer: path → last-event time. A ticker
  every 500 ms fires `record` for each path quiet for **2 s**. This turns an
  editor's "write temp file, rename over original" into one capture.
- A folder-level event (rename/remove of a directory, or `notify` reporting
  overflow / `Rescan`) runs `scan` on that directory instead of per-file
  records.
- Watcher errors (e.g. Linux `max_user_watches` exhausted): log, and fall
  back to running `scan(root)` every 5 minutes for that folder.
- If the root itself disappears, stop its watcher and keep its registry entry
  until the next launch prunes it.

### Ignore list (D12)

Base names: `.DS_Store`, `Thumbs.db`, `desktop.ini`, `4913` (Vim's write
test). Patterns: `~$*` (Office lock), `.~lock.*#` (LibreOffice lock), `*.swp`,
`*.swo`, `*.swx`, `*~`, `*.tmp`, `.#*` (Emacs lock), `*.crdownload`,
`*.part`, `*.download`. Keep the list in one constant in `versions/mod.rs`.

### Rebuilding a version (`store::read(hash)`)

Follow `base` links down to a `full` object (≤ 19 steps), decompress it, then
apply each delta upwards (decompress with the previous result as the
reference prefix). Verify the BLAKE3 hash of the final bytes; on mismatch
return an error ("This version is damaged and can't be restored.") and never
write anything.

### Restoring

- **Restore Copy**: rebuild, then write to a new name next to the file:
  `<stem> (<YYYY-MM-DD HH-MM>)<.ext>` in local time, e.g.
  `notes (2026-10-24 14-05).txt`. No `:` (Finder shows it as `/`), and the
  names sort by date. If that name exists, append ` 2`, ` 3`, … before the
  extension. Never overwrites. Missing parent folders are created.
- **Replace…** (D8): confirmation dialog first (below). Then `record` the
  current file synchronously, write the rebuilt version to a temporary name in
  the same folder, move the current file to the Trash (`trash` crate), rename
  the temporary file into place. Same staging rule as paste-replace in
  `fs_ops`. If the current file no longer exists, this is a plain restore to
  the original path.
- **Deleted files**: `deleted_files(dir)` returns paths under `dir` whose
  latest row is a tombstone and that don't exist now. Restoring one rebuilds
  its last non-tombstone version to the original path; if something exists
  there by then, fall back to the Restore Copy naming.

The watcher will see the restored file and record it as a new version with
an existing hash, so restoring adds a history row but no new object.

### Clearing and stopping (D7)

Both act on the versioned root only:

- **Clear History…**: close the connection and stop the watcher, move
  `.versions/` to the Trash, create a fresh `.versions/`, run `scan` (the
  current state becomes version 1), restart the watcher.
- **Stop Keeping Versions…**: close, stop, move `.versions/` to the Trash,
  remove the root from `versioned.json`.

### Interplay with existing operations

- **Moving/renaming a versioned root inside the app** (`move_entry`,
  `rename_entry`): after success, update `versioned.json` (prefix replace for
  the root and any registered folders under it), reopen and restart the
  watcher at the new path.
- **Copying a versioned folder**: `.versions/` is copied with it; register the
  copy after the copy succeeds. Both now have independent histories.
- **Trashing a versioned root**: stop its watcher and remove it from the
  registry *before* trashing. Its history goes to the Trash with it.
  Restoring it from the Trash brings `.versions/` back; it is re-registered
  when next opened.
- **Navigation**: whenever the browser opens a folder, the frontend calls
  `versioned_root(path)` once. If an ancestor (or the folder itself) has
  `.versions/`, it is registered and watched if it wasn't (this re-finds
  folders moved outside the app). The result drives the menus below.
- **Listing / counts / search / Recent**: `.versions` is filtered out in
  `fs_ops::read_entries`, `count_items`, Starred/Recent collection and the
  search filter regardless of `show_hidden` (D10).

## Commands (`lib.rs`) and wrappers (`api.ts`)

All errors are user-facing strings in GNOME wording.

| Command | Signature (Rust) | Notes |
|---|---|---|
| `make_versioned_dir` | `async (path: String) -> Result<(), String>` | Creates folder + `.versions/`, registers, starts watcher. |
| `versioned_root` | `async (path: String) -> Option<String>` | Nearest ancestor-or-self with `.versions/`; registers it if new. |
| `list_versions` | `async (path: String) -> Result<Vec<VersionInfo>, String>` | Newest first. |
| `version_diff` | `async (path: String, version: i64) -> Result<VersionDiff, String>` | Selected version vs current file (D9). |
| `restore_version_copy` | `async (path: String, version: i64) -> Result<String, String>` | Returns the new file's path. |
| `replace_with_version` | `async (path: String, version: i64) -> Result<(), String>` | D8. |
| `deleted_files` | `async (dir: String) -> Result<Vec<DeletedFile>, String>` | Recursive under `dir`. |
| `restore_deleted` | `async (path: String) -> Result<String, String>` | Returns where it was restored. |
| `history_info` | `async (root: String) -> Result<HistoryInfo, String>` | For Properties. |
| `clear_history` | `async (root: String) -> Result<(), String>` | D7. |
| `stop_keeping_versions` | `async (root: String) -> Result<(), String>` | D7. |

```rust
#[derive(Serialize)]
pub struct VersionInfo {
    pub id: i64,
    pub size: u64,
    pub mtime: i64,        // unix seconds (for display)
    pub recorded_at: i64,  // unix seconds
    pub deleted: bool,     // tombstone row
    pub current: bool,     // same hash as the file on disk now
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum VersionDiff {
    Text { lines: Vec<DiffLine>, truncated: bool },
    Same,
    Binary,      // either side not text
    TooLarge,    // either side > 1 MiB
    Deleted,     // the selected row is a tombstone
}

#[derive(Serialize)]
pub struct DiffLine { pub tag: char /* ' ', '-', '+' */, pub text: String }

#[derive(Serialize)]
pub struct DeletedFile { pub path: String, pub deleted_at: i64, pub size: u64 }

#[derive(Serialize)]
pub struct HistoryInfo { pub bytes: u64, pub versions: u64, pub files: u64 }
```

"Text" means valid UTF-8 with no NUL byte in the first 8 KiB. The diff is a
unified line diff (`similar`, 3 lines of context), capped at 2,000 lines
(`truncated: true` beyond that).

## UI

All dialogs use `openDialog`, menus use `openMenu`, messages use
`showToast` (AGENTS.md). File names are inserted with `textContent` or
`escapeHtml`. Dates use `src/format.ts`.

### New Folder dialog

Below the Folder Name row, a `check-row` (the style the paste-conflict dialog
already uses):

```
┌ New Folder ─────────────────────────────┐
│ Folder Name  [ Projects               ] │
│                                         │
│ ☐ Keep versions   [Experimental]        │
│   Keeps every saved version of the      │
│   files in this folder.                 │
│                                         │
│              [Cancel]  [Create]         │
└─────────────────────────────────────────┘
```

- "Experimental" is a small pill using `--accent-bg`-derived tokens.
- Disabled, with the hint "Already inside a folder that keeps versions",
  when the current location is inside a versioned folder (D2).
- Unticked by default; not remembered between dialogs.
- Create calls `make_versioned_dir` when ticked, `makeDir` otherwise.

### Item context menu (file inside a versioned folder)

Add **Versions…** directly above **Properties**, enabled for a single
regular file only. Not shown outside versioned folders.

### Background context menu (inside a versioned folder)

Add **Deleted Files…**.

### Versions… dialog (`className: "versions-dialog"`, wider)

```
┌ Versions of “notes.txt” ──────────────────────┐
│ ● Today 14:05        2.1 kB   Current         │
│ ○ Today 11:40        2.0 kB                   │
│ ○ Yesterday 18:02    1.7 kB                   │
│ ○ 21 Oct 09:15       —        Deleted         │
│───────────────────────────────────────────────│
│  unchanged line                               │
│ -old line                                     │
│ +new line                                     │
│───────────────────────────────────────────────│
│                 [Restore Copy]  [Replace…]    │
└───────────────────────────────────────────────┘
```

- List newest first; selecting a row loads `version_diff` (debounced; show
  a spinner row while loading). Long lists are fine to render fully up to
  1,000 rows; beyond that, render the newest 1,000 and a "Showing the newest
  1,000 versions" note (versions of one file is not the 10,000-item folder
  case that requires virtualization).
- Diff area: monospace, `-` lines on a destructive-tinted background, `+`
  lines on a success-tinted background (tokens with dark-mode values). For
  `Same`: "This version is the same as the current file." `Binary`: "No
  preview for this type of file." `TooLarge`: "This file is too large to
  compare." `Deleted`: "The file was deleted at this point."
- Buttons are disabled for the Current row and for Deleted rows.
- **Restore Copy** → toast "Restored as “notes (2026-10-24 14-05).txt”"
  and select the new file in the view if it is in the current folder.
- **Replace…** → confirmation: heading "Replace “notes.txt”?", body "The
  current file will be moved to the Trash and replaced with the version from
  Yesterday 18:02. It stays in the history too.", responses Cancel (default)
  and Replace (destructive style).

### Deleted Files… dialog

A list of deleted files under the current folder (relative path, deleted
date, size) with **Restore** per row. Empty state: "No Deleted Files".
Restore → toast "Restored “draft.md”" (or "Restored as “…”" when renamed).

### Properties (versioned root only)

Two extra rows in the existing Properties dialog: **Version History** —
"12.3 MB, 214 versions of 37 files" — and a row with **Clear History…** and
**Stop Keeping Versions…** buttons. Both confirm first with Cancel as the
default response:

- Clear: "Clear Version History?" / "All earlier versions of the files in
  “Projects” will be moved to the Trash. The current files are not changed."
  / [Cancel] [Clear History] (destructive).
- Stop: "Stop Keeping Versions?" / "Earlier versions will be moved to the
  Trash and new changes won't be recorded. The current files are not
  changed." / [Cancel] [Stop Keeping Versions] (destructive).

Inside a versioned folder (but not its root), Properties of a file shows a
"Versions" row with the count, e.g. "7".

## Error handling

| Situation | Behaviour |
|---|---|
| Index can't be opened (corrupt, newer schema, permissions) | That folder isn't watched; Versions… shows the error in the dialog; nothing is written. |
| Object missing or hash mismatch on rebuild | "This version is damaged and can't be restored." Never write partial output. |
| Disk full while writing an object | Leave the temp file cleanup to the next launch (clear `tmp/` at open); no row inserted; log; retry on the next change. |
| File locked / unreadable while capturing | Skip; the next event or launch scan retries. |
| Watcher can't be created | Fall back to a 5-minute periodic scan for that folder. |
| `versioned.json` unreadable | Treat as empty; folders are re-found by `versioned_root` on navigation. |
| Restore target name exists | Restore Copy picks the next free name; Replace goes through the Trash. Nothing is overwritten. |

Backend logs use `eprintln!` like the rest of the backend (there is no
logging crate).

## Performance

- Capture, scan, rebuild and diff all run on Tauri's blocking pool via
  `blocking(…)`; none on the async workers (AGENTS.md).
- At most one capture runs at a time per versioned folder (the folder's
  connection mutex serializes them).
- Hashing streams the file; deltas hold at most base + new (≤ 128 MiB each)
  in memory, which is why D5 exists.
- Browsing non-versioned folders costs one extra `versioned_root` call per
  navigation (a few `stat`s up the tree) and nothing else.
- The launch scan compares `size`/`mtime` from the index before reading any
  file, so an unchanged folder of 10,000 files scans with stat calls only.

## Testing

Unit tests live next to each module (`#[cfg(test)]`), use `tempfile`, and
must run without Tauri (AGENTS.md's scratch-crate rule applies:
`versions/` may depend on `rusqlite`, `zstd`, `blake3`, `notify`, `similar`,
`trash`, `serde`, `serde_json` only).

Required tests:

- **store**: full round trip; delta round trip; chain of 19 deltas rebuilds;
  the 20th version becomes a full copy; delta ≥ 90 % falls back to full;
  a tampered object fails the hash check; a crash between object write and
  row insert (simulate by writing the object only) leaves the index valid.
- **index**: schema creation; refusing a newer `schema_version`; queries
  return newest first; tombstones.
- **capture**: unchanged mtime/size records nothing; same content after a
  touch records nothing; identical content in two files stores one object;
  deletion creates one tombstone (not two on a repeated event); ignore list;
  `.versions/` and nested versioned subtrees skipped; symlinks skipped;
  > 1 GiB skipped (use a sparse file); file changing during read is re-queued.
- **scan**: picks up new, changed and deleted files; leaves unchanged ones.
- **watch**: the coalescer (with an injected clock) fires once per path after
  the quiet period; overflow triggers a directory scan.
- **restore**: Restore Copy never overwrites and picks ` 2`, ` 3`; Replace
  sends the old file to the Trash (verify on the platform's Trash layout,
  like `trash_ops` tests) and records the current file first; restoring a
  deleted file recreates missing parent folders; a damaged version writes
  nothing.
- **diff**: text vs text, Same, Binary, TooLarge, truncation at 2,000 lines.
- **registry**: move/rename updates paths; trashing unregisters; missing
  folders are pruned at launch.
- **fs_ops**: `.versions` hidden from listings and counts even with
  `show_hidden = true`.

Frontend: `npx tsc --noEmit`, `npm run build`, and a Playwright pass in
`npm run dev:mock` (Mac user agent) covering the checkbox (enabled and
disabled), Versions… (select rows, diff states, Restore Copy, Replace
confirmation), Deleted Files…, and the Properties rows in light and dark
mode.

## Platform notes

- **macOS** (unverified from Linux; say so when reporting): `notify` uses
  FSEvents, which batches events and may report a directory instead of a
  file; the directory-scan path must handle that. Full Disk Access affects
  which folders can be watched. Trash behaviour for Replace and Clear uses the
  existing `trash` crate path.
- **Linux** (unverified on a Mac): inotify needs one watch per directory;
  large trees can hit `fs.inotify.max_user_watches`, hence the periodic-scan
  fallback. Trash uses the freedesktop layout; tests must cover it as
  `trash_ops` tests do.
- Paths in the index are stored NFC-normalized with `/` separators so a
  folder moved between macOS and Linux keeps matching.

## Delivery

One feature branch, e.g. `feat/keep-versions`, in this order (each step
builds and passes tests on its own):

1. `versions/store.rs` + `index.rs` with tests; crates added; `Cargo.lock`
   updated minimally.
2. `capture.rs` + `scan` + registry with tests.
3. `watch.rs` + startup wiring in `lib.rs`.
4. `make_versioned_dir`, `versioned_root`, hiding `.versions`; New Folder
   checkbox; mocks.
5. `restore.rs` + diff; Versions… dialog; Deleted Files… dialog.
6. Properties rows, Clear History…, Stop Keeping Versions….
7. README: feature list entry marked Experimental, a sentence that history is
   local and is **not a backup** (it lives on the same disk), and "Not
   supported yet" entries for the non-goals above. AGENTS.md: add the new
   files to the Layout table and the `versions/` crates to the scratch-crate
   note.

Before finishing: `npx tsc --noEmit`, `npm run build`, `cargo test` pass, and
the UI has been checked in `npm run dev:mock`. Commits follow the
conventional style (`feat:`, `fix:`, `docs:`).

## Acceptance criteria

1. Creating a folder with Keep versions ticked produces `<folder>/.versions/`
   with a valid index, and the folder appears registered and watched.
2. Saving a text file three times in a versioned folder (from any editor)
   produces three versions; Versions… lists them with correct dates and
   sizes and shows a correct diff against the current file.
3. Restore Copy creates a correctly named copy whose content matches the
   chosen version byte for byte; nothing is overwritten.
4. Replace… asks first (default Cancel), sends the current file to the Trash,
   and the file then matches the chosen version; the replaced state is still
   in Versions….
5. Deleting a file shows it in Deleted Files…, and Restore brings it back.
6. Editing a file while the app is closed and relaunching records the new
   state.
7. A 10 MB text file edited by one line ten times uses well under 2× its size
   in `.versions/objects/` (deltas work).
8. `.versions` never appears in listings, counts, search, Starred or Recent.
9. Clear History… and Stop Keeping Versions… move `.versions/` to the Trash
   after a confirmation with Cancel as the default.
10. Browsing a non-versioned folder of 10,000 items is as fast as before.
11. All tests listed above exist and pass; type-check and build pass.
