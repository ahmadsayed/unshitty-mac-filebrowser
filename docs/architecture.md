# Architecture

How the app is put together after the deep-module refactors (#136–#183):
which module owns which rule, how a call travels from the window to the
filesystem, and where a change lands. The module list itself, with one line
per file, is the Layout table in [AGENTS.md](../AGENTS.md).

![Block diagram of the app: the frontend's main.ts asks four groups of rule and state modules and calls typed commands in api.ts; Tauri IPC carries them to lib.rs, where every path goes through the place.rs gate before reaching placement.rs, trash_ops::Trash, this computer's places-local crate, Drive, shares, SSH or Arbesk; placement uses the Trash (in places-local) for Replace and places-local's rename.rs and platform.rs for renames that never replace.](architecture.svg)

Teal boxes are the modules the refactors added or deepened, with their PR
numbers. Dashed boxes are outside the app (the operating system) or stand in
for it in tests (`dev/mock-tauri`).

## How a call travels

**In the window.** `main.ts` handles the DOM, events, views, menus and keys.
It owns no rules: it asks small modules that have no DOM.

- **Where am I, what may run.** `paths.ts` breaks a path into its kind
  (`placeOf`: local, Drive, share, SSH, Arbesk), name, parent and crumbs.
  `locations.ts` turns a path into a `LocationKind` (Recent, Starred,
  Network, the Trash, a folder in it, a share, an SSH server, Arbesk, Drive,
  a folder) and says what that kind of place allows. `item-actions.ts` holds
  one rule per action that several surfaces reach (menus, keys, the preview,
  the row star), asked inside the action itself. `preview-actions.ts` builds
  the Space preview's buttons from those rules, and `preview-versions.ts`
  its version control for a Keep Versions file or an Arbesk asset. `models.ts` says which files
  the preview shows as 3D models; `model-viewer.ts` (three.js, loaded only
  then) reads and draws them.
- **View state and stored paths.** `selection.ts` (selection, cursor,
  anchor), `search.ts` (`SearchSession`: the walk, its pause, its limit,
  late answers), `folder-summaries.ts` (item counts and version badges),
  `listing-failure.ts` (what a folder that can't be listed turns into, and
  whether a Drive failure needs a credential: `drive-auth.ts`) and
  `path-refs.ts` (stars, bookmarks, the clipboard and tab history follow a
  moved or renamed item).
- **Pictures.** `item-icons.ts` decides which picture an item shows and when
  it is asked again; `itemIcon` in `icons.ts` composes the picture with its
  emblems. `native-icons.ts` and `thumbnails.ts` are the caches behind them.
- **Widgets and dialogs.** `ui.ts`: menus, popovers, toasts, and dialogs
  whose busy mode runs one action at a time while the backend works.
  `online-accounts.ts` is the Online Accounts dialog, built from
  `providers.ts`: one row per provider (Google Drive and Arbesk), the accounts in
  one shape whatever their provider, and which account a path belongs to.

**Across the boundary.** `api.ts` has one typed wrapper per Tauri command.
In the tests, `dev/mock-tauri` answers the same commands from an in-memory
filesystem.

**In the backend.** `lib.rs` registers thin commands. Every path a command
receives goes through `place.rs` first: its kind, whether it is a local path
(remote, malformed and relative paths never reach `std::fs`), and whether
the place may be written. Then:

- copies, moves and Drive downloads go through `placement.rs`: the refusals
  and their wording, staging under a hidden name, and Replace (a local copy
  or move is `local_transfer.rs`, over `places-local`'s file primitives); a copy or
  move to, from or inside Google Drive is `engine.rs`'s: `lib.rs` takes
  the Drive provider from the registry, and the engine builds the transfer
  from that provider's transfer session (its `places::Transfer`
  primitives: what is at the target, write, read, a copy on the server,
  rename, move, Trash and take back) and its declared `Capabilities`, runs
  Drive's Replace the same way, and places what lands here through
  `placement.rs`;
- Move to Trash, Restore, Delete Permanently and Empty Trash go through
  `trash_ops::Trash` (in the `places-local` crate): the trash log, where an item came from (sidecar, log,
  Finder's Put Back), and both Trash layouts;
- listing, whether an item exists, path info, thumbnails and opening ask
  the place's provider in the `places::Registry`, picked by the path's
  scheme (`providers.rs`: this computer's provider from the `places-local`
  crate, handed the app's thumbnails, Google Drive's, Arbesk's and SSH's own crates' providers, and
  received shares' provider in `share/provider.rs`, registered as they
  are), one lookup and one call per command, on a blocking thread (a
  share's provider waits on the async runtime from there);
- listing, counts, opening and reading go to the `crates/places-local`
  workspace crate (this computer: `fs_ops.rs`, the renames that never
  replace, the Trash and the platform's file pieces; no Tauri, never
  depending on the app, which hands it the version badge's ownership check
  and its thumbnailer), the
  `crates/places-gdrive` workspace crate (Google Drive: accounts and
  sign-in without the window, the REST client, listing, opening,
  thumbnails, its own writes and its transfer primitives; no Tauri),
  `share/` (iroh shares), the `crates/places-sftp` workspace crate (SSH,
  read only: connections, known hosts, listing and downloads; no Tauri,
  the Connect to Server commands stay in `lib.rs`) and the
  `crates/arbesk` workspace crate (Arbesk: its read path rebuilt from public
  sources, assets built into the app cache to open, preview and copy out,
  each asset's saved thumbnail kept on disk, manifests and thumbnails in a
  bounded content-addressed cache, the deployment config kept briefly for
  reads and fetched fresh by writes while `tokenURI` stays live; named
  collections created (a name minted
  through the wallet relay), shared by email by adding a Viewer or Editor to
  the editor Merkle tree on IPFS and the chain, and local model files copied
  in with Arbesk's own storage protocol, uploads and the collection update
  through Arbesk's relay, under a session kept in the keychain). Its
  `Client` is a `places::Provider`: the registry holds it, and the
  `arbesk_*` commands and copies in and out reach it through it (its
  `Transfer`'s `write` and `export`, and its `Versions`, `Preview`,
  `Sharing` and `Accounts` extensions, whose own records reach the frontend
  as the crate serialises them, `places::Json`). The route table stays in
  `place.rs`. Beyond that, `lib.rs` sees only the crate's facade (`Client`,
  `SCHEME`, `Location` and the path helpers `place.rs` parses with); the
  modules behind it are private, so the feature can change or move without
  leaking into the app.

Replace sends the old item to the Trash through `trash_ops::Trash` and, if
the last rename fails, takes it back out and forgets its log entry. Every
final rename goes through `places-local`'s `rename_new` and its
`platform.rs`'s `rename_exclusive` (`renameat2` on Linux, `renamex_np` on
macOS), so it never replaces an item that took the name meanwhile. That
`platform.rs` holds this computer's other file differences between macOS
and Linux too (XDG paths and volumes); the app's own `platform.rs` keeps
opening files, windows, dragging out and installs.

## Where a change lands

| Change | Lands in |
| --- | --- |
| Allow or refuse an action in a kind of place | one cell in `item-actions.ts` |
| A new kind of location (SMB, WebDAV) | a row in `paths.ts`, a kind in `locations.ts`, a `Kind` in `place.rs`, a provider in `providers.rs` and its registry line |
| A new online account provider (OneDrive, Dropbox) | a row in `providers.ts` and in `PROVIDER_ACTIONS` (`main.ts`), plus the location rows above |
| Arbesk's listings, builds, versions or uploads | the private modules of `src-tauri/crates/arbesk`, answered through its provider (`provider.rs`: `places::Provider`, `Transfer` and the extensions); a new kind of answer is a method on a `places` extension, not a new type on the facade |
| How copy, move or Replace behaves | `placement.rs` and `local_transfer.rs`, and `engine.rs` for the routes to, from and inside Google Drive |
| Listing, search, counts, renames or path info on this computer | `src-tauri/crates/places-local` (`fs_ops.rs`, `rename.rs`), its provider in `provider.rs` |
| What Google Drive is asked, its sign-in, its paths | `src-tauri/crates/places-gdrive`; a step of a Drive copy or move is one of its `places::Transfer` primitives, and their order is `engine.rs`'s |
| The trash log, Put Back, Restore, Empty Trash | `trash_ops::Trash`, in `src-tauri/crates/places-local` (a test on each layout) |
| Something new that remembers paths | one holder in `path-refs.ts` |
| What a folder that can't be listed shows | a row in `listing-failure.ts` |
| Folder counts and version badges | `folder-summaries.ts` |
| Search timing, limits and sections | `search.ts` |
| Which picture or emblem an item shows | `item-icons.ts`, and `itemIcon` in `icons.ts` |
| A dialog that waits on the backend | `openDialog`'s `busy` option in `ui.ts` |

When a module here changes what it owns, update this page and its diagram
(`docs/architecture.svg`) in the same PR.
