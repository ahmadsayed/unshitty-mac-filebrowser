# Google Drive Writes (M2 + M3) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let users create, rename, copy, move, upload and trash items in Google Drive, with Undo, and copy or move items between local folders and Drive by paste and by drag and drop.

**Architecture:** The existing path-based commands (`make_dir`, `rename_entry`, `copy_entry`, `move_entry`, `trash_entry`, `restore_entry`, `path_exists`) gain Drive branches, as the spec's command surface says. A destination that does not exist yet is written as a *pending* last segment, `…/~<encoded-name>`, so the frontend's `runTransfer`, conflict dialog and `freeCopyName` work unchanged. Within-Drive operations live in a new `cloud/write.rs`; local ↔ Drive transfers live in a new `cloud/transfer.rs`. Write commands now return the item's final path, because a Drive item's URI is only known after Drive assigns its id.

**Tech Stack:** Tauri 2, Rust (edition 2024, `reqwest` blocking, `serde_json`, `trash`), TypeScript + Vite, Vitest + jsdom. No new dependencies.

**Spec:** `docs/superpowers/specs/2026-10-24-google-drive-design.md` (D2, D9, D10, D17, "Command surface", "Data flows", "Menus", M2/M3 acceptance criteria). This plan replaces Tasks 11–13 of `docs/superpowers/plans/2026-10-24-google-drive.md`.

## Global Constraints

- Platform agnostic: no `cfg!(target_os)` in `src-tauri/src/cloud/`.
- **No command permanently deletes Drive data.** Delete means `trashed=true`; Undo means `trashed=false` (D10). `delete_permanently` and `empty_trash` keep refusing `gdrive://`.
- A failed upload or copy never leaves an item with the final name (D17). A move removes the source only after the copy fully succeeded and was verified by size (D17).
- Name clashes use the existing Replace / Skip / Keep Both dialog; nothing is overwritten unless the user chose Replace (D17). Replace trashes the old item, never deletes it.
- Every refusal from `fs_ops::check_transfer` also applies to Drive: vanished source, same path, destination inside the source, existing target without `overwrite`, replacing a folder with its own child. Each gets a test.
- Writes need the scope `https://www.googleapis.com/auth/drive` (`oauth::SCOPE_FULL`); `drive.readonly` accounts get a message, not a 403.
- Keep Versions never applies to Drive (D9): no "Keep versions" checkbox in a Drive New Folder dialog.
- GNOME wording: "A folder with that name already exists.", "Move to Trash", "“name” moved to trash".
- Every new or changed Tauri command has a mock in `dev/mock-tauri/core.ts` and a case in `tests/mock-core.test.ts`.
- Network work runs in `async fn` commands through `blocking(…)` in `lib.rs`.
- Before each commit: `npx tsc --noEmit`, `npm run build`, `npm test`, `cd src-tauri && cargo test`; the pre-commit CRAP gate must pass.

## Review Focus

1. **Two Drive items with the same name** in the destination: `path_exists` is true, and Replace must not guess. Replace refuses with "Several items in this folder are named “X”. Rename or remove the extra ones in Google Drive first." (Task 4 test `replace_refuses_an_ambiguous_name`).
2. **Google Docs, Sheets and Slides copied out of Drive** have no bytes: the item is refused with "“X” is a Google Docs file and can only be copied within Google Drive." and the rest of the batch continues; a folder containing one is copied without it and the error names it (Task 5 tests `a_google_doc_cannot_be_downloaded`, `a_folder_with_a_google_doc_reports_it`).
3. **A connection made before this change is read-only**: every write says "Google Drive is connected read-only. Disconnect and connect again to allow changes." before any request (Task 3 test `readonly_accounts_cannot_write`).
4. **A transfer that fails halfway through a folder**: no item with the final name remains (the partial tree is trashed), and a move leaves the source untouched (Task 5 tests `a_failed_upload_trashes_the_partial_folder`, `a_failed_move_keeps_the_source`).
5. **Drive names that are not valid local names** (`a/b`, `..`, control characters) when downloading: they are made safe with `service::safe_name`, never written outside the destination (Task 5 test `downloaded_names_are_made_safe`).

---

### Task 1: Pending destinations, Drive `path_exists`, and write commands that return the final path

**Files:**
- Modify: `src-tauri/src/cloud/location.rs`
- Modify: `src-tauri/src/cloud/service.rs`
- Modify: `src-tauri/src/lib.rs` (`path_exists`, `make_dir`, `rename_entry`, `copy_entry`, `move_entry` return `String`)
- Modify: `src-tauri/src/fs_ops.rs` (no behaviour change; callers return `to`)
- Modify: `src/paths.ts`, `src/api.ts`, `src/main.ts` (`joinPath`, `runTransfer`, rename, new folder use the returned path)
- Modify: `dev/mock-tauri/core.ts`, `tests/mock-core.test.ts`, `tests/paths.test.ts`

**Interfaces:**
- Produces (Rust): `Segment { id: String, name: String }` where an **empty `id` is allowed only on the last segment** and means "a child with this name that may not exist yet". `DriveLocation::is_pending(&self) -> bool`; `DriveLocation::parent(&self) -> Option<DriveLocation>` (drops the last segment; `None` at the root); `DriveLocation::child(&self, id: &str, name: &str) -> DriveLocation`.
- Produces (Rust): `service::path_exists(state: &CloudState, path: &str) -> bool`.
- Produces (TS): `joinCloudPath(dir: string, name: string): string` in `src/paths.ts`, encoding exactly like `location.rs`'s `NAME_SET` (everything except ASCII letters, digits, `.`, `-`, `_` becomes `%XX` of its UTF-8 bytes).
- Produces (TS): `makeDir`, `renameEntry`, `copyEntry`, `moveEntry` in `src/api.ts` return `Promise<string>`, the item's final path.

- [ ] **Step 1: Write failing Rust tests in `location.rs`**

```rust
#[test]
fn a_pending_last_segment_names_a_new_child() {
    let loc = DriveLocation::parse("gdrive://acct/root~My%20Drive/~New%20Folder").unwrap();
    assert!(loc.is_pending());
    assert_eq!(loc.name(), Some("New Folder"));
    assert_eq!(loc.parent().unwrap().file_id(), Some("root"));
    assert_eq!(loc.to_uri(), "gdrive://acct/root~My%20Drive/~New%20Folder");
}

#[test]
fn only_the_last_segment_may_be_pending() {
    assert!(DriveLocation::parse("gdrive://acct/~a/f1~b").is_err());
    assert!(DriveLocation::parse("gdrive://acct/root~My%20Drive/~").is_err()); // no name
}

#[test]
fn child_appends_an_id_anchored_segment() {
    let root = DriveLocation::root("acct", "root", "My Drive");
    assert_eq!(root.child("f1", "a b").to_uri(), "gdrive://acct/root~My%20Drive/f1~a%20b");
    assert!(root.parent().is_none());
}
```

- [ ] **Step 2: Run them to see them fail**

Run: `cd src-tauri && cargo test cloud::location`
Expected: FAIL (no `is_pending`, `parent`, `child`; the pending URI is rejected as an invalid id).

- [ ] **Step 3: Implement pending segments in `location.rs`**

`Segment::parse` accepts an empty id when the name is non-empty; `DriveLocation::parse` rejects an empty id anywhere but the last segment; `to_uri` writes `~name` for it.

- [ ] **Step 4: Write failing tests for `service::path_exists`** (fake server from `cloud::test_server`)

```rust
#[test]
fn a_pending_path_exists_when_the_parent_has_that_name() // files.list returns one match → true; asserts the query contains "name = 'a.txt'" and "'root' in parents" and "trashed = false"
#[test]
fn a_pending_path_is_free_when_no_child_matches()        // files.list returns [] → false
#[test]
fn an_id_path_exists_unless_trashed()                    // files.get {"trashed": true} → false; 404 → false
#[test]
fn names_with_quotes_are_escaped_in_the_query()          // name "it's" → query contains "name = 'it\\'s'"
```

- [ ] **Step 5: Implement** `DriveClient::find_children(&self, parent_id: &str, name: &str) -> Result<Vec<DriveFile>, DriveError>` in `drive.rs` (query `name = '<escaped>' and '<parent>' in parents and trashed = false`, escaping `\` and `'`) and `service::path_exists`. Add `trashed: bool` (default false) to `DriveFile` and to the `fields` strings.

- [ ] **Step 6: Make the write commands return the final path**

`lib.rs`: `make_dir`, `rename_entry`, `copy_entry`, `move_entry` return `Result<String, String>`; the local branches return `to` (or `path`) after the existing `fs_ops` call. `path_exists` becomes an `async fn path_exists(app, path: String) -> bool` that dispatches to `service::path_exists` for cloud paths and `fs_ops::path_exists` otherwise.

- [ ] **Step 7: Frontend**

`paths.ts`: `joinCloudPath`. `main.ts`: `joinPath(dir, name)` returns `joinCloudPath(dir, name)` when `isCloud(dir)`. `runTransfer` pushes the returned path into `done` and passes it to `followMove`; rename and New Folder select the returned path. `tests/paths.test.ts`:

```ts
test("Drive child paths encode the name like the backend", () => {
  expect(joinCloudPath("gdrive://a/root~My%20Drive", "Q3 ~ draft (v2).txt"))
    .toBe("gdrive://a/root~My%20Drive/~Q3%20%7E%20draft%20%28v2%29.txt");
  expect(joinCloudPath("gdrive://a/root~My%20Drive/", "é")).toBe("gdrive://a/root~My%20Drive/~%C3%A9");
});
```

Mock: `make_dir`, `rename_entry`, `copy_entry`, `move_entry` return the target path; `tests/mock-core.test.ts` asserts the returned value for one local copy.

- [ ] **Step 8: Run everything**

Run: `npx tsc --noEmit && npm test && (cd src-tauri && cargo test)`
Expected: all pass.

- [ ] **Step 9: Commit** `feat: name new Google Drive items before they exist`

---

### Task 2: Drive client write calls

**Files:**
- Modify: `src-tauri/src/cloud/drive.rs`
- Modify: `src-tauri/src/cloud/mod.rs` (`test_server::serve` gains response headers)

**Interfaces:**
- Consumes: `DriveClient` and `send` retry from `drive.rs`; `DriveFile` with `trashed` (Task 1).
- Produces, all on `DriveClient`, all returning `Result<_, DriveError>`:
  - `create_folder(&self, parent_id: &str, name: &str) -> DriveFile` — `POST files` with `{"name", "mimeType": "application/vnd.google-apps.folder", "parents": [parent_id]}`.
  - `update(&self, id: &str, change: &FileChange) -> DriveFile` — `PATCH files/{id}`; `FileChange { name: Option<String>, add_parent: Option<String>, remove_parent: Option<String>, trashed: Option<bool> }`; parents go in the query (`addParents`, `removeParents`), the rest in the JSON body.
  - `copy_file(&self, id: &str, parent_id: &str, name: &str) -> DriveFile` — `POST files/{id}/copy`.
  - `upload(&self, parent_id: &str, name: &str, local: &Path) -> DriveFile` — resumable upload.
  - `list_all(&self, folder_id: &str) -> Vec<DriveFile>` — every page (moves the page loop out of `service::list_all`, which then calls it).
- Every write sends `supportsAllDrives=true` and requests the same `fields` as `get`.

- [ ] **Step 1: Extend the test server** so a canned response can carry headers: `serve(Vec<Reply>)` with `struct Reply { status: u16, body: Vec<u8>, content_type: &'static str, headers: Vec<(&'static str, String)> }` and a `Reply::json(status, body)` helper; it also records each request's first line and body for assertions (`serve` returns `(base, Arc<Mutex<Vec<Request>>>, JoinHandle)`). Update existing callers.

- [ ] **Step 2: Write failing tests**

```rust
#[test] fn create_folder_posts_the_folder_mime_type_and_parent()   // body has "application/vnd.google-apps.folder" and "parents":["p1"]
#[test] fn update_moves_with_add_and_remove_parents()               // request line has addParents=p2&removeParents=p1
#[test] fn update_trashes_and_untrashes()                           // body {"trashed":true} then {"trashed":false}
#[test] fn copy_file_names_the_copy_and_its_parent()                // POST /files/f1/copy, body name + parents
#[test] fn upload_starts_a_session_and_sends_the_bytes()            // 1st: POST /upload/files?uploadType=resumable → 200 + Location: {base}/session/1; 2nd: PUT /session/1 with Content-Range "bytes 0-4/5" → 200 file JSON
#[test] fn upload_resumes_after_a_308()                             // chunk 1 → 308 Range: bytes=0-262143; chunk 2 starts at 262144
#[test] fn upload_of_an_empty_file_sends_no_chunk_range()           // Content-Range "bytes */0"
#[test] fn a_failed_upload_session_is_an_error()                    // PUT → 400 three times not retried → Err
```

- [ ] **Step 3: Run them to see them fail**

Run: `cd src-tauri && cargo test cloud::drive`
Expected: FAIL (methods missing).

- [ ] **Step 4: Implement the methods**

Upload endpoint: production `https://www.googleapis.com/upload/drive/v3`; `DriveClient::new` derives it as `{base}/upload` when `base != API_BASE` (tests). Chunks are 8 MiB (`CHUNK = 8 * 1024 * 1024`, a multiple of 256 KiB). A `308` reply's `Range: bytes=0-N` sets the next offset to `N+1`; a `308` without `Range` restarts at 0. 5xx/429 on a chunk are retried by `send` (the chunk is re-read from the file at its offset).

- [ ] **Step 5: Run the tests to see them pass**, then `cargo test` for the whole crate.

- [ ] **Step 6: Commit** `feat: Google Drive client calls for writes and uploads`

---

### Task 3: Ask for write access, and refuse writes on read-only connections

**Files:**
- Modify: `src-tauri/src/lib.rs` (`drive_connect_start` requests `SCOPE_FULL`; `drive_connect_finish` default scope)
- Modify: `src-tauri/src/cloud/service.rs`
- Modify: `src/main.ts` (Connect dialog text), `dev/mock-tauri/core.ts` (`DRIVE_SCOPE` becomes the full scope)
- Modify: `docs/google-drive.md` (step 6 of "Connect File Browser", Privacy, Troubleshooting)

**Interfaces:**
- Produces: `service::writable(state: &CloudState, path: &str) -> Result<(DriveLocation, DriveAccount), String>` — parses the path, loads the account, and returns `Err(READONLY)` unless `account.scopes` contains `SCOPE_FULL`, where `pub const READONLY: &str = "Google Drive is connected read-only. Disconnect and connect again to allow changes."`. Every write in Tasks 4–5 starts with it.

- [ ] **Step 1: Failing test** `readonly_accounts_cannot_write`: an account with only `SCOPE_READONLY` → `writable` errors with `READONLY` and the fake server receives no request; with `SCOPE_FULL` → `Ok`.

- [ ] **Step 2: Implement** `writable`, and request `[SCOPE_FULL]` in `drive_connect_start` (and as the fallback scope in `drive_connect_finish`).

- [ ] **Step 3: Copy updates.** Connect dialog: "File Browser asks Google for access to see, edit, create and delete your Drive files. The token is kept in your system keychain. …". Guide step 6: "Allow File Browser to **see, edit, create and delete all of your Google Drive files**." Privacy: replace the read-only bullet with "It can change files in your Drive only when you ask it to: New Folder, Rename, Paste, drag and drop and Move to Trash. It never deletes anything permanently; Move to Trash uses Drive's Trash, which keeps items for 30 days." Troubleshooting row: "Google Drive is connected read-only" → "Your connection was made before File Browser could change files. Disconnect and connect again."

- [ ] **Step 4: Run** `cargo test`, `npm test`. **Commit** `feat: ask Google Drive for write access`

---

### Task 4: New Folder, Rename, Copy, Move, Trash and Undo within Drive

**Files:**
- Create: `src-tauri/src/cloud/write.rs` (registered in `cloud/mod.rs`)
- Modify: `src-tauri/src/lib.rs` (Drive branches of `make_dir`, `rename_entry`, `copy_entry`, `move_entry`, `trash_entry`, `restore_entry`; all `async` through `blocking`)

**Interfaces:**
- Consumes: `writable` (Task 3); `DriveClient::{find_children, create_folder, update, copy_file, list_all, get}` (Tasks 1–2); `authorized_with` and `Refresh` from `service.rs` (make them `pub(crate)`).
- Produces, each returning the final URI as `Result<String, String>` unless noted:
  - `write::make_dir(state, path)` — `path` is pending; refuses when the name exists: "A folder with that name already exists."
  - `write::rename(state, from, to)` — `from` has an id, `to` is a pending sibling; refuses an existing name: "A file or folder with that name already exists."; keeps the id.
  - `write::copy(state, from, to, overwrite)` and `write::move_item(state, from, to, overwrite)` — both Drive, same account.
  - `write::trash(state, path) -> Result<Option<String>, String>` — returns `Some(path)` (the same URI; the id survives trashing) so the frontend's Undo works unchanged.
  - `write::restore(state, path)` — `trashed=false`; returns `path`.
- Transfer rules (`fn check(…)` shared by copy and move, mirroring `fs_ops::check_transfer`, tested one by one):
  - source missing or trashed → "“X” no longer exists"
  - same item, or destination parent equal to source parent with the same name on a move → "The source and destination are the same"
  - destination parent is the source or below it (any segment id of the destination equals the source id) → "Cannot copy or move a folder into itself"
  - target name exists and `!overwrite` → "Target already exists: X"
  - target exists and is an ancestor of the source → "Cannot replace a folder with an item inside it"
  - more than one existing item with the target name and `overwrite` → the Review Focus 1 message
  - source and destination in different accounts → "Copying between Google accounts is not supported."
- Copy: files with `copy_file`; folders recursively — `create_folder` under a staging name, copy children, then `update` the name. Staging name: `.<name>.filebrowser-copy`. On any failure the staged folder is trashed and the error returned.
- Move: `update` with `add_parent`/`remove_parent` (and `name` when it changes). Folders move in one call.
- Replace (`overwrite` and one existing target): stage the copy (files: `copy_file` with the staging name), trash the old target, rename the staged item into place. A move with Replace trashes the old target first, then moves.

- [ ] **Step 1: Write failing tests in `write.rs`** using the fake server and a `test_state` with `SCOPE_FULL`:

```rust
#[test] fn make_dir_creates_a_folder_and_returns_its_uri()          // returns ".../new-id~Docs"
#[test] fn make_dir_refuses_an_existing_name()
#[test] fn rename_keeps_the_id_and_returns_the_new_uri()
#[test] fn copy_of_a_file_is_server_side_and_gets_a_new_id()
#[test] fn copy_of_a_folder_is_staged_then_renamed()                // request order: create ".Docs.filebrowser-copy", copy child, PATCH name "Docs"
#[test] fn a_failed_folder_copy_trashes_the_staged_folder()         // child copy → 403; last request PATCH {"trashed":true} on the staged id
#[test] fn move_updates_parents_in_one_call()
#[test] fn moving_into_its_own_subfolder_is_refused()               // no request sent
#[test] fn the_same_path_is_refused()
#[test] fn an_existing_target_without_overwrite_is_refused()
#[test] fn replacing_a_folder_with_its_own_child_is_refused()
#[test] fn replace_refuses_an_ambiguous_name()                      // Review Focus 1
#[test] fn replace_trashes_the_old_item_and_never_deletes()         // no DELETE request appears in the log
#[test] fn trash_and_restore_flip_the_trashed_flag()
#[test] fn a_vanished_source_is_refused()
#[test] fn copying_between_accounts_is_refused()
```

- [ ] **Step 2: Run them to see them fail** — `cargo test cloud::write` → FAIL.

- [ ] **Step 3: Implement `write.rs`** to the interfaces above.

- [ ] **Step 4: Wire `lib.rs`.** When both paths are cloud, call `write::*`; `trash_entry` and `restore_entry` dispatch for cloud paths; `delete_permanently` keeps its refusal. Add a `lib.rs`-level test-free check: `grep -n "unsupported()" src-tauri/src/lib.rs` should now list only `folder_size`, the versions commands, and the mixed local/Drive branches that Task 5 replaces.

- [ ] **Step 5: Run** `cargo test`; **commit** `feat: create, rename, copy, move and trash Google Drive items`

---

### Task 5: Copy and move between local folders and Drive

**Files:**
- Create: `src-tauri/src/cloud/transfer.rs`
- Modify: `src-tauri/src/lib.rs` (`copy_entry` / `move_entry` when exactly one side is cloud)

**Interfaces:**
- Consumes: `writable` (Task 3, for uploads); `DriveClient::{upload, create_folder, update, list_all, get, download}`; `service::safe_name`; `VersionsState::trash` (to send a moved local source to the Trash the same way `trash_entry` does).
- Produces:
  - `transfer::upload(state, from: &Path, to: &str, overwrite: bool) -> Result<String, String>` — `to` is a pending Drive path.
  - `transfer::download(state, from: &str, to: &Path, overwrite: bool) -> Result<String, String>` — `to` is a local path.
  - `transfer::move_up(state, from: &Path, to: &str, overwrite: bool, trash_source: impl FnOnce(&Path) -> Result<(), String>) -> Result<String, String>` — upload, then `trash_source(from)`; `lib.rs` passes a closure that calls `VersionsState::trash`.
  - `transfer::move_down(state, from: &str, to: &Path, overwrite: bool) -> Result<String, String>` — download, then `update(trashed=true)` on the Drive item.
  - `lib.rs` moves: upload or download first; then the source goes to the Trash — local sources through `VersionsState::trash(…, || trash_ops::trash_and_locate(…))`, Drive sources with `update(trashed=true)`. A failure removing the source after a successful copy returns "Moved, but could not move the original to the Trash: …" (matching `fs_ops`).
- Upload: a file uploads straight to its final name (Drive creates the file only when the last chunk arrives); after it returns, its `size` must equal the local length, otherwise the new item is trashed and the error is "The upload to Google Drive was incomplete.". A folder is created as `.<name>.filebrowser-upload`, filled recursively (symlinks are skipped, as `copy_recursive` does), then renamed; any failure trashes it. Replace: upload under the staging name, trash the old item, rename.
- Download: files go to `fs_ops`-style staging (`.<name>.filebrowser-<pid>-<n>` next to the target), are checked against the Drive `size`, then renamed into place; folders are staged the same way and filled recursively; any failure removes the staging path (it is the app's own temporary file, never user data). Google-native files are refused per Review Focus 2; inside a folder they are skipped and named in the error after the rest is copied. Names pass through `service::safe_name`.
- Same-path and into-itself refusals cannot happen across the boundary; existing-target and replace-with-own-child checks use the local side's `fs_ops` rules (download) or `find_children` (upload).

- [ ] **Step 1: Write failing tests**

```rust
#[test] fn uploading_a_file_returns_its_drive_uri()
#[test] fn a_short_upload_is_trashed_and_reported()                 // returned size 3, local 5
#[test] fn uploading_a_folder_is_staged_then_renamed()
#[test] fn a_failed_upload_trashes_the_partial_folder()             // Review Focus 4
#[test] fn downloading_a_file_stages_and_verifies_its_size()
#[test] fn a_short_download_leaves_no_file()                        // body 3 bytes, size 5 → Err, target and staging absent
#[test] fn downloading_a_folder_copies_the_tree()
#[test] fn a_google_doc_cannot_be_downloaded()                      // Review Focus 2
#[test] fn a_folder_with_a_google_doc_reports_it()                  // other files present, Err names the doc
#[test] fn downloaded_names_are_made_safe()                         // Drive name "a/b" → local "a_b", inside the target folder
#[test] fn an_existing_local_target_without_overwrite_is_refused()
#[test] fn a_failed_move_keeps_the_source()                         // Review Focus 4: upload fails → local file still present
#[test] fn a_drive_to_local_move_trashes_the_drive_item_last()      // last request is PATCH {"trashed":true}
```

`move_up` is tested with a `tempfile` source and a recording `trash_source` closure, so no test touches the real Trash.

- [ ] **Step 2: Run them to see them fail** — `cargo test cloud::transfer` → FAIL.

- [ ] **Step 3: Implement `transfer.rs`** and the `lib.rs` branches.

- [ ] **Step 4: Run** `cargo test`; **commit** `feat: copy and move between local folders and Google Drive`

---

### Task 6: Turn on writes in the Drive UI

**Files:**
- Modify: `src/main.ts` (menus, `paste`, `trashSelection`, `startRename`, `promptNewFolder`)
- Modify: `dev/mock-tauri/core.ts` (mutable Drive tree), `tests/mock-core.test.ts`, `tests/drive.test.ts`
- Modify: `README.md` (feature list and "Not supported yet")

**Interfaces:**
- Consumes: the commands from Tasks 1, 4 and 5 (unchanged names; they now succeed for Drive paths).
- Menus (spec "Menus"): `cloudItemMenu` becomes Open, Open in Browser (docs), Open Item Location, —, Cut, Copy, —, Rename…, —, Move to Trash, —, Add to Bookmarks (folders), —, Properties. Drive background and location menus gain New Folder… and Paste.
- Progress stays as it is for local transfers: the per-item "Copying “X”…" toast after 600 ms. No byte-level progress in this milestone.
- Remove the Drive guards in `paste`, `trashSelection`, `startRename` and `promptNewFolder`; the New Folder dialog hides the "Keep versions" checkbox when `isCloud(location)` (D9).

- [ ] **Step 1: Mock.** Replace the static `DRIVE_FILES` map with a mutable tree keyed by id (`{ id, name, parent, isDir, size, trashed, doc }`) built from the same seed items. Implement the Drive branches of `make_dir`, `rename_entry`, `copy_entry`, `move_entry` (including local ↔ Drive, copying size only), `trash_entry`, `restore_entry` and `path_exists` (pending and id paths), with the same refusals and messages as Tasks 4–5. `tests/mock-core.test.ts`: one test per command, plus "a Google Doc cannot be copied to a local folder".

- [ ] **Step 2: Failing UI tests in `tests/drive.test.ts`**

```ts
test("New Folder in Drive has no Keep versions option and creates the folder")
test("Rename in Drive keeps the item selected under its new name")
test("Copy and Paste inside Drive asks about a clash and Keep Both adds (copy)")
test("Move to Trash in Drive can be undone")       // toast "“hello.txt” moved to trash" → Undo → item back
test("Paste from a local folder into Drive uploads the file")
test("Paste from Drive into a local folder downloads it")
```

- [ ] **Step 3: Implement the menu and guard changes**; run `npm test` until the new tests pass.

- [ ] **Step 4: README.** Feature bullet: Drive is "Experimental": browse, open, New Folder, Rename, Copy, Move, Paste between local and Drive, drag and drop, Move to Trash with Undo; never deletes permanently. "Not supported yet": shared drives, Shared with me, the Drive Trash view, editing Google Docs in place, uploading changes to an opened snapshot.

- [ ] **Step 5: Look at it** in `npm run dev:mock` (light and dark): Drive item menu, New Folder dialog, the clash dialog over a Drive folder.

- [ ] **Step 6: Commit** `feat: Google Drive writes in the UI`

---

### Task 7: Drag and drop between local folders and Drive

**Files:**
- Modify: `src/main.ts` (`dropOp`, `dropTargetAt` for the Google Drive sidebar row)
- Modify: `tests/drag.test.ts`, `tests/drive.test.ts`

**Interfaces:**
- Consumes: `runTransfer` (unchanged) and the commands above.
- `dropOp`: a drop between a local folder and Drive is a **copy** by default (different storage, like another volume); within Drive it is a **move**; the existing modifier keys still force copy or move. Drive items are never dropped on the app's Trash row (the op is `null` for cloud sources onto `trashPath`). The Google Drive sidebar row is a drop target for the account root when connected.

- [ ] **Step 1: Failing tests**

```ts
test("dropping a local file on a Drive folder copies it")       // source still in the local folder
test("dropping a Drive item on a local folder copies it")
test("dragging inside Drive moves the item")
test("a Drive item dropped on the Trash row does nothing")
test("forcing a move from Drive to local trashes the Drive item only after the copy") // mock records order
```

- [ ] **Step 2: Implement** `dropOp`'s cloud rule (compare `isCloud(src)` with `isCloud(dir)` before the device check) and the sidebar target.

- [ ] **Step 3: Run** `npm test`; **commit** `feat: drag files between local folders and Google Drive`

---

### Task 8: Close out the docs

**Files:**
- Modify: `docs/superpowers/specs/2026-10-24-google-drive-design.md` ("Implementation status": M2 and M3 done; note the pending-segment grammar under D2 and the final-path return values under "Command surface")
- Modify: `docs/superpowers/plans/2026-10-24-google-drive.md` (Tasks 11–13 point to this plan and are checked)

- [ ] **Step 1:** Update both files. **Step 2:** Run the full check set. **Step 3:** Commit `docs: record Google Drive writes as implemented`.
