# Keep Versions — Design

**Date:** 2026-10-24
**Revision:** 4 (all delivery steps implemented)
**Status:** Implemented. The storage layer, capture/watch/restore, commands,
frontend dialogs and documentation are in the tree; see the status addendum in
`2026-10-24-keep-versions-design-review-2.md`.
**Related:** `2026-10-23-file-browser-design.md` (the app's overall design),
`AGENTS.md` (rules every change must keep)

## Changes in revision 4 (implementation)

- `history.rs`, `capture.rs`, `watch.rs`, `restore.rs`, `registry.rs`,
  `state.rs` were added; `lib.rs` owns the managed `VersionsState` and the
  copy/move/rename/trash hooks.
- The copy/move helper uses the quiesce variant: stop watchers, checkpoint and
  close the source histories, let `copy_recursive` copy the closed tree, then
  reopen the copy with a new `folder_id` and scan it. This replaces the earlier
  online-backup sketch; both are safe, but quiescing needs no page-by-page copy.
- The frontend has `src/versions.ts` (Versions…, Deleted Files…, Properties
  rows), the New Folder checkbox, context-menu entries and mocks. The diff has
  a request token; Clear/Stop close the stale Properties dialog.
- `history_info` sums objects + index/WAL for the root and its owned nested
  histories.
- Remaining follow-ups are listed in the review: Finder copies while running,
  mtime-preserving writes, coalescing of rapid saves, a watcher cap on Linux,
  and reachability of type-changed/deleted histories.

## Changes in revision 3 (Step 1 implementation)

The storage layer is now code, and three details were settled while writing it:

- **Crates:** `rusqlite` is `default-features = false` with `bundled` + `backup`
  (`backup` is not enabled by `bundled`). `zstd` 0.14.3 and `blake3` 1.8 are in.
- **Schema gate:** `PRAGMA user_version = 1` is written with the schema; the
  ownership rows in `meta` are written only after the WAL check and the test
  transaction have passed (R6). A failed creation therefore leaves no marker.
- **Object encoding:** the object file is named by the BLAKE3 of the decoded
  content, so a `full` frame and a `delta` frame of the same bytes would share
  a name. The `objects` row decides the encoding. `capture` must check
  `objects` before writing and use `store::write_full_unless` /
  `write_delta_unless`; the plain `write_full`/`write_delta` always install the
  encoding they are asked for and must never replace an object a row points at.
  A file with no row is an orphan and may be overwritten under the folder lock.

Also implemented from the review: paged `list_versions` (R8), the indexed BLOB
range for directory prefixes (R7), separate read-only and read-write opens
(R5), the `.versions/meta.json` ownership gate (R1), holding the lock through
the Trash move (R2), `create_new` + copy for restores instead of hard links
(R4), and canonical roots (R9). The remaining follow-ups (Finder copies, nested
history size, deleted-history reachability, mtime-preserving writes, coalescing
of rapid saves) are noted in the review and the README limitations.

## Changes in revision 2

All ten P0 items and the P1 items of the design review are folded in:
ownership marker (D14), raw byte paths (D15), live-index snapshots for copy
and move, directory tombstones, separate capture and database locks, a cheap
stat-based "Current", streamed full copies, restore staging in
`.versions/tmp/` with `create_new`, a local-disk check (D18), frontend-made
restore names (D19), a registry operation matrix, `history_info`
definitions, a per-folder lock file (D16), path and hash validation, scan
batching, type changes (D17), a diff request token, startup ordering and one
`is_internal_path` helper. The review's open questions are answered in
D15–D19. One suggestion is declined: deleting orphan objects at open (see
"Orphan objects").

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
  destroy a user's file or a history it does not own.

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
| D1 | Hash: **BLAKE3** of the raw (uncompressed) content, 64 lowercase hex characters. | Fast; hashing raw content keeps identity independent of how it is stored. |
| D2 | A versioned folder tracks **all subfolders recursively, on the same filesystem**. It does not cross mount points. Creating a versioned folder inside one is not offered (checkbox disabled). A subtree with its own *owned* `.versions/` (D14) is skipped by the outer folder. | One owner per file, no double tracking; staging in `.versions/tmp/` can always be renamed into place. |
| D3 | Only **regular files** are versioned. Symlinks, folders, sockets and the like are not. | Keeps v1 small. |
| D4 | History follows the **path**. A rename is recorded as a deletion of the old path plus a new file. | Rename detection is guesswork; can be added later without changing stored data. |
| D5 | Files over **1 GiB** are not versioned. Files over **128 MiB** are stored as streamed full copies, never deltas. Only deltas hold file contents in memory. | Bounded memory. |
| D6 | Delta chains: a full copy when the chain would reach depth **20**, or when the delta is ≥ 90 % of the raw size. | Restoring never replays more than 19 deltas. |
| D7 | **Clear History…** and **Stop Keeping Versions…** move `.versions/` to the **Trash**, and only after the ownership check (D14). | "Never lose data" (AGENTS.md). |
| D8 | **Replace…** first records the current file as a version, then sends it to the Trash and renames the staged version into place. | The current state is never lost. |
| D9 | The Versions… diff compares the **selected version with the current file**. | The question is "should I restore this?". |
| D10 | `.versions` is **always hidden**: listings, item counts, search, Starred and Recent, even with Show Hidden Files on, through one `is_internal_path` helper. Properties' folder size skips `.versions/` in its walk and adds the history's size from the index. | Implementation detail; disk usage stays honest without walking huge histories. |
| D11 | Turning versions on for an **existing** folder is out of scope for v1. | The user asked for the New Folder entry point. |
| D12 | Editor temp files are ignored (list below). | They would flood history with noise. |
| D13 | If turning versions on fails after the folder was created, the folder is **kept** and an error says so. Nothing is ever removed with `remove_dir_all`. | Never delete; an empty folder is harmless. |
| D14 | **Ownership marker.** A `.versions/` is ours only if its index opens and `meta` holds `app = 'unshitty-mac-filebrowser'` and a `folder_id`, mirrored in a tiny `.versions/meta.json` (app, folder_id, schema_version) that `versioned_root` can read without opening SQLite. Anything else is foreign: never registered, watched, shown, cleared or trashed. `make_versioned_dir` refuses to write into an existing `.versions/`. | We must never trash a directory another tool made, and navigation must not open an index. |
| D15 | **Raw byte paths.** Paths are stored as the exact on-disk bytes (`OsStr` bytes, both targets are Unix) relative to the root, plus a lossy display string. No Unicode normalization. Cross-platform *matching* (a folder moved macOS ↔ Linux with NFD names) is not a goal; not corrupting anything is. | Normalizing breaks Linux; lossy strings alias distinct names. Answers review question 1. |
| D16 | **Lock file per history.** The process that owns a history holds an exclusive lock on `.versions/lock` (`std::fs::File::try_lock`, Rust ≥ 1.89; the toolchain is 1.99). A second app instance that can't get the lock does not watch or capture and treats that history as read-only: Versions… and Deleted Files… work; Restore Copy works; Replace, Clear and Stop refuse with "The version history of this folder is in use by another window." | No single-instance plugin needed; prevents duplicate captures and unsafe `tmp/` cleanup. Answers review question 2. |
| D17 | **Type change = deletion.** When a path that had versions becomes a folder, symlink or other non-regular file, a tombstone is recorded. | The history stays restorable and nothing is shown as current. Answers review question 3. |
| D18 | **Local disks only.** Creating a versioned folder requires that the index opens in WAL mode and survives a test transaction; otherwise "Version history needs a folder on a local disk." Network and cloud-synced folders are unsupported in v1. | WAL is unreliable on SMB/NFS. |
| D19 | **Restore names come from the frontend.** It formats `<stem> (<YYYY-MM-DD HH-MM>)<ext>` in local time and passes it in; the backend validates it and enforces no-overwrite. No date crate in the backend. | Rust std has no local time zone; the frontend already formats dates. Answers review question 5. |
| D20 | `history_info`: `bytes` = `SUM(stored_size)` over distinct objects **plus** `index.sqlite` and its `-wal` (and, for a versioned root, the histories of owned nested roots); `versions` = rows that are not tombstones; `files` = distinct paths with at least one non-tombstone version, including files deleted since. UI: "12.3 MB, 214 versions of 37 files". | Answers review question 4; dedupes identical content and keeps the disk-usage figure honest. |

## Non-goals (v1)

- Background capture while the app is closed.
- Splitting container formats (zip-based, glTF) into parts.
- The "folder at time T" slider (Arbesk scene clock).
- Pruning or thinning old versions; size limits.
- Turning versions on for existing folders.
- Rename/move detection inside a versioned folder.
- Versioning symlinks, folder metadata, permissions or extended attributes.
- Matching history across macOS ↔ Linux Unicode normalization (D15).
- Network, cloud-synced or removable-and-shared volumes (D18).
- Any network, sync, IPFS or blockchain feature.
- Image or document previews of old versions (text diff only).

## Known limitations (v1)

State these in the README:

- History is **not a backup**: it lives on the same disk as the files.
- Changes made and overwritten while the app is closed are not kept.
- Versioned folders must be on a local disk.
- Only one app instance records history for a folder at a time (D16).
- A folder moved between macOS and Linux with differently normalized names
  keeps its history, but those files' new versions start a new history.

## Architecture

Backend logic lives in plain functions, testable without Tauri (AGENTS.md);
only `lib.rs` touches Tauri types.

### Backend: `src-tauri/src/versions/`

| File | Responsibility | Depends on |
|---|---|---|
| `mod.rs` | Public API for `lib.rs`; constants (limits, ignore list); `is_internal_path`; path and hash validation; the folder registry (`versioned.json`). | the files below, `config_store` |
| `history.rs` | One open history: the lock file, ownership check, the capture lock, a writer connection and a reader connection. Create, open, close, checkpoint. | `index`, `std::fs::File::try_lock` |
| `store.rs` | Object store in `.versions/objects/`: streamed full writes, delta writes, chain rebuild with size cap and hash check. Knows nothing about paths or versions. | `zstd`, `blake3` |
| `index.rs` | `index.sqlite`: schema, migrations, all queries, online backup. | `rusqlite` |
| `capture.rs` | Record one path; tombstone a directory prefix; the launch scan. | `history`, `store`, `index` |
| `watch.rs` | One `notify` watcher per owned history; a pure coalescer with an injected clock. | `notify`, `capture` |
| `restore.rs` | Rebuild to `.versions/tmp/`; Restore Copy; Replace; deleted files; text diff. | `history`, `store`, `index`, `trash`, `similar` |
| `registry.rs` | `versioned.json`: read/write, register/unregister, prefix rewrites for moves. | `config_store` |
| `state.rs` | The managed set of open histories and watchers: open/register, canonical roots, suspend/resume for copy/move/trash, startup scan. | `history`, `watch`, `capture`, `registry` |

Add to `src-tauri/Cargo.toml`, updating `Cargo.lock` minimally (AGENTS.md):

- `rusqlite` with `default-features = false` and the `bundled` + `backup`
  features (SQLite compiled in; `backup` is a separate non-default feature;
  the current quiesce copy does not need it, but keep it for future transfer
  work and for `integrity_check`-style snapshots)
- `zstd` (0.14.x; the review reports `Encoder::with_ref_prefix` and
  `Decoder::with_ref_prefix` there, verify when adding)
- `blake3`
- `notify` (FSEvents on macOS, inotify on Linux)
- `similar` (line diff)

All are MIT/Apache/BSD licensed, compatible with GPL-3.0-or-later; SQLite is
public domain. Add them to AGENTS.md's scratch-crate note.

### Wiring in `lib.rs`

- `VersionsState` in Tauri managed state: a map from root path to
  `Arc<History>` and its watcher. Map access is a short lock; nothing slow
  happens while holding it.
- **Startup:** after the window is up, a background thread reads
  `versioned.json` and, one root at a time: check the root and its
  `.versions/` exist; open the history (lock, ownership, schema); start the
  watcher; run the launch scan. A root whose index is foreign, corrupt or
  newer is left untouched, logged, and reported in Versions…; it is never
  recreated automatically. Missing roots are dropped from the registry.
- **Commands** below. Anything that reads file contents, hashes, compresses
  or walks a tree is an `async fn` running through `blocking(…)`.

### Frontend

- `src/versions.ts` (new): Versions… dialog, diff view, Deleted Files…
  dialog, Properties rows, restore-name formatting (D19).
- `src/main.ts`: the checkbox in `promptNewFolder`, menu entries,
  `versioned_root` on navigation.
- `src/api.ts`: typed wrappers.
- `dev/mock-tauri/core.ts`: mocks for every new command, with a fake
  versioned folder `/Users/me/Documents/Versioned` holding a text file with
  several versions, a binary file, a file over 1 MiB, and one deleted file.
- `src/styles.css`: libadwaita tokens only; every colour has a dark value.

## On-disk layout

```
<folder>/
└── .versions/                 mode 0700
    ├── index.sqlite           mode 0600  (+ -wal / -shm while open)
    ├── meta.json              app + folder_id + schema_version (D14)
    ├── lock                   held exclusively by the owning process (D16)
    ├── objects/
    │   └── 3f/
    │       └── 3fa9…e1        one object, named by its BLAKE3 hex hash
    └── tmp/                   staging for objects and restores
```

Object files are raw zstd frames. Full copy or delta, and the base, are
recorded in the index; the hash in the file name is checked against the
decoded content on every read.

### Schema (`index.sqlite`, version 1)

```sql
PRAGMA journal_mode = WAL;      -- must return 'wal' (D18)
PRAGMA synchronous = NORMAL;    -- a power loss can lose the last batch, not corrupt
PRAGMA foreign_keys = ON;
PRAGMA busy_timeout = 5000;
-- PRAGMA user_version = 1;   -- the schema gate; written with the tables

CREATE TABLE meta (
  key   TEXT PRIMARY KEY,
  value TEXT NOT NULL
);
-- rows, written after the WAL check and test transaction:
--   schema_version = '1'
--   app            = 'unshitty-mac-filebrowser'   (D14)
--   folder_id      = random 128-bit hex
--   created_at     = unix seconds

CREATE TABLE objects (
  hash        TEXT PRIMARY KEY CHECK (length(hash) = 64),
  kind        TEXT NOT NULL CHECK (kind IN ('full', 'delta')),
  base        TEXT REFERENCES objects(hash),
  depth       INTEGER NOT NULL,
  size        INTEGER NOT NULL,     -- raw size in bytes
  stored_size INTEGER NOT NULL,     -- size of the object file
  codec       TEXT NOT NULL DEFAULT 'zstd',
  CHECK ((kind = 'full'  AND base IS NULL     AND depth = 0) OR
         (kind = 'delta' AND base IS NOT NULL AND depth BETWEEN 1 AND 19))
);

CREATE TABLE versions (
  id          INTEGER PRIMARY KEY,  -- grows with time; the version id
  path        BLOB NOT NULL,        -- exact relative path bytes, '/' separators (D15)
  display     TEXT NOT NULL,        -- lossy UTF-8 of path, for UI only
  hash        TEXT REFERENCES objects(hash),  -- NULL = tombstone
  size        INTEGER NOT NULL,     -- 0 for tombstones
  mtime       INTEGER NOT NULL,     -- unix nanoseconds; 0 for tombstones
  recorded_at INTEGER NOT NULL      -- unix seconds
);
CREATE INDEX versions_by_path ON versions(path, id);
```

The previous version of a row is the row with the same `path` and the
next-lower `id`. Rows are immutable except `mtime` of a path's latest row
(capture step 5). `index.rs` refuses a `schema_version` newer than it knows
("This folder's history was made by a newer version of the app.").
Migrations run inside one transaction.

### Validation (every read)

Before using index data for I/O:

- `path`: relative; no empty, `.` or `..` components; no NUL; not inside
  `.versions/`. Joined onto the root with `OsStr` APIs, never via `String`.
- `hash`: exactly 64 ASCII lowercase hex characters before building
  `objects/<first two>/<hash>`.
- `objects`: the CHECK constraints above, plus `delta.depth = base.depth + 1`
  while following a chain.

A violation is "This folder's version history is damaged." Nothing is
written.

## Locks and connections

Per open history (`history.rs`):

- `lock` file: exclusive for the process lifetime (D16).
- `capture_lock: Mutex<()>`: serializes captures, scans, restores that write,
  and transfers for this history.
- `writer: Mutex<Connection>`: opened `READ_WRITE`; held only for short
  transactions (latest-row read, inserts, `mtime` update, checkpoint). Never
  across file reads, hashing, compression or directory walks.
- `reader: Mutex<Connection>`: opened `READ_ONLY` with `query_only = ON`; for
  `list_versions`, `deleted_files`, `history_info`, `version_diff` lookups. WAL
  lets it read while the writer commits. A read-only open never sets
  `journal_mode`/`synchronous`, never migrates and never clears `tmp/`.

`versioned_root` never opens an index or waits on any of these: it reads the
small `.versions/meta.json` marker and uses the registry map plus `stat` calls.
Opening a newly found history happens on a background thread, which then
verifies the DB `meta` rows against `meta.json`.

## Data flow

### Creating a versioned folder

1. User ticks **Keep versions** in New Folder and presses Create.
2. Frontend calls `make_versioned_dir(path)`.
3. Backend: `fs_ops::make_dir`. Then, refusing if `<path>/.versions` already
   exists (D14): create `.versions/` (0700), `tmp/`, `objects/`, `lock`;
   take the lock; create the index (0600) with the schema and
   `PRAGMA user_version = 1`; verify `journal_mode` is `wal` and run a test
   write/read transaction (D18); only then write the `meta` rows (app,
   `folder_id`, schema_version, created_at); register; start the watcher.
4. If anything in step 3 fails, keep the folder (D13) and return "The folder
   was created, but versions could not be turned on: <reason>". A partly
   created `.versions/` is left as is. If the failure happens before the
   `meta` write it has no marker, so it is not ours by D14 and is never
   adopted; a history whose `meta` was written but whose watcher failed is
   valid and can be adopted later.

### Capturing one path (`capture::record(history, rel_path)`)

Takes `capture_lock` for the whole operation.

1. Skip if the path is internal (`is_internal_path`: any component equal to
   `.versions`), inside an owned nested history (D2), on another filesystem
   than the root (D2), or matches the ignore list (D12).
2. `symlink_metadata`:
   - Missing → tombstone the path if its latest row isn't one. Done.
   - Exists but not a regular file → tombstone if the latest row isn't one
     (D17). Done.
   - Larger than 1 GiB (D5) → done.
3. Latest row has the same `size` and `mtime` → done. (Short `writer`
   transaction for the lookup.)
4. **Full copy path** (file > 128 MiB, no usable base, or the base chain is at
   depth 19): stream the file through BLAKE3 and a zstd encoder into
   `tmp/obj-<pid>-<n>`, level 19 for ≤ 1 MiB else 6, long-distance matching
   for > 8 MiB. Stat again; if size or mtime changed, delete our temp file and
   re-queue the path. If the hash equals the latest row's, update `mtime` and
   delete the temp file. Before installing, check `objects` for the hash: if a
   row exists, delete the temp file and insert only a `versions` row, using
   that row's kind/base/depth (a full frame and a delta frame of the same
   content share a name, so the existing row is authoritative). Otherwise
   `fsync`, rename to `objects/xx/<hash>`, then insert `objects` + `versions`
   rows in one short transaction. `store::write_full_unless` performs the
   check inside the streaming pass with an index callback.
5. **Delta path** (file ≤ 128 MiB and the latest object qualifies): read the
   file into memory, stat again (re-queue if it changed), hash. Same hash →
   update `mtime`, done. Known hash in `objects` → insert a row using the
   existing row, done (no write). Otherwise rebuild the base (`store::read`,
   ≤ 128 MiB), compress with the base as zstd reference prefix, window log
   covering base + new (up to 28; decoder `window_log_max` 28). If the delta
   is ≥ 90 % of the raw size, store a full copy instead. Write to `tmp/`,
   `fsync`, rename, insert rows. `store::write_delta_unless` checks the index
   before compressing.
6. A crash between rename and insert leaves an orphan object (see "Orphan
   objects"); a row never points at a missing object. Never rewrite an object
   that a row references.

### Tombstoning a directory (`capture::tombstone_prefix(history, rel_dir)`)

For every path under `rel_dir/` (the indexed BLOB range `path >= rel_dir ||
X'2F' AND path < rel_dir || X'30'`, so `foo/…` matches but `foobar` does not)
whose latest row is not a tombstone and which no longer exists as a regular
file, insert a tombstone. The root uses no range. Used for directory remove and
rename events (old path), and by the launch scan for vanished directories.

### Launch scan (`capture::scan(history)`)

Walk the root without following symlinks or crossing filesystems, skipping
internal paths and owned nested histories. For each regular file, compare
`size`/`mtime` with its latest row and call `record` when they differ or
there is no row. Then tombstone every indexed path whose latest row is not a
tombstone and that isn't a regular file any more. Inserts are batched in
transactions of up to 250 rows; `PRAGMA wal_checkpoint(TRUNCATE)` at the end.
Unchanged files cost a `stat` only.

### Watching (`watch.rs`)

- `notify::RecommendedWatcher`, recursive, on each owned root this process
  holds the lock for.
- Event paths go into a coalescer: path → last-event time. A ticker every
  500 ms fires `record` for paths quiet for **2 s**. This turns "write temp
  file, rename over original" into one capture.
- Directory remove/rename: `tombstone_prefix` on the old path, `scan` on the
  parent. Rename events with both paths: tombstone the old one directly.
  Overflow / `Rescan`: `scan` the root.
- After a burst of captures (coalescer idle for 30 s), checkpoint the WAL.
- Watch errors (Linux `max_user_watches`, permissions): log and fall back to
  `scan` every 5 minutes for that root. Every registered root gets a watcher;
  there is no cap in v1. On Linux each directory costs one inotify watch,
  so very large trees may hit the fallback; document this in the README.
- If the root disappears, stop the watcher and close the history; the
  registry entry is pruned at the next launch.

### Ignore list (D12)

Base names: `.DS_Store`, `Thumbs.db`, `desktop.ini`, `4913`. Patterns:
`~$*`, `.~lock.*#`, `*.swp`, `*.swo`, `*.swx`, `*~`, `*.tmp`, `.#*`,
`*.crdownload`, `*.part`, `*.download`, and the app's own staging names
`.*.filebrowser-*` (from `fs_ops`). One constant in `versions/mod.rs`.

### Rebuilding a version (`store::read(hash)`)

Validate, then follow `base` links down to a full object (≤ 19 steps),
decompress it, apply each delta upwards. Cap every decompressed output at
that object's `size`; a frame that expands past it is an error. Verify the
BLAKE3 hash of the final bytes. On any failure: "This version is damaged and
can't be restored." and nothing is written. Restore of a version > 128 MiB
(always a full object) streams from the object file to the staging file
instead of buffering.

### Restoring (`restore.rs`)

All staging happens in `.versions/tmp/restore-<pid>-<n>`: same filesystem as
any tracked file (D2), invisible to capture.

- **Restore Copy** `(path, version, name)`: rebuild into staging, `fsync`.
  Create missing parent folders. Then place it at `name` (from the frontend,
  D19; validated as a single file name) without overwriting:
  `OpenOptions::create_new(true)` and stream-copy the staging bytes into it.
  `create_new` is the portable no-overwrite primitive; hard links are not
  available on exFAT/FAT local disks, so they are at most an optional fast
  path with a copy fallback. On `AlreadyExists` try `<stem> 2<ext>`, ` 3`, ….
  Returns the final path. The copy gets `0666 & umask`, not the staging mode.
- **Replace…** `(path, version)`: requires the lock (D16). `record` the
  current file first (D8). Rebuild into staging, `fsync`. Move the current
  file to the Trash (`trash` crate), then `rename` staging into place. If the
  rename fails after the Trash move, return an error naming where the old file
  went (the Trash) and where the restored copy is (`.versions/tmp/…`); delete
  neither. If the current file doesn't exist, this is a plain no-overwrite
  placement at the original path.
- **Deleted files** `(dir)`: paths under `dir` whose latest row is a
  tombstone and where no regular file exists now. Restoring one places its
  last non-tombstone version at the original path without overwriting; if
  something occupies it, the frontend's Restore Copy name is used.

The watcher then sees the restored file and records a row with an existing
hash: one history row, no new object.

### Clearing and stopping (D7)

Both act on an owned root and require the lock (D16):

1. Re-check ownership (D14) immediately before acting; refuse if it fails.
2. Stop the watcher and checkpoint, but **keep the lock open**.
3. Move `.versions/` to the Trash while the lock is still held (moving a
   directory with an open locked file is fine on both targets); release it
   only after the move succeeds. If the move fails, keep the history, the
   registry and the lock state unchanged.
4. **Clear History…**: create a fresh history as in "Creating" (new
   `folder_id`), run `scan` (current state becomes the first versions),
   restart the watcher. **Stop Keeping Versions…**: remove the root from the
   registry only after the move succeeded.

### Orphan objects (review suggestion declined)

A crash between an object's rename and its row insert leaves an object file
no row points to. The review suggests deleting these at open. v1 leaves them:
they appear only after a crash, cost little space, are harmless, and
deleting files inside a history is exactly the kind of code this project
avoids. They are removed with everything else by Clear History…. `tmp/`
*is* cleared at open, which is safe because the process holds the lock
(D16).

## Registry and existing operations

`versioned.json` in the app config dir:

```json
{ "version": 1, "roots": ["/Users/me/Documents/Projects"] }
```

Written with `config_store::write_json` (temp file + rename) under a
`VERSIONS_REGISTRY: Mutex<()>`, the same pattern as `TRASH_LOG` in `lib.rs`.

| Operation | What happens |
|---|---|
| `versioned_root(path)` on navigation | Walk up from `path` with `stat`: the nearest ancestor-or-self that has a `.versions/meta.json` whose `app` is ours. Never for paths in the Trash. If it isn't registered, register it and open it on a background thread; the open verifies the DB `meta` matches `meta.json`, and only then does the result count (until then, and if verification fails, the result is `None`). |
| Same-volume move/rename of a registered root | Stop watcher, checkpoint, close, release lock; `rename`; rewrite the registry prefix; reopen; restart watcher. Roots are canonicalised (`fs::canonicalize`) before registration, so a symlinked path does not open the same history twice. |
| Same-volume move of a folder containing registered roots | The same for every registered root under it. |
| Move a root out of a versioned parent | As above; it stays its own root. The parent's watcher sees the removal and tombstones the paths it had recorded there, if any. |
| Copy (same or cross volume) of a folder containing owned histories | Stop the watchers and close the source histories (a checkpoint is folded in), then let the ordinary `copy_recursive` copy the tree. A closed, checkpointed index is a consistent file; the copy therefore contains a valid history without copying a live WAL. For each copied history: open it, give it a new `folder_id` (DB `meta` + `meta.json`), run `PRAGMA integrity_check`, register, start its watcher, and `scan` it. Foreign `.versions/` directories are copied as ordinary files. (This is the quiesce variant of the review's online-backup suggestion; it is simpler and avoids copying objects separately.) |
| Cross-volume move | The copy above, verified, then the existing "delete the source only after the copy fully succeeded" rule; stop and close the source history before its removal. |
| Trash a registered root, or a folder containing roots | Stop watcher, close, unregister **before** the Trash move. |
| Restore a root from the Trash | No automatic registration; `versioned_root` finds it on navigation. |

## Commands (`lib.rs`) and wrappers (`api.ts`)

All errors are user-facing strings in GNOME wording.

| Command | Signature (Rust) | Notes |
|---|---|---|
| `make_versioned_dir` | `async (path: String) -> Result<(), String>` | D13, D14, D18. |
| `versioned_root` | `async (path: String) -> Option<String>` | Never blocks on a history; reads `meta.json`. |
| `list_versions` | `async (path: String, limit: Option<u32>, offset: Option<u32>) -> Result<Vec<VersionInfo>, String>` | Newest first, page size clamped to 5,000 (default 1,000); index only, works for files no longer on disk (R8). |
| `version_count` | `async (path: String) -> Result<u64, String>` | Total rows, so the dialog can say "showing the newest 1,000". |
| `version_diff` | `async (path: String, version: i64) -> Result<VersionDiff, String>` | D9. |
| `restore_version_copy` | `async (path: String, version: i64, name: String) -> Result<String, String>` | Returns the final path. |
| `replace_with_version` | `async (path: String, version: i64) -> Result<(), String>` | D8, D16. |
| `deleted_files` | `async (dir: String) -> Result<Vec<DeletedFile>, String>` | Recursive under `dir`. |
| `restore_deleted` | `async (path: String, fallback_name: String) -> Result<String, String>` | Returns where it went. |
| `history_info` | `async (root: String) -> Result<HistoryInfo, String>` | D20. |
| `clear_history` | `async (root: String) -> Result<(), String>` | D7, D14, D16. |
| `stop_keeping_versions` | `async (root: String) -> Result<(), String>` | D7, D14, D16. |

Paths cross the IPC boundary as strings, as everywhere in this app. Files
whose names aren't valid UTF-8 are still recorded with their exact bytes
(D15), but in v1 the UI can't address them: they don't appear in Versions…
or Deleted Files… (the app's listings can't show such names either).

```rust
#[derive(Serialize)]
pub struct VersionInfo {
    pub id: i64,
    pub size: u64,
    pub mtime: i64,        // unix seconds (display)
    pub recorded_at: i64,  // unix seconds
    pub deleted: bool,     // tombstone row
    pub current: bool,     // see below
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum VersionDiff {
    Text { lines: Vec<DiffLine>, truncated: bool },
    Same,
    Binary,      // either side not text
    TooLarge,    // either side > 1 MiB, decided from sizes before any rebuild
    Deleted,     // the selected row is a tombstone
    NotOnDisk,   // there is no regular file at the path now
}

#[derive(Serialize)]
pub struct DiffLine { pub tag: char /* ' ', '-', '+' */, pub text: String }

#[derive(Serialize)]
pub struct DeletedFile { pub path: String, pub deleted_at: i64, pub size: u64 }

#[derive(Serialize)]
pub struct HistoryInfo { pub bytes: u64, pub versions: u64, pub files: u64 }
```

- **`current`** is true for exactly one row at most: the path's latest
  non-tombstone row, and only if a regular file exists at the path with the
  same `size` and `mtime`. No hashing when listing. Content equality of other
  rows is reported by `version_diff` (`Same`).
- **Text** means valid UTF-8 with no NUL in the first 8 KiB. Unified line
  diff (`similar`, 3 lines of context), capped at 2,000 lines.
- **`TooLarge`** is decided from `versions.size` and the current file's
  `metadata().len()` before rebuilding anything.

## UI

All dialogs use `openDialog` (its `extra`, `className` and
`setResponseEnabled` cover this), menus use `openMenu`, messages use
`showToast`. File names go in with `textContent` or `escapeHtml`. Dates use
`src/format.ts`.

### New Folder dialog

Below the Folder Name row, a `check-row` (the style the paste-conflict dialog
uses):

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

- "Experimental" is a small pill using accent tokens.
- Disabled, with the hint "Already inside a folder that keeps versions",
  when `versioned_root(location)` is set (D2).
- Unticked by default; not remembered.
- Create calls `make_versioned_dir` when ticked, `makeDir` otherwise.

### Item context menu (file inside a versioned folder)

**Versions…** directly above **Properties**, enabled for a single regular
file only. Not shown outside versioned folders.

### Background context menu (inside a versioned folder)

**Deleted Files…**.

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

- Newest first. Up to 1,000 rows rendered; beyond that the newest 1,000 and
  a "Showing the newest 1,000 versions" note.
- Selecting a row requests `version_diff` with a per-dialog increasing
  request token; responses with a stale token are dropped. While a request is
  in flight the diff area shows a spinner (never the previous diff) and both
  buttons are disabled.
- Diff area: monospace; `-` lines on a destructive-tinted background, `+`
  lines on a success-tinted one (tokens with dark values). Messages:
  `Same` "This version is the same as the current file." · `Binary` "No
  preview for this type of file." · `TooLarge` "This file is too large to
  compare." · `Deleted` "The file was deleted at this point." · `NotOnDisk`
  "The file is not in the folder any more."
- Buttons disabled for the Current row and Deleted rows. Replace… is also
  disabled for `NotOnDisk` (Restore Copy restores it) and when the history is
  read-only (D16), with that message as a tooltip.
- **Restore Copy** → toast "Restored as “notes (2026-10-24 14-05).txt”";
  select the new file if it is in the current folder.
- **Replace…** → confirmation: heading "Replace “notes.txt”?", body "The
  current file will be moved to the Trash and replaced with the version from
  Yesterday 18:02. It stays in the history too.", responses Cancel (default)
  and Replace (destructive).

### Deleted Files… dialog

Deleted files under the current folder (relative path, deleted date, size)
with **Restore** per row. Empty state: "No Deleted Files". Restore → toast
"Restored “draft.md”", or "Restored as “…”" when the name was taken.

### Properties

On a versioned root: a **Version History** row ("12.3 MB, 214 versions of
37 files", D20) and a row with **Clear History…** and **Stop Keeping
Versions…**, each confirmed with Cancel as the default:

- "Clear Version History?" / "All earlier versions of the files in
  “Projects” will be moved to the Trash. The current files are not changed."
  / [Cancel] [Clear History] (destructive).
- "Stop Keeping Versions?" / "Earlier versions will be moved to the Trash
  and new changes won't be recorded. The current files are not changed." /
  [Cancel] [Stop Keeping Versions] (destructive).

On a file inside a versioned folder: a **Versions** row with the count.

Folder size (D10): `properties::folder_size` skips `.versions/` directories;
for a versioned root the dialog shows the walked size plus
`history_info.bytes`.

## Error handling

| Situation | Behaviour |
|---|---|
| Foreign or unmarked `.versions/` (D14) | Ignored everywhere; Clear/Stop/creation refuse to touch it. |
| Index corrupt, newer schema, unreadable | Not watched; Versions… shows the error; never recreated automatically. |
| Lock held by another instance (D16) | Read-only: no capture; Replace/Clear/Stop refuse. |
| Invalid path/hash/object row | "This folder's version history is damaged."; nothing written. |
| Missing object or hash mismatch | "This version is damaged and can't be restored."; nothing written. |
| Disk full | Capture leaves its temp file for the next `tmp/` cleanup; no row; retried on the next change. |
| File unreadable, locked, or slow (e.g. iCloud dataless) | Capture runs on the blocking pool; on error, skip; the next event or scan retries. |
| WAL not available at creation (D18) | Creation refused; folder kept (D13). |
| Watcher can't be created | 5-minute periodic scan for that root. |
| `versioned.json` unreadable | Treated as empty; roots are re-found on navigation. |
| Restore target taken | Next free name (Restore Copy) or Trash first (Replace). Never overwrite. |
| Replace: rename fails after Trash move | Error names the Trash and the staged file; both kept. |

Backend logs use `eprintln!`, as elsewhere.

## Performance

- Capture, scan, rebuild, diff and transfer run on Tauri's blocking pool via
  `blocking(…)`.
- The capture lock serializes writes per history; reads use the reader
  connection and never wait on a capture (see "Locks and connections").
- Full copies stream; deltas hold at most base + new, each ≤ 128 MiB.
- Browsing a non-versioned folder costs one `versioned_root` call (a few
  `stat`s up the tree).
- The launch scan does a `stat` per file and reads only changed files;
  inserts are batched.
- Watchers: FSEvents is cheap; inotify uses one watch per directory, with the
  periodic-scan fallback when the limit is hit.

## Testing

Unit tests next to each module (`#[cfg(test)]`, `tempfile`), runnable
without Tauri: the scratch-crate rule in AGENTS.md applies, with
`rusqlite`, `zstd`, `blake3`, `notify`, `similar`, `trash`, `serde`,
`serde_json` allowed. Use test hooks (an injected clock, an injected delay in
capture, a fake reader) where timing or platforms can't be reproduced.

**store**
- [ ] Full and delta round trips; a 19-delta chain rebuilds; the 20th version is full.
- [ ] Delta ≥ 90 % falls back to full.
- [ ] A tampered object, a frame expanding past `size`, and a missing object each fail without output.
- [ ] A 200 MiB file is stored through the streaming path (assert via a counter hook) without buffering it.

**index**
- [ ] Schema and `meta` creation; a newer `schema_version` is refused.
- [ ] CHECK constraints reject a `full` row with a base and a `delta` without one.
- [ ] Hand-crafted bad rows (`../escape`, absolute path, 63-character hash, delta with no base) are rejected by validation.
- [ ] Online backup produces an index that passes `integrity_check`.

**history**
- [ ] A `.versions/` without our `meta` is not opened as ours; one with it is.
- [ ] A second `History::open` on a locked history comes back read-only.
- [ ] `make_versioned_dir` over an existing `.versions/` fails and leaves its contents untouched; a failure after `mkdir` keeps the folder.
- [ ] A simulated `journal_mode` failure refuses creation and registers nothing.

**capture**
- [ ] Unchanged size/mtime records nothing; a touch with the same content updates `mtime` only.
- [ ] Identical content in two files stores one object.
- [ ] Deletion records one tombstone (not two on a repeated event).
- [ ] A file replaced by a folder or symlink is tombstoned (D17).
- [ ] Ignore list, internal paths, owned nested histories and other filesystems are skipped; a foreign nested `.versions/` is skipped as internal but its siblings are tracked.
- [ ] A sparse file over 1 GiB is skipped without reading it.
- [ ] A file changing during capture is re-queued.
- [ ] NFC and NFD names that look identical get separate histories; a non-UTF-8 name on Linux is recorded with its exact bytes (D15).
- [ ] With a slow capture in progress (delay hook), `list_versions` and `versioned_root` return promptly.

**scan / tombstones**
- [ ] Scan picks up new, changed and deleted files, and leaves unchanged ones alone.
- [ ] Deleting a folder tombstones every file under it immediately; renaming one tombstones the old paths and records the new ones.
- [ ] Deleting a nested owned root does not tombstone anything in the outer history.

**watch**
- [ ] The coalescer fires once per path after the quiet period (injected clock).
- [ ] Overflow triggers a root scan; directory events trigger prefix tombstones.

**restore**
- [ ] Restore Copy never overwrites: a taken name gives ` 2`, ` 3`; it fails safely if the frontend name is invalid.
- [ ] Replace records the current file first, sends it to the Trash (both Trash layouts, as `trash_ops` tests do) and leaves no extra version for staging files.
- [ ] A rename failure after the Trash move keeps the staged file and the Trash entry.
- [ ] Restoring a deleted file recreates missing parent folders.
- [ ] A damaged version writes nothing.

**diff / current**
- [ ] Text, Same, Binary, TooLarge (decided without calling `store::read`, via a hook), Deleted, NotOnDisk, truncation at 2,000 lines.
- [ ] Two rows with identical content: only the latest can be Current; a deleted file has no Current row; listing a 1 GiB file does not hash it.

**history_info**
- [ ] Two identical files plus one tombstone: `bytes` counts the shared object once; `versions` and `files` follow D20.

**transfer / registry**
- [ ] Copying a versioned folder while a capture is in flight: the copy opens, `integrity_check` is `ok`, a version restores, no `-wal`/`-shm` was copied, and the copy has a new `folder_id`.
- [ ] Cross-volume move (simulated through the copy fallback) leaves a usable history and no source.
- [ ] Move/rename a root, move a folder containing roots, move a root out of a versioned parent: registry and watcher follow.
- [ ] Trashing a root unregisters it first; `versioned_root` never registers paths in the Trash; missing roots are pruned at launch.

**hiding**
- [ ] `.versions` and anything under it are absent from listings, counts, Starred and Recent, even with `show_hidden = true`.
- [ ] `folder_size` skips `.versions/`.

**Frontend** — `npx tsc --noEmit`, `npm run build`, and a Playwright pass
in `npm run dev:mock` (Mac user agent): checkbox enabled/disabled;
Versions… row selection (including fast clicking, which must not show a
stale diff), every diff message, Restore Copy, Replace confirmation;
Deleted Files…; Properties rows; light and dark mode.

## Platform notes

- **macOS** (unverified from Linux; say so when reporting): FSEvents batches
  events and may report a directory instead of a file, which the directory
  path handles. Full Disk Access limits which folders can be watched. iCloud
  "Optimise Storage" can make reads block or fail; capture retries.
- **Linux** (unverified on a Mac): one inotify watch per directory; the
  periodic-scan fallback covers `max_user_watches`. Trash uses the
  freedesktop layout; tests cover it as `trash_ops` does.
- Both targets are Unix, so `OsStrExt::as_bytes` gives exact path bytes (D15)
  and `0700`/`0600` permissions apply.

## Delivery

One feature branch, e.g. `feat/keep-versions`. Each step builds and passes
tests on its own:

1. Crates added (`Cargo.lock` minimal); `index.rs` + `store.rs` with tests.
2. `history.rs` (lock, ownership, connections) + creation, with tests.
3. `capture.rs` (record, tombstones, scan) with tests.
4. `watch.rs`, registry, startup wiring in `lib.rs`.
5. `is_internal_path` in listings/counts/Starred/Recent/folder size;
   `make_versioned_dir`, `versioned_root`; New Folder checkbox; mocks.
6. `restore.rs` + diff; Versions… and Deleted Files… dialogs.
7. Copy/move/rename/trash hooks in `state.rs` and `lib.rs`; registry matrix tests.
8. Properties rows, Clear History…, Stop Keeping Versions….
9. README: feature entry marked Experimental, the Known limitations above,
   and "Not supported yet" entries for the non-goals. AGENTS.md: new files in
   the Layout table, crates in the scratch-crate note, and "Anything that
   touches `.versions/` must check ownership (D14)" under the data rules.

Before finishing: `npx tsc --noEmit`, `npm run build`, `cargo test` pass, and
the UI has been checked in `npm run dev:mock`. Commits use the conventional
style.

## Acceptance criteria

1. Creating a folder with Keep versions ticked produces an owned
   `.versions/` (marker, lock, WAL index), registered and watched; on a
   non-local disk it is refused and the folder is kept.
2. Saving a text file three times (from any editor) gives three versions;
   Versions… lists them with correct dates and sizes, marks only the latest
   Current, and shows a correct diff against the current file.
3. Restore Copy creates a correctly named copy, byte-identical to the chosen
   version; nothing is overwritten.
4. Replace… asks first (default Cancel), sends the current file to the Trash,
   and the file then matches the chosen version; the replaced state is in
   Versions….
5. Deleting a file or a whole folder shows the files in Deleted Files… right
   away, and Restore brings them back.
6. Editing a file while the app is closed and relaunching records the new
   state.
7. A 10 MB text file edited by one line ten times uses under 2× its size in
   `.versions/objects/`.
8. `.versions` never appears in listings, counts, search, Starred or Recent.
9. Clear History… and Stop Keeping Versions… move only an owned `.versions/`
   to the Trash, after a confirmation with Cancel as the default.
10. Copying or moving a versioned folder (same or other disk) gives a history
    that opens, passes `integrity_check` and restores versions.
11. A foreign `.versions/` is never registered, shown, cleared or trashed.
12. Browsing a non-versioned folder of 10,000 items is as fast as before.
13. All tests above exist and pass; type-check and build pass.
