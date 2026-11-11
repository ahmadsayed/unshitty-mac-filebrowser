# Architecture

How the app is put together after the deep-module refactors (#136–#183)
and the provider crates (#253–#260, design
[`2026-11-10-provider-crates-design.md`](superpowers/specs/2026-11-10-provider-crates-design.md)):
which module owns which rule, how a call travels from the window to the
filesystem, and where a change lands. The module list itself, with one line
per file, is the Layout table in [AGENTS.md](../AGENTS.md).

![Block diagram of the app. Frontend: main.ts asks four groups of rule and state modules (where am I and what may run, with capabilities.ts; view state; pictures; widgets) and calls typed commands through api.ts, or dev/mock-tauri in tests. Backend: lib.rs asks the places::Registry in providers.rs directly for reads, and passes writes, copies and moves through the place.rs gate first; the registry holds one provider per scheme: Keep Versions' decorator around places-local for this computer, places-gdrive, arbesk, places-sftp and the app's received shares. Copies and moves go to the transfer engine (engine.rs and placement.rs), which runs them on a provider's Transfer session. Every provider implements the places crate's interface; the registry's capabilities go back to the frontend's capabilities.ts.](architecture.svg)

Teal boxes are the modules the refactors added or deepened, with their PR
numbers. Dashed boxes are outside the app (the operating system and the
servers) or stand in for it in tests (`dev/mock-tauri`).

## The shape in one paragraph

Every place the app shows is a **provider**: this computer, Google Drive,
Arbesk, SSH servers and received shares. Each implements one interface,
the `places` crate's `Provider` (POSIX-like verbs on the paths the app
already uses: `/home/me/x`, `gdrive://…`, `arbesk://…`, `sftp://…`,
`share://…`), lives in its own crate (`places-local`, `places-gdrive`,
`arbesk`, `places-sftp`; shares stay in the app's `share/`), and says what
it allows and how safely as data (`Capabilities`). The app holds them in a
`places::Registry` that picks a provider by the path's scheme, so a command
is one lookup and one call. What must never differ between places stays
above them, in the app: the transfer engine (every copy and move, built
from each provider's `Transfer` primitives) and Keep Versions (a decorator
around this computer's provider). The frontend reads each place's
capabilities instead of keeping its own list.

## How a call travels

**In the window.** `main.ts` handles the DOM, events, views, menus and keys.
It owns no rules: it asks small modules that have no DOM.

- **Where am I, what may run.** `paths.ts` breaks a path into its kind
  (`placeOf`: local, Drive, share, SSH, Arbesk), name, parent and crumbs.
  `locations.ts` turns a path into a `LocationKind` (Recent, Starred,
  Network, the Trash, a folder in it, a share, an SSH server, Arbesk, Drive,
  a folder) and says what that kind of place allows. `item-actions.ts` holds
  one rule per action that several surfaces reach (menus, keys, the preview,
  the row star), asked inside the action itself. What a place allows (read
  only, creates, renames, trashes, copies in or out, knows sizes, searches)
  is the backend's providers' own answer: `capabilities.ts` holds their
  table, one row per kind (Arbesk's account, collection and asset apart,
  and an asset's other file),
  read once at startup with nothing waiting for it, and both modules read
  it rule by rule; only the frontend's own views (Recent, Starred, Network,
  the Trash) and rules no capability matches keep their own cells. `preview-actions.ts` builds
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
filesystem. The `capabilities` command answers from the registered
providers (`providers::capability_table`); `tests/capabilities.json` pins it,
and the frontend's built-in copy and the mock's answer against it.

**In the backend.** `lib.rs` registers thin commands and runs slow work on
Tauri's blocking pool (`blocking`). The read commands (listing,
existence, path info, thumbnails, opening, the preview's file) ask the
registry directly and never call `place.rs`. `place.rs` is a narrow gate,
not a router: it gates the commands that work on this computer only
(`local`, `local_write`: remote, malformed and relative paths never reach
`std::fs`), the writes (`writable`: whether the place may be written), the
copy, move and rename route table (`transfer`, `copy`: which pairs may
transfer) and the batches answered for local paths only (`local_only`).
Then:

- **The registry.** `providers.rs` builds the `places::Registry` that
  `lib.rs` manages: one provider per scheme, matched as the exact
  `<scheme>://` prefix, anything else going to this computer's provider,
  which refuses a path that isn't absolute. Listing, whether an item
  exists, path info, thumbnails, opening and the preview's file ask the
  path's provider (`provider_for`; `answer_each` sends a batch across
  places to each provider once); a command that belongs to one place (the
  `arbesk_*` commands) takes it by its scheme (`Registry::provider`). No
  crate but the app depends on Tauri, and no provider depends on another
  or on the app: what they share is in `places`, and what they need of the
  app is handed in through a trait the app answers.
- **The providers.**
  - This computer: the `places-local` crate (`fs_ops.rs`, the renames that
    never replace in `rename.rs`, both Trash layouts and the trash log in
    `trash_ops.rs`, the platform's file pieces in `platform.rs`), whose
    `Local` creates, renames, trashes and restores, and whose
    `LocalTransfer` gives the engine its primitives. The registry holds it
    inside **Keep Versions' decorator** (`versions/versioned.rs`,
    `Versioned`): Rename and Move to Trash close the histories inside the
    items they touch and reopen them where the items went, a local copy or
    move runs inside `VersionsState::transfer` (the session's `within`),
    and Replace's Trash of a Drive transfer's local end (a Drive download's
    replaced item, a moved local source of an upload) goes through
    `VersionedTrash`. Keep Versions is not a place: it needs
    the app's watcher and SQLite, so it stays in the app.
  - Google Drive: the `places-gdrive` crate (accounts and sign-in without
    the window, the REST client, listing, opening, thumbnails, and its
    `Transfer` primitives on one `Ctx` per transfer). Drive's own New
    Folder, Rename, Move to Trash and Put Back still go from `lib.rs` to
    its `write.rs`; the provider answers those methods `Unsupported`.
  - Arbesk: the `arbesk` crate, whose `Client` is the provider: its read
    path rebuilt from public sources, assets built into the app cache to
    open, preview and copy out, New Collection as `create`, a local model
    file copied into a collection (`Transfer::write`; Replace saves a new
    version, keeping the asset's other files) and an asset out
    (`Transfer::export`), and the `Versions`, `Preview`, `Representations`
    (an asset's other files: their list, and one fetched into the app
    cache), `Sharing` and `Accounts` extensions, whose own records reach
    the frontend as the crate serialises them (`places::Json`). Its asset
    rows carry `places::Entry.representations`, a summary of those files
    (count, out of date, formats), the one key besides Drive's `cloud` a
    provider may add to an entry. Behind `Client` the modules are
    private.
  - SSH servers: the `places-sftp` crate (`SftpState`: connections, known
    hosts, listing, downloads), read only; the Connect to Server commands
    stay in `lib.rs`.
  - Received shares: `share/provider.rs` in the app (iroh stays there),
    read only; its async listing and download wait on Tauri's runtime with
    `block_on`, which is why every provider is called from a blocking
    thread.
- **The transfer engine.** Copies and moves on this computer and to, from
  and inside Google Drive are `engine.rs`'s. `lib.rs` takes the provider
  from the registry and opens its transfer session (`drive_transfer`,
  `in_transfer`: `places::with_transfer`, one `places::Transfer` for the
  whole transfer), then hands `engine::dispatch` an `engine::Remote` (the
  session and its capabilities) or `engine::local` a `&dyn Transfer`; the
  engine never sees the registry. It builds the route from the session's
  primitives (what is at the target, write, read, a copy on the server,
  rename, move, Trash and take back, discard) and its declared
  `Capabilities` (`names_unique`, `put_is_whole`), never from its scheme.
  Arbesk skips the engine: a local model file copied into a collection
  (`Route::IntoArbesk`) is `lib.rs`' `write_into_place`, Arbesk's own
  `Transfer::write`, and an asset copied out is first built by its
  `Transfer::export` (`exported_source`), then copied by the engine like
  any local file. On this computer (`engine::local`)
  and for whatever lands here (a Drive download) it places through
  `placement.rs`: the refusals and their wording, staging under a hidden
  name, Replace through the Trash with take-back, and a move's source
  removed only after the whole copy. Inside Drive, Replace is the engine's
  `swap_in`. A provider gives primitives only; the sequences that keep data
  safe are written once, here.
- **What stays outside the registry** calls its module through
  `place.rs`' gate: Drive's own writes, Properties, folder sizes, item
  counts, search and measure (their `places` extensions are empty slots for
  now), and Delete Permanently and Empty Trash (`trash_ops::Trash`, inside a
  Trash folder only).
- **Capabilities to the frontend.** The `capabilities` command answers
  `providers::capability_table`: each registered provider's own
  `capabilities` for a path of each kind the frontend tells apart
  (`providers::KINDS`: a folder, Drive, a share, an SSH server, Arbesk's
  account, collection and asset, and an asset's other file), booleans only. `capabilities.ts` loads it
  at startup into a synchronous cache, answering from a built-in copy of
  the same table until it lands; `tests/capabilities.json` pins the
  backend's table, the built-in copy and the mock's. The refusal wording
  stays the frontend's own.

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
| Allow or refuse an action in a kind of place | the place's provider's `capabilities` (and `tests/capabilities.json`, `capabilities.ts`' built-in table) where a capability answers the rule; otherwise one cell in `item-actions.ts` |
| A new kind of location (SMB, WebDAV, another cloud) | a provider crate and its registry line in `providers.rs`, then the rows AGENTS.md's "Adding a provider" lists: `providers::KINDS`, `capabilities.ts` and `tests/capabilities.json`, a `Kind` in `place.rs`, a row in `paths.ts`, a kind in `locations.ts`, the `Record` tables' cells, the mock |
| A new online account provider (OneDrive, Dropbox) | a row in `providers.ts` and in `PROVIDER_ACTIONS` (`main.ts`), plus the location rows above |
| Arbesk's listings, builds, versions or uploads | the private modules of `src-tauri/crates/arbesk`, answered through its provider (`provider.rs`: `places::Provider`, `Transfer` and the extensions); a new kind of answer is a method on a `places` extension, not a new type on the facade |
| How copy, move or Replace behaves | `engine.rs` (every route: `local` on this computer, the routes to, from and inside Google Drive) and `placement.rs` (what lands on this computer); a step of a copy or move is one of the provider's `places::Transfer` primitives (`places-local`'s `transfer.rs`, `places-gdrive`'s `transfer.rs`), never a sequence in a provider |
| Which places may copy or move to which | `place.rs`' route table (`transfer`, `copy`) and its table test |
| What a place's provider can do, and its words | the provider's crate (`provider.rs`); a non-POSIX feature is a `places` extension (`ext`), not a method on every provider |
| Listing, search, counts, renames or path info on this computer | `src-tauri/crates/places-local` (`fs_ops.rs`, `rename.rs`), its provider in `provider.rs`; what Keep Versions does around a write or a transfer, `versions/versioned.rs` |
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
