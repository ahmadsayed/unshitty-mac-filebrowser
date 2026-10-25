# Folder Sharing over iroh — Design

**Date:** 2026-10-25
**Status:** Design approved in conversation; not started.
**Related:** `2026-10-23-file-browser-design.md` (the app's overall design),
`2026-10-24-keep-versions-design.md` (version history, BLAKE3 objects),
`2026-10-24-google-drive-design.md` (the `gdrive://` location pattern this
reuses), `AGENTS.md` (rules every change must keep)

## Intent and the why

The user wants to share a specific folder with someone else so that person
can see the folder and open any file in it at any time — directly between the
two computers over [iroh](https://www.iroh.computer), with no cloud in the
middle.

The sharing choice should be as simple as OneDrive's: **anyone with the link**
or **a specific person**. Shared folders show up in the sidebar's **Network**
place (today a placeholder that says "Network locations aren't supported
yet."), each marked with who shared it.

The storage model follows the user's own project
[Arbesk](https://github.com/ahmadsayed/arbesk) and the Keep Versions store:
content-addressed, immutable manifests that point at content by hash. Keep
Versions already names content by plain BLAKE3, and iroh-blobs identifies
content by BLAKE3 too, so a file's hash is the same in `.versions/` and on
the wire.

## Goals

- Share a local folder read-only, with any number of "anyone" links and
  named-person links, each removable on its own.
- The recipient browses the folder live in this app, opens files on demand
  and copies files or folders out to their own disk.
- Changes on the sharer's side appear in the recipient's open view within a
  few seconds without re-sharing.
- Identical behaviour on macOS and Linux; no OS sharing services (SMB, AFP,
  GVFS), no servers of our own.
- Never lose data on either side (AGENTS.md): the recipient can't change the
  sharer's files, and nothing the recipient copies ever overwrites a file.

## Non-goals (v1)

- Write access of any kind (adding, renaming, deleting in someone's share).
- Offline access: when the sharer is unreachable the recipient sees "offline"
  and nothing else — no cached listing, no "Keep Available Offline".
- Sharing single files (only folders), sharing Drive or `share://` locations.
- A background service: sharing works only while the app is running.
- Clicking a link to open the app (a registered URL handler on both OSes).
  v1 links are pasted into Network.
- Browsing a share's history ("as it was at time T"), sharing past versions
  from `.versions/`. The design keeps the door open (D3); nothing more.
- Running our own relay servers.

## Decisions

### Made by the user

- **Read only.** Recipients browse, open and copy out. Nothing else.
- **OneDrive-style choice:** "Anyone with the link" or "Specific person".
- **A specific-person link locks to the first app that opens it.** Forwarding
  it is useless; a person with two computers needs two links.
- **Shares appear under Network,** each showing who shared it.
- **Offline means nothing to browse:** an "Ahmed Is Offline" page.
- **Build on iroh-blobs with Arbesk-style manifests** (approach "revised B"),
  not a custom file protocol and not iroh-docs.
- Confirmed defaults: open downloads to a temporary file then opens it; copy
  out through the normal transfer flow; live view; subfolders included;
  `.versions/` never shared; symlinks leaving the folder hidden; hidden files
  follow the viewer's "Show Hidden Files"; paste-only links in v1; share
  emblem and a "Shared" sidebar row on the sharer's side; display name asked
  once; sharing only while the app runs.

### Proposed by the designer

**D1. One iroh node per app, started on demand.** One secret key per
installation (`share-key`, 0600, in the app config dir) is the app's identity.
The endpoint binds only when there is at least one share or received share;
with neither, nothing touches the network. Relays and discovery use iroh's
default preset (n0's public relays), which only ever carry encrypted traffic.

**D2. Folder snapshots are trees of structure manifests.** Each folder
becomes one manifest blob listing its entries: name, kind (file / folder),
size and mtime (nanoseconds since the epoch). A folder entry also carries that subfolder's manifest hash; a
file entry carries **no** content hash. The share's **root hash** therefore
names the whole tree's structure at one moment, and any add, remove, rename or
modification (size or mtime) gives a new root. Encoding is canonical (entries
sorted by name bytes, fixed field order, postcard), so the same structure
always yields the same hash, whatever has been hashed so far.

**D3. Structure is walked eagerly; content is hashed lazily.** (User's
choice, 2026-10-25, resolving eager tree hashes against lazy browsing.) When
sharing starts, the sharer walks the folder — a directory walk like
Properties' folder size, no file reads — and builds every manifest; after a
change only the changed folder and its ancestors are rebuilt. A file's
content is hashed only when a visitor first asks for it (`Content`, D5).
Content hashes are cached on the sharer by (path, size, mtime); inside a
versioned folder they come from the Keep Versions index, which hashes the same
way. A file is hashed only when its size and mtime are equal before and after
reading; otherwise `Content` answers "changed". Roots are not stored as a
chain in v1, but nothing prevents it later (history browsing, sharing a past
version). A very large tree (around a million entries) takes noticeable time
and memory to walk when sharing starts; the Share dialog shows "Preparing…"
until the first root exists.

**D4. Content is served by reference.** The sharer serves file bytes from the
file itself, not from a copy inside a blob store. Before serving, the sharer
checks size and mtime against the manifest; a mismatch fails the request and
re-hashes the file. iroh's verified streaming catches anything that slips
through. *Open question for the spike (Delivery, task 1): does iroh-blobs'
`FsStore` serve external files in place? If not, a thin custom store does.*

**D5. A thin control protocol does what blobs can't.** ALPN
`filebrowser/share/1`, one postcard request per bidirectional stream,
messages capped at 64 KiB:

| Request | Reply |
| --- | --- |
| `Claim{share, token}` | `Ok{folder_name, sharer_name}` or `Denied(reason)` |
| `Root{share}` | `Ok{root, folder_name, sharer_name}` or `Denied(reason)` |
| `Content{share, root, names}` | `Ok{hash, size}`, `Changed` or `Denied(reason)` |
| `Watch{share}` | a stream of new root hashes until the stream closes |

`Content` names a root and the entry names leading to one file; the sharer
resolves the names by walking that root's manifests (the current or previous
root only) to a path of its own (never one from the request), checks size and mtime against the entry, hashes the file (or finds
the hash cached), publishes the hash for that share and replies. Manifests and
file bytes themselves go through the iroh-blobs protocol.

**D6. Claim rules.**
- Anyone link, token matches: the visitor's key is recorded under that link.
- Person link, never used: bind it to the visitor's key. The binding is
  written to disk **before** the reply, so a crash can't bind it twice.
- Person link, same key again: `Ok` (idempotent).
- Person link, a different key: `Denied("This link was already used on
  another computer.")`.
- Unknown or removed token: `Denied("This link is no longer valid.")`.

**D7. Access is checked on every request; revoke is immediate.** The blobs
provider's events gate both connections (the key must have access to at
least one share) and requests (the hash must be published by a share the key
can access — one of its manifests, or a content hash it returned from
`Content`). Each share
has its own published-hash set, so access to share A never opens share B.
Remove on a person deletes their entry, saves, and closes their connections
for that share. Remove on the anyone link invalidates its token and every key
admitted through it. Stop Sharing deletes the whole share.

**D8. Visitors send hashes and entry names, never paths.** A request names a
published root and, for `Content`, the entry names leading to one file from
it; the sharer resolves them through its own manifests. There is no path to traverse on the
wire. The safety rules live in the sharer's manifest builder:
- Symlinks are followed only when they resolve inside the shared folder.
- `.versions/` is always excluded.
- Sockets, FIFOs and device files are excluded.
- Unreadable folders are left out quietly; an unreadable file is listed, and
  `Content` answers that it can't be read.
- Hidden files are included; the viewer's setting decides what they see.

On the visitor's side every manifest name is validated before anything is
written (no `/`, empty, `.`, `..`, NUL), with the same rule as Drive
downloads (`safeLocalName`).

**D9. Link format.** `share://` + base32 (lowercase, no padding) of a
postcard struct: format version, sharer endpoint address (ID, relay URL,
direct addresses), share ID (16 random bytes), token (32 random bytes), link
kind, sharer name. The name in the link is a placeholder until the first
`Root` reply; the sharer's app is the authority. Tokens are stored so Copy
Link works again later. A link shares its scheme with D10's locations and is
told apart by shape: a ticket is 64 or more base32 characters with nothing
after it, a received ID is 16 hex characters. Links made before the switch
start with `fbshare1:` and still decode.

**D10. Recipient paths are `share://<received-id>/<name>/<name>…`.** Names are
percent-encoded like `gdrive://` names. A path resolves by walking manifests
from the current root. The frontend treats `share://` as another read-only
cloud location, through the same dispatch `lib.rs` uses for `gdrive://`.

**D11. Downloads stage, then rename.** Open downloads to the app cache and
opens the file. Copy-out writes each file to a staging name next to the
destination and renames it into place — the existing never-overwrite flow,
with Replace / Skip / Keep Both. A folder copy follows the root it started
from; files whose content changed meanwhile are skipped and listed at the end.
The visitor's blob store lives in the app cache, releases blobs once they are
exported, and is emptied on startup.

**D12. Display names are labels, keys are identities.** The sharer's name is
asked once ("People you share with will see this name.", pre-filled with the
account name) and editable later. Names aren't verified; two shares from the
same key are the same sharer.

**D13. Limits.** Manifests are capped at 64 MiB (about 500k entries); at most
32 concurrent visitor connections, beyond which visitors see "busy"; connects
time out after 10 s and count as offline.

**D14. One sharing instance.** The key file is locked while sharing; a second
app instance doesn't share and says "Sharing is active in another Files
window."

**D15. Debug-only second identity.** A debug-build-only
`FILEBROWSER_CONFIG_DIR` environment variable points the app at another config
dir, so two instances on one machine can share with each other.

## Architecture

### Backend modules (`src-tauri/src/share/`)

Platform-neutral plain functions, testable without Tauri; only `lib.rs` uses
`AppHandle`.

| Module | Responsibility |
| --- | --- |
| `identity.rs` | Load or create `share-key` (0600), lock it, display name |
| `manifest.rs` | Manifest format and canonical encoding, the tree builder with the D8 exclusions and incremental rebuild, content-hash cache |
| `registry.rs` | `shares.json` (shares, links, bindings) and `received.json` (received shares); claim rules (D6) |
| `host.rs` | Endpoint + router, the blobs provider and its gate (D7), the control protocol (D5), the folder watcher that rebuilds manifests and pushes roots |
| `guest.rs` | Connect, claim, root, watch with reconnect backoff, manifest walk (D10), downloads (D11) |
| `link.rs` | Encode / decode links (D9) |

New crates: `iroh` (1.x), `iroh-blobs` (`default-features = false`,
`fs-store`), `postcard`, `data-encoding` (base32). The Tauri tokio runtime runs
the node; blocking file work still goes through `blocking(…)`.

### Frontend

- `src/api.ts`: typed wrappers for the commands below.
- `src/main.ts`: the Share dialog, Network view, `share://` handling in the
  location dispatch, read-only menus, emblem, Shared sidebar row.
- `dev/mock-tauri/core.ts`: an in-memory fake of sharing and of received
  shares (a fake sharer whose tree tests can change and take offline).

### Command surface

Sharer side:

| Command | Returns |
| --- | --- |
| `share_identity()` | `{ name: string \| null }` |
| `set_share_name(name)` | — |
| `share_folder(path)` | `ShareInfo` (creates the share, no links yet) |
| `share_info(path)` | `ShareInfo \| null` |
| `list_shares()` | `ShareSummary[]` (for the Shared row and emblems) |
| `create_link(share, kind, person?)` | `{ link, entry: AccessEntry }` |
| `share_link(share, entry)` | `string` (Copy Link again) |
| `remove_access(share, entry)` | — |
| `stop_sharing(share)` | — |

`ShareInfo` = `{ id, path, status: "ok" \| "missing", access: AccessEntry[] }`;
`AccessEntry` = `{ id, kind: "anyone" \| "person", name, state: "unused" \|
"online" \| "offline", visitors }`.

Recipient side:

| Command | Returns |
| --- | --- |
| `add_received(link)` | `ReceivedShare` |
| `list_received()` | `ReceivedShare[]` |
| `remove_received(id)` | — |

`ReceivedShare` = `{ id, folder, sharer, status: "online" \| "offline" \|
"no-access" \| "missing" }`. Listing, open, Copy and drag out of `share://`
paths go through the existing `list_dir`, open and transfer dispatch.

Events: `share-root-changed {received, root}` (refresh the open view),
`share-status-changed {received, status}`, `share-access-changed {share}`
(the sharer's dialog and emblems).

### Data flows

Recipient:
1. Paste a link in Network → `Claim` → the share is saved to `received.json`.
2. Open Network → `Root` for each received share → online or offline.
3. Browse `share://…` → fetch the root manifest, then each manifest down the
   path (cached in memory by hash) → render as a normal listing.
4. Open a file → `Content` gives its hash → download the blob, verified →
   export to the app cache → open.
5. Copy out → staged download → rename into place (D11).
6. `Watch` pushes a root → `share-root-changed` → the view refreshes; only
   changed manifests are fetched again.

Sharer:
- Sharing starts → walk the folder and build every manifest (D3) → the first
  root exists.
- `Content` arrives → resolve, check, hash or reuse the cached hash → publish.
- A blobs request arrives → the gate checks the key and the hash (D7) → the
  blob is served from the file (D4).
- The watcher sees a change → rebuilds that folder's manifest and its
  parents' → pushes the new root to watchers.

## UI

GNOME wording; `openDialog`, `openMenu`, `showToast` only.

### Sharer

- **Share…** in a local folder's context menu and the path-bar menu. Not for
  files, Trash, Drive or `share://`.
- **First share:** "Your Name" dialog (D12).
- **Share dialog** (`Share “Photos”`):
  ```
  Who can open this folder
  ( ) Anyone with the link
  (•) Specific person  [ Sara            ]
  [ Copy Link ]                     Read only

  People with access
   Sara      Hasn't opened the link yet [Remove]
   Anyone with the link — 2 people      [Remove]
                                  [ Stop Sharing ]
  ```
  Copy Link creates the link (or copies the existing anyone link) and shows
  "Link copied". Person rows show "Hasn't opened the link yet", "Online" or
  "Offline". Stop Sharing confirms — "Stop sharing “Photos”? People you shared
  it with will no longer be able to open it." — with Cancel as the default.
- **Share emblem** on shared folders in grid and list views.
- **Shared** sidebar row next to Starred, listing your shared folders; a
  missing one says "Folder not found" and offers Stop Sharing.

### Recipient

- **Network** lists received shares and connected SSH servers, sorted like
  any folder (there are no sections): a share has
  a folder icon with a network emblem and "Shared by Ahmed" under its grid
  label; a server has a server icon and "SSH server". List columns Name ·
  Details · Status ("Online"/"Offline"/… or "Connected"); offline shares
  dimmed.
- **Bottom bar:** `[ Enter server address or share link ] [Connect]`. A share
  link is claimed, with errors inline; anything else opens Connect to Server
  for that address and connects at once.
- **Empty state:** "No Network Locations" — "Enter a server address or paste a
  share link below."
- **Context menu:** Remove from Network (forgets the link only) on a share;
  Disconnect on a server.
- **Offline page:** "Ahmed Is Offline" — "Shared folders open while the
  computer sharing them is on and Files is running."
- **Inside a share:** path bar starts with the share name and network icon;
  subtitle "Shared by Ahmed · Read only". Menus: Open, Open With…, Copy, Copy
  Location, Properties, Show Hidden Files, sort, view. No Cut, Rename, Move to
  Trash, New Folder, Paste or Star. Drops onto a share are refused.
- **Open:** progress toast "Downloading “video.mp4”… 45%" with Cancel.
- **Copy out:** drag or Copy + Paste to a local folder; copy is the default;
  the normal transfer progress and clash dialog.
- **Live refresh** keeps selection and scroll position where possible.
- **Access lost:** "No Access" — "Ahmed stopped sharing this folder with you."
  The Network item shows "No access" until removed.

## Error handling

| Situation | What the user sees |
| --- | --- |
| Shared folder moved or renamed in this app | Nothing; the share follows it (like stars) |
| Shared folder moved outside the app, trashed, or its drive unplugged | Sharer: "Folder not found" in Shared. Recipient: "Not Available — the shared folder was moved or deleted." Recovers if it reappears at the same path |
| File changes while hashing | Retried shortly; listed once stable |
| File changes while downloading | "The file changed while downloading. Try again." View refreshes from the new root |
| Sharer unreachable within 10 s | Offline page |
| More than 32 visitors | "Ahmed's computer is busy. Try again in a moment." |
| Sleep / wake / network change | iroh re-binds; `Watch` reconnects with backoff; offline meanwhile |
| Second app instance | "Sharing is active in another Files window." |
| Same link pasted twice | "This folder is already in Network." |
| Own link pasted | "This is your own shared folder." Opens the local folder |
| Protocol version mismatch | "Ahmed's Files app uses a different sharing version. Both of you need the same version." |
| Download cancelled / disk full | Staging file and partial blob removed |
| Sharer goes offline mid-copy | Completed files stay; "Ahmed went offline. 12 of 40 items were copied." Nothing overwritten |
| Folder changes mid-copy | Copy follows the starting root; changed files skipped and listed |
| Sharer's key lost | All their links stop working; recipients see offline until they remove the share. Documented: back up `share-key` |
| Recipient's key lost | Their person links reject them; they need new ones. Anyone links still work |

## Security and privacy

- Connections are QUIC with TLS keyed to each app's iroh key; relays see only
  encrypted bytes.
- On a direct connection each side sees the other's IP address. Documented.
- Tokens and bindings live in 0600 files; the key file is 0600 and locked.
- The gate (D7) and the manifest builder (D8) are the only ways in; both are
  tested on their own.
- Display names aren't verified (D12).

## Testing

**Rust unit tests (no network):**
- `manifest.rs`: same folder → same hash regardless of order and of which
  files were hashed; a changed name, size or mtime changes the hash of that
  folder and its parents only;
  exclusions (symlink out, symlink in kept, `.versions/`, FIFO, unreadable);
  hash cache hit and miss; size cap.
- The Keep Versions hash of some bytes equals their iroh-blobs hash.
- `link.rs`: round trip; garbage, wrong version and truncated text rejected.
- `registry.rs`: each D6 rule; binding saved before the reply; anyone links
  record visitors.
- `Content`: an entry name not in the named manifest, a folder entry, and a
  root that is neither current nor previous are denied; a size or mtime mismatch answers `Changed`.
- Gate: unpublished hash denied; share A's hash denied to a share-B-only
  visitor; a removed visitor denied on the next request.
- Recipient name safety: `../x`, `a/b`, empty, `.`, `..` rejected before
  writing.

**Rust integration tests:** two endpoints in one test over loopback with
relays and discovery off; every test has a hard timeout.
- Claim → Root → list → download 3 MB; bytes match.
- Copy a folder out with Keep Both; nothing overwritten.
- `Watch` pushes a new root after an add and after a rename.
- Remove access drops the connection; the next request is denied.
- File modified after hashing → "changed"; new root has the new hash.
- Sharer shuts down mid-copy → completed files intact, count reported.
- Connection cap → busy.
- One `#[ignore]` test over n0's real relays, for running by hand.

**Frontend (Vitest + jsdom):** a mock and a `tests/mock-core.test.ts` case for
every new command; UI tests for the name prompt, Share dialog (both link
kinds, Copy Link, Remove, Stop Sharing defaulting to Cancel), emblem and
Shared row, Network list and empty state, paste errors (invalid, duplicate,
own), offline page, read-only menus, refused drop, live refresh from a mock
event, and the No Access page. A look in `npm run dev:mock`.

**Manual:** two instances on one machine via `FILEBROWSER_CONFIG_DIR` (D15);
a Linux ↔ Mac checklist for direct vs relayed connections, the macOS firewall
prompt, watching on macOS, and sleep / wake.

All of tsc, `npm run build`, `npm test`, `cargo test` and the CRAP gate pass
on every commit.

## Platform notes

Everything in `share/` is platform-neutral. Only checkable on a Mac: whether
macOS asks to allow incoming connections, folder watching (FSEvents through
`notify`), and sleep / wake behaviour. The report for each milestone says so.

## Delivery

1. **Spike (throwaway):** does `FsStore` serve external files by reference
   (D4), and how much do the crates add to the release binary? If by-reference
   serving isn't available, the next task builds a thin custom store first.
2. **M1 — share and browse:** identity, manifests, registry, host, guest,
   links; Share dialog, Network view, offline page, read-only browsing.
3. **M2 — open and copy out:** downloads, staging, progress, clash handling.
4. **M3 — live and lifecycle:** watcher and `Watch`, emblem and Shared row,
   remove / stop sharing with connection drop, folder-missing handling, docs.

## Acceptance criteria

- Sharing a folder and pasting its link in a second instance shows it under
  Network as "Shared by <name>" and lets the recipient browse every subfolder.
- A person link used on one instance is refused on a third.
- Removing a person or the anyone link cuts off the affected recipients at
  their next request, without affecting others.
- Opening a file opens a verified copy; copying out never overwrites.
- Adding or renaming a file on the sharer appears in the recipient's open view
  within a few seconds.
- Quitting the sharer's app shows "Ahmed Is Offline" to the recipient.
- `.versions/`, outside-pointing symlinks and special files never appear.
- No command lets a recipient change a byte of the sharer's files.

## Open questions

- D4: by-reference serving in `FsStore`. Answered by the spike: it works.
  `store.blobs().add_path_with_opts(AddPathOptions { mode:
  ImportMode::TryReference, .. })` references the file in place; after serving
  a 3 MB file to a second endpoint, A's store directory held about 1 MB
  (redb preallocation), against 4 MB with `ImportMode::Copy`. D4 stands.
- Binary size growth: dropped by the user; not measured, sharing is not put
  behind a Cargo feature.
