# Google Drive Implementation Plan

> **For agentic workers:** This plan is documentation only until approved. When
> implementation starts, work one task at a time, keep the checkboxes in sync,
> and run the stated checks before moving on.

**Goal:** Add Google Drive to the file browser as a first-class location, using
the Drive v3 REST API directly so macOS and Linux share one implementation — no
OS mounts, no GVFS, no Google Drive for desktop.

**Architecture:** A location dispatcher in front of the existing path-based
commands. Local paths keep using `std::fs`; `gdrive://` paths go to a Drive
client behind a `DriveApi` trait. OAuth 2.0 (PKCE + loopback) runs in the Rust
backend, tokens live in the OS keychain, and the frontend keeps its current
grid/list/breadcrumb UI with a small number of cloud-aware branches.

**Tech stack:** Tauri 2, Rust (edition 2024), existing `serde`/`serde_json`,
new crates for OAuth (`oauth2`), HTTP (`reqwest` + rustls), token storage
(`keyring`) and percent-encoding; TypeScript + Vite with no new frontend
dependencies.

**Spec:** `docs/superpowers/specs/2026-10-24-google-drive-design.md`
**Tracking issue:** https://github.com/ahmadsayed/unshitty-mac-filebrowser/issues/12

**Decision (2026-10-24):** the OS-mount route is rejected. Fedora 45 / GNOME 51
ships no `gvfs-google` and no `libgdata`, so Linux has no Google Drive backend
to discover; macOS would work and Linux would not. The app therefore uses the
Drive v3 API itself, with OAuth in the backend and the same code on both
platforms. See "Why not the OS mount" in the spec.

**Status:** M1 (Tasks 1–10) implemented. M2/M3 (Tasks 11–13) were replaced
by `docs/superpowers/plans/2026-10-25-google-drive-writes.md` and are
implemented.
Task 0's live OAuth check is done: sign-in and browsing were tested with a
real Gmail account on 2026-10-25.

## Global Constraints

- **Platform agnostic.** No `cfg!(target_os)` in `src-tauri/src/cloud/`; no
  dependency on a Drive client, GVFS, rclone or FUSE. One code path on macOS
  and Linux.
- **Local browsing must not regress.** Every existing local command keeps its
  current behaviour and error text. Cloud support is additive.
- **Never lose data.** M1 is read-only. M2 uses Drive Trash, never permanent
  deletion. Cross-boundary moves remove the source last. No overwrite without
  the existing conflict dialog.
- **No Keep Versions on Drive.** Cloud paths never get a version marker, the
  New Folder checkbox is disabled, and version commands refuse them.
- **Cloud code is plain Rust.** Only `lib.rs` may touch `AppHandle`, events or
  Tauri types, per `AGENTS.md`. Modules must be unit-testable in a scratch
  crate.
- **No network in tests.** The `DriveApi` trait and a fixture transport keep
  `cargo test` and Vitest offline. Live tests are `#[ignore]` and opt-in.
- **Commands that wait on the network run through `blocking(…)`.** A slow Drive
  request must not stall other commands.
- **Every new command gets a mock** in `dev/mock-tauri/` and a test. Every new
  refusal gets a test.
- **Tokens never touch disk in plain text** unless the keyring is unavailable,
  and then only in a `0600` file with a visible warning.
- **Cargo.lock is committed.** Update it minimally when crates are added and
  check the diff only adds entries.
- **README and the "Not supported yet" section** are updated with the feature,
  its limits and the OAuth setup, in the same change that ships it.

## Review Focus

The parts most likely to bite first:

1. **Duplicate names.** Drive allows two files with the same name in one
   folder. If paths were name-based, selection, drag and the clipboard would
   merge them. The ID-anchored URI (D2) is the guard; test it explicitly.
2. **Stale names.** A file renamed in the Drive web UI keeps a valid id but an
   old name in a saved pin or back-stack entry. Operations must use the id and
   the UI must tolerate a mismatch.
3. **Pagination and rate limits.** A folder with 20,000 children is several
   pages and can hit per-user quotas; progress, cancel and backoff are not
   optional.
4. **Token expiry mid-operation.** A refresh must happen once, transparently,
   and a revoked token must produce "sign in again", not a raw 401.
5. **Google-native files.** Docs/Sheets/Slides have no media bytes; calling
   `files.get?alt=media` on them fails. They must open by URL.
6. **Cross-boundary moves.** A failed upload must not delete the local source;
   a failed download must not trash the Drive original.
7. **OAuth ergonomics.** Google requires the loopback IP literal, a state
   nonce and PKCE. An unverified app shows a warning screen; the UI must
   explain it rather than look broken.
8. **Cache growth.** Downloaded snapshots can be gigabytes; the LRU cap and
   `.part` cleanup need tests.
9. **Silent divergence.** A user edits a downloaded copy and expects it to
   sync. The notice in D11 and the README must be explicit.
10. **Version history.** `versioned_root` currently walks ancestors; it must
    short-circuit on `gdrive://` before any path logic.

---

## Task 0: Approval and dependency spike

**Files:**
- Modify: `src-tauri/Cargo.toml` (only when the spike starts)
- Modify: `src-tauri/Cargo.lock` (generated)
- Create (throwaway, not committed): `src-tauri/examples/drive_spike.rs`

**Interfaces:**
- Decides the OAuth client strategy (shared vs bring-your-own), the scope for
  M1, and confirms `oauth2`/`reqwest`/`keyring` versions compile on both
  targets.

- [ ] **Step 1:** Answer the open questions in the spec that affect M1: OAuth
      client, scope, one account, shared drives, downloaded-copy notice.
      **Answered:** one shipped client ID, `drive.readonly`, one account, My
      Drive only, downloaded-copy notice accepted.
- [ ] **Step 2:** Add the crates and record the exact versions in this plan.
      Expected: `oauth2` (PKCE + loopback), `reqwest` with
      `default-features = false, features = ["json", "rustls-tls", "stream"]`,
      `keyring`, `percent-encoding`. Do not add a frontend dependency.
- [ ] **Step 3:** Write the throwaway example: start the loopback flow, print
      the URL, accept one callback, exchange the code, call `about.get`, list
      one folder, download one file to `/tmp`. Never commit the example or any
      token.
- [ ] **Step 4:** Run it on Linux and (when available) macOS. Confirm the
      keyring round-trip works or the file fallback is used.
- [ ] **Step 5:** Remove the example, keep the dependencies only if the plan is
      approved, and commit `chore: add Google Drive client dependencies`.
      Include the verified versions in the commit body.

**Exit criteria:** the flow works end to end on at least one platform, and the
crate set is confirmed. No user-facing code yet.

---

## Task 1: Location grammar and dispatcher

**Files:**
- Create: `src-tauri/src/cloud/mod.rs`
- Create: `src-tauri/src/cloud/location.rs`
- Modify: `src-tauri/src/lib.rs` (add `mod cloud;`)
- Modify: `src-tauri/src/fs_ops.rs` (`Entry.cloud`)
- Test: `src-tauri/src/cloud/location.rs` `#[cfg(test)]`

**Interfaces:**
- Produces:
  ```rust
  pub enum Location { Local(PathBuf), Drive(DriveLocation) }
  pub struct DriveLocation { pub account: String, pub id: String, pub segments: Vec<Segment> }
  pub struct Segment { pub id: String, pub name: String }
  pub fn parse(path: &str) -> Location;
  pub fn is_cloud(path: &str) -> bool;
  pub fn format_segment(id: &str, name: &str) -> String;
  pub fn decode_segment(seg: &str) -> Option<Segment>;
  ```
- `Entry` gains
  `#[serde(skip_serializing_if = "Option::is_none")] pub cloud: Option<CloudEntry>`
  with `CloudEntry { account, id, mime_type, is_google_doc, is_shortcut,
  web_url, can_download }`.

- [ ] **Step 1:** Write failing tests for the grammar:
      `gdrive://acct/ID~Name` round-trip; spaces, unicode, `%`, `~`, `#`, `?`
      in names; duplicate names produce different URIs through different ids;
      root with a missing name; invalid ids (`..`, `/`, spaces); a name with a
      slash is rejected; a bare `gdrive://` is rejected; a local path never
      parses as Drive.
- [ ] **Step 2:** Implement `location.rs` with `percent-encoding`; keep it free
      of I/O and Tauri types. Run `cargo test` (expect PASS after Step 1).
- [ ] **Step 3:** Add `CloudEntry` to `Entry` with an optional field so local
      JSON is unchanged; add a test that local `list_dir` output has no `cloud`
      key.
- [ ] **Step 4:** Add a small `resolve(path) -> Location` dispatcher and tests
      proving local paths still resolve to `Local`.
- [ ] **Step 5:** `cargo test`, `npx tsc --noEmit` (no frontend change yet),
      commit `feat: add Google Drive location grammar`.

---

## Task 2: Account store and token storage

**Files:**
- Create: `src-tauri/src/cloud/accounts.rs`
- Create: `src-tauri/src/cloud/token.rs`
- Modify: `src-tauri/src/lib.rs` (managed `CloudState`, commands)
- Test: both modules' `#[cfg(test)]`

**Interfaces:**
- Produces:
  ```rust
  pub struct DriveAccount { pub id: String, pub email: String, pub name: String,
                            pub root_id: String, pub root_path: String,
                            pub scopes: Vec<String>, pub connected_at: i64 }
  pub struct Accounts { /* accounts.json, at most one in v1 */ }
  pub trait TokenStore { fn get(&self, account: &str) -> Result<Option<Token>, String>;
                         fn set(&self, account: &str, token: &Token) -> Result<(), String>;
                         fn delete(&self, account: &str) -> Result<(), String>; }
  ```
- Commands: `drive_accounts`, `drive_disconnect`.
- `Token` must implement `Debug` manually and print only `"Token(<redacted>)"`.

- [ ] **Step 1:** Write failing tests: round-trip `accounts.json` through a temp
      dir; a missing file yields empty; malformed JSON yields empty plus a
      warning, not a crash; the file never contains a token; redaction.
- [ ] **Step 2:** Implement `accounts.rs` using `config_store::write_json`.
- [ ] **Step 3:** Write failing tests for `token.rs` with an in-memory
      `TokenStore`: key names are `drive:<account-id>`; fallback file is `0600`
      (Unix mode test); delete is idempotent; keyring errors surface as a
      message, not a panic.
- [ ] **Step 4:** Implement the keyring wrapper with the file fallback and a
      `used_fallback` flag the UI can show.
- [ ] **Step 5:** Register `drive_accounts` and `drive_disconnect`, register
      `CloudState`, and add the mocks in `dev/mock-tauri/core.ts`.
- [ ] **Step 6:** `cargo test`, `npm test`, commit
      `feat: store Google Drive accounts and tokens`.

---

## Task 3: OAuth flow

**Files:**
- Create: `src-tauri/src/cloud/oauth.rs`
- Modify: `src-tauri/src/lib.rs` (`drive_connect_start`,
  `drive_connect_finish`, `drive_connect_cancel`)
- Modify: `src/api.ts`, `dev/mock-tauri/core.ts`
- Test: `src-tauri/src/cloud/oauth.rs` `#[cfg(test)]`

**Interfaces:**
- Produces:
  ```rust
  pub struct Flow { pub id: String, pub auth_url: String, /* listener, verifier, state */ }
  pub fn start(client: &OAuthConfig) -> Result<Flow, String>;
  pub fn finish(flow: Flow, timeout: Duration) -> Result<Token, String>;
  pub fn refresh(config: &OAuthConfig, refresh_token: &str) -> Result<Token, String>;
  pub fn revoke(token: &str);
  ```
- Commands return `{ flow_id, auth_url }`, then `DriveAccount`, then `void`.

- [ ] **Step 1:** Write failing tests with an injectable token transport:
      RFC 7636 PKCE vector; state is random and compared; callback parsing
      accepts `code` + `state`, rejects `error=access_denied`, rejects a
      mismatched state, rejects a second callback; timeout returns a clean
      "cancelled" error; the verifier never appears in an error; a missing
      `refresh_token` is a distinct error.
- [ ] **Step 2:** Implement PKCE, state, the loopback listener on
      `127.0.0.1:0`, and the exchange against a configurable token URL (so
      tests use a local fake). Bind only the loopback interface.
- [ ] **Step 3:** Implement `refresh` and `revoke`; map `invalid_grant` to
      "Sign in again."
- [ ] **Step 4:** Implement the three commands. `start` returns immediately;
      `finish` runs in `blocking()` with the 180 s timeout; `cancel` closes the
      listener. Call `about.get` before returning the account so a bad scope
      fails at connect time.
- [ ] **Step 5:** Write the frontend wrappers and mock commands. The mock never
      opens a browser; tests drive `start`/`finish`/`cancel`.
- [ ] **Step 6:** `cargo test`, `npx tsc --noEmit`, `npm test`, commit
      `feat: connect Google Drive with OAuth PKCE`.

---

## Task 4: Drive client

**Files:**
- Create: `src-tauri/src/cloud/drive.rs`
- Create: `src-tauri/src/cloud/mapping.rs`
- Test: both modules' `#[cfg(test)]`

**Interfaces:**
- Produces:
  ```rust
  pub trait DriveApi {
      fn about(&self) -> Result<About, CloudError>;
      fn list(&self, folder: &str, page: Option<String>) -> Result<ListPage, CloudError>;
      fn get(&self, id: &str) -> Result<DriveFile, CloudError>;
      fn download(&self, id: &str, dest: &Path, progress: &dyn Fn(u64, u64)) -> Result<(), CloudError>;
      fn export(&self, id: &str, mime: &str, dest: &Path) -> Result<(), CloudError>;
      fn create_folder(&self, parent: &str, name: &str) -> Result<DriveFile, CloudError>;
      fn rename(&self, id: &str, name: &str) -> Result<DriveFile, CloudError>;
      fn copy(&self, id: &str, parent: &str, name: &str) -> Result<DriveFile, CloudError>;
      fn move_to(&self, id: &str, from: &str, to: &str) -> Result<DriveFile, CloudError>;
      fn trash(&self, id: &str) -> Result<(), CloudError>;
      fn restore(&self, id: &str) -> Result<(), CloudError>;
      fn upload(&self, parent: &str, name: &str, src: &Path, progress: &dyn Fn(u64, u64)) -> Result<DriveFile, CloudError>;
  }
  ```
- `DriveFile` mirrors the fields in the spec's field mask.
- `CloudError` covers the classes in D18.

- [ ] **Step 1:** Write fixture JSON for `about`, a two-page listing, a Google
      Doc, a shortcut, a shared file, a 404 and a 429. Store them under
      `src-tauri/src/cloud/testdata/` (or inline strings; keep the test crate
      self-contained).
- [ ] **Step 2:** Write failing tests with a fake transport:
      pagination follows `nextPageToken`; the `q` string escapes quotes and
      backslashes; the field mask is stable; `size` is absent for folders and
      Google docs; `trashed` and `starred` map correctly; 401 → one refresh →
      retry; 429 honours `Retry-After` with an injected sleeper; 403 quota and
      permission are distinct; 404 is not retried; `download` on a Google doc
      is refused with the typed error.
- [ ] **Step 3:** Implement the `reqwest` client with a shared connection pool,
      30 s connect / 60 s request timeouts and at most 4 in-flight requests.
      Use `https://www.googleapis.com/drive/v3` and
      `https://oauth2.googleapis.com/revoke`.
- [ ] **Step 4:** Implement retry with exponential backoff and jitter
      (max 5 attempts) and streaming downloads to `.part` with a progress
      callback.
- [ ] **Step 5:** Implement `mapping.rs`: `DriveFile` → `Entry` with the
      `gdrive://` URI, file-kind hints for Docs/Sheets/Slides, and Drive
      metadata → the `FileProperties` subset.
- [ ] **Step 6:** `cargo test`, commit `feat: add the Google Drive API client`.

---

## Task 5: Read-only dispatcher in the existing commands

**Files:**
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/src/fs_ops.rs` (`list_dir`, `path_info`,
  `folder_summary`/`item_counts` guards)
- Modify: `src-tauri/src/versions/*` (short-circuit cloud paths)
- Test: dispatch tests in `cloud/mod.rs`

**Interfaces:**
- `list_dir` resolves `Location::Drive` and calls `DriveApi::list`; it returns
  `Entry` values and emits `drive-list-progress` `{ path, loaded, page }` while
  paging.
- `path_info` returns `{ is_dir, device: 0 }` for Drive.
- `item_counts` returns `None` for cloud folders.
- Local-only commands return `Err(CloudError::Unsupported.message())`.

- [ ] **Step 1:** Write failing tests for `resolve` plus the guards: a Drive
      path never reaches `std::fs`; `versioned_root` returns `null` for Drive;
      `file_properties`, `folder_size`, `deleted_files`, `list_versions`,
      `trash_info`, `empty_trash` all return the typed message; a local path
      still works.
- [ ] **Step 2:** Implement the dispatch in the command wrappers, keeping the
      plain logic in testable functions. Do not duplicate command names.
- [ ] **Step 3:** Add a page cap and a 10,000-child notice; wire the progress
      event and cancellation flag. Add tests for the cap and cancellation.
- [ ] **Step 4:** Add the mock `list_dir` branch for the fake Drive tree and a
      `tests/mock-core.test.ts` case.
- [ ] **Step 5:** `cargo test`, `npm test`, `npx tsc --noEmit`, commit
      `feat: list Google Drive folders through the existing commands`.

---

## Task 6: Sidebar entry and connect dialog

**Files:**
- Modify: `index.html` (sidebar section if needed)
- Modify: `src/main.ts` (`buildSidebar`, `sidebarRow`, connect flow)
- Modify: `src/ui.ts` (dialog layout only; reuse `openDialog`)
- Modify: `src/styles.css`
- Modify: `src/icons.ts` (generic cloud/drive glyph)
- Modify: `dev/mock-tauri/core.ts`
- Test: `tests/drive.test.ts`

**Interfaces:**
- `driveAccounts()`, `driveConnectStart(scopes?)`, `driveConnectFinish(id)`,
  `driveConnectCancel(id)`, `driveDisconnect(account)` in `api.ts`.
- A module-level `driveAccount: DriveAccount | null` and
  `driveConnecting: { flowId: string } | null`.

- [ ] **Step 1:** Write failing frontend tests: signed out shows the row and
      opens the dialog; Open Browser calls `drive_connect_start`; the waiting
      state can be cancelled; a finished flow replaces the row with the account
      label; an auth error shows the classified message; disconnect asks for
      confirmation and clears the row.
- [ ] **Step 2:** Add the sidebar row and its states. Keep the row above
      mounted volumes, below Trash. Use a generic cloud glyph (no Google
      assets).
- [ ] **Step 3:** Build the dialog with `openDialog`: title, scope explanation,
      keychain note, unverified-app note, Cancel / Open Browser, waiting state.
- [ ] **Step 4:** Wire navigation to the account's root URI on click. Add the
      expired-token emblem and "Sign In Again" action.
- [ ] **Step 5:** Add the mock tree and commands; ensure `npm run dev:mock`
      works offline.
- [ ] **Step 6:** `npm test`, `npx tsc --noEmit`, `npm run build`, commit
      `feat: add a Google Drive sidebar entry and sign-in dialog`.

---

## Task 7: Drive navigation and breadcrumbs

**Files:**
- Modify: `src/main.ts` (`basename`, `parentOf`, `crumbsFor`, `segments`,
  `locationName`, `submitPathEntry`, `navigate`, `childOf`, `revealPath`)
- Modify: `src/icons.ts` for Docs/Sheets/Slides kinds
- Test: `tests/drive.test.ts`

**Interfaces:**
- `isCloud(path: string): boolean` and `cloudSegments(path): { id, name }[]`.
- `crumbsFor` gains a Drive branch with the account crumb and "My Drive".

- [ ] **Step 1:** Write failing tests: navigating to a fake root builds
      `Google Drive / My Drive / Documents`; labels are percent-decoded; back,
      forward and up work; a pin to a Drive folder renders its real name; the
      window title uses the folder name; a duplicate-name file keeps two
      distinct selection targets.
- [ ] **Step 2:** Implement the URI-aware helpers without changing local
      behaviour. `parentOf` drops the last segment; the account/root is the
      boundary.
- [ ] **Step 3:** Update the path entry: pre-fill the URI, accept a pasted
      `gdrive://` URI or a `drive.google.com` URL (resolve to an id), reject a
      POSIX path inside Drive with the D14 message.
- [ ] **Step 4:** Add the Docs/Sheets/Slides file kinds and generic icons.
- [ ] **Step 5:** Add focus refresh for cloud paths with the 3 s debounce and a
      test that two rapid focus events cause one list call.
- [ ] **Step 6:** `npm test`, `npx tsc --noEmit`, commit
      `feat: navigate Google Drive with breadcrumbs and refresh`.

---

## Task 8: Open, download cache and "Open in Browser"

**Files:**
- Create: `src-tauri/src/cloud/cache.rs`
- Modify: `src-tauri/src/lib.rs` (`drive_open`, `drive_download`)
- Modify: `src/api.ts` (`openFile` dispatch), `src/main.ts` (`openEntry`)
- Modify: `dev/mock-tauri/core.ts`
- Test: `src-tauri/src/cloud/cache.rs`, `tests/drive.test.ts`

**Interfaces:**
- `drive_open(path) -> { local_path: Option<String>, web_url: Option<String> }`
- Cache path: `<app cache>/drive/<account>/<id>/<modified>-<sanitized-name>`.
- LRU budget: 2 GB; `.part` files removed on error/cancel.

- [ ] **Step 1:** Write failing tests: opening a binary downloads once and
      reuses the cached path for the same `modifiedTime`; a changed
      `modifiedTime` downloads again; a Google Doc returns only `web_url`; a
      failed download leaves no `.part`; eviction removes the least recently
      used entry; the sanitized name cannot escape the cache directory.
- [ ] **Step 2:** Implement `cache.rs` with temp-then-rename, a size check and
      the LRU index.
- [ ] **Step 3:** Implement `drive_open` and `drive_download`; emit
      `drive-download-progress`. Use the opener on the returned local path.
- [ ] **Step 4:** Update `openFile` to branch on `isCloud`; show the
      once-per-session "Opened a local copy…" toast; add Recent for opened
      cloud files only if the user keeps that behaviour (see open question 6).
- [ ] **Step 5:** Add the mock download (returns an existing temp path) and
      tests.
- [ ] **Step 6:** `cargo test`, `npm test`, `npx tsc --noEmit`, commit
      `feat: open Google Drive files from a local snapshot`.

---

## Task 9: Drive thumbnails

**Files:**
- Modify: `src-tauri/src/lib.rs` (cloud branch in `thumbnail`)
- Create or extend: `src-tauri/src/cloud/cache.rs` for thumbnail bytes
- Test: `src-tauri/src/cloud/cache.rs`, `tests/drive.test.ts`

**Interfaces:**
- `thumbnail` on a Drive URI fetches `thumbnailLink=w<size>-h<size>` (or
  `=s<size>`), decodes and re-encodes PNG, and returns the bytes.
- Cache key: `(account, id, modifiedTime, size)`.

- [ ] **Step 1:** Write failing tests: a cached thumbnail is returned without a
      second fetch; a changed `modifiedTime` invalidates it; non-image MIME
      types return empty bytes; decoding failures return empty (the frontend
      falls back to the type icon); the 64 MB budget still applies.
- [ ] **Step 2:** Implement the fetch/normalize/cache path using the existing
      `image` crate. Keep it off the local `thumbnails.rs` code path except for
      PNG encoding helpers.
- [ ] **Step 3:** Add the mock thumbnail and a test that only rendered rows
      request thumbnails.
- [ ] **Step 4:** `cargo test`, `npm test`, commit
      `feat: show Google Drive thumbnails`.

---

## Task 10: Properties, menus and M1 polish

**Files:**
- Modify: `src/main.ts` (item menus, background menu, properties)
- Modify: `src/versions.ts` (defensive cloud guards)
- Modify: `src/styles.css`
- Modify: `README.md`
- Test: `tests/drive.test.ts`, `tests/menus.test.ts`

**Interfaces:**
- A cloud item menu with Open, Open in Browser (Docs), Copy, Cut, Rename,
  Move to Trash, Star, Properties.
- Hidden/disabled: Open Item Location, Versions…, Deleted Files….

- [ ] **Step 1:** Write failing tests for the menu contents in Drive vs local;
      Properties rows for a cloud file; the absence of version controls; the
      New Folder checkbox disabled inside Drive.
- [ ] **Step 2:** Implement the menu and Properties branches. Use
      `CloudError::Unsupported` for anything not implemented yet.
- [ ] **Step 3:** Add the expired-token banner/row state and the fallback-token
      warning.
- [ ] **Step 4:** Update the README: feature list, OAuth setup, the local-copy
      notice, read-only M1, and the "Not supported yet" section.
- [ ] **Step 5:** Run the full M1 gate: `npx tsc --noEmit`, `npm run build`,
      `npm test`, `cargo test`, `npm run crap`. Smoke-test `npm run dev:mock`
      in light and dark themes.
- [ ] **Step 6:** Commit `feat: finish Google Drive browsing` and tag M1.

---

## Task 11: M2 — New Folder, Rename, Copy and Move

**Files:**
- Modify: `src-tauri/src/cloud/drive.rs` (completed in Task 4; this task wires it)
- Modify: `src-tauri/src/lib.rs` and `fs_ops.rs` dispatch
- Modify: `src/main.ts` (conflict dialog already exists)
- Test: dispatch, conflict and safety tests

**Interfaces:**
- `make_dir`, `rename_entry`, `copy_entry`, `move_entry` gain Drive branches.
- Cross-boundary copy/move uses `drive_download` / `drive_upload`.

- [ ] **Step 1:** Write failing tests: new folder inside Drive; rename keeps the
      id and updates the URI; copy within Drive is server-side and gets a new
      id; move within Drive updates parents; a local → Drive move uploads then
      deletes the local source only on success; a Drive → local move downloads
      then trashes the original only after the copy is complete; destination
      inside the source is refused; same-path is refused; an existing target
      without `overwrite` is refused.
- [ ] **Step 2:** Implement the write branches with temporary names and the
      existing conflict prompt. Never overwrite by default.
- [ ] **Step 3:** Add progress events and the mock commands; add tests for a
      cancelled upload leaving no temporary file.
- [ ] **Step 4:** `cargo test`, `npm test`, commit
      `feat: create, rename, copy and move Google Drive items`.

---

## Task 12: M2 — Upload, Drive Trash and Undo

**Files:**
- Modify: `src-tauri/src/lib.rs` (`drive_upload`, `trash_entry`,
  `restore_entry`)
- Modify: `src/main.ts` (Undo bar, paste)
- Test: safety tests per refusal

**Interfaces:**
- Upload: resumable, temp name, verify, rename.
- Trash: `trashed=true`; Undo: `trashed=false`.
- No permanent-delete command for Drive.

- [ ] **Step 1:** Write failing tests: paste from local uploads a file and
      leaves the source intact; a failed upload leaves no final-name file; Move
      to Trash sets the flag and Undo clears it; the app's Trash view never
      lists Drive items; `delete_permanently` and `empty_trash` refuse Drive
      URIs.
- [ ] **Step 2:** Implement upload and the trash/restore branches; reuse the
      existing Undo toast.
- [ ] **Step 3:** Add progress and the mock tree mutations.
- [ ] **Step 4:** `cargo test`, `npm test`, commit
      `feat: upload to Google Drive and use Drive Trash`.

---

## Task 13: M3 — Drag and drop between local and Drive

**Files:**
- Modify: `src/main.ts` (`dragover`/`drop`, `dragTo`, cross-boundary routing)
- Modify: `src-tauri/src/lib.rs` if a combined transfer command is needed
- Test: `tests/drag.test.ts`, `tests/drive.test.ts`

**Interfaces:**
- Local → Drive: upload (copy or move per modifier).
- Drive → local: download (copy or move; move trashes only after success).
- Drive → Drive: server-side move/copy.

- [ ] **Step 1:** Write failing tests: local file dropped on a Drive folder
      uploads; a Drive item dropped on a local folder downloads; a cross-
      boundary move never deletes the source on a simulated failure; drag
      ghosts and drop targets use the existing layout maths.
- [ ] **Step 2:** Implement the routing, keeping the existing drag code and
      using `isCloud` guards.
- [ ] **Step 3:** `npm test`, `cargo test`, commit
      `feat: drag files between local folders and Google Drive`.

---

## Task 14: M4 backlog (not scheduled)

- Shared drives and Shared with me.
- Server-side Drive search.
- Multiple accounts in the sidebar.
- Drive Trash browser.
- Offline pinning of Drive folders.
- "Download a Copy…" and "Upload…" menu items.
- Optional change polling (`changes.list`) while a Drive folder is open.

---

## Self-Review Notes

- **Does the path model survive external changes?** Yes: the id is the
  identity; names are display hints. Operations never resolve by name.
- **Could the read-only milestone write anything?** No command capable of a
  write is registered before M2; the Drive client has write methods but they
  are unreachable from the UI until the dispatcher wires them.
- **What happens when the token is revoked?** One refresh attempt, then a typed
  auth error and a "Sign In Again" action. No retry loop.
- **What if Drive returns duplicate names?** The URI differs by id, so selection
  and drag stay correct; the "Keep Both" naming rule keeps the visible folder
  unique in M2.
- **What about Keep Versions?** `versioned_root` short-circuits on `gdrive://`;
  the UI never offers the checkbox; version commands refuse with a message.
- **Is any OS-specific code needed?** Only the keyring backend, behind a crate;
  everything else uses cross-platform Rust, the opener plugin and the webview.
- **Does every new command have a mock and a test?** The task steps require
  both; the CRAP gate runs on both languages at every commit.
