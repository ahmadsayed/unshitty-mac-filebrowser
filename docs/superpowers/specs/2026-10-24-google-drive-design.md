# Google Drive — Design

**Date:** 2026-10-24
**Status:** M1 (read-only) implemented. M2 (writes) and M3 (cross-boundary
drag and drop) are not started. Nothing in M2/M3 is implemented.
**Tracking:** https://github.com/ahmadsayed/unshitty-mac-filebrowser/issues/12
**Related:** `2026-10-23-file-browser-design.md` (the app's overall design),
`2026-10-24-keep-versions-design.md` (version history), `AGENTS.md` (rules every
change must keep)

## Intent and the why

The file browser can only see local disks today. Google Drive is the first
remote store it should show, and it must work **identically on macOS and
Linux** — no Google Drive for desktop, no GVFS, no GNOME Online Accounts, no
rclone, no FUSE. The app talks to the Drive v3 REST API itself.

The user's words while planning: *"I want os agnostic approach."* That rules out
the cheap route of discovering OS-provided mounts and makes OAuth and the Drive
API part of the app.

Success looks like: the user adds a Google account once in the file browser,
sees **Google Drive** in the sidebar, browses My Drive with the same grid/list
views and breadcrumbs as a local folder, opens a document in its default app,
and — in the second milestone — creates folders, uploads, renames, moves and
trashes items. Nothing about local browsing changes.

## Implementation status (2026-10-25)

M1 is in the tree:

- `gdrive://` path grammar and the location dispatcher.
- `accounts.json`, keychain token storage with a `0600` fallback, and the
  loopback PKCE sign-in.
- Drive listing (paged), metadata, read-only Properties, thumbnail fetch and
  PNG normalization, and Open/Open in Browser.
- Sidebar row, connect/cancel/disconnect dialog, breadcrumbs, up/back/forward,
  focus refresh and the read-only menus.
- A shipped Desktop client ID and its non-confidential client secret
  (`drive.readonly`), compiled from the git-ignored `.env` or the
  `FILEBROWSER_GOOGLE_CLIENT_ID`/`_SECRET` build environment.
- One shared HTTP client; access tokens cached in memory (the keychain is read
  once per account and written on refresh); refreshes serialized so parallel
  requests refresh once, keeping the refresh token Google omits from refresh
  responses; retry with backoff and `Retry-After` for 429, 5xx and rate-limit
  403s (3 attempts); thumbnail links remembered from listings. The backend has
  no separate concurrency limit: the frontend's 4-request thumbnail queue is
  the only caller that fans out.
- Durable tests: 263 Rust, 98 frontend, plus the mock preview.

Not started: M2 writes (new folder, rename, copy, move, upload, Drive Trash,
Undo), M3 cross-boundary drag and drop, shared drives, multiple accounts,
server-side search. The live consent screen, token exchange and browsing were
tested with a real Gmail account on 2026-10-25; automated tests use a fake
exchange and a local fake server.

## Why not the OS mount

The cheaper alternative is to reuse whatever Google Drive the OS already
exposes: Google Drive for desktop on macOS
(`~/Library/CloudStorage/GoogleDrive-<email>/…`) and GNOME Online Accounts +
GVFS on Linux (`/run/user/<uid>/gvfs/google-drive:…`). That removes OAuth and
the Drive client from the app entirely. It was rejected because it is not
portable — and on the development machine it is not even available.

Checked on Fedora 45 / GNOME 51 on 2026-10-24 (queries only, nothing
installed):

- `gvfs` 1.62, `gvfs-fuse` 1.62 and `gnome-online-accounts` 3.58 are installed.
- `gvfs-goa` ships only the OneDrive backend (`gvfsd-onedrive`).
- Fedora 45 has **no `gvfs-google` subpackage** and no available `libgdata`;
  no installed or available package provides `gvfsd-google`.
- `/run/user/<uid>/gvfs` is empty and no Google account is configured.

So the OS route would work on macOS (a first-party client) but not on the
primary Linux machine without a third-party client (rclone, grive2) or an
out-of-distro legacy stack. It would also mean two mount layouts, two delete
semantics and an OS-specific "not available" state. The app is meant to behave
the same on both systems, so the in-app API is the only route that meets the
requirement.

The price of this decision is that the app owns OAuth (PKCE + loopback +
keychain), the Drive client and its cache. That is exactly the work planned
below.

## Goals

1. **Connect an account in-app.** OAuth 2.0 Authorization Code + PKCE in the
   system browser; refresh tokens stored in the OS keychain (Keychain on
   macOS, Secret Service on Linux). The same code path on both platforms.
2. **Browse Drive as a location.** A sidebar entry, a stable URI scheme, list
   and grid views, sorting, folder navigation, back/forward/up, breadcrumbs,
   and the existing focus refresh.
3. **Open files.** Download a read-only local copy on demand and hand it to the
   default app. Google Docs/Sheets/Slides open in the browser (they have no
   bytes to download).
4. **Thumbnails.** Use Drive's thumbnail URL, normalized to PNG by the existing
   thumbnail pipeline and kept in the existing bounded cache.
5. **Write access in a second milestone.** New Folder, Rename, Copy, Move,
   upload from local, Move to Drive Trash, Undo, and drag and drop between local
   folders and Drive.
6. **Never lose data.** No permanent deletion in the app; Drive Trash is the
   only delete. A cross-boundary move removes the source only after the copy
   has fully succeeded. No overwrite without the existing conflict prompt.

## Non-goals (v1)

- **OS mounts and cloud-sync clients.** No dependency on Google Drive for
  desktop, GVFS, rclone, macFUSE, or OS accounts.
- **Offline editing and sync-back.** A downloaded copy is a snapshot. Edits to
  it are not uploaded.
- **Google Docs editing.** The app opens the web editor; it does not render or
  edit Google-native documents.
- **Live change notifications.** No Drive `changes.watch` push channel. The
  existing focus refresh is the refresh mechanism, plus a manual reload.
- **Keep Versions on Drive.** Version history is local-only; the feature is
  disabled on cloud locations (see D9).
- **Server-side Drive search.** The search bar keeps filtering the current
  folder; Drive-wide search is a later feature.
- **Sharing UI** (permissions, links, "Shared with me"). Shared drives and
  shared-with-me are a later phase, not v1.
- **Multi-account UI** in v1. The data model allows it; the sidebar shows one
  account.
- **Windows.** Not a target today; the design does not add any OS-specific
  code, so a future Windows port only needs a keyring backend.

## Decisions

### Made by the user

- **U1. Platform agnostic.** One implementation for macOS and Linux through
  the Drive API. No OS mount discovery as the integration mechanism.
- **U2. Plan first.** This revision is design and plan only; implementation
  waits for approval.
- **U3. The OS mount is rejected.** Confirmed after checking the Linux side:
  Fedora 45 / GNOME 51 has no `gvfs-google` and no `libgdata`, so there is no
  Drive backend to discover (see "Why not the OS mount"). The app talks to
  Drive itself.
- **U4. One shipped client ID.** The app embeds a single Desktop-app OAuth
  client ID; users do not create a Google Cloud project.

### Proposed by the designer (review these)

**D1. A location abstraction, not a second filesystem.**
Paths stay strings. A location is either a local absolute path or a Drive URI.
Every command that takes a path resolves the kind first:

```rust
pub enum Location {
    Local(PathBuf),
    Drive(DriveLocation), // account + file id + display path
}
```

Local code paths are untouched. Cloud code lives behind the same command names,
so the frontend keeps calling `list_dir`, `make_dir`, `rename_entry`, and so on.

**D2. Drive paths are ID-anchored URIs with names attached for display.**

```text
gdrive://<account-key>/<file-id>~<percent-encoded-name>
gdrive://<account-key>/<file-id>~<percent-encoded-name>/<file-id>~<name>
```

Example:

```text
gdrive://8f3c/R0OT~My%20Drive/1AbC~Documents/9xYz~report.docx
```

- The **file id is authoritative**; the name is a display hint. Operations never
  resolve by name, only by id.
- Drive allows two files with the same name in one folder. IDs keep the paths
  unique, so selection, drag, pins and the clipboard keep working.
- Names in a stored path can go stale when the item is renamed elsewhere. The
  id still points at the right file; the next listing or navigation refreshes
  the label.
- `~` separates the two parts; names are percent-encoded (space, `%`, `#`, `?`,
  `~`, non-ASCII). Drive names cannot contain `/`.
- The root is the account's real root folder id from `about.get`, labelled
  `My Drive`.
- The frontend treats these strings as opaque except in the path helpers that
  learn a `gdrive://` case, and in the breadcrumb builder, which parses the
  segments for labels.

**D3. One account in v1, modeled as a list.**
`accounts.json` stores an array with at most one entry so multiple accounts can
be added later without a migration.

**D4. OAuth 2.0 Authorization Code + PKCE, loopback redirect, system browser.**
- Auth endpoint `https://accounts.google.com/o/oauth2/v2/auth`; token endpoint
  `https://oauth2.googleapis.com/token`.
- Redirect `http://127.0.0.1:<random port>` (the loopback IP literal, which
  Google requires for desktop clients), with a `state` nonce and PKCE S256.
- `access_type=offline` + `prompt=consent` on first connect so Google returns a
  refresh token.
- The flow is split into `drive_connect_start` (bind listener, return the URL,
  the frontend opens it with the opener plugin), `drive_connect_finish`
  (wait for the callback, exchange the code, 180 s timeout) and
  `drive_connect_cancel`. This keeps a slow user from holding a worker.
- Scopes: `drive.readonly` for the read-only milestone; `drive` for the
  write milestone. An unverified OAuth app shows Google's "Google hasn't
  verified this app" warning; this is documented, not worked around.
- The Desktop client's `client_secret` is required by Google's token endpoint
  and is sent in the request body, never in the auth URL. It is built in from
  the git-ignored `.env`; for an installed app Google treats it as
  non-confidential, and PKCE still protects the exchange.

**D5. Tokens live in the OS keychain, never in the config file or logs.**
The `keyring` crate gives one API on macOS (Keychain) and Linux (Secret
Service). If no keyring is available, fall back to a `0600` token file in the
app config directory with an explicit warning in the UI; never silently.
Access tokens are kept in memory; refresh tokens are persisted. A
`Debug` implementation on the token type must redact everything.

**D6. `reqwest` + rustls, synchronous client, wrapped in the existing
`blocking()` helper.**
No OpenSSL, no platform TLS differences. Network waits run on Tauri's blocking
pool so a slow Drive request never stalls other commands. A shared client has a
timeout, a small concurrency limit (4) and retry with exponential backoff and
jitter for 429/5xx; `Retry-After` is honoured. A `401` triggers one token
refresh and one retry.

**D7. The Drive client is tested without Google.**
`DriveClient` is a concrete blocking client with a configurable base URL. Unit
tests point it at a local loopback server serving fixtures; the token exchange
is behind a fake `TokenExchange`. No test opens a connection to Google.

**D8. Read-only is the first shippable milestone.**
Connect, browse, breadcrumbs, open, thumbnails, properties, refresh, sign out.
Writes are milestone 2. This limits the risk while the path model and auth are
new, and matches the project's "never lose data" rule.

**D9. Keep Versions does not apply to Drive.**
`versioned_root` returns `null` for `gdrive://` locations; the New Folder
"Keep versions" checkbox is disabled inside Drive with a short explanation; the
stacked-folder icon never appears; Properties shows no version rows. History is
local storage next to the file, and there is no file to sit next to. A future
"keep a local copy with history" feature is a separate design.

**D10. Move to Trash means Drive Trash.**
`trash_entry` on a Drive location sets `trashed=true`. Undo sets it back with
`trashed=false` (Drive restores the original parents). Drive keeps trashed
items for 30 days. The app's Trash view stays local-only in v1; browsing Drive
Trash is a later feature. **There is no permanent-delete command for Drive**
in v1; the web UI remains the place for that. This is deliberate.

**D11. Open downloads a snapshot to the app cache.**
`open_file` on a Drive path:
1. Fetch metadata (`modifiedTime`, `size`, `mimeType`).
2. If it is a Google-native doc, return its `webViewLink` and open the browser.
3. Otherwise download to
   `<app cache>/drive/<account>/<file-id>/<modified>-<name>` (temp file then
   rename), then open that path with the opener plugin.
4. Show a one-line toast: "Opened a local copy. Changes are not uploaded to
   Google Drive." (Once per session, not per file.)
Edits to the copy are local and disposable; the cache is bounded and evictable.

**D12. Thumbnails come from Drive's `thumbnailLink`.**
The backend appends `=s<size>`, downloads the bytes and decodes/re-encodes them
to PNG with the existing `image` crate, because `src/thumbnails.ts` assumes PNG.
Thumbnails are cached by `(account, file id, modifiedTime, size)` under the app
cache and are subject to the same byte budget as local thumbnails. No thumbnail
link means the normal file-type icon.

**D13. Listing fetches all pages, with a visible limit.**
`files.list` uses `pageSize=1000`, `fields` limited to what `Entry` needs,
`orderBy=name_natural`, and follows `nextPageToken`. Large folders can be slow,
so the command emits progress events and can be cancelled; a folder with more
than 10,000 children shows a notice and the app's normal "large folder" warning.
(The local view is already virtualized, so rendering is not the bottleneck.)
Incremental "load more" is a follow-up, tracked in the plan.

**D14. The path bar is display-only for Drive.**
Breadcrumbs are built from the URI segments: `Google Drive / My Drive /
Documents / report.docx`. Ctrl+L still opens the path entry, but it is
pre-filled with the URI and accepts a pasted `gdrive://` URI or a plain Drive
URL (converted to a URI after resolving the id). Typing a POSIX path while in
Drive is rejected with "Enter a Google Drive location or a full path." A
`cloud_segments` command is not needed because the URI carries the names.

**D15. Frontend changes stay narrow.**
- `Entry` gains an optional `cloud` object (serde omits it for local entries):
  `{ account, id, mime_type, is_google_doc, is_shortcut, web_url, can_download }`.
- `path` for a cloud entry is the URI above.
- `isCloud(path)` guards the new branches.
- `basename`/`parentOf` learn the URI case; `crumbsFor` gains a Drive branch.
- `openFile` dispatches to `drive_open` for cloud paths.
- Sidebar gains a **Google Drive** row and a connect dialog.
- `versions` commands reject cloud paths before reaching the filesystem.
- No UI framework, no new frontend dependencies.

**D16. Local-only commands answer with a typed, human message.**
Versions, Deleted Files, folder sizes, symlink targets, permissions/owner,
"Open Item Location", and local Trash operations return
`"This action is not available for Google Drive items yet."` rather than
falling through to `std::fs` and producing a confusing OS error. The frontend
hides or disables those menu items for cloud selections.

**D17. Data safety rules for writes (milestone 2).**
- Uploads and copies create the target under a temporary name, then rename into
  place when the API supports it; a failed upload never leaves a half file with
  the final name.
- Local → Drive move deletes the local source only after the upload reports
  success and the resulting id is known.
- Drive → local move deletes the Drive item (Drive Trash) only after the local
  copy is complete and verified by size.
- Name clashes use the existing Replace / Skip / Keep Both dialog. Drive permits
  duplicates, but the app keeps its own invariant that a folder shows unique
  names on screen; "Keep Both" appends a suffix as it does locally.
- No operation overwrites an existing Drive file unless the user chose Replace
  in that dialog.

**D18. Errors are classified before they become messages.**
`CloudError` distinguishes: not connected, authorization expired, forbidden,
not found, rate limited, quota exceeded, network/timeout, unsupported item.
Messages follow GNOME wording, e.g. "Google Drive is not connected.",
"Google Drive refused the request. Sign in again.", "Google Drive is
temporarily unavailable. Try again." The frontend offers **Sign In** for auth
errors and **Retry** for network errors.

**D19. Caches are bounded and disposable.**
- `<app cache>/drive/` holds downloaded snapshots and Drive thumbnails.
- The download cache is capped at 2 GB with least-recently-used eviction;
  thumbnails use the existing 64 MB budget.
- A partial download is written to `.part` and removed on failure or cancel.
- Disconnecting an account deletes its cache directory after confirmation;
  the keychain entry is deleted immediately.

**D20. No Google branding assets.**
The sidebar uses a generic cloud/drive glyph drawn from scratch in the Adwaita
style, not Google's logo. The label "Google Drive" is used nominatively to
describe what is being accessed. Google Docs/Sheets/Slides get generic file
icons, not Google's product icons.

**D21. Configuration layout.**

```text
<app config>/accounts.json      accounts (no tokens)
<app config>/drive.json         optional settings (first-connect flag, warning state)
<app cache>/drive/              snapshots and thumbnails
OS keyring: "filebrowser" / "drive:<account-id>"   refresh token
```

`accounts.json` is written with `config_store::write_json`, like pins and
starred.

## Architecture

### Backend modules

All new code is platform-neutral and testable without Tauri, per `AGENTS.md`.
Tauri types stay in `lib.rs`.

| Module | Responsibility |
| --- | --- |
| `src-tauri/src/cloud/mod.rs` | `Location`, `CloudError`, the dispatcher used by commands, shared types |
| `src-tauri/src/cloud/location.rs` | Parse/format `gdrive://` URIs, percent encoding, validation, segments |
| `src-tauri/src/cloud/accounts.rs` | `accounts.json`, account records, keyring references, redaction |
| `src-tauri/src/cloud/oauth.rs` | PKCE, state, loopback listener, code exchange, refresh, revoke |
| `src-tauri/src/cloud/token.rs` | Keyring wrapper with the `0600` file fallback |
| `src-tauri/src/cloud/drive.rs` | Drive v3 client behind the `DriveApi` trait; retry/backoff/pagination |
| `src-tauri/src/cloud/cache.rs` | Download snapshots, thumbnail bytes, LRU and `.part` handling |
| `src-tauri/src/cloud/mapping.rs` | Drive JSON → `Entry` / `PathInfo` / properties subset |

`lib.rs` adds the commands, holds a managed `CloudState` (accounts, client,
in-flight flows), and keeps every non-trivial decision in the modules above.

### Frontend

| File | Change |
| --- | --- |
| `src/main.ts` | `isCloud`, Drive branch in `basename`/`parentOf`/`crumbsFor`, sidebar row, connect dialog, cloud-aware menus, focus refresh |
| `src/api.ts` | Drive account/status/open/download wrappers; `Entry.cloud` |
| `src/icons.ts` | A generic Drive/cloud icon; Google-Docs file kinds |
| `src/styles.css` | Sign-in dialog and sidebar state; tokens only, light and dark |
| `src/versions.ts` | Refuse cloud paths early (defensive) |
| `dev/mock-tauri/core.ts` | Fake account and Drive tree, no network |
| `tests/` | Account flow, navigation, breadcrumbs, open, thumbnails, no-version marker, errors |

### Command surface

Existing commands keep their names and learn the Drive branch:

| Command | Drive behaviour |
| --- | --- |
| `list_dir` | `files.list` with pagination; maps to `Entry` |
| `path_info` | `files.get` (is_dir, no device id) |
| `item_counts` | Returns `null` (counts are skipped for cloud folders) |
| `file_properties` | Drive metadata subset (name, size, modified, created, owner, mime, shared); no permissions |
| `thumbnail` | Drive `thumbnailLink` → PNG |
| `make_dir` | Milestone 2: `files.create` folder |
| `rename_entry` | Milestone 2: `files.update` name |
| `copy_entry` | Milestone 2: `files.copy` (copy within Drive) or upload (local → Drive) |
| `move_entry` | Milestone 2: `files.update` parents, or upload + local delete |
| `trash_entry` | Milestone 2: `trashed=true`, Drive Trash |
| `restore_entry` | Milestone 2: `trashed=false` (Undo) |
| `path_exists` | `files.get` |
| `open_file` | New: download snapshot then open, or open the web URL for Google docs |
| versions / deleted / folder size / empty trash | Typed "not available for Google Drive" error |

New commands:

| Command | Purpose |
| --- | --- |
| `drive_accounts()` | Connected accounts (id, email, name, root id/path, scopes) |
| `drive_connect_start(scopes?)` | Bind loopback, return `{ flow_id, auth_url }` |
| `drive_connect_finish(flow_id)` | Wait for the callback, exchange, store the token, return the account |
| `drive_connect_cancel(flow_id)` | Abort a pending flow |
| `drive_disconnect(account)` | Revoke best-effort, delete the keyring entry, remove the account |
| `drive_open(path)` | Download/open (called by `openFile` for cloud paths) |
| `drive_download(path)` | Explicit "Download a Copy…" later; also used by cross-boundary moves |
| `drive_upload(local_path, dir_uri)` | Milestone 2: resumable upload |

### Auth flow

```text
Frontend                     Rust (cloud/oauth.rs)             Google
   │ drive_connect_start            │                              │
   │────────────────────────────────▶│ bind 127.0.0.1:0            │
   │ { flow_id, auth_url }           │ generate state + PKCE       │
   │◀────────────────────────────────│                              │
   │ openUrl(auth_url) ─────────────────────────────────────────────▶│
   │                                 │                              │
   │  (system browser: consent)      │                              │
   │                                 │◀── GET /?code=…&state=… ─────│
   │                                 │ verify state; exchange code  │
   │                                 │─────────────────────────────▶│
   │                                 │◀── access + refresh token ───│
   │                                 │ keychain store; about.get    │
   │ drive_connect_finish            │                              │
   │────────────────────────────────▶│ returns account + root       │
   │◀────────────────────────────────│                              │
```

Failure modes: user cancels (timeout/cancel → "Sign-in was cancelled."), state
mismatch (discard, "Sign-in failed. Try again."), `access_denied` (same),
network error (retry), no refresh token (re-consent: Google only returns one on
first consent or with `prompt=consent`).

### Data flows

**Browse.** `navigate("gdrive://…")` → `list_dir` → dispatcher → `DriveApi::list`
→ map to `Entry` → sort/filter/render as for a local folder. `item_counts` is
not called for cloud rows.

**Open.** `openEntry` → `openFile(path)` → if cloud, `drive_open` → cache
download (or web URL) → `openPath(local)`.

**Refresh.** Window focus already re-lists real folders. For cloud, debounce to
at most one refresh per 3 s and only when the window was blurred for more than
2 s; a focus while the user never left (common on Linux) does not hit the API.

**Upload (milestone 2).** Local file drop/paste → resumable upload under a
temporary name in the target folder → verify size → rename to the final name.
On any failure, delete the temporary file (Drive Trash) and report.

**Cross-boundary move.** Download/upload first, verify, then remove the source
(last step). No partial move is reported as success.

## UI

### Sidebar

A new **Google Drive** row, above mounted volumes:

- Signed out: label "Google Drive", cloud icon; clicking opens the connect
  dialog.
- Connecting: the row shows "Connecting to Google Drive…" and a Cancel action.
- Signed in: label "Google Drive", tooltip with the account email; clicking
  navigates to the root. The context menu has **Disconnect…** (with a
  confirmation that explains the cache removal and that Drive files are not
  deleted).
- Errors (expired token): a small warning emblem on the row; opening it offers
  **Sign In Again**.

### Connect dialog

libadwaita-style dialog via `openDialog` in `src/ui.ts`:

- Title: "Connect to Google Drive".
- Body: what access is requested (read-only or read/write), where the token is
  stored, and a note that Google may show an "unverified app" screen.
- Buttons: **Cancel** and **Open Browser**.
- After Open Browser: a waiting state with a spinner, "Waiting for Google…",
  and **Cancel**. Errors replace the spinner with the classified message and a
  **Try Again** button.

### Path bar and title

```text
[Google Drive] / [My Drive] / [Documents] / [report.docx]
```

The first crumb is the account row (icon + "Google Drive"), then the root
("My Drive"), then segments. The window title is the current folder name, as
locally. No version badges anywhere.

### Menus

For a cloud selection: Open, Open in Browser (Google docs), Copy, Cut, Rename,
Move to Trash, Star, Properties. Hidden or disabled: Open Item Location,
Versions…, Deleted Files…, anything that needs a local path.

For the background inside Drive: New Folder, Paste, Select All, Reload, and (if
signed in) Disconnect is only in the sidebar menu.

### Properties

Name, type (MIME type in human words), size (files), item count ("—"),
modified, created, owner, shared state, Google Drive URL. No permissions/owner
bits, no local path, no version rows. The dialog labels the location
"Google Drive".

## Error handling

- Every cloud command returns `Result<_, String>` like the rest of the app, but
  the string comes from `CloudError::message()` so wording is consistent.
- `401` after a refresh: account marked expired; UI asks to sign in again.
- `403` with `insufficientPermissions`: same.
- `404`: "“name” no longer exists in Google Drive." (It may have been moved or
  deleted elsewhere.)
- `429`/`5xx`: retried; if exhausted, "Google Drive is temporarily
  unavailable."
- `quotaExceeded`: "Google Drive storage is full."
- Timeouts: 30 s connect, 60 s request overall, 10 min per large download with
  progress and cancel.
- Cancelling a download removes `.part` and does not add a recent entry.

## Performance

- Listing: one request per 1,000 children; fields limited; no per-entry calls.
- Thumbnails: only for rendered rows, through the existing 4-slot queue; the
  backend caches bytes so scrolling back is free.
- Downloads: streamed to disk, not buffered in memory; progress events at most
  every 100 ms.
- Concurrency capped at 4 Drive requests to stay clear of per-user rate limits.
- The frontend's virtualization and layout maths are untouched.
- `item_counts` is skipped for cloud folders; the floating bar shows the loaded
  count instead.

## Security and privacy

- PKCE S256, `state` nonce, loopback bound to `127.0.0.1`, and a single-use
  redirect handler. No custom URL scheme (which other apps could register).
- Refresh token in the keychain; `0600` fallback file; redacted `Debug`.
- Tokens never appear in errors, logs, `recent.json`, `accounts.json`, or the
  webview's localStorage.
- Only the scopes needed are requested; `drive.readonly` in milestone 1.
- Disconnect revokes the token at Google when possible, then removes it
  locally even if revocation fails.
- Downloaded files stay in the app cache, not in the user's home folder, and
  are removed on disconnect (after confirmation).
- No Google logos or brand assets; see D20.

## Testing

**Rust (no network):**
- Location grammar: round-trip, unicode, spaces, `%`, `~`, duplicate names,
  invalid ids, traversal attempts, root.
- OAuth: PKCE vector (RFC 7636), state generation, callback parsing, timeout,
  cancel, denied consent, missing refresh token, refresh-and-retry once.
- Accounts: JSON persistence, redaction, keyring fake, fallback-file
  permissions.
- Drive client: fixture transport; pagination; field masks; 401/403/404/429/5xx
  mapping; backoff with an injected clock; Google Docs export refusal.
- Cache: key format, `.part` cleanup, LRU eviction, no reuse across
  `modifiedTime` changes.
- Dispatcher: local paths still route to the local functions; every
  local-only command refuses Drive paths with the typed message.

**Frontend (Vitest + jsdom + mock):**
- Signed-out sidebar opens the dialog; start/finish/cancel transitions.
- Signed-in sidebar navigates to the fake root; breadcrumbs; back/forward/up.
- List/grid rendering, sorting, search filtering, thumbnails.
- Open triggers download then open; Google Docs opens the web URL.
- No version marker or New Folder checkbox inside Drive.
- Expired-token and network-error messages; disconnect removes the row.
- Focus refresh calls the lister at most once per debounce window.

**Mock:** `dev/mock-tauri/core.ts` gets a fake account (`me@example.com`), a
small tree (`My Drive/Documents`, a shared folder, a Docs file, an image) and
every new command, so `npm run dev:mock` works offline.

**Opt-in live tests:** `#[ignore]` tests behind `FILEBROWSER_DRIVE_LIVE=1`,
using a test account; never part of `cargo test` or CI.

**Gates:** `npx tsc --noEmit`, `npm run build`, `npm test`, `cargo test`,
`npm run crap` for every task, as required by `AGENTS.md`.

## Platform notes

- No `cfg!(target_os)` in `cloud/`. The only platform-dependent piece is the
  keyring backend, hidden behind the `keyring` crate; the fallback is the same
  file-based store on both systems.
- The system browser is opened through the existing opener plugin, which is
  already cross-platform.
- Linux desktops without a Secret Service (minimal WMs) take the fallback path;
  the warning must be visible, not a silent downgrade.
- macOS sandboxing/notarization: the app is unsigned locally; the OAuth
  loopback listener needs no entitlement. If the app is ever sandboxed, network
  client and server entitlements are required — noted for later.
- Drive API quotas are per project, not per OS; the same client id works on
  both platforms.

## Delivery

| Milestone | Scope | User-visible result |
| --- | --- | --- |
| **M0 — spike** (behind a dev flag) | OAuth end-to-end, list root, download one file | Proves the stack; no UI promise |
| **M1 — read-only** | Connect/disconnect, sidebar, browse, crumbs, open, thumbnails, properties, refresh | Drive is usable as a viewer |
| **M2 — writes** | New Folder, Rename, Copy, Move, Upload, Drive Trash, Undo, internal drag and drop | Everyday file management |
| **M3 — cross-boundary drag and drop** | Local ↔ Drive moves and copies with progress | Same gesture as local |
| **M4 — later** | Shared drives, Shared with me, server-side search, multi-account, Drive Trash view, offline pinning | Parity with GNOME Files' cloud support |

Each milestone ends with the full check set and a README update. M1 must not
ship a write command, so the read-only promise is enforceable.

## Acceptance criteria

**M1:**
1. On a clean profile on macOS and on Linux (no Drive client installed), the
   user can connect an account through the system browser and land in My Drive.
2. Folders list with correct names, types and modification dates, including
   names with spaces, unicode and duplicates; pagination is followed.
3. Breadcrumbs, back, forward, up, reload and focus refresh work.
4. Double-clicking a file downloads it, opens it in the default app, and shows
   the "local copy" notice; Google Docs open in the browser.
5. Image thumbnails appear for rendered rows and are cached; scrolling back
   does not re-download.
6. No version badge, no Keep Versions checkbox, no version dialog on any Drive
   path; local folders behave exactly as before.
7. Disconnecting removes the account row and the tokens; the keychain entry is
   gone; Drive files are untouched.
8. Every existing frontend and Rust test still passes, plus the new tests; the
   mock preview works with no network.

**M2:** New Folder, Rename, Copy, Move, Upload and Move to Trash work on Drive;
Undo restores from Drive Trash; no command permanently deletes Drive data;
cross-boundary moves never remove the source before the copy is complete; name
clashes always prompt.

## Open questions

Answer these before M1 implementation starts; the defaults above are
recommendations, not blockers.

1. **OAuth client.** Decided: one Desktop client ID and its non-confidential
   secret are built in from the git-ignored `.env`. PKCE still protects the
   exchange.
2. **Scope for M1.** Decided: `drive.readonly`. Writes request `drive` when M2
   starts.
3. **Multiple accounts.** Confirm one account is enough for v1.
4. **Shared drives / Shared with me.** Needed in M1, or is My Drive enough?
5. **Downloaded copies.** Is the "changes are not uploaded" toast acceptable,
   or should the cache copy be marked read-only?
6. **Recent and Starred.** Should Drive files appear there (they are keyed by
   URI, so it works), and should stars map to Drive's starred flag or stay
   app-local?
7. **Google-native docs.** Opening the web editor is proposed; is downloading
   an exported Office/PDF copy needed in M1?
8. **Drive Trash.** Is "Move to Trash goes to Drive Trash, not the app's Trash
   view" acceptable for M2, with a Drive Trash browser later?
