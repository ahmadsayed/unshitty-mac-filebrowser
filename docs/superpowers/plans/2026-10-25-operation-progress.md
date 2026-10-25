# Operation Progress Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A GNOME Files-style operations button and popover showing progress, time left and Cancel for copies, moves, trash batches, emptying the Trash and opening remote files.

**Architecture:** A plain-Rust `Progress` handle (atomics + cancel flag) is threaded through the backend modules; `lib.rs` registers it per operation id and streams snapshots over a Tauri `Channel`. The frontend keeps an operation list in a new `src/operations.ts` (pure model + widget) and `main.ts` drives it from the existing transfer, trash and open flows.

**Tech Stack:** Rust (Tauri 2 `ipc::Channel`, std `io::copy`/`copy_file_range`, reqwest blocking, ssh2, iroh-blobs 0.103), TypeScript + Vite, Vitest/jsdom with `dev/mock-tauri/`.

**Spec:** `docs/superpowers/specs/2026-10-25-operation-progress-design.md`

## Global Constraints

- Cancel error message is exactly `Cancelled.`; the frontend treats it as a cancel, not an error.
- Progress updates at most every 100 ms, plus one final update when a command ends.
- Linux local copies: 1 MiB slices through `io::copy` over `Read::take`. macOS: `fs::copy` per file, destination size polled every 100 ms.
- Time left only after 2 s of work; the popover opens by itself once, for the first operation still running after 1 s; the button stays 3 s after the last operation ends.
- Backend modules take `&Progress` and never Tauri types; only `lib.rs` uses `Channel`/`AppHandle`.
- Never lose data: cancel goes through existing failure cleanup; a move removes its source only after the copy fully succeeded; both Trash layouts tested.
- Colours from libadwaita tokens with dark values; widgets via `ui.ts` (`openPopover`); GNOME wording from the spec's UI section verbatim.
- Every new or changed mock command gets a case in `tests/mock-core.test.ts`.
- Before each commit: `npx tsc --noEmit`, `npm test`, `cd src-tauri && cargo test` pass; the pre-commit CRAP gate passes (split functions, don't bypass).

## Review Focus

- A cancel arriving after the last byte but before a move removes its source must still complete the move (source gone, destination whole), never leave both or neither → Task 3 test `cancel_after_copy_still_finishes_the_move`.
- Zero-byte files and empty folders must still advance item progress and reach 100% → Task 2 test `empty_files_and_folders_count_as_items`, Task 8 test "an empty file copy finishes at 100%".
- Measuring fails (unreadable subfolder) → the copy still runs with item progress, no error → Task 2 test `measure_skips_unreadable_folders`, Task 8 test "measure failure falls back to items".
- Cancelling a queued (waiting) batch must not touch the running one → Task 8 test "cancelling a waiting paste removes only it".
- Closing the popover while an operation runs must not cancel it, and reopening shows current progress → Task 7 test "closing the popover keeps the operation".

---

### Task 1: `Progress` handle and counted copy

**Files:**
- Create: `src-tauri/src/progress.rs`
- Modify: `src-tauri/src/lib.rs` (add `mod progress;`)

**Interfaces:**
- Produces:
  - `pub const CANCELLED: &str = "Cancelled.";`
  - `#[derive(Clone, Default)] pub struct Progress` (Arc inside) with `add_bytes(u64)`, `set_bytes(u64)`, `add_item()`, `set_total(u64)`, `set_phase(Phase)`, `cancel()`, `is_cancelled() -> bool`, `check() -> Result<(), String>` (Err(CANCELLED) when cancelled), `snapshot() -> ProgressUpdate`.
  - `#[derive(Clone, Copy, Serialize, PartialEq, Debug)] #[serde(rename_all = "lowercase")] pub enum Phase { Preparing, Working }`
  - `#[derive(Clone, Serialize, PartialEq, Debug)] pub struct ProgressUpdate { pub bytes: u64, pub items: u64, pub total: Option<u64>, pub phase: Phase, pub done: bool }` (`total` is `None` until `set_total`; `done` is set only on the final update by Task 6).
  - `pub fn copy_counted(reader: &mut impl Read, writer: &mut impl Write, progress: &Progress) -> io::Result<u64>`: 1 MiB slices, `check()` before each slice (cancel → `io::Error::other(CANCELLED)`), `add_bytes` after each.
  - `pub struct Registry` (plain, `Mutex<HashMap<String, Progress>>`): `register(op: &str) -> Progress` (returns the existing one if the op is already registered, so a batch shares one), `cancel(op: &str)` (unknown op: no-op), `remove(op: &str)`.

- [ ] **Step 1: Write failing tests in `progress.rs`**
  - `counts_bytes_items_and_total`: after `add_bytes(3)`, `add_bytes(4)`, `add_item()`, `set_total(10)`, `snapshot()` == `{bytes: 7, items: 1, total: Some(10), phase: Preparing, done: false}`.
  - `check_fails_with_cancelled_after_cancel`: `check()` Ok, then `cancel()`, `check()` == `Err("Cancelled.")`; a clone sees the cancel.
  - `copy_counted_copies_and_counts`: 3 MiB + 5 bytes from a `Cursor` into a `Vec`; returns that length, bytes match, `snapshot().bytes` equals it.
  - `copy_counted_stops_when_cancelled`: a reader whose `read` calls `progress.cancel()` after the first slice; result is an error whose message is `Cancelled.`, and fewer than all bytes were written.
  - `registry_shares_one_progress_per_op_and_cancels_it`: two `register("a")` calls return handles where cancelling via `registry.cancel("a")` makes both report cancelled; `cancel("missing")` doesn't panic; after `remove("a")`, `register("a")` is fresh.

- [ ] **Step 2: Run** `cd src-tauri && cargo test --lib progress` — expected: compile failure (module missing).
- [ ] **Step 3: Implement `progress.rs`** per the interfaces; add `mod progress;` to `lib.rs`.
- [ ] **Step 4: Run** `cargo test --lib progress` — expected: 5 passed.
- [ ] **Step 5: Commit** `feat: add a Progress handle with cancel and counted copy`.

### Task 2: Local copy, move and measure with progress

**Files:**
- Modify: `src-tauri/src/fs_ops.rs` (`copy_recursive`, `copy_entry`, `move_entry`, new `copy_file_progress`, new `measure`)
- Modify: every caller of these (`lib.rs`, `archive.rs`, `versions/`, tests) to pass `&Progress::default()` where no progress is wanted.

**Interfaces:**
- Consumes: Task 1 `Progress`, `copy_counted`, `CANCELLED`.
- Produces:
  - `pub fn copy_entry(from: String, to: String, overwrite: bool, progress: &Progress) -> Result<(), String>` and `move_entry(…, progress: &Progress)`. A cancel returns `Err("Cancelled.")` exactly (not wrapped in "Failed to copy: …").
  - `pub fn copy_recursive(from: &Path, to: &Path, progress: &Progress) -> io::Result<()>`: `check()` before each entry; `add_item()` after each file, symlink and folder.
  - `fn copy_file_progress(from: &Path, to: &Path, progress: &Progress) -> io::Result<()>`: Linux (`cfg(target_os = "linux")`) = open both, `copy_counted` over `File::take` slices (std uses `copy_file_range`), then copy permissions; other platforms = `fs::copy` on a thread while polling `to`'s size every 100 ms into `set_bytes(base + len)`, `check()` before and after.
  - `#[derive(Serialize, Default, PartialEq, Debug)] pub struct Measure { pub bytes: u64, pub items: u64 }` and `pub fn measure(paths: &[PathBuf]) -> Measure`: apparent sizes, symlinks not followed and counted as items, unreadable folders skipped silently.

- [ ] **Step 1: Write failing tests in `fs_ops.rs`**
  - `copy_reports_bytes_and_items`: tree `a/{x: 1000 bytes, y: 24 bytes, sub/z: 0 bytes}`; after `copy_entry`, `bytes == 1024`, `items == 5` (x, y, z, sub, a).
  - `empty_files_and_folders_count_as_items`: copying an empty folder → `items == 1`, `bytes == 0`.
  - `cancelled_copy_leaves_nothing`: pre-cancelled `Progress`; `copy_entry` returns `Err("Cancelled.")`; target doesn't exist; source untouched.
  - `cancelled_copy_mid_file_leaves_nothing` (`cfg(target_os = "linux")`): 3 MiB file, a `Progress` cancelled from a second thread once `bytes >= 1 MiB` (spin on `snapshot`); result `Err("Cancelled.")`, target absent.
  - `cancelled_replace_keeps_the_original`: existing target with content "old", `overwrite: true`, pre-cancelled → target still reads "old", no staging file left in the folder.
  - `cancel_after_copy_still_finishes_the_move`: the cross-device branch of `move_entry` is split into the copy (`copy_recursive` with progress) and a new `fn finish_cross_device_move(from: &Path, dest: &Path, to: &Path, replacing: bool) -> Result<(), String>` that takes **no** `Progress` (staged replace, then source removal). Test: copy a tree with a `Progress`, `cancel()` it, then call `finish_cross_device_move` → source gone, destination complete.
  - `measure_counts_and_skips_unreadable_folders` (`cfg(unix)`): tree with a `0o000` subfolder; `measure` returns the readable bytes/items and doesn't error.

- [ ] **Step 2: Run** `cargo test --lib fs_ops` — expected: compile failures (new signatures).
- [ ] **Step 3: Implement** the interfaces; map an `io::Error` whose message is `Cancelled.` to `Err(CANCELLED.into())` in `copy_entry`/`move_entry`/`move_by_copy` before cleanup returns. Update callers with `&Progress::default()`.
- [ ] **Step 4: Run** `cargo test` — expected: all pass.
- [ ] **Step 5: Commit** `feat: report progress and honour cancel in local copies and moves`.

### Task 3: Drive transfers and opens with progress

**Files:**
- Modify: `src-tauri/src/cloud/write.rs` (`Ctx` gains `pub(crate) progress: &'a Progress`; new `live_with(state, progress, work)`; `live` uses a default progress)
- Modify: `src-tauri/src/cloud/drive.rs` (`download`, `upload`)
- Modify: `src-tauri/src/cloud/transfer.rs` (`upload`, `download`, `dispatch` take `&Progress`; items counted, `check()` between items)
- Modify: `src-tauri/src/cloud/service.rs` (`open` takes `&Progress`; new `measure`)

**Interfaces:**
- Consumes: Task 1.
- Produces:
  - `DriveClient::download(&self, id: &str, dest: &Path, progress: &Progress) -> Result<(), DriveError>` via `copy_counted`; a cancel maps to `DriveError::Unavailable("Cancelled.")` and the caller's existing staging cleanup runs.
  - `DriveClient::upload(&self, parent_id, name, local, progress: &Progress)`: `check()` before each chunk, `add_bytes(chunk_len)` after Drive accepts it.
  - `transfer::dispatch(state, from, to, overwrite, moving, progress: &Progress, trash_local)`; `transfer::upload(state, from, to, overwrite, progress)`, `transfer::download(state, from, to, overwrite, progress)`.
  - `service::open(state, path, cache_dir, progress: &Progress) -> Result<OpenTarget, String>`: `set_total(file size)` before downloading.
  - `service::measure(state: &CloudState, paths: &[String]) -> Result<fs_ops::Measure, String>`: recursive sizes from file metadata; Google Docs count as items with no bytes.
  - Error text for a cancel reaching the frontend is exactly `Cancelled.` (through `write::message`).

- [ ] **Step 1: Write failing tests** next to the existing Drive fakes:
  - `download_counts_bytes` (drive.rs): fake server body of 2 MiB + 3 → `snapshot().bytes` equals it.
  - `cancelled_download_leaves_no_staging_file` (transfer.rs): pre-cancelled progress; `download` returns `Err("Cancelled.")`; no target and no staging path in the folder.
  - `cancelled_upload_creates_nothing` (transfer.rs): pre-cancelled; returns `Err("Cancelled.")`; the fake Drive received no file creation (and no staging folder remains un-trashed).
  - `upload_counts_chunks` (drive.rs): a file of `CHUNK + 10` bytes → `bytes == CHUNK + 10`.
  - `drive_copy_counts_items` (transfer.rs): Drive→Drive copy of a folder with two files → `items == 3`.
  - `open_sets_total_and_counts_bytes` (service.rs): total `Some(size)`, bytes == size.
  - `measure_sums_drive_sizes_and_counts_docs` (service.rs): folder with a 100-byte file and a Google Doc → `{bytes: 100, items: 3}`.
- [ ] **Step 2: Run** `cargo test --lib cloud` — expected: failures (signatures).
- [ ] **Step 3: Implement** the interfaces; existing callers without progress pass `&Progress::default()`.
- [ ] **Step 4: Run** `cargo test` — expected: all pass.
- [ ] **Step 5: Commit** `feat: report progress and honour cancel in Google Drive transfers`.

### Task 4: SSH and shared-file opens with progress

**Files:**
- Modify: `src-tauri/src/sftp.rs` (`open`, `download`)
- Modify: `src-tauri/src/share/state.rs` (`open`), `src-tauri/src/share/guest.rs` (`fetch`, `download`, `download_with_progress`)

**Interfaces:**
- Consumes: Task 1.
- Produces:
  - `SftpState::open(&self, path: &str, progress: &Progress) -> Result<String, String>`: `set_total` from the remote file's `stat` size, `copy_counted`; a cancel removes the temporary folder and returns `Err("Cancelled.")`.
  - `ShareState::open(&self, path, versions_hint, progress: &Progress)`; `Guest::fetch(&self, r, loc, dest, progress: &Progress)`: `set_total(expected)`, `DownloadProgressItem::Progress(n)` → `set_bytes(n)`, `check()` on every stream item; cancel → a new `Fail::Cancelled` whose message is `Cancelled.`; the existing removal of the open folder runs.

- [ ] **Step 1: Write failing tests**
  - `cancelled_share_open_leaves_nothing` (share/state.rs, using the existing two-node test setup): pre-cancelled progress → `Err("Cancelled.")`, no folder left under `share-open/`.
  - `share_open_reports_total_and_bytes` (same setup): after open, `total == Some(file size)` and `bytes == file size`.
  - SFTP has no test server: no new SFTP test; `copy_counted` (Task 1) covers the copy. State this in the commit body.
- [ ] **Step 2: Run** `cargo test --lib share` — expected: compile failures.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** `cargo test` — expected: all pass.
- [ ] **Step 5: Commit** `feat: report progress and honour cancel when opening SSH and shared files`.

### Task 5: Empty Trash counts items and honours cancel

**Files:**
- Modify: `src-tauri/src/trash_ops.rs` (`empty`)

**Interfaces:**
- Consumes: Task 1.
- Produces: `pub fn empty(trash_root: &Path, progress: &Progress) -> Result<(), String>`: `check()` before each top-level item, `add_item()` after; cancel returns `Err("Cancelled.")` with removed items staying removed and the rest untouched. (`trash_entry` / `delete_permanently` stay per item; their batches cancel in the frontend, Task 8.)

- [ ] **Step 1: Write failing tests** for both layouts:
  - `empty_counts_items_freedesktop` and `empty_counts_items_macos_layout`: three items → `items == 3`, trash empty, freedesktop `files/` and `info/` still exist.
  - `cancelled_empty_keeps_the_rest_freedesktop` / `…_macos_layout`: a progress cancelled after the first `add_item` (wrap via a thread watching `snapshot().items >= 1`, or a pre-cancelled progress expecting all three kept) → `Err("Cancelled.")`; remaining items still present.
- [ ] **Step 2: Run** `cargo test --lib trash_ops` — expected: failures.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** `cargo test` — expected: all pass.
- [ ] **Step 5: Commit** `feat: count and cancel Empty Trash`.

### Task 6: Commands, channel ticker, `cancel_operation`, `measure`

**Files:**
- Modify: `src-tauri/src/lib.rs`

**Interfaces:**
- Consumes: Tasks 1–5.
- Produces (Tauri commands; argument names are what the frontend sends):
  - `copy_entry(from, to, overwrite, op: String, progress: Channel<ProgressUpdate>)`, same for `move_entry`; `empty_trash(op, progress)`; `drive_open(path, op, progress)`; `sftp_open(path, op, progress)`; `share_open(path, op, progress)`.
  - `cancel_operation(op: String)` (sync; uses the managed `progress::Registry`).
  - `measure(paths: Vec<String>) -> Result<fs_ops::Measure, String>` (async, `blocking`): local paths via `fs_ops::measure`, `gdrive://` via `cloud::service::measure`, summed.
  - `async fn tracked<T>(registry, op, channel, work: impl FnOnce(Progress) -> Result<T, String>) -> Result<T, String>`: registers `op`, sets phase `Working`, spawns a tokio interval (100 ms) sending `snapshot()` until the work ends, then sends a final snapshot with `done: true`, and removes `op` from the registry when the command ends; the next item's command registers it afresh. A cancel between items is handled by the frontend's own flag (Task 8).
  - Keep `trash_entry` and `delete_permanently` unchanged.
- [ ] **Step 1: Write a failing test** for the one piece of logic outside Tauri: move the ticker's "send while running, then once more with done" loop into `progress::ticker(progress: Progress, send: impl Fn(ProgressUpdate), stop: Arc<AtomicBool>, every: Duration)` and test `ticker_sends_a_final_done_update` (stop immediately → exactly one call, with `done: true`) and `ticker_sends_while_running` (run 250 ms with 50 ms period → at least 3 calls, last `done: true`).
- [ ] **Step 2: Run** `cargo test --lib progress` — expected: failures.
- [ ] **Step 3: Implement** `ticker` and the commands; register `cancel_operation` and `measure`; `app.manage(progress::Registry::default())`.
- [ ] **Step 4: Run** `npm run build && cd src-tauri && cargo test` — expected: all pass.
- [ ] **Step 5: Commit** `feat: stream operation progress to the frontend and add cancel and measure`.

### Task 7: Operations model and widget (frontend)

**Files:**
- Create: `src/operations.ts`
- Modify: `index.html` (button `#btn-operations` in `.hb-end`, before `#btn-location-menu`, `hidden`), `src/styles.css`, `src/icons.ts` (symbolic `operation-done` checkmark)
- Create: `tests/operations.test.ts`

**Interfaces:**
- Produces (exported from `src/operations.ts`):
  - `type OpKind = "copy" | "move" | "trash" | "delete" | "empty" | "open"`; `type OpState = "waiting" | "preparing" | "working" | "cancelling"`.
  - `interface ProgressUpdate { bytes: number; items: number; total: number | null; phase: "preparing" | "working"; done: boolean }` (mirrors Rust).
  - `interface Operation { id: string; title: string; state: OpState; totalBytes: number | null; totalItems: number; doneBytes: number; doneItems: number; current: ProgressUpdate | null; startedAt: number; cancel(): void }`.
  - `opTitle(kind: OpKind, names: string[], destName?: string): string` — exact strings: "Copying “a.txt”", "Copying 3 files to “Documents”", "Moving “a.txt”", "Moving 3 files to “Documents”", "Moving 3 files to the Trash" (1 file: "Moving “a.txt” to the Trash"), "Deleting 3 files" (1: "Deleting “a.txt”"), "Emptying the Trash", "Opening “a.txt”".
  - `opDetail(op: Operation, now: number): string` — "Waiting…", "Preparing…", "Cancelling…", "12.4 MB of 80.1 MB — 1 minute left" (sizes via `formatSize`; time left only when `now - startedAt >= 2000`, smoothed speed), "12.4 MB of 80.1 MB" before 2 s, "3 of 10 files" when no byte total.
  - `opFraction(op: Operation): number` (0–1; bytes when `totalBytes`, else items).
  - `startOperation(kind, title, opts: { totalBytes?: number | null; totalItems: number; onCancel: () => void; waiting?: boolean }): Operation`, `updateOperation(op, update: ProgressUpdate)`, `finishItem(op, bytes: number)`, `endOperation(op)`, `setOperationState(op, state)`.
  - `initOperationsButton(button: HTMLElement): void` — renders the pie (inline SVG circle, `stroke-dasharray` from the overall fraction, `--accent-bg` / `--dim`), shows/hides (3 s after the last ends, unless the popover is open), opens the popover via `openPopover` on click, auto-opens it once for the first op still running after 1 s, shows `operation-done` briefly when all finish. Rows: title, `<progress>`-style bar (div), detail, ✕ button calling `op.cancel()`.

- [ ] **Step 1: Write failing tests in `tests/operations.test.ts`** (pure functions, no app boot):
  - each `opTitle` string above;
  - `opDetail`: waiting/preparing/cancelling strings; `{totalBytes: 80.1e6, doneBytes: 12.4e6}` at 1 s → "12.4 MB of 80.1 MB"; at 3 s with steady speed → matches `/^12\.4 MB of 80\.1 MB — .+ left$/`; no total, 3 of 10 items → "3 of 10 files";
  - `opFraction` for bytes, for items, and 0 when totals are 0 with nothing done; an empty-file copy (`totalBytes: 0, totalItems: 1, doneItems: 1`) → 1.
- [ ] **Step 2: Run** `npx vitest run tests/operations.test.ts` — expected: fail (module missing).
- [ ] **Step 3: Implement** `src/operations.ts`, the button markup, CSS (tokens only, light + dark) and the icon.
- [ ] **Step 4: Run** `npx vitest run tests/operations.test.ts` and `npx tsc --noEmit` — expected: pass.
- [ ] **Step 5: Commit** `feat: add the operations model and header-bar widget`.

### Task 8: Wire operations into transfers, trash, delete, empty and open

**Files:**
- Modify: `src/api.ts` (`Channel` from `@tauri-apps/api/core`; `copyEntry/moveEntry(from, to, overwrite, op: string, onProgress: (u: ProgressUpdate) => void)`, `emptyTrash(op, onProgress)`, `openFile(path, op?, onProgress?)`, new `cancelOperation(op)`, `measure(paths)`)
- Modify: `src/main.ts` (`runTransfer`, `trashPaths`, `deleteEntries`, `runEmptyTrash`, `openEntry`; remove the 0.6 s copying toast)
- Modify: `dev/mock-tauri/core.ts` (export `class Channel<T> { onmessage?: (m: T) => void }`; `cancel_operation`, `measure`; long commands report progress; `__mockOps = { hold(): void; step(bytes: number): void; release(): void; reset(): void }` — while held, progress commands send `{bytes, items, total, phase: "working", done: false}` per `step` and resolve on `release`, rejecting with "Cancelled." if `cancel_operation` was called for their op)
- Modify: `tests/mock-core.test.ts`, create `tests/progress.test.ts`

**Interfaces:**
- Consumes: Task 6 commands, Task 7 exports.
- Produces: behaviour only.

- [ ] **Step 1: Write failing tests**
  - `tests/mock-core.test.ts`: `measure` sums a fake file and folder; `cancel_operation` on a held `copy_entry` rejects it with "Cancelled."; a held copy delivers a step through the `Channel`.
  - `tests/progress.test.ts` (boot the app; `__mockOps.hold()`):
    - "a slow paste shows the operations button and opens the popover once": paste a file, advance past 1 s → `#btn-operations` visible, popover row title "Copying “x”", detail contains " of ".
    - "Cancel ends without an error toast and keeps finished items": paste 2 files, `step` + finish the first, click ✕ → no toast starting "Could not", the first copy exists, the second doesn't.
    - "a second paste is queued and runs after the first": second row reads "Waiting…", runs after `release`.
    - "cancelling a waiting paste removes only it" (Review Focus).
    - "closing the popover keeps the operation" (Review Focus): close, `step`, reopen → detail updated, op still running.
    - "measure failure falls back to items" (Review Focus): mock `measure` rejects → detail "1 of 2 files" style.
    - "an empty file copy finishes at 100%" (Review Focus).
    - "a trash batch shows progress and can be cancelled": cancel after the first item → first in Trash with Undo toast, second untouched.
    - "the button hides 3 s after the last operation".
    - "opening an SSH file shows Opening “name” and can be cancelled".
- [ ] **Step 2: Run** `npx vitest run tests/progress.test.ts tests/mock-core.test.ts` — expected: failures.
- [ ] **Step 3: Implement** in `main.ts`:
  - `runTransfer`: replace the `transferring` early return with a FIFO queue (`waiting` ops); per batch: `startOperation` (state preparing), `measure(paths)` (on failure: `totalBytes: null`), then per item a `copyEntry/moveEntry` with the batch's `op` id and an `onProgress` → `updateOperation`; `finishItem` after each; on ✕ call `cancelOperation(op)` and set a local `cancelled` flag checked before the next item; a "Cancelled." rejection ends the batch silently (reload + select done items as now).
  - `trashPaths` / `deleteEntries`: an item-counted operation; cancel checked between items (no backend call needed).
  - `runEmptyTrash`: `emptyTrash(op, onProgress)`, item total from the current Trash listing.
  - `openEntry`: for `gdrive://`, `sftp://`, `share://` paths start an "open" operation with `openFile(path, op, onProgress)`; local opens unchanged; "Cancelled." → no toast.
- [ ] **Step 4: Run** `npx tsc --noEmit && npm test` — expected: all pass.
- [ ] **Step 5: Visual check:** `npm run dev:mock`, slow paste with `__mockOps.hold()` from the console; screenshot popover light and dark.
- [ ] **Step 6: Commit** `feat: show progress and Cancel for transfers, trash, delete, empty and open`.

### Task 9: Docs and final verification

**Files:**
- Modify: `README.md` (feature list: operations button with progress and Cancel; "Not supported yet": no pause, transfers run one at a time, macOS cancel waits for the current file)
- Modify: `docs/superpowers/specs/2026-10-25-operation-progress-design.md` (Status: implemented), `docs/superpowers/specs/2026-10-24-google-drive-design.md` (note M3 progress now done, pointing at this spec)

- [ ] **Step 1: Update docs.**
- [ ] **Step 2: Run** `npx tsc --noEmit && npm run build && npm test && (cd src-tauri && cargo test)` — expected: all pass.
- [ ] **Step 3: Commit** `docs: describe operation progress`.
