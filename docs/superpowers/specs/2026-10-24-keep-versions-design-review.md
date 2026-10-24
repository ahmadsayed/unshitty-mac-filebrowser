# Keep Versions — Design Review

**Date:** 2026-10-24
**Reviewed document:** `2026-10-24-keep-versions-design.md`
**Status:** Review only. The reviewed spec is still "Draft, pending approval"; nothing below changes it.
**Scope:** Architecture, data safety, correctness, performance, testability. Not copy-editing.

## Verdict

The design is coherent and close to implementable. Approach A (own object store + SQLite
index) fits the project: plain functions, testable without Tauri, content-addressed objects,
immutable history, tombstones, bounded delta chains, and Trash instead of delete.

I would not start Step 1 until the P0 items below are resolved. Items 1–3 change the
schema or the creation flow, so fixing them after `store.rs`/`index.rs` exist means rework.
Items 4–10 are implementation traps that are much cheaper to design out now than to debug
later.

---

## P0 — Resolve before implementation

### 1. `.versions/` ownership is not proven; Clear History could trash user data

**Where:** D7, D13, "Creating a versioned folder", "Clearing and stopping",
`versioned_root` in "Interplay with existing operations".

**Problem:** `versioned_root` registers any ancestor-or-self that contains `.versions/`.
If that directory was left by another tool, was partially created, or arrived with a folder
moved in from elsewhere, the app will adopt it. "Clear History…" and "Stop Keeping
Versions…" then move the whole directory to the Trash. That is user data, not our history.

**Suggested change:**

- Write an ownership marker atomically when the history is created, for example:
  ```sql
  INSERT INTO meta(key, value) VALUES
    ('app', 'unshitty-mac-filebrowser'),
    ('folder_id', '<uuid>'),
    ('created_at', '<unix seconds>');
  ```
  A `meta.json` next to the index works too, but the DB row is harder to separate from the
  index and is already covered by the "refuse newer schema" check.
- `versioned_root` must treat `.versions/` as ours only when the marker exists and matches.
  Otherwise ignore it (do not register, do not watch, do not show Versions…).
- `clear_history` and `stop_keeping_versions` must re-check the marker immediately before
  the Trash move and refuse with a clear error if it is missing or foreign.
- `make_versioned_dir` must refuse to write into an existing `.versions/`, even if the folder
  itself was just created (double-click, retry after partial failure). Keep the folder
  (D13) and return "The folder was created, but versions could not be turned on: …".
- A partial `.versions/` that *we* created but failed to finish should be cleaned up only
  when the marker proves ownership. Never `remove_dir_all` a directory we cannot identify.

**Tests:** foreign `.versions/` is not registered; Clear/Stop refuse it; a fresh creation
over a leftover `.versions/` fails without touching its contents; a partial creation is
reported and the folder is kept.

---

### 2. Copying or cross-volume moving a live versioned folder copies a live SQLite database

**Where:** "Interplay with existing operations", `copy_entry`/`move_entry` in `fs_ops.rs`.

**Problem:** `.versions/index.sqlite` is opened in WAL mode and written by the capture
queue. `copy_recursive` copies `index.sqlite`, `-wal` and `-shm` as ordinary files, in
directory order, with no quiescing. A file-level copy of a live WAL database is not a
supported snapshot: the copy can be inconsistent, can lose the last transactions, or can
fail to open. Cross-volume `move_entry` does the same copy and then deletes the source, so
the only good history can be destroyed by a bad copy. This is the most serious bug in the
spec's interaction with existing operations.

**Suggested change:** objects are immutable and content-addressed, so only the index needs
special handling.

- **Copy (same or cross volume):** while the source folder's capture lock is held, take a
  consistent snapshot with `rusqlite`'s online backup API into
  `<dest>/.versions/index.sqlite`; release the lock; copy `objects/` (an extra unreferenced
  object is harmless because objects are never deleted). Open the copy, run
  `PRAGMA integrity_check`, then run `scan(copy)` so the copied working tree and history
  agree. Do not copy `-wal`/`-shm`.
- **Same-volume move/rename:** stop the watcher, `PRAGMA wal_checkpoint(TRUNCATE)`, close
  the connection, `rename`, reopen at the new path, restart the watcher. Relative paths in
  the index are unaffected.
- **Cross-volume move:** snapshot + copy objects to the destination, verify the copy, then
  remove the source with the existing "only after the copy fully succeeded" rule.
- **Trash a root:** stop the watcher and unregister before the Trash move.
- Do not hold the capture lock across the entire object copy if it can be long; the backup
  snapshot only needs to be consistent with itself. Objects referenced by the snapshot are
  already on disk and are never deleted, so copying them afterwards is safe.

**Tests:** copy a versioned folder while a capture is in flight; the copy opens,
`PRAGMA integrity_check` returns `ok`, and restoring a version works. Cross-volume move
(with a simulated copy fallback) leaves a usable history and no source. Move a root and a
folder containing roots; the registry and watcher follow.

---

### 3. NFC-normalised paths break on Linux and non-UTF-8 names collide

**Where:** "Platform notes" (last bullet), `versions.path` in the schema, "Capturing one path".

**Problem:** The spec stores paths NFC-normalised and later joins them back onto the root.
That is safe on APFS, which is normalization-insensitive, but not on Linux: on ext4, NFC
`é` and NFD `é` are different byte sequences. A folder moved from macOS to Linux with NFD
names will not match its index (or, worse, a restore creates a second, visually identical
file). The stated goal — "so a folder moved between macOS and Linux keeps matching" — is
not achieved by normalising the stored path.

Separately, `to_string_lossy()` is lossy on Linux: two distinct non-UTF-8 names can collapse
to the same string, so two files share one history and a restore can write the wrong bytes
to the wrong path.

**Suggested change:** store the real relative path exactly as it exists on disk, and use a
separate normalised key only for *matching*, never for I/O.

- On Unix, keep `OsStr` bytes. If `path` must remain `TEXT`, use a reversible encoding
  (percent-encoding of the raw bytes is enough) or add a `path_bytes BLOB` column; do not
  use `to_string_lossy`.
- If cross-platform matching is wanted, add `path_key TEXT NOT NULL` (NFC, `/` separators)
  with an index, and resolve `path` for I/O. Do not join `path_key` onto the root.
- For v1, skipping non-UTF-8 names (with a log line) is an acceptable narrower policy, but
  it must be stated in the spec and tested; silent lossy conversion is not.
- `restore.rs` should take a `Path`/`OsStr` internally rather than rebuilding from a
  `String`.

**Tests:** a folder with NFD and NFC names that look identical, moved between the two
representations; a non-UTF-8 name on Linux is either skipped or preserved exactly, never
aliased. Restore writes to the path that was recorded.

---

### 4. Deleting or renaming a directory leaves stale, non-tombstoned paths

**Where:** "Watching" (folder-level events), `capture::scan`, D4.

**Problem:** A `Remove(dir)` or `Rename(dir)` event is handled by "run `scan` on that
directory". If the directory is gone, the scan walks nothing and cannot discover which
files used to be inside it. Their latest rows stay non-tombstone, so Deleted Files… does
not show them, and `deleted_files` reports nothing until a later launch scan (which *does*
compare index paths against the filesystem).

**Suggested change:** add a prefix operation and call it whenever a directory disappears:

```sql
-- For each path under :prefix whose latest row is not a tombstone,
-- insert a tombstone at :now.
```

Implementation can be a `SELECT path, MAX(id) FROM versions WHERE path LIKE :prefix || '/%'
GROUP BY path` followed by inserts for paths that do not exist. Run it for directory
Remove, directory Rename (old path), and for missing directories found by the launch scan.
A file rename that only reports the new path is handled by scanning the parent, which
already tombstones missing children; if the event reports both paths, tombstone the old one
directly.

**Tests:** delete a folder with several versioned files → each appears in Deleted Files…
immediately, not after relaunch. Rename a folder → old paths tombstoned, new paths
recorded. Delete a nested versioned root that the outer folder skips → the outer folder
does not tombstone it.

---

### 5. The database lock must not cover hashing, compression, or object writes

**Where:** "Performance" ("the folder's connection mutex serializes them"),
`VersionsState` in "Wiring in `lib.rs`".

**Problem:** A `Mutex<Connection>` held for a whole `capture` means `list_versions`,
`version_diff` and even `versioned_root` wait behind a hash + zstd encode of up to 128 MiB
(or a 1 GiB full copy). The Versions dialog will look hung, and navigation can stall.

**Suggested change:**

- One `capture_lock: Mutex<()>` per folder serializes captures.
- A writer connection behind a short-lived mutex/transaction for row inserts and latest-row
  reads.
- Read paths use a separate connection (WAL allows concurrent readers) or a small read
  pool. Never hold any connection across file I/O, hashing, compression or a directory walk.
- `versioned_root` needs the registry plus a `stat` for `.versions/`; it must not open the
  index or wait on capture.
- Keep the object store independent of the index: write the object first, then take the DB
  lock for the transaction.

**Tests:** start a slow capture (large file) and assert `list_versions` still returns
promptly; `versioned_root` does not block. This needs a test hook for the capture delay.

---

### 6. `current` is expensive and semantically ambiguous

**Where:** `VersionInfo.current` ("same hash as the file on disk now"), `list_versions`,
Versions… dialog.

**Problem:** Computing it by hashing the current file reads up to 1 GiB every time the
dialog opens. Also, if the user reverts content, two rows (an old one and the latest) can
share the current hash; both would be labelled Current and both would have Restore/Replace
disabled.

**Suggested change:** define `current = (row.id == latest non-tombstone id) AND
(row.size, row.mtime) == current file's stat`. If the file is missing or no longer a
regular file, no row is current. Content equality can be reported by `version_diff`
(`Same`) when a row is selected, where the bytes are rebuilt anyway.

Add a diff state for "the file is not on disk any more" (`CurrentMissing`/`NotOnDisk`) so
the dialog can say so and still offer Restore Copy. `list_versions` for a path that no
longer exists should work from the index alone.

**Tests:** two rows with identical content only mark the latest as Current; a touched file
(latest row updated) is Current; a deleted file has no Current row and the diff returns the
missing state; opening Versions… on a 1 GiB file does not hash it.

---

### 7. Large files: stream full copies; do not rebuild just to say "too large"

**Where:** D5, "Capturing one path" step 4, `version_diff`, `store::read`.

**Problem:** D5 allows regular files up to 1 GiB, and the capture flow says "Read the file",
which implies buffering it. A 1 GiB buffer plus a base for delta is a memory spike the
project has otherwise been careful to avoid. Also, `version_diff` would rebuild a chain
before discovering that either side is over the 1 MiB text limit.

**Suggested change:**

- **Full object:** stream the file through BLAKE3 and the zstd encoder into `tmp/`, fsync,
  then rename to `objects/xx/<hash>`. Only a delta buffers (base ≤128 MiB + new ≤128 MiB).
- **Delta eligibility:** check size and depth from the index first; if the file is over
  128 MiB, go straight to a streamed full copy.
- **Diff:** if `versions.size > 1 MiB` or `metadata(current).len() > 1 MiB`, return
  `TooLarge` without rebuilding. For ≤1 MiB, the chain rebuild is bounded.
- **`store::read`:** cap decompressed output at `objects.size`; error if the frame expands
  beyond it, and validate the final BLAKE3 hash before returning.

**Tests:** a sparse file near the 1 GiB skip boundary does not allocate its logical size;
a 200 MiB file is stored as a full object without a large allocation (watch peak RSS or
assert the streaming path via a counter); selecting an oversized version returns TooLarge
without calling `store::read` (a test hook or mock).

---

### 8. Restore staging under the watched folder will be captured as history

**Where:** "Restoring", "Replace…".

**Problem:** If the rebuilt bytes are written next to the target under a temp name, the
watcher records that temp file (the existing `.filebrowser-…` staging name is hidden but
not on the ignore list). That is history noise, and a partial temp file could be captured.
Also, `exists()` followed by `write` in Restore Copy is a race and can overwrite.

**Suggested change:**

- Stage under `.versions/tmp/` (for example `restore-<pid>-<n>`). It is on the same
  filesystem, invisible to capture, and rename into the target folder is atomic.
- Restore Copy opens the final path with `create_new(true)`; on `AlreadyExists`, advance
  the suffix and retry. Never write over an existing file.
- Replace: stage in `.versions/tmp`, move the current file to the Trash, rename the staged
  file into place. If the rename fails after the Trash move, report where the old file went
  and leave the staged copy; do not delete it.
- Add the `.versions/` prefix to the capture skip check by *path*, not only by name.

**Tests:** Replace leaves no extra version for a temp file; Restore Copy under a same-second
name collision produces ` 2` and never overwrites; a failed rename leaves the staged file
and the Trash entry recoverable.

---

### 9. WAL on network/cloud volumes needs an explicit policy

**Where:** "On-disk layout", "Platform notes".

**Problem:** SQLite WAL mode is not reliable on SMB/NFS and can fail to create or corrupt
under some remote filesystems. A versioned folder on a network share or a cloud-synced
folder is a realistic user choice. iCloud "Optimise Storage" also means a dataless file may
block capture while it downloads.

**Suggested change:**

- At creation, open the index and verify `PRAGMA journal_mode=WAL` returns `wal`; also
  perform a small write/read transaction. If that fails, refuse with "Version history needs
  a folder on a local disk."
- Document that versioned folders on network/cloud volumes are unsupported in v1; do not
  silently fall back to a mode that has its own locking problems.
- Capture must tolerate a slow/blocked read (the blocking pool handles it); log and retry
  on the next event or scan. Do not hold the capture lock while waiting forever if it can
  be avoided.

**Tests:** a simulated `journal_mode` failure makes creation fail without leaving a
registered root; a dataless/hanging read is retried by the next scan (can be a unit test
with a fake reader if the platform cannot be simulated).

---

### 10. Missing dependency for the local-time Restore Copy name

**Where:** "Restoring" (`<stem> (<YYYY-MM-DD HH-MM>)<.ext>` in local time), dependency list.

**Problem:** Rust's standard library has no local timezone conversion. The dependency list
(`rusqlite`, `zstd`, `blake3`, `notify`, `similar`) does not include anything that can
produce the local `YYYY-MM-DD HH-MM` string.

**Suggested change:** add `chrono` (or `time` with `local-offset`) to `Cargo.toml` and the
scratch-crate list, or have the frontend compute the name and pass it to
`restore_version_copy`/`restore_deleted`. The backend should still enforce no-overwrite.
Whichever is chosen, add it to the AGENTS.md scratch-crate note and the README credits if
the README lists dependencies.

**Tests:** a restore around a DST boundary still produces a non-empty, sortable name; the
name contains no `:` on macOS.

---

## P1 — Pin down before coding

### Registry operations need an explicit matrix

The spec says "update `versioned.json` (prefix replace for the root and any registered
folders under it)" but a move can go in several directions. Define and test:

| Operation | Registry action |
| --- | --- |
| Move/rename a registered root | Remove old prefix, add new prefix, close/reopen watcher |
| Move a folder containing registered roots | Rewrite the prefix of every registered root under it |
| Move a registered root *out* of a versioned parent | Remove old prefix, add the new path (it remains its own root) |
| Copy a registered root | Register the copy, snapshot the index, scan the copy |
| Copy a folder containing roots | Register every copied root under the destination |
| Trash a root | Stop watcher and unregister *before* the Trash move |
| Restore a root from Trash | Do not auto-register; `versioned_root` on navigation does it |
| `versioned_root` on a path under Trash | Never register |

Also add a `version` field to `versioned.json` and write it through the existing
temp-file+rename/config mutex.

### `history_info` accounting

- `bytes`: `SUM(stored_size)` over distinct `objects`, not over `versions` rows. Two files
  with identical content share one object; summing per version double-counts.
- `versions`: define whether tombstones count (I would exclude them; the UI says "214
  versions of 37 files").
- `files`: define whether deleted files count (I would say "files with at least one
  non-tombstone version", which includes files that were later deleted).
- Properties' folder size should skip `.versions/` in the recursive walk and add
  `history_info.bytes`; otherwise it walks gigabytes of objects and counts a live WAL.
- Add a test with two identical files and one tombstone.

### Watcher scale and startup cost

One watcher per registered root is cheap on FSEvents but one inotify watch per directory on
Linux; a large tree can exhaust `fs.inotify.max_user_watches` and trigger the 5-minute
fallback for everything. Decide:

- a cap on simultaneously watched roots (say 32), with periodic scan for the rest; or
- lazy watchers for roots under the current location plus those seen recently; or
- one recursive watcher per volume where the API allows it.

Document the battery/IO cost and the fallback in the spec's Performance section.

### Single instance and `tmp/` cleanup

"Clear `tmp/` at open" can delete another process's in-flight object. Tauri single-instance
is not enabled. Either enable it, use a lock file per `.versions/`, or name temp files with
the PID and only remove files older than an hour. Also decide what two simultaneous captures
do: SQLite's `busy_timeout` protects the index, but duplicate version rows are possible;
a unique constraint on `(path, hash, mtime)` would not prevent them because `recorded_at`
differs.

### Object garbage collection

Objects are written before rows. A crash leaves an orphan file. Retention is forever and
rows are never deleted except by Clear (which moves the whole directory), so orphans
accumulate. At open, after acquiring the folder lock, delete final-named object files with
no `objects` row (never touch `tmp/`). Do this before starting the watcher, and log the
count. Add a test for a crash-simulated orphan.

### Path safety on restore

A corrupt or foreign index must not escape `.versions/`:

- validate `versions.path`: relative, no `RootDir`/`ParentDir`/empty components, no NUL;
- validate `hash`: exactly 64 ASCII hex characters before joining `objects/xx/hash`;
- validate `objects.kind`/`base` consistency (a `full` row must have `base IS NULL`; a
  `delta` must have a base with `depth + 1`);
- set `.versions/` to `0700` and the index to `0600` on Unix.

Add tests with a hand-crafted bad index: `../escape`, an absolute path, a 63-character
hash, a delta with no base.

### Scan batching and WAL size

Per-file commits with `synchronous=FULL` make a 10,000-file launch scan slow. Use WAL,
`synchronous=NORMAL` (history is derived; a power loss can lose the last batch, not
corrupt), batch inserts into transactions of 100–500, and `PRAGMA wal_checkpoint(TRUNCATE)`
after a scan or a burst of captures. Keep the current "stat before read" rule so unchanged
files cost no I/O.

### Type changes and symlinks

"Not a regular file → done" leaves the old regular-file version looking current when a path
becomes a symlink or a folder. Decide: insert a tombstone when the path exists but is no
longer a regular file, or define the behaviour explicitly and make `current` false for those
paths. `deleted_files` currently requires the path not to exist, which would hide a
tombstoned file replaced by a folder; that is probably fine, but it should be stated.

### Frontend diff race

Debounced `version_diff` responses can arrive out of order when the user clicks rows quickly.
Use a monotonically increasing request token per dialog and ignore responses whose token is
not current. Also disable the row buttons while a rebuild is in flight, and show a spinner
row rather than a stale diff.

### Startup ordering

Open connections and start watchers on a background thread, one root at a time, after the
window is up. Do not block `setup` on opening every index. If a root's index is corrupt or
newer, leave `.versions/` untouched, log, and show the error in Versions… only; never
recreate it automatically, because that would discard history.

### `.versions` hiding in Starred/Recent

D10 lists listings, counts, search and Recent. Add a single `is_internal_path` helper used
by `read_entries`, `count_items`, `entries_for` (Starred), and `recent::collect`. Filter by
path *component* so a folder named `.versions` anywhere is hidden. Add a test for a starred
file inside `.versions` and a Recent path inside it.

---

## Verified facts and notes

- **zstd ref-prefix API exists.** `zstd` 0.14.3 (BSD-3-Clause) exposes
  `Encoder::with_ref_prefix(level, &[u8])` and `Decoder::with_ref_prefix(&[u8])`, so the
  spec's "check the exact method name" caveat can be removed. Keep the explicit
  window-log/`window_log_max` handling; a 128 MiB prefix plus 128 MiB input may need
  window log 28 rather than the current cap of 27, which affects ratio more than
  correctness.
- **`openDialog` supports what the Versions dialog needs.** `openDialog` already accepts
  `extra: HTMLElement` for the list + diff + buttons and `className` for the wider layout;
  no new dialog primitive is required. `setResponseEnabled` can disable Restore/Replace
  while a rebuild is running.
- **`copy_recursive` preserves permissions for directories and uses `fs::copy` for files.**
  That is fine for objects, but reinforces why the index needs the backup API rather than
  a raw copy.
- **The app already has a config mutex pattern** (`TRASH_LOG` in `lib.rs`); reuse it (or a
  dedicated one) for `versioned.json`, and write through `config_store::write_json`'s
  temp+rename.
- **`fs_ops::count_items` and `read_entries` are the only listing paths**; Starred/Recent
  use `entry_for`/`entries_for`, so the `.versions` filter needs to be added in both places,
  not just `read_entries`.
- **`properties::folder_size` walks everything recursively** (including `.versions`). It
  already skips other filesystems and counts hard links once; adding a `.versions` skip plus
  `history_info.bytes` is a small change with a large effect on huge histories.

---

## Suggested spec edits (kept separate from the spec)

If the author wants to apply these, the smallest set is:

1. **New decision D14 — ownership marker.** `.versions/` is ours only with a matching
   `meta.app`/`folder_id`; Clear/Stop refuse otherwise.
2. **New decision D15 — raw paths.** Store the on-disk relative path; if a normalized key
   is needed, keep it separate and never use it for I/O. State the non-UTF-8 policy.
3. **Amend D5/Capturing** to say full copies stream and only deltas buffer.
4. **Amend "Restoring"** to stage under `.versions/tmp/` and use `create_new`.
5. **Amend "Performance"** to separate `capture_lock` from DB access and to batch scans.
6. **Amend "Interplay"** with the registry matrix and the live-DB snapshot flow.
7. **Add "Known limitations (v1)"**: network/cloud volumes unsupported, non-UTF-8 names
   skipped or preserved (as chosen), one instance assumed (or enforced), history is not a
   backup.
8. **Add the extra tests** below to the Testing section.
9. **Add `chrono`/`time`** (or frontend-formatted names) to the dependency list and to the
   AGENTS.md scratch-crate note.

---

## Additional tests to add to "Required tests"

Beyond the spec's list:

- [ ] Foreign or unmarked `.versions/` is not registered; Clear/Stop refuse it.
- [ ] Copy a versioned folder during a capture: copy opens, `integrity_check` is `ok`,
      history restores; no `-wal`/`-shm` copied as live files.
- [ ] Move/rename a root, move a container of roots, move a root out; registry and watcher
      follow each.
- [ ] Delete a folder and rename a folder: every path under it is tombstoned immediately.
- [ ] Reverted content: an older row with the same hash is not marked Current.
- [ ] NFD vs NFC names and a non-UTF-8 name on Linux: no aliasing, restore hits the recorded
      path.
- [ ] `version_diff` when the current file is missing.
- [ ] Restore Copy under a name race never overwrites (`create_new`).
- [ ] `history_info.bytes` dedupes identical content; Properties' total counts history once.
- [ ] A corrupt index cannot escape `.versions/` via `../` or a bad hash.
- [ ] A crash-simulated orphan object is removed at open; a `tmp/` file is not.
- [ ] A 1 GiB file is skipped; a 200 MiB file is stored without buffering it (peak RSS or
      streaming-path counter).
- [ ] Slow capture does not block `list_versions`/`versioned_root`.

---

## What is strong

- The module split (`store`/`index`/`capture`/`watch`/`restore`) keeps Tauri out of the
  logic and matches the AGENTS.md testability rule.
- Object-before-row ordering plus immutable content-addressed objects is the right crash
  model; an orphan is harmless, a dangling row is not.
- Tombstones and "latest row per path" keep the schema small; no `prev` column is needed.
- D6's depth cap and 90 % fallback bound rebuild cost and avoid pathological deltas.
- D7 (Trash for Clear/Stop) and the Replace ordering respect the project's main anti-goal.
- The UI wording, the Cancel default, and the acceptance criteria are specific enough to
  implement and test.

## Open questions for the author

1. Is cross-platform history (a versioned folder moved macOS ↔ Linux) a real requirement,
   or only "it should not corrupt anything"? That decides raw paths vs a normalized key.
2. Is one app instance guaranteed? If not, decide on a lock or accept duplicate captures.
3. Should a type change (file → folder/symlink) tombstone the file, or leave the history
   visible as "not current"?
4. Should `history_info.files` count files that were deleted, or only files currently on
   disk? The label "versions of 37 files" is ambiguous today.
5. Is `chrono` acceptable, or would you rather the frontend generate the Restore Copy name?
