# One interface for every place — design

Date: 2026-11-10 · Status: proposed, waiting for the human's review

## Goal

Every place the app shows (this computer's files, Google Drive, Arbesk, SSH
servers, received shares, and later other clouds and Windows) is a
**provider** in its own crate, behind **one Rust interface with POSIX-like
verbs**, addressed by the **paths** the app already uses (`/home/me/x`,
`gdrive://…`, `arbesk://…`, `sftp://…`, `share://…`). Each provider keeps its
own ids inside. The app asks a provider what it can do instead of knowing it.

Not every provider can do everything, and the interface says so instead of
pretending: an operation a provider doesn't have is refused with its reason,
and the operations a provider has may differ in what they guarantee (a
rename that is atomic on disk is check-then-act on Drive).

Out of scope: any change a user can see. Every step below keeps wording,
behaviour, the requests each provider sends and the tests as they are. New
providers, new copy routes (SSH to this computer, say) and Windows come
after, each as its own visible change.

## Where things stand

From an inventory of the code (local, Drive, Arbesk, SSH, shares and the glue
between them):

- **Only Arbesk is a crate.** Drive is `src/cloud/`, SSH is `sftp.rs`, shares
  are `src/share/`, local files are spread over `fs_ops.rs`, `trash_ops.rs`,
  `placement.rs`, `platform.rs`, `properties.rs` and `thumbnails.rs`.
- **Dispatch is by hand, per command.** `place.rs` picks a `Kind` from the
  scheme and `lib.rs` (3,100 lines) has a `match`/`if` per command. A new
  provider touches about **33 places** (17 in Rust, the rest in the
  frontend's tables). Several dispatchers end in `_ =>`, so a new kind
  silently falls to "local" or "none" instead of failing to compile
  (`list_remote`, `cached_target`, `exists_at`, `path_infos`, `thumbnail`).
- **Cross-provider transfers are wired pairwise:** `Route` knows only
  `Drive | Local | IntoArbesk`, and Drive re-detects local↔Drive pairs
  (`cloud/transfer.rs::dispatch_in`).
- **Shared types leak across providers.** `fs_ops::Entry` carries a
  Drive-only `cloud: Option<CloudEntry>`; `OpenTarget` lives in
  `cloud::service`; `Progress` is defined in the Arbesk crate. Drive and SSH
  also call app modules: `placement`, `fs_ops`' `Measure`/`PathInfo`/
  `FileProperties`, `progress::copy_counted`, `downloads::mark`,
  `config_store::write_json`.
- **Data-safety logic exists twice.** Local copy/move and Drive downloads go
  through `placement.rs`; Drive-to-Drive Replace has its own copy of the same
  sequence (`cloud/write.rs::finish_staged`: stage, trash old, rename in,
  take back).
- **The frontend restates what the backend knows.** `locations.ts` and
  `item-actions.ts` hold per-kind rules (read only, has sizes, can search,
  can copy out) that the backend also enforces, sometimes with different
  wording on purpose (the "Kept:" cells).
- **Blocking everywhere but shares.** Local, Drive, SSH and Arbesk are
  synchronous and run inside `blocking(…)`; received shares' list and open
  are awaited directly on Tauri's async workers (`lib.rs:273, 544`).

## Decisions

**D1. Paths are the address; ids stay inside.** The interface takes and
returns path strings, exactly as the app stores them today (stars, bookmarks,
history, the clipboard). A provider parses its own scheme and may embed ids
in the path, as Drive (`<id>~<name>`) and Arbesk (`<token>~<name>`) already
do. A pending name (a target that doesn't exist yet) keeps today's form
(`…/~<name>`). Paths the backend returns are never rewritten (AGENTS.md).

**D2. Crates.** A workspace under `src-tauri/crates/`:

| Crate | Holds | Depends on (illustrative) |
|---|---|---|
| `places` | the interface: `Provider`, the extension traits, `Entry`, `Capabilities`, `Error`, `Progress`/`Phase`/`Snapshot`/`CANCELLED`, `OpenTarget`, `Registry`, and the shared helpers providers use today from the app: `PathInfo`, `Measure`, `FileProperties`, `Refusal`'s wording, `copy_counted`/`is_cancel`, a clock, an atomic JSON save (`write_json`; the 0600 `write_json_private` stays in `config_store`), the `.downloaded` mark (one copy, which `downloads.rs` and Drive both use) | `serde`, `serde_json` |
| `places-local` | this computer: listing, stat, mkdir, the exclusive rename, the Trash (both layouts) and its log, and its provider; `cfg` per OS (Linux, macOS; Windows later). Properties and thumbnails stayed in the app (step 6) | `places`, `trash`, `libc`, `unicode-normalization` |
| `places-gdrive` | Google Drive (what was `src/cloud/`, minus the local↔Drive half of `transfer.rs` and `finish_staged`, which joined the engine: D6) | `places`, `reqwest`, `oauth2`, `keyring`, `chrono`, `image` |
| `arbesk` | stays its own crate; its `Client` implements `places::Provider` itself (`provider.rs`) | `places` |
| `places-sftp` | SSH servers (what was `sftp.rs`) | `places`, `ssh2` |
| app (`src-tauri`) | commands, Tauri state, **the transfer engine** (D6), Keep Versions, shares (iroh stays in the app for now), the frontend | all of the above |

Crate names are a proposal; the split is what matters. No crate but the app
depends on Tauri. Each new crate joins `members` and `default-members` in
`src-tauri/Cargo.toml` (so `cargo test` from `src-tauri` keeps running
everything), gets a cargo-deny run, and changes `Cargo.lock` minimally.

**D3. The interface is blocking and small.** Commands call providers only
inside `blocking(…)`; making the interface async would be a second migration
with no gain. Shares, the one async provider, keep the runtime handle
captured at setup (`tauri::async_runtime::handle()`) and call `block_on` only
on those blocking threads, never from an async worker (where it would
panic); the share arms move into `blocking(…)` when shares join (step 5).

```rust
pub trait Provider: Send + Sync {
    /// "" for this computer, else the scheme without "://".
    fn scheme(&self) -> &'static str;
    /// What this path allows, and how (D5). Cheap: no network.
    fn capabilities(&self, path: &str) -> Capabilities;

    fn list(&self, dir: &str, opts: &ListOptions, cx: &Cx) -> Result<Vec<Entry>>;
    fn stat(&self, path: &str, cx: &Cx) -> Result<Option<Entry>>;
    /// Every item at a pending `parent/~name`: more than one where names
    /// aren't unique (Drive). Not the engine's path: a transfer asks its
    /// `Transfer::landing` (D6), which also says where the source is.
    fn lookup(&self, pending: &str, cx: &Cx) -> Result<Vec<Entry>>;
    /// `path_exists` as today: false where the place can't tell.
    fn exists(&self, path: &str, cx: &Cx) -> bool;
    /// `path_info` for this provider's paths of a batch, one call per batch.
    fn path_info(&self, paths: &[String], cx: &Cx) -> Vec<Option<PathInfo>>;

    /// For open and preview only: a local file with this item's bytes (the
    /// file itself here, a download or a build in the app cache elsewhere),
    /// or a URL to open instead (a Google Doc, an Arbesk asset).
    fn fetch(&self, path: &str, purpose: Fetch, cx: &Cx) -> Result<Fetched>;

    /// New folder or empty file at `parent/name`, never over an existing
    /// name. Returns the final path (Drive learns its id only now).
    fn create(&self, parent: &str, name: &str, kind: NewKind, cx: &Cx) -> Result<String>;
    /// Rename in place, never over another item (D6); returns the new path.
    fn rename(&self, path: &str, new_name: &str, cx: &Cx) -> Result<String>;
    /// Move to Trash, as the user asks for it; each outcome says where the
    /// item went, for Undo.
    fn trash(&self, paths: &[&str], cx: &Cx) -> Vec<Trashed>;
    fn restore(&self, trashed: &str, cx: &Cx) -> Result<String>;

    /// Run `work` once on the transfer primitives the engine (D6) builds
    /// copy and move from, for one whole transfer: every step on the same
    /// session (Drive: one context, so what `landing` fetched serves the
    /// later steps). Never runs it when this provider can't be a transfer
    /// target (read only). Callers use `places::with_transfer`, which hands
    /// back `work`'s answer, or `None`.
    fn transfer(&self, cx: &Cx, work: &mut dyn FnMut(&dyn Transfer)) {}
    /// Optional, non-POSIX parts (D4).
    fn extensions(&self) -> Extensions<'_> { Extensions::default() }
}
```

`Cx` is the context of one call: the `Progress` it reports into (and whose
cancel flag `check()` reads between chunks and items) and the app cache
folder (`cache_dir`) where a provider may leave downloads and builds.

`exists` and `path_info` are methods of their own, not answers built from
`stat` and `lookup` (found in step 2): an `Entry` has no device number,
which drag and drop needs, and Drive answers the two differently today
(existence ignores trashed items and looks a pending `…/~name` up by name;
path info asks for the item, trashed or not), so building them from `stat`
would change what Drive is asked and answers. A batch across places goes
through `Registry::answer_each`: each provider gets its own paths, in order,
in one call, so this computer still asks the disk once.

`rename_entry` keeps its `(from, to)` arguments: the command checks that
`to`'s parent is `from`'s, keeping Drive's wording ("Rename keeps an item in
its folder; use Move instead.") for Drive and today's for the others (pinned
by a test), then calls `rename(path, name_of(to))`.

Every method a provider can't do returns `Error::Unsupported` with its own
wording, so today's messages ("Arbesk is read only.", "SSH servers are read
only for now.", "This action is not available for Google Drive items yet.")
come from the provider that owns them.

**D4. What isn't POSIX is an extension, not a method on everyone.**

| Extension | Who has it today | Commands it replaces |
|---|---|---|
| `Thumbnails` | local, Drive, Arbesk (with today's per-window cancel tickets) | `thumbnail`'s per-kind arms |
| `Properties` (+ folder size, cancellable by walk id) | local, Drive | `file_properties`, `folder_size` |
| `Counts` | local | `item_counts` |
| `Search` (streamed batches, per-window cancel) | local | `search_dir` |
| `Measure` | local, Drive | `measure` |
| `Versions` | Arbesk (saves) | `arbesk_versions`; Keep Versions stays an app decorator (D7) |
| `Accounts` | Drive, Arbesk, SSH, shares | `drive_*`, `arbesk_connect_*`, `sftp_connect`, share links: same shape, each with its own sign-in step |
| `Sharing` | Arbesk (collections) | `arbesk_share_collection` (folder and QR sharing stay app features on local paths) |
| `Preview` | Arbesk (scenes and parts) | `arbesk_scene`, `arbesk_part` |
| `LocalOnly` | local: Make Link, permissions, Batch Rename, archives, Open With, drag out, OS icons, Recent | today's local-only commands, unchanged |
| `PermanentDelete`, `EmptyTrash` | local | only inside the Trash, as today |

An extension a provider lacks is simply absent; the app checks for it instead
of matching on a kind.

Step 1 declares only the slots: each trait gets its methods in the step that
first implements it, shaped like the command it replaces so nothing visible
changes. Those shapes are: thumbnails as PNG bytes with the per-window cancel
ticket; item counts as `FolderSummary`, version badges included; search with
`show_hidden`, its result limit, the per-window stop and `SearchSummary`;
a folder size as bytes, cancelled by walk id; sharing with Arbesk's numeric
role and its `Shared` result.

**D5. Capabilities are data, and the frontend reads them.** `Capabilities`
answers per path, without the network:

- booleans: `read_only`, `can_create`, `can_rename`, `can_trash`,
  `can_copy_out`, `knows_sizes`, `knows_counts`, `searchable`,
  `has_thumbnails`;
- how operations behave: `rename_atomic` (exclusive on disk; check-then-act
  on Drive), `replace` (`Trash` on disk and Drive, `NewVersion` on Arbesk),
  `names_unique` (false on Drive), `put_is_whole` (a Drive upload appears
  only when complete);
- the backend's refusal wording for each refused operation.

Capabilities are a **table per provider kind**, fetched once at startup by a
new `capabilities()` command into a synchronous cache, so `locations.ts` and
`item-actions.ts` stay synchronous pure functions. Nothing waits for it (as
done in step 7): the frontend has a built-in copy of the table, which
answers until the command's answer lands (or if it fails) and says the
same. The mock (`dev/mock-tauri/core.ts`) serves the same table, with a case
in `tests/mock-core.test.ts` and a shared fixture (`tests/capabilities.json`)
pinned against the Rust table test (as `tests/fold-cases.json` does for
`fold`) and against the built-in copy. `tests/locations.test.ts`,
`tests/item-actions.test.ts` and the three `item-actions-matrix-*.test.ts`
files run unchanged against the built-in copy, which the fixture pins.

The frontend reads **only the booleans** in this migration. It keeps its own
toast wording and every "Kept:" cell exactly as it is; moving a message to
the backend's wording is a later, visible decision for the human. Rules about
UI places rather than providers (Recent, Starred, Network, the Trash view)
stay in the frontend.

**D6. The data-safety rules live once, above the providers.** The
**transfer engine** in the app is `engine.rs` (the routes over a provider's
`Transfer`, since step 4) beside `placement.rs` (what lands on this
computer, as before). Together they keep everything `placement.rs` decides
today: the refusals and their order and wording, the name-clash decision
(Replace / Skip / Keep Both), staging, Replace via the Trash with take-back,
"delete the source only after the whole copy", "a move that replaces nothing
is one rename". It also takes over the local↔Drive half of
`cloud/transfer.rs` and Drive's `finish_staged`.

**Which pairs may transfer is the engine's, not the providers'.** The engine
keeps today's route table as a function of `(source kind, target kind)`, with
each refusal's wording and today's order (`writable` on both sides first, then
the local gate), ported with `place.rs`' tests: local↔local, local↔Drive,
Drive↔Drive (same account only), Arbesk asset → local or Drive, local model
file → Arbesk collection. Everything else is refused as today (SSH and shares
as sources, Arbesk collections, Drive between accounts, Drive → Arbesk).
Opening a new pair is a later, visible change.

Each provider's `Transfer` offers the primitives below, as safely as it can,
and declares how in `Capabilities`:

| Primitive | Local | Drive | Arbesk |
|---|---|---|---|
| write a file or a whole folder from a source into a target path the engine chose, reporting skipped items | staged file, folder filled in place (`Stage::by_kind`) | resumable upload; a file whose upload is whole goes straight to its final name, folders and Replace staged as `.<name>.filebrowser-upload`; size checked after each file | `put` of a local model file (`write`, whose `replace` flag asks for a new version); Replace saves a new version |
| read an item into that writer | the file or tree itself | in two steps (`read`): the item is fetched and refused if it has no bytes, then handed to the engine (`Reading`), which picks the local target by its kind and has it filled: download streamed into the target, Docs and shortcuts skipped and named, duplicate names made unique, size checked | the built `.gltf`/`.3mf` (`export`, a primitive of its own: a local file in the app cache the engine copies from as a local source, as today; `read_into` is unsupported) |
| rename that never replaces | `renameat2`/`renamex_np` | check-then-act: one PATCH of the name, the engine having looked it up (`landing`) just before; the final rename of a staged copy doesn't check again, as before | — |
| move the provider's own item to any path in it, never replacing, and move it back | `rename_new` | one PATCH, the name when it changes from the one Drive last gave the item, the parents when the folder changes, after the engine's `landing`; put back the same way, to the item's old folder and Drive's exact name (`Source::home`; where a path can't say that name, a `/` in it, the context remembers it and sends it) | — |
| send to its Trash for Replace (or a source a move to another place copied away), and take it back | `Trash::send` / `Trash::untrash` (no parent folders made, log entry forgotten) — not `restore` | `trashed=true` / `false`; Drive's take-back is best effort, as before: if it fails, the rename's error is the transfer's | — (Replace = new version) |
| copy the provider's own item inside it, its bytes never passing through this computer (`copy_within`) | — (Unsupported: a local copy is the engine's staged copy) | server-side `files.copy` (Docs and shortcuts too), a folder staged then renamed in, as `copy_in` and `finish_staged` do today | — |
| discard a staged or partial copy the engine made with this provider's own primitives | removed for good | `trashed=true`, never deleted | — |

A transfer between two different providers meets at a local path (a write
reads a local source, a read fills a local target); a copy inside one
provider (Drive↔Drive, same account) is `copy_within`, so its requests stay
the ones Drive's tests assert today.

Three more primitives let the engine decide without knowing a provider's
ids (added in step 4): `check`, the place's own refusals before any
request (Drive's, by id: into itself, the same path, between accounts, a
read-only connection; for a source a move must later send away, for a
target, or both); `landing`, what is at the pending target (the items
already called that, each saying whether the source is inside it, and the
source itself for a transfer inside the place, with where a move puts it
back); and `pending`, the pending path of a name in a folder, encoded as
the place's parser reads it. The engine then makes the name-clash
refusals itself, in today's order: a taken name without `overwrite`, an
ambiguous one where names aren't unique, a folder replaced by its own
child. Drive's primitives run on one context per transfer (`Ctx`), which
remembers what `landing` fetched, so no step asks Drive twice. Arbesk answers
them for a copy in: `check` accepts a name pending in a collection (the rest
of the refusals stay `write`'s), `pending` builds one as `parse_target`
reads it, and `landing` is unsupported, since `write` finds the asset it
replaces as it saves a new version.

The policies that differ by route stay per route, as `DiscardFn` is today,
and never go through a provider's `discard`: a Drive download's partial local
folder goes to the logged local Trash (step 4 adds a test for it); moving a local source away after a local→local copy across volumes removes
it for good (`remove_any`, as `fs_ops` does); after a local→Drive upload it
goes to the logged Trash through Keep Versions' decorator; a Drive source
after a download goes to Drive's Trash. A staged **original** (the user's own
item, moved under a staging name during a move that replaces) is only ever
moved back, never discarded or trashed, on every provider.

Before a move between providers the engine asks the source's place whether
it can be moved away (`Transfer::check` with the source alone: Drive's
writability) before copying, as `move_down` does, so a read-only source never
leaves a copy behind. Skipped items make the transfer an error, so a move
keeps its source. Cancel removes partial staged files. What is at the target
comes from `Transfer::landing`; where `names_unique` is false the engine keeps
the "Several items in this folder are named “…”." refusal, and where
`put_is_whole` is false a new file is staged too. The engine reads both from
the provider's `Capabilities`, and the way a transfer runs (inside a place,
up from this computer, down to it) from `place::kind` of its two ends; it
names no provider's scheme. Downloads keep
streaming straight into the staged target: no copy goes through the app
cache, so disk use, progress phases and what the cache prune sees don't
change. `fetch` (to the cache) is for open and preview only, and every file
it leaves there is marked with the `.downloaded` mark, so the week-old prune
still removes only unchanged copies.

**D7. Keep Versions stays in the app, as a decorator.** It wraps the local
provider's rename, transfer and the Replace-trash primitive (as
`VersionsState::transfer` and `VersionedTrash` wrap them now: histories are
closed before `send` and reopened on `take_back`) and needs the app's watcher
and SQLite. It is not a provider and doesn't move into `places-local`.

**D8. One `Entry`; only Drive fills the `cloud` key.** `places::Entry` keeps
today's fields and its typed `cloud: Option<CloudEntry>`, with
`#[serde(skip_serializing_if = "Option::is_none")]`, so the JSON the frontend
gets is byte-for-byte what it gets now (a test pins a local, a Drive and an
Arbesk entry). Only `places-gdrive` may set it, because the frontend treats
any `cloud` key as a Drive item; a later provider that needs data of its own
gets its own key and an explicit check in the frontend. `Progress`, `Phase`,
`Snapshot`, `CANCELLED` and `OpenTarget` move to `places`; `arbesk` depends on
`places` and re-exports them, and so does `progress.rs`, until callers move.

**D9. Errors are typed; the wording stays the same.** `places::Error` is
`NotFound | Exists | ReadOnly | Unsupported | AuthRequired | Unreachable |
TooLarge | Cancelled | Conflict | Other`, each with the message the user sees
today. Commands still return `String`, so the frontend (and
`drive-auth.ts`, which reads the wording) doesn't change. The kinds let the
engine and `listing-failure.ts` decide without parsing text.

**D10. The registry replaces the hand dispatch.** `Registry` holds one
provider per scheme and matches the exact, case-sensitive prefix
`<scheme>://` at the start of a path. A path that matches no registered
prefix goes to the local provider, which refuses a non-absolute path with
today's “{path}” is not a local file., so `file://`, `smb://`, `GDRIVE://`,
relative paths and a remote prefix inside a local path behave as now. A
malformed path with a registered prefix (`gdrive://`, `share://r1/%2E%2E`)
still reaches its provider, whose parser refuses it. Each provider also gives
its "This action is not available for …" text for the local-only gate, as
its `Capabilities::refused` entry for `Operation::LocalOnly`.
`place.rs` shrinks to the registry's lookup plus that gate; its table test
keeps every row. Each command becomes one lookup and one call (a copy or
move: the remote end's provider, its `Capabilities` and its transfer
session, handed to the engine); the frontend's
`Record<LocationKind, …>` tables keep their compile-time check. A command
that belongs to one place (the `arbesk_*` commands: New Collection, sharing,
saves, scenes, accounts) takes that place's provider by its scheme instead
(`Registry::provider`), so the place's own parser answers any path it gets,
malformed ones included, as its client did before the registry.

**D11. Windows lands in `places-local`.** What is platform-specific is
already confined: the exclusive rename (`MoveFileExW` without
`MOVEFILE_REPLACE_EXISTING`), the Trash (the `trash` crate's Recycle Bin),
volumes (drive letters), opening files (`ShellExecute`), icons and
thumbnails (the shell's), Recent. Paths stay plain OS strings; the frontend's
`paths.ts` gains a Windows local layout. Not part of this migration.

## Migration

Each step is one PR that changes no behaviour and keeps every test green; the
next starts only after the human merged the last.

1. **`places` crate with the types and shared helpers** (D2's list, D8).
   `arbesk` and the app re-export them; nothing else changes. `Entry.cloud`
   stays typed, so Drive's tests don't change.
2. **Registry and read-side dispatch:** `list`, `exists`, `path_info`,
   `thumbnail` (`ext::Thumbnails`), `fetch` (open, and the preview's file
   for `preview_bytes`), through providers that
   wrap today's code in place (`src-tauri/src/providers.rs`, Arbesk's
   adapter too until step 3). The `_ =>` fallthroughs go away; the
   registry's prefix rule (D10) keeps `place.rs`' table test passing
   unchanged. `stat`, `lookup` and the writes keep their commands' own
   dispatch until the steps that need them, and each adapter says so if
   asked. Received shares stay async in this step: `list_dir` and
   `open_target` still await their listing and download (D3), and their
   provider answers the rest. So two commands still look at the provider's
   scheme, the only exceptions to D10's one lookup and one call: `in_share`
   sends a share's listing and download to the async code, and
   `open_target` opens a path of this computer's provider (scheme "")
   without registering a progress operation, as it did. Both go in step 5,
   when shares run in `blocking(…)` and a fetch can say whether it reports
   progress. Capabilities are filled per kind, and per
   path where today's refusals differ by path (an Arbesk asset can be
   copied out, a collection can't; a relative path is refused as not
   local), with a table test against `place.rs`' refusals.
3. **Arbesk implements `Provider`** (it is already a crate): its `Client`
   is the provider (`crates/arbesk/src/provider.rs`), registered as it is,
   and step 2's Arbesk adapter goes. Reads (list, exists, path info, fetch
   for Open and Preview, `Thumbnails`), its capabilities path by path (copy
   in only into a collection, copy out only of an asset, with the route
   table's words), New Collection as `create` (a folder in an account,
   which its capabilities allow there only, `can_create` with no Create
   refusal; a collection or an asset refuses it with `create`'s own
   “Create collections in the account's folder.”, a malformed path with its
   parser's words, and a new file is refused by `create` as read only;
   nothing is renamed, trashed or restored: “Arbesk is read only.”), its `Transfer` for copy-in (`write`, a local model file into a
   collection; `replace` saves a new version) and copy-out (`export`, the
   asset's own file built in the app cache, which the route then copies as
   a local source, as today), and the extensions `Preview` (scene and
   part), `Versions`, `Sharing` and `Accounts`. Their answers are `places`
   types where one fits (`Version`, `Shared`, `SignIn`) and otherwise the
   provider's own record as it serialised it (`places::Json`: an account,
   a scene), so every command's JSON stays byte for byte. The `arbesk_*`
   commands take Arbesk's provider by its scheme (`Registry::provider`),
   so its parser answers any path they get, as the client did; the route
   table stays in `place.rs` (D6). The app's tests run the real client with
   no account (it asks no server); the answers are tested in the crate
   against its fake Arbesk.
4. **The transfer engine takes the local↔Drive half of `cloud/transfer.rs`
   and Drive's `finish_staged`**, then Drive becomes `places-gdrive`. The
   riskiest step, reviewed on its own, in two commits. First the engine
   (`src-tauri/src/engine.rs`, beside `placement.rs`) runs every route to,
   from and inside Drive from Drive's `places::Transfer` primitives (D6,
   with `check`, `landing`, `pending` and the two-step `read`), Replace in
   Drive included (`swap_in`: trash the old item, rename the staged one in,
   take the old one back, discard a staged copy or move a staged original
   back); downloads keep `placement::Target::place` and `VersionedTrash`.
   Then `src/cloud/` moves into the `places-gdrive` crate with Drive's
   `places::Provider` (`Calls`, `Live`, `Drive`), which replaces step 2's
   adapter; the sign-in window, the `drive_*` commands and the managed
   `CloudState` stay in `lib.rs`, which runs a copy or move through the
   registry like any provider: the Drive provider's transfer session
   (`Provider::transfer`, one context for all the steps) and its
   `Capabilities`, handed to `engine::dispatch`. Drive's tests came along
   unchanged in body (the 46 that copy or move now run the engine, from
   `engine.rs`, through wrappers named as the routes were), the request
   sequences they assert are identical, and the engine has a fake-Drive
   suite for every primitive in D6's table, plus each Drive primitive's
   requests on its own. `Transfer` is no longer `Send + Sync`: the engine
   uses one on the thread that runs the transfer.
5. **SSH becomes `places-sftp`**; shares implement `Provider` in the app, and
   their arms move into `blocking(…)` (D3) with a test that a share listing
   runs through the registry from a blocking thread; `in_share` and
   `open_target`'s scheme check go with them (step 2). `SftpState`, moved
   with its tests into the crate, is SSH's provider itself (as Arbesk's
   `Client` is): listing, opening as today's download into the app cache,
   no existence or path info, the preview refused, every write refused with
   “SSH servers are read only for now.”; `lib.rs` manages it in an `Arc`
   for the Connect to Server commands (the known-host and password prompts
   stay in the frontend and `lib.rs`) and registers the same `Arc`.
   Received shares are `share::provider::ReceivedShares`, which reaches the
   managed `ShareState` and the Keep Versions hint through `Shares` (answered
   for the app handle) and waits on `tauri::async_runtime::handle()`'s Tokio
   handle. Both read-only places take what they answer alike from
   `places`: `Capabilities::read_only_place`, and the `read_only_place!`
   macro (`stat` and `lookup` refused with their local-only words, since
   nothing is pasted there; no existence or path info; every write refused
   with their read-only words) with `refuse_preview`; each writes only
   `list` and `fetch`. Whether opening starts
   a tracked operation is provider data, `Provider::fetch_reports_progress`
   (true by default, false for this computer, whose file opens as itself),
   not a capability: the frontend has no use for it. `open_target` looks
   the provider up once, on a blocking thread, asks it that, and fetches
   from it, registering the operation only when it reports progress.
6. **Local becomes `places-local`** (fs_ops' file operations, the Trash and
   its log, the exclusive rename, platform pieces). The engine stays in the
   app (D6); Keep Versions stays as its decorator (D7). Listing 10,000
   entries is timed before and after (no regression: about 11 ms either
   way, release build). As done: `fs_ops.rs` and `trash_ops.rs` moved whole
   (`git mv`), the platform's file pieces (volumes, `%XX` decoding,
   `file://` URIs, the XDG data folder, `rename_exclusive`) into its
   `platform.rs`, which the app's `platform.rs` re-exports, and
   `placement.rs`' renames that never replace (with the staging names)
   into its `rename.rs`, which `placement` re-exports; local copy and move
   stay in the app as `local_transfer.rs` beside `placement.rs`, built on
   the crate's file primitives. The crate never depends on the app: Keep
   Versions' folder name moved into it (listings hide it; `versions`
   re-exports it) and the version badge's ownership check is passed in
   (`fs_ops::OwnsHistory`), and its `Local` provider gets the app's
   thumbnailer. The provider keeps step 2's read side; its writes, and a
   local `Transfer`, are step 6b, since the engine's local routes still go
   through `placement.rs` directly. Until then its write methods answer
   `Unsupported` (“… doesn't go through the places registry yet.”), and
   its capabilities describe what the app's commands allow on this
   computer, not what the provider itself does. The Trash fixtures and the
   source guards' walker (`test_support::source_roots`, one for every
   guard) are the crate's `testing` feature, which the app's tests enable;
   the batch scatter behind every local-only answer is the crate's
   `provider::answer_kept`, which `place::local_only` calls with its gate.
6b. **Keep Versions' decorator wraps `places_local::Local`; local writes and
   the local `Transfer` go through the registry** (D6, D7). The local
   provider's `create`, `rename`, `trash` and `restore` act (through
   `rename_new`/`rename_in_place`, `Trash::send`, `Trash::restore`), and its
   `Transfer` gives the engine `rename_new` as the atomic rename, `Trash::send`
   and `Trash::untrash` as Replace's send and take back, and the staged write
   (`Stage::by_kind`). Keep Versions becomes the decorator in the app that
   wraps the provider (capture before a write, `VersionedTrash` around the
   Trash step), registered in its place; `engine.rs` then runs local↔local
   routes from the primitives as it runs Drive's, and `placement.rs` keeps
   the refusals (`check_transfer`), `Target::place` and `Stage::by_kind`,
   which the engine's local route and downloads share. Each moved
   sequence keeps its tests unchanged in body; the not-yet answers and their
   test go. Steps 7 and 8 come after it. As done: the decorator is
   `versions/versioned.rs`'s `Versioned`, which the registry holds for this
   computer; it wraps Rename in `VersionsState::transfer` and Move to Trash
   in `trash_many` as the commands did (Keep Versions' part around a write
   is the commands': the histories inside the items it touches are closed,
   the capture in progress finishing first, then reopened and scanned
   again where the items are; "capture before a write" above means that),
   and New Folder, New File, Put Back and Batch Rename pass through, as
   before (Batch Rename never moved the histories in the folders it
   renames, and still doesn't). Rename takes a name in the item's folder
   only: a name with a slash, or a `to` in another folder, is refused in
   Drive's words rather than moved past the transfer's refusals. The local `Transfer` is
   `places-local`'s `transfer.rs` (`LocalTransfer`): `read` hands the
   engine the item itself, whose fill copies a file whole at the path the
   engine gives and makes a folder there and fills it in place; `move_to`
   (and `rename_new`) is `rename::rename_new`, answering `Unsupported` for
   an item on another volume so the engine copies it; `trash_for_replace`
   and `take_back` are `Trash::send` and `Trash::untrash` through the
   app's logged Trash (`LoggedTrash`); `discard` removes the engine's own
   copy for good; `check`, `landing`, `pending`, `write`, `read_into`,
   `export` and `copy_within` are other places' and answer `Unsupported`.
   `Stage::by_kind` stays the engine's (`placement.rs`), which stages a
   file and fills a new folder in place around that `read`. A new
   `Transfer::within` runs a whole transfer inside one place as it keeps
   it (default: just the transfer): on this computer it words failures as
   the copy's or the move's, and the decorator runs it inside
   `VersionsState::transfer`, so a local copy or move closes the histories
   in both ends for the whole route and its Replace uses the plain logged
   Trash, while Replace's Trash of a route that only lands here (a Drive
   download, a moved local source of an upload) goes through
   `VersionedTrash` from the same session. The local route is
   `engine::local`: `placement::check_transfer`'s refusals, then
   `Target::place` (a move that replaces nothing is one `move_to` onto the
   free target; across volumes the source is removed with `remove_any` only
   after the copy is in place, never through `discard`), so
   `placement.rs` serves the engine's local route as well as downloads.
   `local_transfer.rs` is gone; its tests are `engine::local_tests`, bodies
   unchanged. Batch Rename is the `LocalOnly` extension's first method
   (`rename_many`); Move to Trash's log is written by this computer's
   provider (`Local::trash`, `Trash::record`), `trash_routed` only routes
   the batch; the app hands the provider its Trash as a
   `places_local::TrashReach` (`lib.rs`' `AppTrash`); and `Trashed.result` became
   `Result<Option<String>>`, since this computer can't always say where an
   item went. Copying a 10,000-file folder (release build, on tmpfs) takes
   about 92 ms before and 91 ms after.
7. **Capabilities reach the frontend** (D5): the command, its mock and
   mock-core case, the startup cache, the shared fixture; `locations.ts` and
   `item-actions.ts` read the booleans rule by rule. As done: the command
   is `capabilities` (`lib.rs`, a plain command: it asks no server and
   touches no file), answered by `providers::capability_table`, which asks
   each registered provider's `capabilities` for a path of each kind the
   frontend tells apart (`providers::KINDS`: a folder, Drive, a share, an
   SSH server, and Arbesk's account, collection and asset, which differ) and
   keeps only the booleans (`KindCapabilities`), plus `can_copy_in`, true
   where the place has no copy-in refusal (only an Arbesk collection takes
   copies among Arbesk's rows). No refusal wording crosses: the frontend
   keeps its toasts. `src/capabilities.ts` holds the cache, its built-in
   table and `loadCapabilities`, which `main.ts` starts at startup without
   waiting. `locations.ts` picks a row per kind (`capabilityRow`; an Arbesk
   item with no path counts as a collection, anything but a collection or an
   asset as the account; a path's row is read for its copy flags only,
   `copiesOf`, since the provider refuses New Collection at the Arbesk root
   and on a malformed path, where the account row allows it, a Rust test
   pinning that) and reads `read_only` (`isReadOnlyKind`), `knows_sizes`,
   `searchable` (recursive search), `can_create` with `read_only` (New
   Folder and New Document), `can_copy_in` with `read_only` (Paste and
   drops, `kindTakesPastes`) and `can_copy_in` alone (Arbesk's "Paste into a
   collection."), and `can_copy_out` (the copy-out refusal);
   `item-actions.ts` reads `can_rename` and `can_trash` (`byPlace`) and
   cut's read-only check through `isReadOnlyKind`, its toast picked by the
   item's kind (Arbesk's words for Arbesk, a share's otherwise). Kept as the frontend's own: the answers for its
   views (Recent, Starred, Network, the Trash, `ViewKind`), and the rules no
   capability matches: a local location and local files, versions, the
   remote preview, an Arbesk asset's own thumbnail (`has_thumbnails` says a
   place has thumbnails, not one per file), Arbesk's model-file check,
   Delete Permanently, Star, Add to Bookmarks, Properties, Open Item
   Location. `tests/capabilities.json` holds the table and each row's path;
   `providers.rs`' test, `tests/capabilities.test.ts` (the built-in table,
   the row each path picks, the rules reading a changed table) and
   `tests/mock-core.test.ts` check it.
8. **Docs:** AGENTS.md's layout, `docs/architecture.md` and its diagram.

**Guard tests move with the code.** The tests that enforce "only
`trash_ops` calls the `trash` crate" and "only listed modules call
`std::fs::rename`" scan `src-tauri/src` and every crate under
`src-tauri/crates/` (extended in step 1). Later steps only add allowed
entries for code that moves, in the same PR, and step 6 moved the
trash-crate exemption with `trash_ops` to `places-local`
(`crates/places-local/src/trash_ops.rs`, its guard test there) and
`rename_new`'s `std::fs::rename` fallback from `placement.rs` to
`crates/places-local/src/rename.rs`.

After the migration, a new provider is: a crate and a registry line; a row in
`paths.ts`' SCHEMES and a `Place`; a `LocationKind` and a cell in each
`Record` table (item actions, menus, `listing-failure.ts`, `item-icons.ts`);
its capabilities row; a `providers.ts` row and `PROVIDER_ACTIONS` entry if it
signs in; the mock and its tests; a cache folder in `downloads::DIRS`. That
is about 10 places instead of 33, each one a table row the compiler or a test
points at. Collapsing the frontend tables further is a later step.

## Risks

- **A leaky lowest common denominator.** Mitigated by D4: the core trait
  stays small and honest; what only one provider has is an extension.
- **Data safety while moving code.** The engine's primitives (D6) name every
  per-provider and per-route difference that exists today, and the take-back,
  staging and discard tests move with them unchanged. Step 4 is reviewed on
  its own; the guard tests follow the code.
- **Huge local folders.** `places-local::list` stays one `read_dir` with the
  same per-entry work; the registry lookup is a string prefix check.
- **Wording drift.** Every message keeps its exact text; tests that assert
  wording stay as they are, and the frontend keeps its own toasts (D5).

## Testing

Each step keeps the existing suites green (`cargo test` for the app and every
crate, `npm test`, the CRAP gate). New tests: the registry (each scheme
reaches its provider; an unregistered prefix reaches local and is refused
with today's text; a malformed registered path reaches its own parser), the
route table (every allowed and refused pair, wording and order), the engine
against a fake provider for each D6 primitive and each `replace` /
`rename_atomic` / `names_unique` / `put_is_whole` combination, the serialised
JSON of entries (D8), the capabilities fixture shared with the mock (D5), and
the guard tests over every crate.

## Files

New: `src-tauri/crates/places`, `places-local`, `places-gdrive`,
`places-sftp`. Changed over the steps: `src-tauri/Cargo.toml` (workspace),
`src-tauri/src/lib.rs`, `place.rs`, `engine.rs` and `placement.rs` (the engine), `fs_ops.rs`,
`trash_ops.rs`, `cloud/`, `sftp.rs`, `share/`, `downloads.rs`,
`progress.rs`, the `arbesk` crate, `src/locations.ts`, `src/item-actions.ts`,
`src/api.ts`, `dev/mock-tauri/core.ts`, `tests/mock-core.test.ts`, AGENTS.md,
`docs/architecture.md`.
