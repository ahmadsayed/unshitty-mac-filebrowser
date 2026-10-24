# Keep Versions — Review of Revision 2

**Date:** 2026-10-24
**Reviewed document:** `2026-10-24-keep-versions-design.md` (Revision 2)
**Previous review:** `2026-10-24-keep-versions-design-review.md`
**Status:** Follow-up review. Revision 2 is much closer; the P0 items from the first
review are resolved, but a few new/remaining issues should be fixed before Step 1.

## Verdict

Revision 2 fixes every P0 from the first review, and the new decisions (D14–D20) are the
right ones. The design is now implementable, but I would not start coding until the points
below are resolved. Two of them are small spec edits (the `rusqlite` feature flag and the
lock-held-through-Trash ordering); two affect the schema or the open path
(`meta.json` and read-only connections); one affects a real user scenario (exFAT/FAT
restores).

---

## P0 items from the first review: resolved

| # | Item | Where it was fixed |
|---|---|---|
| 1 | Ownership marker | D14, creation flow, Clear/Stop re-check |
| 2 | Live-index copy/move | `transfer.rs`, online backup, registry matrix |
| 3 | Raw byte paths / non-UTF-8 | D15, `path BLOB`, `display`, validation |
| 4 | Directory tombstones | `tombstone_prefix`, watcher events, scan |
| 5 | Capture vs DB lock | `history.rs`, `capture_lock`, writer/reader |
| 6 | Cheap `current` | `VersionInfo.current` definition |
| 7 | Streamed large files / diff shortcut | D5, capture step 4, `TooLarge` from sizes |
| 8 | Restore staging | `.versions/tmp/`, hard-link no-overwrite |
| 9 | Network/cloud volumes | D18, WAL check |
| 10 | Local-time restore names | D19, frontend |

The one declined suggestion (orphan-object GC) is explained and acceptable: an orphan is
harmless, and not deleting files inside a history is consistent with the project's
anti-goal. `tmp/` cleanup is safe because the process holds the lock.

---

## Remaining blockers

### R1. `versioned_root` cannot prove ownership without opening the index

**Where:** D14, "Locks and connections" (`versioned_root never opens an index`),
registry table row 1, "Startup".

**Problem:** D14 says a `.versions/` is ours only if its index opens and the `meta` table
has `app` + `folder_id`. But "Locks and connections" says `versioned_root` never opens an
index and only uses the registry map plus `stat`. Those two statements cannot both hold for
a folder discovered on navigation (moved in from outside the app): to know whether the
`.versions/` is ours, you must read its metadata. Opening the DB just to check ownership is
also slow, and creating/taking `.versions/lock` before the check writes into a directory
that may belong to another tool.

**Suggested fix:** add a tiny `.versions/meta.json` (or `.versions/owner`) written
atomically at creation:

```json
{ "app": "unshitty-mac-filebrowser", "folder_id": "<128-bit hex>", "schema_version": 1 }
```

- `versioned_root` reads this small file (one `stat` + one `read`) and never opens the DB.
- `History::open` opens the index read-only, verifies the DB `meta` matches `meta.json`,
  then takes the lock and opens the writer. If either side is missing/mismatched, the
  directory is foreign or damaged.
- `clear_history`/`stop_keeping_versions` verify both `meta.json` and the DB row while
  holding the lock, immediately before the Trash move.
- `meta.json` also lets `versioned_root` report a corrupt/unreadable index without opening
  it, and avoids creating a lock file inside a foreign directory.
- The DB `meta` remains authoritative for migrations; the JSON file is the cheap gate.

**Tests:** a foreign `.versions/` with only `index.sqlite` is not registered; one with
`meta.json` but a mismatched DB row is refused; a corrupt index with a valid `meta.json` is
reported, not adopted.

---

### R2. Clear/Stop release the lock before moving `.versions/` to the Trash

**Where:** "Clearing and stopping", steps 2–3.

**Problem:** The sequence is: re-check ownership, stop watcher, checkpoint, close
connections, **release the lock**, then move `.versions/` to the Trash. Between release and
move, a second app instance (or the same user opening the folder) can acquire the lock and
start watching/capturing into the history that is about to be moved. On a slow Trash
operation (network volumes are excluded, but a large directory is still slow), this is a
real race. The history could be moved while another instance holds it open.

**Suggested fix:** hold the lock through the Trash move and release it only after
`.versions/` is gone. On Unix, moving a directory that contains an open locked file is
fine; the open `File` keeps the lock and the path is irrelevant. Order:

1. Acquire/re-check ownership while holding the lock.
2. Stop the watcher; checkpoint; close the DB connections.
3. Move `.versions/` to the Trash **while the lock file is still open and locked**.
4. Drop the lock handle (or, for Clear, keep the directory in place only if the move
   failed; never recreate over a failed move).
5. Clear: create the fresh `.versions/`, lock it, create the index, scan, start watcher.
   Stop: remove the registry entry only after the move succeeded.

**Tests:** a test hook pauses between the move and release, then a second `History::open`
must observe the lock and come back read-only; a failed Trash move leaves the registry and
lock intact. Also test Clear/Stop with a read-only second instance.

---

### R3. `rusqlite`'s `backup` API is not enabled by `bundled`

**Where:** Architecture dependency list, `transfer.rs`.

**Problem:** `bundled` compiles SQLite in, but the `backup` module is a separate
non-default feature in `rusqlite` (verified against 0.40.2: `backup` is listed separately,
and `bundled` only enables `libsqlite3-sys/bundled` + `modern_sqlite`). The transfer design
depends on the online backup API, so the crate line must be:

```toml
rusqlite = { version = "0.40", features = ["bundled", "backup"] }
```

Without `backup`, `rusqlite::backup` is not compiled and Step 1 fails.

Also, `folder_id` is "random 128-bit hex" but no randomness dependency is listed. Either
add `getrandom`/`uuid` (and update AGENTS.md's scratch-crate list), or read 16 bytes from
`/dev/urandom` (both targets are Unix) and hex-encode them. Do not derive it from a
timestamp/path alone if two copies of the same folder should be distinguishable.

**Tests:** a build check that `rusqlite::backup::Backup` is available; `folder_id` is 32
lowercase hex chars and differs between a folder and its copy.

---

### R4. Restore uses `fs::hard_link`, which exFAT/FAT local disks do not support

**Where:** "Restoring" (Restore Copy), D18.

**Problem:** D18 allows any local disk, including the exFAT USB stick that many users keep
photos and documents on. exFAT and FAT32 have no hard links; `fs::hard_link` fails with
`EPERM`/`ENOTSUP`. The restore then fails even though the history is healthy. The
"no-overwrite" intent is good, but the implementation assumes a filesystem feature D18 does
not require.

**Suggested fix:** place the file with `OpenOptions::new().write(true).create_new(true)`
and stream-copy from the staging file (staging is on the same filesystem, so this is a
local copy, not a cross-device copy). On `AlreadyExists`, advance ` 2`, ` 3`, …. Keep
`fsync`. Optionally try `hard_link` first and fall back on `Unsupported`/`PermissionDenied`
for speed, but the portable path must be the default/tested one.

Also set the restored file's mode to `0666 & umask` (or preserve the current file's mode
where it still exists) rather than inheriting the staging file's mode; if staging is
created `0600`, a hard-link restore would produce a `0600` copy, which is surprising for a
document.

**Tests:** simulate a hard-link failure (test hook or a temp dir on a filesystem without
links, if available) and restore still succeeds; a restored copy has readable permissions;
no-overwrite still holds under a same-second name collision.

---

### R5. Read-only WAL connections need a defined open path

**Where:** D16, `history.rs`, `index.rs` schema pragmas.

**Problem:** The second instance is "read-only", but the spec does not say how its
connection is opened. Setting `PRAGMA journal_mode=WAL` on a read-only connection is a write
and can fail. A read-only SQLite connection to a WAL database also needs the `-shm` file
(which the owner has) and must not attempt to create or recover it. If the second instance
opens read-write without the lock, a bug could write.

**Suggested fix:** define two open modes in `index.rs`:

- **Owner:** `READ_WRITE | CREATE`, set `journal_mode=WAL` (verify `wal`), `synchronous`,
  `foreign_keys`, `busy_timeout`, then take the lock.
- **Reader/read-only:** `SQLITE_OPEN_READ_ONLY`, `busy_timeout`, `query_only=ON`; never set
  `journal_mode`/`synchronous`; never migrate; never clear `tmp/`.
- The owner's `reader` connection can use the read-only mode too (same process, `-shm`
  exists).
- If the read-only open fails because `-shm` is missing, report "This version history is in
  use; try again" rather than falling back to a writable connection.

**Tests:** a second connection opens read-only while the writer is committing; it sees a
consistent snapshot; it cannot write; it does not create a `-shm` or a `lock` file; the
owner's `tmp/` cleanup is not run by the read-only instance.

---

### R6. A failed creation after the `meta` commit is still "ours"

**Where:** "Creating a versioned folder" step 4, D13/D14.

**Problem:** The spec says a partly created `.versions/` "has no `meta` row, so it is not
ours". But the flow creates the schema and `meta` in one transaction, then verifies WAL and
runs a test write/read transaction. If the WAL/test step fails after `meta` committed, the
directory *does* have the marker; a later `versioned_root` will adopt it, and Clear/Stop
would be allowed to trash it. That contradicts the intended "failed creation is inert"
state.

**Suggested fix:** order the checks so the marker is the last thing committed:

1. Create `.versions/`, `tmp/`, `objects/`, `lock`.
2. Create the DB and schema **without** `meta`; set/verify WAL; run the test transaction.
3. If all of that passes, insert `meta` (app, folder_id, schema_version, created_at) in a
   transaction.
4. Only then register and start the watcher.

If step 3 fails, leave the directory (D13) and report; it has no marker, so D14 ignores it.
If the marker is written but registration/watcher fails, the history is valid; the next
`versioned_root` can adopt it, which is fine.

**Tests:** inject a WAL/test failure after schema creation but before `meta`; the directory
is left, is not registered, and is not considered ours. Inject a failure after the meta
commit; the history is valid and can be adopted.

---

### R7. `tombstone_prefix` needs a real BLOB prefix query

**Where:** "Tombstoning a directory".

**Problem:** "BLOB prefix match on the bytes, with the `/` separator" is not SQL. `LIKE`
and `GLOB` are text operators; applying them to a `BLOB` column is not a reliable or
indexable prefix match. With `versions_by_path(path, id)`, a range query is the efficient
answer:

```sql
-- Children of rel_dir: prefix = rel_dir bytes + b'/'
WHERE path >= :prefix AND path < :upper
```

A correct `:upper` for a bytewise BLOB index is the prefix with its last byte incremented
(and carries), or, specifically for a directory prefix, `:prefix` with `'/'` replaced by
`'0'` (0x30) because every child starts with `prefix + 0x2F + anything`, and `0x2F < 0x30`.
The latter works for any child name, including one starting with `!` or a byte below `/`.
Do not use `prefix || X'FF'` as the upper bound if a filename can start with `0xFF`.

- Exclude the directory itself (`path = :prefix_without_slash`).
- Test `foo` vs `foobar` (must not match `foobar`), and children named `!x`, `0x`, `z`.
- Add a test with a path whose name contains a `/`-adjacent byte.

Alternatively, add a `path_text` column with a reversible or lossy-but-unique encoding for
`LIKE`; the range query is cheaper and needs no schema change.

---

### R8. `version_diff` and `list_versions` need bounded work and short DB locks

**Where:** "Locks and connections", commands, diff.

**Problem A:** `list_versions` returns every row for a path. With retention forever and a
coalescing window of 2 s, a script that saves every few seconds for a year can produce
hundreds of thousands of rows. The UI renders 1,000, but the IPC payload is unbounded.
Add `limit`/`offset` (or "newest N + total") to the command; the index can return the newest
1,000 with `ORDER BY id DESC LIMIT 1000`, and `history_info` already has the total.

**Problem B:** `version_diff` must not hold the reader connection while `store::read`
rebuilds a 19-delta chain. Read the chain metadata under the reader, release it, rebuild
the bytes, then take the reader again if needed. Otherwise a slow diff blocks
`list_versions`/`deleted_files` in the same history. The same rule applies to `restore`:
short DB lookups, long file I/O outside the lock.

**Tests:** `list_versions` with 5,000 rows returns the requested page and total; a diff
with an injected slow `store::read` does not block a concurrent `list_versions` (test
hook).

---

### R9. Canonicalise roots; symlinked paths can create two histories

**Where:** registry, `versioned_root`, transfer.

**Problem:** On macOS `/tmp` is a symlink to `/private/tmp`; a user's home or a project
folder can be reached through a symlink. If one code path registers `/tmp/x` and another
sees `/private/tmp/x`, the same history is opened twice (the second is read-only due to the
lock, so capture silently stops), or a move/rename rewrites the wrong registry prefix.
Case-insensitive APFS can produce the same issue with different casing.

**Suggested fix:** canonicalise the root when registering/opening (`fs::canonicalize`,
which resolves symlinks) and store the canonical path in `versioned.json`; keep the path
the user navigated to only for display. `versioned_root` should canonicalise the candidate
root before comparing/registering. Prefix rewrites must use canonical paths and a component
boundary (`/foo` must not match `/foobar`).

**Tests:** a root reached through a symlink registers once and both paths resolve to the
same `History`; moving the symlink vs the target behaves predictably; `/foo` vs `/foobar`
prefix test.

---

## Follow-ups worth adding

### F1. External Finder copies of a live history

The transfer design covers copies made by the app. A Finder "Duplicate" or a copy to an
external drive copies `index.sqlite`, `-wal` and `-shm` while the app may be writing, and
the copy can be inconsistent. On open, `integrity_check` fails and the spec says never
recreate; the history is effectively lost (the working files are intact). Add a
"Known limitations" line ("copy versioned folders with the app, or while it is not
running") and a recovery attempt at open: if the DB fails, try removing a stale `-shm`
and reopening (SQLite can usually recover a copied WAL), then report. Never delete
`.versions/`.

### F2. `history_info.bytes` and nested histories

D20 defines `bytes` as `SUM(stored_size)` over objects. That omits `index.sqlite`,
`-wal` (and `-shm`), so the "Version History" row and the folder-size sum are not fully
honest. Include the index and WAL sizes (and document that `-shm` is transient), and define
whether a versioned root's Properties includes the histories of nested owned roots (the
outer walk skips all `.versions/`, so it currently does not). Add a test with a nested
root.

### F3. Type-changed and deleted history reachability

D17 tombstones a path that becomes a folder/symlink, but `deleted_files` only returns
tombstones where "no regular file exists now". If a folder now occupies the path, the
tombstone is hidden, and the file's history is unreachable in the UI (`Versions…` is only
on existing files, `Deleted Files…` filters it out). Either include such tombstones in
Deleted Files with Restore Copy only (the original path is occupied), or add a
"Versions…" action on Deleted Files rows so earlier versions of a deleted file can be
browsed. The same gap applies to a deleted file whose parent folder is gone.

### F4. `record` should use an open handle, not path + stat + read

Between `symlink_metadata` and `read`, the path can be swapped (editor atomic save, another
process). Open the file once, `fstat` the handle, hash/compress from that handle, and
`fstat` again after the read; compare the handle's identity/mtime, not a later path
lookup. This avoids capturing a different file under the same path and is cheap. The same
applies to the file-size cap: use the handle's metadata.

### F5. Missed changes when mtime is preserved

Step 3 skips when `size` and `mtime` match. Tools that preserve mtime (`rsync -t`, some
build systems, a `cp -p`/restore) can change content without changing either, so the
change is never captured. Both targets are Unix; storing `ctime` (which changes on writes)
in a `latest`/`versions` row and comparing `(size, mtime, ctime)` catches this. It also
re-reads on chmod/chown, which is harmless. If ctime is not added, state the limitation.

### F6. Coalescing loses rapid saves

The 2 s quiet period intentionally collapses an editor's temp+rename save, but it also
collapses two genuine saves within 2 s. The user's success criterion is "every saved
version"; a quick Ctrl+S twice loses the intermediate state. Either document this as a
known limitation ("versions are captured after 2 s of quiet; rapid saves may merge") or
lower the quiet period for repeated distinct hashes. Do not silently promise every save.

### F7. `fs_ops` ↔ `transfer` layering

`transfer.rs` is described as "called from `fs_ops` copy/move", but it needs the object
copy walker and `fs_ops::copy_recursive` is also used for ordinary copies. A module-level
cycle is legal in Rust but makes `fs_ops` history-aware and changes the behaviour of every
copy. Prefer orchestration in `lib.rs` (or a `versions::transfer` that wraps the copy and
takes a filter callback), keeping `fs_ops` generic. Cross-volume `move_entry` has its copy
fallback inside `fs_ops`, so expose a hook or move the fallback into the orchestrator.
Add a test that copying a non-versioned folder is byte-for-byte unchanged and costs no
extra stats.

### F8. Read-only mode and Restore Copy/Deleted restore

D16 lists Restore Copy as read-only-safe, which is right (it writes only the working
folder). Add `restore_deleted` to that list, or state that it also works. Replace/Clear/
Stop must refuse in read-only mode, with the message in the dialog. The Deleted Files
dialog should disable nothing except a "Replace" action if one is added.

### F9. Restore name validation and parent

`restore_version_copy(path, version, name)` must validate `name` as a single file name:
no `/`, no NUL, not `.` or `..`, not `.versions`, not empty; and the target must be the
original file's parent, not an arbitrary path. `restore_deleted(path, fallback_name)` needs
the same. Add tests with `../escape`, `sub/file`, `.versions`, and an absolute path.

### F10. `VersionInfo.current` when the watcher is behind

If the file changed but the watcher hasn't captured it, the latest row's mtime differs, so
no row is Current. That is honest, but the dialog may look like it has no current version.
Consider a small "current file not recorded yet" note, or trigger a synchronous `record`
when opening Versions… (expensive). At minimum, document the behaviour. `current` should
never hash the file (it doesn't).

### F11. Restored permissions

As in R4: the staging file's mode should be `0666 & umask` (or the current file's mode
when replacing), not `0600` inherited from staging. Test on Unix.

### F12. `list_versions` for deleted files

The spec says it works for files no longer on disk, but the UI only reaches it from an
existing file's context menu. Deleted Files should offer it (see F3), or remove the claim.

### F13. `versioned.json` and registry failure

If the registry write fails after a successful creation, the folder is versioned but not
watched until the next navigation/launch. Report it or retry; don't claim "turned on"
silently. Prefix rewrites should use path components and canonical paths (R9).

### F14. Scan tombstone pass cost

The launch scan stats every indexed path to find deletions. With a long history and many
deleted files, that is more stats than the folder has files. It is bounded by the number of
distinct paths, not versions, but it grows forever (retention). Consider a `latest` table
(path, id, hash, size, mtime) to make this one scan, or accept and document; the current
`(path, id)` index makes the latest-per-path query efficient, but the `stat` loop is the
cost.

### F15. Checkpoint/close ordering

`PRAGMA wal_checkpoint(TRUNCATE)` can block on a reader. Close the reader first (or let it
finish), set `busy_timeout`, and treat a failed checkpoint as non-fatal (the DB is still
valid); do not fail Clear/move because a checkpoint could not truncate.

---

## Verified facts

- **Toolchain:** local `rustc`/`cargo` are 1.99.1 (2026-10-13), matching the spec's
  assumption. `File::try_lock` is stable since 1.89.0 and returns
  `Result<(), std::fs::TryLockError>` with `WouldBlock`/`Error`; open the lock file
  read-write (the docs say behavior is unspecified if it is not open for writing).
- **`rusqlite`:** current 0.40.2; `backup` is a non-default feature (R3). `bundled`
  enables `libsqlite3-sys/bundled` and `modern_sqlite`, not `backup`.
- **`zstd`:** 0.14.3 exposes `Encoder::with_ref_prefix` and
  `Decoder::with_ref_prefix`; the prefix-delta design is supported.
- **`openDialog`:** already has `extra`, `className`, and `setResponseEnabled`; the
  Versions dialog needs no new primitive.
- **`File::create_new`:** atomic no-overwrite, available since 1.77; a better portable
  default than `hard_link` for R4.

## Tests to add for revision 2

- [ ] Foreign `.versions/` with only an index is not registered; mismatched `meta.json` is
      refused (R1).
- [ ] Clear/Stop hold the lock through the Trash move; a second instance sees read-only
      (R2).
- [ ] `rusqlite` builds with `backup`; `folder_id` is 32 hex chars and differs per copy
      (R3).
- [ ] Restore works when hard links are unsupported (R4).
- [ ] Read-only WAL connection sees a consistent snapshot and cannot write (R5).
- [ ] Failure before `meta` leaves an inert `.versions/`; failure after `meta` leaves an
      adoptable history (R6).
- [ ] BLOB prefix tombstone matches `foo/...` but not `foobar`, and handles `!`, `0`, `z`
      children (R7).
- [ ] `list_versions` is paged; a slow diff does not block a listing (R8).
- [ ] A symlinked root registers once; `/foo` vs `/foobar` prefix rewrite (R9).
- [ ] Finder-style copy of a live folder reports damage and never deletes `.versions/`
      (F1).
- [ ] `history_info.bytes` includes the index/WAL; nested histories counted or defined
      (F2).
- [ ] Type-changed path history is reachable, or the limitation is documented (F3).
- [ ] Capture uses an open handle and rejects a swapped path (F4).
- [ ] mtime-preserving content change is caught or documented (F5).
- [ ] Restore name validation (`../`, `sub/file`, `.versions`, absolute) (F9).
- [ ] Non-versioned copies are unchanged and add no stats (F7).

## Suggested spec edits

1. Add `.versions/meta.json` (R1), and state that `versioned_root` reads it, not the DB.
2. Hold the lock until after the Trash move in Clear/Stop (R2).
3. Change the dependency line to `rusqlite = { features = ["bundled", "backup"] }` and
   add the folder-id randomness source (R3).
4. Replace `hard_link` with `create_new` + stream copy, or document the fallback (R4).
5. Add a "Read-only open" paragraph to `index.rs`/`history.rs` (R5).
6. Reorder creation so `meta` is the last commit (R6).
7. Specify the BLOB prefix range query (R7).
8. Add `limit`/`total` to `list_versions`; keep DB locks short during diff/restore (R8).
9. Canonicalise roots in the registry (R9).
10. Add F1–F6 to "Known limitations" or the design.

## Bottom line

Revision 2 is approved in spirit. Fix R1–R5 before Step 1; R6–R9 before the modules they
touch; F1–F15 can be handled as they come up, but F1 (Finder copies) and F3 (reachability)
deserve a line in the spec because they affect what the feature promises.

If these are applied, I would start with Step 1 (`index.rs` + `store.rs`) immediately.

---

## Addendum after Step 1 (2026-10-24)

Step 1 is implemented: `versions/mod.rs`, `versions/index.rs`, `versions/store.rs` and
their tests (33 at the time of writing). The review fixes R3, R5, R6, R7 and R8 are in
the code; the spec's revision 3 notes the rest.

One new issue was found while writing the store tests, and it is a **new P0 for capture**:

### The object name does not encode the storage kind

Objects are named by the BLAKE3 of the **decoded** content. A `full` frame and a `delta`
frame of the same bytes therefore have the same `objects/<hash>` path. If capture writes a
full copy after the same content was stored as a delta, it overwrites the delta frame that
an existing row still describes; `store::read` then decodes it with the wrong prefix and
reports "damaged".

The implementation fixes the store side by adding `write_full_unless` /
`write_delta_unless`, which take an `accept(&hash)` callback. Capture must pass a check
against the `objects` table: when the row exists, the temp frame is discarded, `None` is
returned, and capture inserts only a `versions` row using the existing row's
`kind`/`base`/`depth`. A file with no row is an orphan and may be overwritten under the
folder lock. The spec's capture steps now say this.

**Required test in Step 3:** store a change as a delta; capture the same content again
(for example a copy of the file, or a revert); assert the `objects` row is unchanged, the
file is not rewritten, and both versions restore. Also test an orphan object file with no
row is replaced and the new row describes the installed encoding.

### Remaining Step 2 items

- `.versions/meta.json` (R1) and the DB `meta` cross-check.
- Lock held through the Trash move (R2).
- Canonical roots (R9) and the `versioned_root` registration cache.
- `history_info.bytes` including the index/WAL and nested roots (F2).
- `list_versions` + `version_count` commands wired, with the page size clamped.

---

## Status after implementation (2026-10-24)

All delivery steps are now in the tree: `history.rs`, `capture.rs`, `watch.rs`,
`restore.rs`, `registry.rs`, `state.rs`, the Tauri commands, `src/versions.ts`,
the New Folder checkbox, context menus, Properties rows, mocks and README/
AGENTS updates. The review fixes R1–R9 are implemented; the tests grew to 162
backend tests. Copy/move uses the **quiesce** variant (stop, checkpoint, close,
copy, reopen with a new `folder_id`) rather than a page-by-page online backup;
it is simpler and equally safe for a local history.

Deliberately still open, documented in the README:

- A Finder copy of a live versioned folder can copy an inconsistent index; use
  the app, or close it first.
- `size`/`mtime` skip means a tool that preserves mtime can hide a content
  change (F5).
- The 2 s coalescer merges rapid saves (F6).
- Watchers are not capped; Linux inotify limits fall back to scans.
- Type-changed/deleted histories are not reachable from the UI in every case
  (F3).


