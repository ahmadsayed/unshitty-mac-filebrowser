# Operation Progress — Design

**Date:** 2026-10-25
**Status:** Implemented (plan `docs/superpowers/plans/2026-10-25-operation-progress.md`).
**Related:** `2026-10-24-google-drive-design.md` (M3 promised local ↔ Drive
transfers "with progress" and cancellable downloads; never built),
`2026-10-25-iroh-sharing-design.md` (planned an "Opening… 45%" toast with
Cancel), `AGENTS.md` (rules every change must keep)

## Intent and the why

Copying between Google Drive and a local folder shows nothing but a
"Copying “name”…" toast after 0.6 s: no amount, no time left, no way to stop
it. A large SSH download looked hung for the same reason. The user wants the
progress display GNOME Files has: an operations button in the header bar
whose popover lists each running operation with a progress bar, how much is
done, the time left and a cancel button.

Success: every copy, move, trash batch, empty-Trash and remote open that
takes more than a moment shows live progress in one place and can be
cancelled, and a cancel never leaves a half-written item in place or loses a
source.

## Goals

- One operations button and popover for all long operations.
- Byte progress (with total and time left) wherever bytes can be counted;
  item progress elsewhere.
- Cancel for every operation, ending through the existing failure cleanup.
- A second paste or drop during a transfer is queued, not silently dropped.

## Non-goals

- Running transfers in parallel (they stay one at a time).
- Pausing or resuming.
- Progress for listing folders, thumbnails, Keep Versions scans or
  Properties' folder size.
- Holding the SFTP connection lock less (browsing a server still waits while
  a file from it downloads).

## Scope

| Operation | Progress | Cancel takes effect |
|---|---|---|
| Local copy / cross-volume move | bytes | Linux: within a file; macOS: after the current file (D4) |
| Same-volume move (rename) | items | between items |
| Local → Drive (upload) | bytes | between upload chunks |
| Drive → local (download) | bytes | between read chunks |
| Drive → Drive copy / move | items (server-side) | between items |
| Open a Drive / SSH / shared file | bytes | between read chunks |
| Move to Trash (a batch) | items | between items |
| Delete Permanently / Empty Trash | items | between items |

## Decisions

**D1. A plain-Rust `Progress` handle.** A cheap-to-clone shared value (an
`Arc` of atomics): `bytes_done`, `items_done`, a phase (`Preparing`,
`Working`) and a cancel flag. Modules (`fs_ops`, `cloud::transfer`,
`cloud::drive`, `sftp`, `share`, `trash_ops`) take `&Progress`, add to it as
work happens and call `check()` (an error when cancelled) between chunks and
items. No Tauri types in those modules; tests build a `Progress` directly.

**D2. Cancel is a failure with a known message.** `check()` returns the error
"Cancelled." through the path a failure already takes, so existing cleanup
runs unchanged: a staged copy or upload is removed (a Drive staging item goes
to Drive's Trash), a download's staging file is deleted, an open's temporary
file is deleted, and a move removes its source only after the copy fully
succeeded. The frontend treats "Cancelled." as a cancel, not an error.

**D3. `lib.rs` wires it to a Tauri `Channel`.** The long commands gain
`op: String` and `progress: Channel<ProgressUpdate>`: `copy_entry`,
`move_entry`, `trash_entries`, `delete_permanently`, `empty_trash`,
`drive_open`, `sftp_open`, `share_open`. A registry in managed state maps
`op` to its `Progress`; `cancel_operation(op)` sets the flag (cancelling an
unknown or finished op does nothing). While a command runs, a ticker sends
`{ bytes, items, phase }` at most every 100 ms, and once more when it ends;
the registry entry is removed when the command returns. An operation may span
several commands (a batch): every command of that batch uses the same `op`.

**D4. Local file copies keep the platform's fast paths.**
- Linux: a file is copied in 1 MiB slices with `io::copy` over
  `Read::take`, which std turns into `copy_file_range`, so btrfs still
  reflinks and progress and cancel work inside a file. Permissions are
  copied as `fs::copy` does.
- macOS: each file still goes through `fs::copy` (`fcopyfile`: keeps
  extended attributes, clones on APFS). Progress comes from polling the
  destination's size every 100 ms; cancel is checked before and after each
  file, so a single huge file finishes before a cancel takes effect.
- Symlinks and folders are counted as items and copied as now.

**D5. Measuring first.** A new `measure(paths) -> { bytes, items }` command
runs while the operation shows "Preparing…": local apparent sizes (symlinks
not followed), Drive sizes from the API (Google Docs count as items with no
bytes), the remote size for SSH and shared files. If measuring fails, the
operation continues with item progress only. Measuring is cancellable too.

**D6. Remote opens count bytes while reading.** Drive downloads and SSH
reads go through a counting reader in slices instead of one `io::copy`. A
shared file's download reports bytes if iroh-blobs exposes download progress
for the request; otherwise it counts as one item.

**D7. Trash and delete batches count items.** A Move to Trash is one
`trash_entries` command for the whole selection. It trashes in as few calls
as possible, one per Trash folder and 100 items (`TRASH_CHUNK`). On macOS each
call is one Finder request, and Finder plays its sound once per request, so a
selection makes one sound rather than one per item. Items with the same name
go in separate calls, so each can still be found in the Trash for Undo. If a
call fails, the items it left are tried one by one, so each gets its own error.
Progress counts each item as it leaves its folder, which is checked every
50 ms while the call runs. A running call can't be stopped, so Cancel takes
effect before the next one; the rest end with "Cancelled.".
`delete_permanently` stays one call per item, all in one `op`; `empty_trash`
counts its top-level items. Cancel stops before the next item or call; items
already trashed or deleted stay so (trashed ones keep Undo). Both Trash
layouts are covered.

**D8. Transfers queue.** `runTransfer` no longer returns early while another
transfer runs: a new batch joins a FIFO queue, shows as "Waiting…" in the
popover and starts when the previous one ends. Cancelling a waiting batch
removes it. Remote opens don't queue.

**D9. The frontend owns the operation list.** Each operation records its id,
title, measured total, the bytes and items of finished items, the running
item's latest update, its start time and its state (waiting, preparing,
working, cancelling). The overall fraction is bytes when a byte total is
known, else items. Time left uses an exponentially smoothed speed and appears
only after 2 s of work.

## UI

**Operations button.** A flat button in `.hb-end`, before the current-folder
menu, visible while any operation exists and for 3 s after the last one ends
(unless its popover is open). Its icon is a symbolic pie filled to the
overall fraction of all operations (drawn from scratch in the Adwaita style;
`--accent-bg` fill, track at `--dim`); when everything has finished it
briefly shows a checkmark.

**Popover** (via `openPopover`), one row per operation:

```
 Copying 3 files to “Documents”                     [✕]
 ▓▓▓▓▓▓▓▓▓▓▓░░░░░░░░░░░░
 12.4 MB of 80.1 MB — 1 minute left
```

- Titles: "Copying “name”", "Copying N files to “Folder”", "Moving …",
  "Moving N files to the Trash", "Deleting N files", "Emptying the Trash",
  "Opening “name”".
- Detail: "Preparing…", "Waiting…", "12.4 MB of 80.1 MB — 1 minute left"
  (sizes via `format.ts`), "3 of 10 files", "Cancelling…".
- ✕ cancels (`cancel_operation`); the row reads "Cancelling…" until the
  command returns, then disappears. Finished rows disappear; failures keep
  the existing error toasts.
- The first operation still running after 1 s opens the popover by itself
  once; later ones only update the button.

The 0.6 s "Copying “name”…" toast is removed. Clash dialogs, Undo, the
read-only rules and wording elsewhere are unchanged. Colours are libadwaita
tokens with dark-mode values.

## Mock and tests

- `dev/mock-tauri/core.ts` exports a fake `Channel` (an `onmessage`
  callback) and `cancel_operation`, `measure`; the long mock commands accept
  `op`/`progress` and, when a test turns on slow mode, report progress in
  steps the test advances (`__mockOps.step()`), honouring cancel.
- Rust, per module: counters add up to the measured total; cancelling
  mid-file leaves no partial file and keeps the source; a cancelled
  cross-volume move removes nothing; a cancelled upload leaves no Drive item;
  a cancelled download or open leaves no staging or temporary file; a
  cancelled trash batch keeps finished items in the Trash (both layouts);
  `Progress` itself (counting, phase, `check`).
- Frontend (Vitest): button appears and hides; popover opens by itself once;
  row titles and detail text; Cancel ends without an error toast and keeps
  finished items; a second paste is queued and runs after the first; trash
  batch progress. New mock commands get cases in `tests/mock-core.test.ts`.
- Visual check in `npm run dev:mock`, light and dark.
- Not verifiable from Linux: D4's macOS path and Finder's Trash; reported as
  such.
