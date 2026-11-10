# AGENTS.md

Guidance for AI coding agents working in this repository.

## What this is

A file browser for **macOS and Linux** that imitates **GNOME Files 50**
(Nautilus + libadwaita). Tauri 2 app: Rust backend for every filesystem
operation, plain TypeScript + Vite frontend (no UI framework). Personal hobby
project, GPL-3.0-or-later; it contains no GNOME code and must not copy any (see
"Licensing" below).

Every place the app shows (this computer, Google Drive, Arbesk, SSH
servers, received shares) is a **provider** behind one Rust interface, the
`places` crate's `Provider`, held in a `places::Registry` that picks the
provider by the path's scheme. Each provider lives in its own crate under
`src-tauri/crates/` (received shares stay in the app, in `share/`); the
app keeps the commands, the transfer engine (`engine.rs` + `placement.rs`)
and Keep Versions, which wraps this computer's provider. The frontend reads
what each place allows from the providers (`capabilities.ts`). The design
and how it was built: `docs/superpowers/specs/2026-11-10-provider-crates-design.md`;
the picture: `docs/architecture.md`.

Platform differences are confined to the `places-local` crate (this
computer's provider): its `platform.rs` (volumes, percent-decoding, XDG
paths, the rename that refuses a taken name) and `trash_ops.rs` (the two
Trash layouts), both under `src-tauri/crates/places-local/src/`; and to the
app's `src-tauri/src/platform.rs` (opening files and links, Open With,
window buttons, dragging out, DMG and AppImage installs);
`thumbnails.rs` asks Quick Look on macOS, `recent.rs` branches for its
source and `file_icons.rs` for whose icons it asks (NSWorkspace on macOS,
GIO and the icon theme on Linux). The remote providers are
platform-neutral, and Windows, when it comes, lands in `places-local`
(design D11). Everything else must stay platform-neutral.

Design intent and decisions live in `docs/superpowers/specs/` and the task plan
in `docs/superpowers/plans/`. Read the spec before changing behaviour.
[docs/architecture.md](docs/architecture.md) shows how the modules fit
together and where each kind of change lands; keep it and its diagram in
step when a module's role changes.

## Layout

| Path | What it holds |
| --- | --- |
| `src/main.ts` | App state, navigation, path bar, search, virtualized grid/list views, selection, menus, clipboard + paste, drag and drop, bookmarks, keyboard shortcuts, sidebar, init |
| `src/ui.ts` | libadwaita-style widgets: popovers, popover menus, dialogs, toasts; a dialog's busy mode (one action in flight, what stays live, the status line, failures in words, the buttons put back) and `cleanError` |
| `src/icons.ts` | Icon sprite (symbolic `sym-*`, full-colour `fc-*`), drawn for the theme; file-kind detection; `itemIcon`, every item icon's markup: its picture (drawing, OS icon, thumbnail, glyph) and which emblems (link, versions, shared) go on it, in what order |
| `src/icons-macos.ts` | The macOS theme's drawings under the same symbol ids: folders, file pages, line icons |
| `src/thumbnails.ts` | Thumbnail request queue (4 concurrent) and bounded blob-URL cache |
| `src/item-icons.ts` | Which picture an item shows (thumbnail, the SSH server's and each online account provider's glyph in Network, OS icon, drawing), the shown size, which places have OS icons, the first-screenful wait before a folder shows, the macOS folder icons asked again per listing and when recounted; the dialogs' icons |
| `src/native-icons.ts` | The OS icons' cache: ids in one batch, one image per type and size, rechecking cached ids; `item-icons.ts` decides what to ask for and when |
| `src/format.ts` | GNOME-style size and date strings |
| `src/api.ts` | Typed wrappers for every Tauri command (`capabilities`, the table `capabilities.ts` reads at startup, among them) |
| `src/styles.css` | All styling: layout, views and widgets, reading every look-related value from theme tokens |
| `src/themes/gnome.css`, `src/themes/macos.css` | The two themes' tokens (colours, font, sizes, radii, glass), light + dark; macOS also has a few scoped rules |
| `src/theme.ts` | The platform's theme (macOS on a Mac, GNOME elsewhere) and the per-theme layout sizes the virtualized views use |
| `src/qr-share.ts` | Send with QR Code…: the menu item and the dialog (code, link, progress, downloads) |
| `src/models.ts` | 3D models in the Space preview without three.js: which files are models (`modelFormat`), and which companion files a .gltf may load, only in its folder or below (`companionPath`, `gltfCompanions`) |
| `src/model-gestures.ts` | Turning and zooming the 3D preview with a pointer, without three.js: a drag turns, the wheel and a pinch zoom (two touch points, Ctrl/⌘ + wheel from a trackpad, WebKit's `gesture*` events on a Mac), none zooming the page, and a double-click picks a point (`attachGestures`, `wheelZoom`) |
| `src/model-viewer.ts` | The 3D preview, loaded on demand in its own chunk: three.js reads glTF/GLB/STL/3MF (`loadModel`, no WebGL needed; Draco-compressed glTF through one shared decoder, `dracoLoader`; a glTF fetches only its companions and its own data and blob URLs, `resourceUrl`; STL and 3MF stood up from Z-up; files without a material drawn grey) and draws it to turn and zoom (`showModel`, released by `dispose`); an Arbesk scene copies each part per placement, a skinned part with its own skeleton and bones (`placeScene`); the model turns about the middle of its visible vertices (`shownBounds`) or what is double-clicked (`pickedPoint`: the small object clicked, or the point on a large one; `recentred`) and the camera only moves in and out; Rotate Left, Rotate Right and Reset View over the model, the arrow keys, Home, + and - on it (`modelControls`, `keyAction`, `turned`, `dragTurns`, `zoomed`) |
| `src/preview-versions.ts` | Versions in the Space preview: a Keep Versions history (without deletion markers) or an Arbesk asset's saves as one list, oldest first (`fromLocal`, `fromArbeskSaves`), and the header control that steps through it (`drawVersionControl`: Older Version, "Version N of M" over its date opening the list, Newer Version); `main.ts` shows the picked version through `previewEntry`'s `PreviewAt` |
| `src/preview-actions.ts` | Which actions the Space preview offers (Open, Rename…, Move to Trash, Open Item Location) for where the item is, from `item-actions.ts`'s rules plus the preview's kept exceptions |
| `src/selection.ts` | The view's selection, cursor and anchor, and the user's intent counter a paste checks before selecting what it made (#50) |
| `src/path-refs.ts` | What remembers paths (stars, bookmarks, the clipboard, the background tabs' folders and every tab's Back and Forward) kept pointing at moved, renamed and trashed items: `moved(pairs)` and `gone(paths)`, one rule per holder, each holder replaced (and saved) at most once per report, a paste reporting each item as it lands; `followPath`, the prefix rewrite for what is inside a moved folder |
| `src/latest-save.ts` | `LatestSave`: one file's saves (`starred.json`, `pins.json`) one write at a time, each reading the state as it starts, so the last write is the latest state; saves asked for meanwhile share the next write, and each learns whether it was written |
| `src/search.ts` | The search without the DOM: `SearchSession` (open or not, the text, the walk after a 160 ms pause, at most 1000 results, late answers dropped, what a tab saves), `arrangeSearch` / `searchSections` (what the view shows and its "In This Folder" / "In Subfolders" headers), and `fold`, tested against the backend's `fold` (`places-local`'s `fs_ops.rs`) with `tests/fold-cases.json` |
| `src/locations.ts` | Which kind of place a path is (`locationKind`: Recent, Starred, Network, the Trash, a folder in it, a share, an SSH server, Arbesk, Drive, a folder), in one precedence built on `paths.ts`'s `placeOf`, and what each kind allows: read only, local, recursive search, versions, new items, its own thumbnails, known sizes; which row of `capabilities.ts`' table answers for a kind (`capabilityRow`: Arbesk's account, collection or asset by path, none for the frontend's own views, `ViewKind`: Recent, Starred, Network, the Trash), so read only, known sizes, recursive search, new items (`can_create`), paste and drops (`kindTakesPastes`, `takesDrops` for a drop's path: `can_copy_in`), copy out and the collection a paste into Arbesk needs come from the providers' capabilities (a path's row for its copy flags only, `copiesOf`: the Arbesk root and a malformed path share the account's row but not its New Collection), while local, versions, the preview, its own thumbnails and the views' answers stay its own rules (each says why); the copy-out refusal's wording; what a paste into Arbesk accepts (`arbeskPasteRefusal`, `isArbeskModelFile`, `arbeskTargetName`); which items are local files (`isLocalFile`, `isLocalRegularFile`) |
| `src/item-actions.ts` | Whether an action several surfaces reach (cut, copy, paste, rename, trash, delete permanently, star, bookmark, properties, open location) may run in a kind of place, and the toast that says why not: rename and Move to Trash from the place's capability (`byPlace`: `can_rename`, `can_trash`) with a cell for each of the frontend's own views (`Record<ViewKind, …>`), cut, copy and paste through `locations.ts`'s predicates (which read the capabilities; paste reads `can_copy_in`, paste into Arbesk has its own rule; cut's read-only toast is picked by the item's kind), and a `Record<LocationKind, …>` where no capability matches (delete permanently, star, bookmark, properties) or a rule (open location); the toasts are its own; differences between surfaces kept on purpose are marked "Kept:" |
| `src/capabilities.ts` | What each kind of place allows, as the backend's providers say it (design D5): the booleans of their `places::Capabilities`, one row per kind (`CapabilityRow`: a folder, Drive, a share, an SSH server, Arbesk's account, collection and asset), read once at startup (`loadCapabilities`, which nothing waits for) into a synchronous cache; until it lands, or if it fails, the built-in table answers, the same as the backend's: `tests/capabilities.json` pins the backend's (`providers.rs`' test), the built-in one and the mock's |
| `src/paths.ts` | Each kind of path's layout (`gdrive://`, `share://`, `sftp://`, `arbesk://`, local) as one row: its kind (`placeOf`), name, parent, child join (encoded as the backend decodes it) and crumbs; a collection (`isArbeskCollection`); containment (`isUnder`, `isWithin`: not interchangeable), which paths a window may open, share links, what the path bar resolves typed text to |
| `src/folder-summaries.ts` | Folder item counts and version badges for the view: batches of 400, skips remote, known and in-flight folders, drops late answers, forgets counts on a new location or Show Hidden setting, asks again after a reload for those the view asks about (showing the old ones until then); says which answers were asked for one by one (`recounted`) |
| `src/listing-failure.ts` | What a folder that couldn't be listed turns into, one table row per kind and phase (first view, navigate, reload, path entry): share status page, Connect to Server, go to the parent, toast with Grant Access, error page; a Drive failure that lost its credential also carries the answer from `drive-auth.ts`, which opens the sign-in dialog |
| `src/providers.ts` | Online account providers, one row each (`PROVIDERS`: label, icon, which paths are its own, the account key in a path), the accounts in one shape whatever their provider (`Account`), which account a path belongs to (`accountOf`); what a provider does is `PROVIDER_ACTIONS` in `main.ts`, a `Record<ProviderId, …>` |
| `src/online-accounts.ts` | Online Accounts, as GNOME Settings' panel: "Your Accounts" and "Connect an Account" as boxed lists of rows, and each account's dialog (Sign In Again when its last request needed one, Disconnect…, the provider's links); built from `providers.ts` |
| `src/drive-auth.ts` | Which failed Google Drive request needs a credential, and which: the client JSON (`client`) or a new sign-in (`sign-in`); the wording is tested against the backend's messages |
| `src/open-with.ts` | Open With…: the searchable application chooser (filter, keyboard selection, launch and retry) |
| `src/updates.ts` | Check for Updates dialogs: offer, download progress, restart, failures |
| `src/drag-out.ts` | Dragging files out of the window: when the pointer has left it, and the system drag (our GTK command on Linux, the drag plugin on macOS) |
| `src-tauri/crates/places/` | The interface every place implements (design: `docs/superpowers/specs/2026-11-10-provider-crates-design.md`); no Tauri, depends on `serde` and `serde_json` (with `raw_value`) only. `Provider`: `scheme`, `capabilities`, the reads (`list`, `stat`, `lookup`, `exists`, `path_info`, `fetch`, and `fetch_reports_progress`: whether opening is a tracked operation, false only for this computer), the writes (`create`, `rename`, `trash`, `restore`), `transfer` (a `Transfer` session for one whole copy or move, which callers reach through `with_transfer`) and `extensions`. `Transfer`: the primitives the app's engine builds copy and move from (`check`, `landing` with `Landing`/`Existing`/`Source`, `pending`, the two-step `read` with `Reading`/`Unread`, `write`, `read_into`, `export`, `copy_within`, `rename_new`, `move_to`, `trash_for_replace`, `take_back`, `discard`, and `within`, which runs a whole transfer inside one place as it keeps it); not `Send + Sync`, one per transfer's thread. `Capabilities`: the booleans the frontend reads, how operations behave (`rename_atomic`, `replace`, `names_unique`, `put_is_whole`) and the words of each refused `Operation`; `Capabilities::read_only_place`, the `read_only_place!` macro and `refuse_preview` are what an SSH server and a received share answer alike. The `ext` traits in `Extensions`: `Thumbnails`, `Versions`, `Preview`, `Sharing`, `Accounts` and `LocalOnly` (`rename_many`, Batch Rename) have methods; `Properties`, `Counts`, `Search`, `Measure`, `PermanentDelete` and `EmptyTrash` are empty slots, their commands still calling their modules. `Error` (typed kinds, each carrying the words the user sees) and `Registry` (`provider_for`: the exact, case-sensitive `<scheme>://` prefix, anything else to this computer's provider; `answer_each` for a batch across places; `provider(scheme)` for a command that belongs to one place). And what places share, so no provider reaches into another or into the app: `Entry` (its `cloud` key, `CloudEntry`, set by Google Drive only), `OpenTarget`, `PathInfo`, `Measure`, `FileProperties`, `Progress`/`Phase`/`Snapshot`/`CANCELLED` with `copy_counted`/`cancelled_io`/`is_cancel`, `Refusal`'s wording, the records extensions answer (`Version`, `Shared`, `SignIn`, `Json`: a provider's own record handed on as it serialised it), the clock (`now`), `write_json` and the `.downloaded` mark; the app's old homes re-export what they used to define (`progress`, `placement::Refusal`, `config_store::write_json`, `downloads::mark`, `properties::FileProperties`, `recent::now`) |
| `src-tauri/src/providers.rs` | The `places::Registry` of every place, which `lib.rs` manages: `registry(states, trash, folders, held, runtime)` registers this computer's `places_local::Local` (handed the app's thumbnails as `LocalThumbnails` and its Trash and home folder as a `places_local::TrashReach`: `lib.rs`' `AppTrash`) inside Keep Versions' decorator, `versions::versioned::Versioned`, which is what the registry holds for this computer; then Google Drive's `places_gdrive::Drive` (reaching its state through `Reach`), Arbesk's `Client` and SSH's `places_sftp::SftpState` (`Held`: the same `Arc`s the app's commands hold) and received shares' `share::provider::ReceivedShares` (through `share::provider::Shares`, waiting on `runtime`, Tauri's async runtime). No Tauri type here: `lib.rs` answers `Reach` and `Shares` for the app handle. `capability_table` answers the `capabilities` command: the booleans of each registered provider's own `capabilities` (`KindCapabilities`, plus `can_copy_in`: no copy-in refusal) for a path of each kind the frontend tells apart (`KINDS`: a folder, Drive, a share, an SSH server, Arbesk's account, collection and asset), pinned by `tests/capabilities.json`. Its tests check every provider's `Capabilities` row by row against `place.rs`' refusals; `providers::testing` stubs Drive's `Calls`, so they run the real providers and registry (Arbesk's real client with no account, SSH's real state with no server, `NoTrash`, `NoShares`) |
| `src-tauri/src/place.rs` | The backend twin of `placeOf`, and a narrow gate, not a router: the read commands (listing, existence, path info, thumbnails, opening, the preview's file) ask the registry directly and never call it; the local-only commands, the writes, the copy/move/rename route table and the local-only batches use it. A path's kind (`Kind`: `Local`/`Drive`/`Share`/`Sftp`/`Arbesk`, from its scheme), a local path or why it isn't one (`local`, `local_write`: remote, malformed or relative), the read-only refusals (`writable`), the route table of copy, move and rename (`transfer`, `Route`; `copy` also takes an Arbesk asset out, built first, and `Route::IntoArbesk` puts a local model file in), and a batch answered for its local paths only (`local_only`, over `places_local::provider::answer_kept`) |
| `src-tauri/src/engine.rs` | The transfer engine (design D6): the copies and moves on this computer and to, from and inside Google Drive (a local model file copied into Arbesk, `Route::IntoArbesk`, skips it: `lib.rs`' `write_into_place` calls Arbesk's `Transfer::write`; an Arbesk asset copied out is built by `Transfer::export` in `lib.rs`, `exported_source`, before the engine copies it), built from the two ends' `places::Transfer` primitives and their declared `Capabilities` (`names_unique`, `put_is_whole`), never naming a provider's scheme; which way a transfer runs comes from `place::kind` of its two ends. It never sees the registry: `lib.rs` takes each provider from it and opens the transfer session (`in_transfer`, `drive_transfer`). `local`: a copy or move on this computer, on the `&dyn Transfer` it is handed and inside that session's `within` (which Keep Versions' decorator runs inside `VersionsState::transfer`), refused by `placement::check_transfer` and placed by `placement::Target::place`: `read` fills a staged file or a folder in place, `move_to` is one rename that never replaces straight onto a free target, an item on another volume is copied and its source removed for good (`remove_any`) only after the copy, Replace goes through `trash_for_replace`/`take_back`. `dispatch` (handed an `engine::Remote`: the remote end's session and capabilities): to, from and inside Google Drive (`upload`, `move_up`, `download`, `move_down`, `copy_within`, `move_within`), with the refusals' order (the place's own `check`, then this computer's, then what `landing` found: a taken name, an ambiguous one, a folder holding the source), the staging names (`.<name>.filebrowser-upload`/`-copy`/`-move`), Replace in Drive (`swap_in`: trash the old item, rename the staged one in, take the old one back if that fails, then discard a staged copy or move a staged original back), downloads placed by `placement::Target::place` with a partial folder and a replaced item going to the logged local Trash (`VersionedTrash`), and a move's source sent away only after the whole copy. Outside its tests it depends on `places`, `place` (`Kind`, `kind`), `placement` and `places_local::transfer::TrashOf` only. Tested on Drive's real primitives against its fake server, on a fake Drive for every primitive, and on this computer by `local_tests` |
| `src-tauri/src/placement.rs` | How the engine lands on this computer (a local copy or move, a Drive download): the refusals and their wording (`Refusal`, `check_transfer`, `check_new`), the `.`/`..` guard, staging names and `Stage::by_kind` (a new file staged, a new folder filled in place), Replace (stage, trash the old item through `ReplaceTrash`, rename in, or take it back out if that fails) and what a failure undoes (`Target::place`); re-exports `rename_new` from `places_local::rename`; only the engine calls it |
| `src-tauri/crates/places-local/` | The `places-local` crate: this computer's files, no Tauri, never depending on the app (what it needs is handed in: the version badge's ownership check, `fs_ops::OwnsHistory`; the thumbnailer; the Trash and home folder, `TrashReach`). `fs_ops.rs` (listing, entries, search, item counts, mkdir, new files and links, Rename and Batch Rename, path info, previews, measure, `copy_recursive`, `make_folder` and the copy errors; `VERSIONS_DIR` and `is_internal_path`, which listings hide and `versions` re-exports), `rename.rs` (the renames that never replace: `rename_new`, `rename_in_place`, `rename_back`, and the staging names), `trash_ops.rs` (see its row), `platform.rs` (volume roots per OS, `%XX` decoding, `file://` URIs, the XDG data dir, `rename_exclusive`: `renameat2` on Linux, `renamex_np` on macOS; the app's `platform.rs` re-exports them), `provider.rs` (`Local`, this computer's `places::Provider`: listing, existence, path info, opening, thumbnails, capabilities, New Folder and New File (`create`), Rename (`rename`; a new name with a slash, or a Rename into another folder, is refused in Drive's words, `RENAME_STAYS`), Batch Rename (`LocalOnly::rename_many`), Move to Trash (`trash`: one logged batch) and Put Back (`restore`); `local`, the words a path that isn't absolute is refused with; `answer_kept`, the batch scatter `place::local_only` uses; `LoggedTrash`, the logged Trash a Replace uses), `transfer.rs` (`LocalTransfer`, its `places::Transfer`: `read` (the item itself: a file copied whole at the path the engine gives, a folder made there and filled in place), `within` (failures worded as the copy's or the move's), `move_to`/`rename_new` (`rename::rename_new`; an item on another volume answers `Unsupported`, so the engine copies it), `trash_for_replace`/`take_back` (`Trash::send`/`Trash::untrash` through `ReplaceTrash`, the trait `placement` places with), `discard` (`remove_any`, only the engine's own copies), the rest `Unsupported`; `TrashOf`, a session's Replace-trash as a `ReplaceTrash`). Its `testing` feature (the app's dev-dependencies only) gives the Trash fixtures to the app's tests and `test_support::source_roots`, the one walker every source guard scans with |
| `src-tauri/crates/places-local/src/trash_ops.rs` | `Trash` (one in `lib.rs`'s state): `trash_log.json` and its lock, where an item came from (sidecar, log, Put Back), send (trash one item and log it), untrash (for a Replace that failed after trashing the old item, forgetting its entry), restore, delete permanently, empty, what is inside the Trash; Move to Trash's batches (`trash_many`; `trash_routed`, which only routes a batch across places) as free functions, logged through `record` by this computer's provider (`Local::trash`); macOS `~/.Trash` and `<volume>/.Trashes/<uid>` + Finder "Put Back" in each one's own `.DS_Store`, Linux `Trash/{files,info}` and `<volume>/.Trash-<uid>` + `.trashinfo`; the layout is a value (`Layout`), so tests lay out either on any host; its fixtures are `trash_ops::testing` |
| `src-tauri/crates/places-gdrive/` | The `places-gdrive` crate: Google Drive, no Tauri (the sign-in window, the `drive_*` commands and the managed `CloudState` stay in `lib.rs`). `CloudState` (accounts, tokens, the user's OAuth client, sign-ins in flight, remembered thumbnail links), `accounts.rs`, `client.rs` (the OAuth client JSON), `token.rs` (keychain with a 0600 file fallback), `oauth.rs` + `loopback.rs` (PKCE sign-in through a loopback wait), `drive.rs` (the REST client: retries, 401 refresh, resumable uploads, downloads), `location.rs` (`gdrive://<account>/<id>~<name>`, pending `…/~<name>`; `is_cloud`), `service.rs` (listing, existence, path info, properties, opening into the app cache, thumbnails), `write.rs` (New Folder, Rename, Move to Trash, Undo, and `Ctx`, the context of one request or transfer), `transfer.rs` (Drive's `places::Transfer` on a `Ctx`: `check`, `landing`, `pending`, `write`, `read`/`read_into`, `copy_within`, `rename_new`, `move_to`, `trash_for_replace`, `take_back`, `discard`; and `measure`), `provider.rs` (`Drive`, its `places::Provider`, over `Calls`, the server calls the app's tests stub: reads, thumbnails, `capabilities`, and its transfer session, one `Ctx` for a whole transfer (`Calls::with`); `Live` over the app's state through `Reach`). Drive's own writes (New Folder, Rename, Move to Trash, Put Back) are still `lib.rs`' calls into `write.rs`, so the provider's `create`, `rename`, `trash` and `restore`, and its `stat` and `lookup`, answer `Unsupported` ("… doesn't go through the places registry yet."). `test_server.rs` and `testing.rs` (the fake Drive server and its fixtures) are also built for the app's tests by the `testing` feature |
| `src-tauri/crates/places-sftp/` | The `places-sftp` crate: SSH servers, no Tauri. `SftpState` (read-only SSH/SFTP connections for Network: agent/key/password auth, known-host checks, listing and downloads into the app cache) is itself SSH's `places::Provider` (`provider.rs`: listing, opening as a download with progress, no existence or path info, the preview refused with the local-only words, every write refused as read only, `Capabilities::read_only_place`); `lib.rs` holds it in an `Arc`, managed for the Connect to Server, server list and Disconnect commands (the known-host and password prompts stay there) and registered in the registry; `READ_ONLY` and `LOCAL_ONLY` are the words `place.rs` refuses with |
| `src-tauri/crates/arbesk/` | The `arbesk` workspace crate. The app sees only its facade — `Client`, `SCHEME`, `Account`, `Entry`, `Scene`, `Version` and `Shared` (both `places` types), `Location` and the path helpers `place.rs` parses with, `Progress` (re-exported from `places`) — and reaches Arbesk through `Client` as a `places::Provider` (`provider.rs`: its capabilities, path by path with the route table's refusal words; listing, existence, path info, opening in the Studio and the preview's build; New Collection as `create`, which its capabilities allow in an account only (a collection or an asset refuses it in `create`'s own words); its `Transfer`, `write` for a local model file into a collection and `export` for an asset's own file out, `check` and `pending` for a name pending in a collection, `landing` unsupported (`write` finds what it replaces), the rest refused as read only; `stat` and `lookup` answer `Unsupported`, since no command routes them through a provider yet; the extensions `Thumbnails`, `Versions`, `Preview` (scene and parts), `Sharing` and `Accounts`, an account and a scene handed on as `places::Json`); every module below is internal. Arbesk accounts in Network, read plus creating named collections, sharing one by email with a Viewer or Editor, and copying local `.gltf`/`.glb`/`.3mf` files into one: `location.rs` (`arbesk://<address>/<token>~<name>/<asset>~<name>.<ext>`, ids decide; `parse_target` for a paste's destination), `chain.rs` (the backend's config, kept for five minutes per app cache folder for reads while writes read it fresh, and its token index, `tokenURI` by hand-written `eth_call` — read live on every listing and Reload — decimal uint256 words), `create.rs` (New Collection and `besk create`: the token id from the account and name by keccak256, the first manifest and its one-editor Merkle root, both uploaded, then the relay's `publish`; an existing name is refused), `merkle.rs` (the editor tree as asset-core and OpenZeppelin build it: leaves, roots, proofs and the sorted-pair hash), `share.rs` (Share Collection: resolve the email through Arbesk, read `editorListURI` and its version, add or change the member, upload the new list and relay `updateEditors` with the caller's proof), `ipfs.rs` (gateway reads, brotli/gzip unpacking, a bounded disk file cache for manifests and Arbesk thumbnails, content-addressed and written whole), `compose.rs` (a stored source back to `.gltf`/`.3mf`, unknown formats refused), `service.rs` (listing, naming an asset from the format its manifest already names and only falling back to the source manifest when it names none; building an asset into the app cache, reused only while unchanged: the preview's file, `build`, or the asset's own `.gltf`/`.3mf` that a copy takes out, `export`; the thumbnail an asset was saved with, remembered by the listing and kept on disk by CID and size; an asset's saves through `prev_asset_manifest_cid`, the newest 50, and the scene of one of them, `versions`/`scene`), `scene.rs` (a manifest's nodes and links resolved into parts and placements), `signin.rs` (`besk login`'s email + code flow through the crate's own loopback wait in `callback.rs`, the callback confirmed with Arbesk, the session handed to `session.rs`), `session.rs` (the sign-in's session, 24 h, in the OS keychain with a 0600 file fallback; ended on Disconnect), `accounts.rs` (`arbesk_accounts.json`: email, address, deployment; no credential), `decompose.rs` (asset-core's decompose ported: GLB textures split, parts brotli-framed with `_arbesk` murmur3-128 metadata, 3MF parts raw), `write.rs` (uploads, manifests, the relay's collection update, one write at a time per collection, the wait after a save that may still land), `testing.rs` (the fake Arbesk the write tests share, test-only), `http.rs` (the `Http` seam tests fake, multipart, the slow relay POST); tested against `asset-core`'s own output (`crates/arbesk/tests/fixtures/arbesk/`, made by `scripts/make-arbesk-fixtures.ts`) |
| `src-tauri/src/share/` | Folder sharing over iroh (received shares are read only): `state.rs` (the Tauri-facing state: one lazily started iroh node, host and guest on one endpoint), `host.rs` and `guest.rs` (the sharer's and the recipient's sides), `manifest.rs`, `proto.rs`, `link.rs`, `identity.rs`, `registry.rs` (which folders are shared, who holds which link), `location.rs` (`share://<received-id>/<name>…`) |
| `src-tauri/src/share/provider.rs` | Received shares as a `places::Provider` in the app (iroh stays here): `ReceivedShares` lists a shared folder and downloads a file by waiting on the app's async runtime with `block_on`, only ever from a blocking thread (D3); no existence or path info, the preview refused with the local-only words, every write refused as read only; `READ_ONLY` and `LOCAL_ONLY` are the words `place.rs` refuses with |
| `src-tauri/src/share/file_share.rs`, `file_link.rs` | Send with QR Code: hashing, the separate iroh endpoint and serving (`file_share`); the ticket, file list and `filebrowser/file/2` wire format (the page still reads version 1 links), compiled into the phone page too (`file_link`) |
| `web/file-get/` | The page a phone opens from the QR code: Rust → WebAssembly (iroh in the browser), `build.sh`, `serve.sh` (test from this computer), published by `.github/workflows/file-page.yml`; `static/get/preview.js` (file types and sizes, the image preview, the rows for several files) is tested in `tests/file-page.test.ts` |
| `src-tauri/src/properties.rs` | Properties dialog data: size, dates, permissions, symlink targets, recursive folder size |
| `src-tauri/src/platform.rs` | The app's platform differences: opening files and links, Open With's application list, window buttons, dragging out (GTK on Linux), DMG and AppImage installs, shell scripts, helper processes; re-exports `places_local::platform` (volumes, `%XX` decoding, `file://` URIs, XDG data dir, `rename_exclusive`) |
| `src-tauri/src/thumbnails.rs` | Thumbnails: the desktop's own on Linux (the shared `~/.cache/thumbnails`, read only, used while its URI and modification time match), Quick Look (`qlmanage`) on macOS, an `image`-crate fallback everywhere; disk cache (decoded photos as small JPEGs, transparent ones as PNG), pruned by age and kept under 100 MB; pictures a remote place hands over are decoded by the `arbesk` crate's own `image.rs` |
| `src-tauri/src/file_icons.rs` | File and folder icons from the OS: Finder's (NSWorkspace) on macOS, the desktop's icon theme (GIO + freedesktop lookup) on Linux |
| `src-tauri/src/updates.rs` | Check for Updates: how this copy was installed (only the AppImage and the Mac app replace themselves), failure kinds, one install at a time; `lib.rs` wraps `tauri-plugin-updater` |
| `src-tauri/src/downloads.rs` | Copies of opened SFTP/Drive/share files and built Arbesk assets in the app cache (one folder per place, `DIRS`): marks them as downloaded (`places::mark`), prunes them after a week (edited copies go to the Trash) |
| `src-tauri/src/progress.rs` | Progress and cancel for long operations as the frontend receives them (at most every 100 ms and once at the end); the counters, cancel flag and phases are `places`' `Progress`, re-exported here |
| `src-tauri/src/recent.rs` | Recent: Spotlight (`mdfind`) on macOS, `recently-used.xbel` on Linux, merged with `recent.json` |
| `src-tauri/src/config_store.rs` | `pins.json` / `starred.json` / `flags.json` in the app config dir, Full Disk Access check |
| `src-tauri/src/versions/` | Keep Versions: `.versions/` ownership, SQLite index, content-addressed zstd objects, capture/watch/restore; `versioned.rs`, `Versioned`, the decorator around this computer's provider (design D7) that the registry holds in its place, a `places::Provider` itself but not a place: Rename inside `VersionsState::transfer`, Move to Trash inside `trash_many`, New Folder, New File, Put Back and Batch Rename passed through untouched (a history in a folder Batch Rename renames isn't moved), a local copy or move inside `transfer` (its session's `within`), and Replace's Trash of any other route landing here (a Drive download, a moved local source of an upload) through `VersionedTrash` |
| `src-tauri/src/lib.rs` | Command registration, commands that need `AppHandle`, startup: the managed `places::Registry` (`providers::registry`), each command one lookup and one call on a blocking thread (`blocking`), the `capabilities` command, and the answers the providers reach the app through (`AppTrash`, Drive's and shares' state) |
| `dev/mock-tauri/` | In-memory fake backend for browser previews and the frontend tests |
| `tests/` | Vitest frontend tests: the real app in jsdom against `dev/mock-tauri/` (`tests/app.ts` boots and drives it) |
| `scripts/crap-gate.mjs` | CRAP gate run by `.githooks/pre-commit` |
| `scripts/check-lockfile.mjs` | Refuses a `package-lock.json` that could install anything but the public npm registry's own tarballs; run by the Release workflow before `npm ci` |
| `.github/dependabot.yml` | Weekly update PRs for npm, both Cargo projects and the actions, with a 7-day cooldown (30 for majors); each one is reviewed like any PR |
| `scripts/session-start.sh` | Where the project stands, from fresh data: main vs GitHub, worktrees, merged branches, drafts, issues |
| `scripts/github.sh` | The scripts' GitHub queries, all through the REST API (`gh api`): cloud sessions can't reach GraphQL |
| `scripts/new-task.sh`, `scripts/end-task.sh` | A worktree per task in `.worktrees/<name>`; finalized (worktree and branches deleted) once its PR is merged |
| `scripts/mock-screenshot.mjs`, `scripts/pr-screenshots.sh` | PR screenshots: capture the mock preview, upload to the `captures` branch |
| `scripts/sign-updater.sh` | Signs the updater packages with minisign in the Release workflow's `sign` job, the only job that gets the key; checked as the updater checks it by `src-tauri/tests/release_signing.rs` |
| `scripts/check-macos-libraries.sh` | Refuses a macOS build that loads a library from outside macOS (0.7.5 loaded the runner's Homebrew liblzma and couldn't open); run by the Release workflow after the macOS build |
| `.github/pull_request_template.md` | The PR body every task follows |
| `HUMANS.md` | The human's side of the workflow: review, merge, test, release |
| `deny.toml` | cargo-deny for both Rust projects: RustSec advisories, licences compatible with GPL-3.0-or-later, crates.io only; run by the Release workflow's `rust-deps` job |

## Commands

```sh
npm ci                      # exactly package-lock.json; see "npm registry" below
npm run tauri dev           # run the app
npm run tauri build         # macOS: bundle/macos/*.app; Linux: bundle/{deb,rpm,appimage}
npx tsc --noEmit            # type-check the frontend
npm run build               # tsc + vite build
npm run dev:mock            # UI in a normal browser with a fake filesystem (port 1420)
cd src-tauri && cargo test  # backend tests
npm test                    # frontend tests (Vitest + jsdom)
npx vitest run tests/<file>.test.ts  # only the frontend tests a change touches
npx vitest                  # the same, in watch mode
npm run crap                # CRAP gate (also runs as the pre-commit hook)
npm run crap:ts             # CRAP gate for TypeScript only: Vitest coverage + fallow
npm run crap:baseline       # accept the current scores into .crap-baseline.json
```

Before finishing a change: `npx tsc --noEmit` and `npm run build` must pass,
plus `npm test` for a frontend change and `cargo test` for a backend one (the
pre-commit hook runs the full suite of each language a commit touches). For UI
changes, also look at the result in `npm run dev:mock`.

### TypeScript-only changes

While a change touches only `src/`, `tests/`, `dev/` or `scripts/`, keep the
loop fast:

- type-check with `npx tsc --noEmit`;
- run the test files it touches, not the whole suite:
  `npx vitest run tests/drive.test.ts tests/listing-failure-table.test.ts`
  (drop `run` to watch them);
- score the gate for TypeScript only with `npm run crap:ts`. That still runs
  every frontend test with coverage, because fallow needs the coverage, but it
  skips the Rust build, tests and coverage; `npm run crap` does both halves.
  The pre-commit hook already picks the languages from the staged files
  (`--staged`), so a TypeScript-only commit never runs Rust.

`cargo test` is not needed when no file under `src-tauri/` changed. The
pre-commit hook runs the whole frontend suite with coverage on every
TypeScript commit, so the PR has that result even though the loop used only
the related files.

**CRAP gate.** `npm install` sets `core.hooksPath` to `.githooks`, whose
`pre-commit` runs `scripts/crap-gate.mjs` for the languages a commit touches.
It fails when a function's CRAP score is above 30 and is new or worse than in
`.crap-baseline.json` (currently empty: nothing is above 30). Both languages
are scored with real coverage, so their tests must pass: Rust through
`cargo llvm-cov` + `cargo-crap` (install with
`cargo install --locked cargo-llvm-cov cargo-crap` and
`rustup component add llvm-tools-preview`), TypeScript through Vitest's
coverage handed to `fallow`. CRAP can never be below a function's cyclomatic
complexity, so a function over 30 needs splitting (lookup tables work well for
key and command dispatch), not just tests. Fix a failure by testing or
simplifying the function; update the baseline only when a score is accepted
on purpose, and say so in the commit message. Don't bypass the hook with
`--no-verify`.

The hook's `--staged` picks the languages itself. To score one language by
hand, `npm run crap:ts` and `node scripts/crap-gate.mjs --rust` skip the
other half.

**Frontend tests** boot the real app (`tests/app.ts`) in jsdom with
`dev/mock-tauri/` as the backend; each test file gets a fresh app and fake
filesystem. `tests/setup.ts` stubs what jsdom lacks (layout sizes,
`ResizeObserver`, `elementFromPoint`, the clipboard, a Mac user agent).
Drive the UI with events (`key`, `mouse`, `goTo`, `respond`) and assert on
the DOM. Code that adds a mock command needs a case in
`tests/mock-core.test.ts`. The mock's `capabilities` serves the frontend's
built-in table (`capabilities.ts`); `tests/capabilities.json` holds the
backend's, which a Rust test, the built-in table's test and the mock's case
all check, so a changed capability is a change to that file too.

**Rust dependencies** are checked against `deny.toml` before every release
(the `rust-deps` job), and a new advisory stops it. When you add or update a
crate, run cargo-deny 0.20.2 (`cargo install --locked cargo-deny@0.20.2`) on
the project you changed:
`cargo deny --locked --manifest-path src-tauri/Cargo.toml --config deny.toml check`.
Fix a finding by updating the crate; ignore an advisory only with the reason
it doesn't apply here, in `deny.toml`, and say so in the PR. A licence that
isn't on the allow-list needs the human's decision (the app is
GPL-3.0-or-later).

## Environment gotchas

- **The Tauri crate needs GTK3 + WebKitGTK 4.1 headers on Linux** (they come with
  the OS; see README). If the host has no root access to install them, build in a
  rootless container instead — the image needs the same packages:
  ```sh
  podman build -t tauri-linux -f dev/tauri-linux.Containerfile   # gtk3-devel, webkit2gtk4.1-devel, libsoup3-devel, rust, cargo
  podman run --rm -v "$PWD:/src:z" -v tauri-cargo:/cargo -w /src/src-tauri \
           tauri-linux cargo test
  ```
  `npm run build` must run first, because `generate_context!` reads `../dist`.
- **Backend modules must still be testable without Tauri.** No crate under
  `src-tauri/crates/` depends on Tauri or on the app, so each one tests
  alone: `cargo test -p places`, `-p places-local`, `-p places-gdrive`,
  `-p places-sftp`, `-p arbesk` (and `cargo test` from `src-tauri` runs the
  app and every crate: they are all in `default-members`). For the app's
  own modules, copy `placement.rs`, `engine.rs`, `config_store.rs`,
  `platform.rs`, `properties.rs`, `thumbnails.rs`, `recent.rs`,
  `downloads.rs`, `file_icons.rs` and `versions/` into a scratch crate with
  `use places_local::{fs_ops, trash_ops};` at its root (as `lib.rs` has) and
  stubs for what they ask of the rest of the app: a `place` module giving
  the `Kind` and `kind` `engine.rs` asks for the way a transfer runs, a
  `progress` module (or `progress.rs` copied: `engine.rs`' tests and
  `versions/state.rs`' use `crate::progress::Progress`), and
  `providers::Reach` plus `providers::testing::NoTrash` for
  `versions/versioned.rs` and its tests (or leave `versioned.rs` out). It depends on `places` and `places-local` (by path,
  `src-tauri/crates/places` and `src-tauri/crates/places-local`, the latter
  also as a dev-dependency with `features = ["testing"]`; `places-gdrive`
  too, as a dev-dependency with `features = ["testing"]`, for the engine's
  tests), `serde`, `serde_json`, `image` (same features as
  `src-tauri/Cargo.toml`), `rusqlite` (`default-features = false`, features
  `bundled` + `backup`), `zstd`, `blake3`, `notify` and `similar` (and `gio`
  on Linux, `objc2`, `objc2-foundation`, `objc2-app-kit` and `xattr` on
  macOS, with the features `src-tauri/Cargo.toml` lists), plus
  dev-dependency `tempfile`; strip the `#[tauri::command]` lines and run
  `cargo test`. Keep new backend logic in plain functions; only `lib.rs` may
  use `AppHandle` and other Tauri types.
- **A provider is its own crate and knows nothing of the app.** Each
  depends on `places` (and its own libraries), never on the app or on
  another provider: what it shares with another place comes from `places`,
  and what it needs of the app is handed in through a trait the app answers
  (`places_local::TrashReach`, `fs_ops::OwnsHistory`, the thumbnailer;
  Drive's `Reach` and `Calls`; shares' `Shares`). What the app asks of a
  place is a method of its `Provider`, its `Transfer` or a `places`
  extension, not a new facade type or a module made public. Per crate:
  - `arbesk`: keep its modules private; the app sees the facade in
    `lib.rs` (`Client`, `SCHEME`, `Location` and the path helpers
    `place.rs` parses with, and a few records).
  - `places-gdrive`: the sign-in window, the `drive_*` commands and the
    managed `CloudState` stay in `lib.rs`. Its copies and moves are the
    app's engine over its `Transfer`, reached through the registry
    (`Provider::transfer`, one `Ctx` for the whole transfer: don't build a
    `Ctx` in `lib.rs`); its `testing` feature (the app's dev-dependencies
    only) gives the engine's tests its fake server.
  - `places-sftp`: the Connect to Server, server list and Disconnect
    commands stay in `lib.rs`, over the `Arc<SftpState>` it manages and
    registers as SSH's provider.
  - `places-local`: the engine (`engine.rs`, `placement.rs`) and Keep
    Versions (its decorator, `versions/versioned.rs`) stay in the app: the
    crate's writes and transfer primitives touch no history.
  - `places`: depends on `serde` and `serde_json` only.
- **Received shares are the app's provider.** `share::provider` runs the
  share's async calls with `block_on` on Tauri's runtime handle, so call it
  only from `blocking(…)`, never from an async command's body, where
  `block_on` panics.
- **The source guards scan every crate.** Only `places-local`'s
  `trash_ops.rs` calls the `trash` crate (checked in its tests); only listed
  modules call `std::fs::rename` (checked in `placement.rs`). Both walk
  `src-tauri/src` and every crate under `src-tauri/crates/` through
  `places_local::test_support::source_roots`; code that moves takes its
  allowed entry with it, in the same PR.
- **A new crate** joins `members` and `default-members` in
  `src-tauri/Cargo.toml`, gets a cargo-deny run and changes `Cargo.lock`
  minimally (below).
- **macOS-only behaviour can't be verified from Linux**: window chrome, traffic
  lights, Quick Look thumbnails, Spotlight results in Recent, where Finder puts
  trashed items and its `.DS_Store` "Put Back" records, Full Disk Access,
  opening files. Say so explicitly when reporting work that touches them. The
  mirror image also holds: Linux-side behaviour (`.trashinfo`, `recently-used.xbel`,
  udisks2 mount paths) is unverified on a Mac.
- **npm registry and the lockfile**: `package-lock.json` is committed and must
  resolve only to `https://registry.npmjs.org/` (an earlier lockfile pinned
  every package to a private corporate registry). The Release workflow runs
  `scripts/check-lockfile.mjs`, which refuses any other host, an entry whose
  tarball isn't its own package's, a missing sha512, or an `.npmrc` that sets
  another registry or credentials, then installs with
  `npm ci --ignore-scripts`. Never add registry URLs or `.npmrc` entries
  pointing at private registries.
  - Install with `npm ci` in the main checkout. **Never run `npm ci` or
    `npm install` inside a worktree**: its `node_modules` is a link to the
    main checkout's, and npm would wipe or rewrite that.
  - Change dependencies in a task (or take Dependabot's PR) and regenerate the
    lock in a scratch copy of the tree, against the public registry and only
    with versions at least a week old:
    `npx -y npm@11.21.1 install --package-lock-only --ignore-scripts --registry=https://registry.npmjs.org/ --min-release-age=7`.
    Copy the lock back, run `node scripts/check-lockfile.mjs`, and check the
    diff touches only what you meant to change.
- `Cargo.lock` *is* committed. When adding a crate, update it minimally (e.g.
  `cargo metadata --format-version 1 >/dev/null` in `src-tauri/`) and check the
  diff only adds entries.

## Mock preview

`npm run dev:mock` aliases `@tauri-apps/api/core`, `/path`, `/webview` and
`@tauri-apps/plugin-opener` to `dev/mock-tauri/` (see `vite.config.ts`).
**Every new Tauri command or plugin call needs a mock** in
`dev/mock-tauri/core.ts` (or a sibling), or the preview breaks.

- Fake home is `/Users/me`; `/big` has 10,000 entries for performance checks;
  `/System` fails with "Operation not permitted".
- `?demo` in the URL hides test-only folders (used for README screenshots).
- `window.__mockDrop(paths, x, y)` simulates files dropped from Finder.
- Playwright works for screenshots and scripted checks; Chromium is at
  `/opt/pw-browsers` in the cloud environment. Use a Mac user agent so the app
  takes its macOS code paths.

## Rules the code must keep

**Never lose data.** This is the project's main anti-goal.
- Deleting always goes to the Trash (`trash` crate), never `remove_*`. The only
  exceptions are Delete Permanently and Empty Trash in `places-local`'s `trash_ops.rs`: they act
  only inside a Trash folder (refused elsewhere, with tests) and only after a
  confirmation dialog whose default response is Cancel.
- Pruning the app cache deletes only what the app made and nobody changed
  (thumbnails, opened version copies, unchanged downloads); a downloaded copy
  that was edited goes to the Trash (`downloads.rs`).
- Restoring never overwrites: an existing item at the original path is an error.
- Anything that touches `.versions/` must prove ownership (the `meta.json` + DB
  `meta` marker, D14) before registering, clearing or trashing it. Object files
  are content-addressed: a full frame and a delta frame of the same bytes share
  a name, so the `objects` row decides the encoding and a referenced object is
  never rewritten.
- Anything that touches the Trash must work for both layouts (macOS `~/.Trash`
  and freedesktop `Trash/{files,info}`); a change there needs a test on each.
  Commands ask `trash_ops::Trash` (`send`, `untrash`, `record`, `info`,
  `restore`, `delete_permanently`, `empty`, `listing_root`, `contains`)
  rather than reading `trash_log.json` or building Trash paths in `lib.rs`.
  Anything the app trashes outside Move to Trash (Replace's old item, a
  Drive move's source, a DMG install's old app, `.versions/`, an edited
  download, a replaced version) goes through `Trash::send` (`send_to_trash`
  in `lib.rs`, or this computer's transfer session, `places_local::LoggedTrash`;
  modules take it as a parameter), which logs
  where it came from. Only `places-local`'s `trash_ops.rs` calls the `trash` crate; a test
  fails on any other caller.
- Replacing an existing item stages the new one under a temporary name, sends
  the old one to the Trash, then renames into place. Local copy and move
  (`engine::local`, over this computer's transfer session) and Drive
  downloads do this only through `placement.rs` (`Target::place`), and
  Drive's uploads, copies and moves only through `engine.rs` (`swap_in`,
  over Drive's primitives); don't build the sequence again at a call site
  or in a provider (`places-local`'s `transfer.rs` gives primitives only).
- A provider says how safe its primitives are instead of pretending:
  `Capabilities` declares whether its rename refuses a taken name atomically
  (`rename_atomic`: on disk yes, on Drive check-then-act), whether names are
  unique in a folder (`names_unique`: not on Drive, where the engine refuses
  an ambiguous name) and whether a written file appears only whole
  (`put_is_whole`: a Drive upload; elsewhere a new file is staged); the
  engine decides from `names_unique` and `put_is_whole`. A staged **original** (the user's own item,
  moved under a staging name during a move that replaces) is only ever
  moved back, never discarded or trashed; `Transfer::discard` takes only
  the engine's own staged or partial copies.
- Placing renames with `rename_new` (`places_local::rename`, re-exported
  by `placement`), never `std::fs::rename`:
  on Unix that replaces a file that took the name after it was checked free.
  Rename and Batch Rename use `places_local::rename::rename_in_place`, which also
  changes only the case on a disk that ignores it. A test in `placement.rs`
  lists the modules that may call `std::fs::rename` (`rename_new`'s checked
  fallback in `places-local`'s `rename.rs`, the app's own saves and caches).
- A new file a copy makes is staged too and appears only whole; a new
  folder fills in place (`Stage::by_kind`, the engine's; this computer's
  `read` fills whatever path it is given) and counts as a partial copy to
  clean up only once the fill has created it, so a failure never removes
  what took the name after the check.
- A move that replaces nothing is one `rename_new` straight onto the target
  (`Target::free_path`, then this computer's `move_to` in `engine::local`): the user's own item never waits under a staging
  name, where a crash would leave it hidden. Only its copy across volumes
  is staged, like any copy.
- If Replace's last rename fails once the old item is in the Trash, the old
  item comes back out (`placement::ReplaceTrash::take_back`; the app's is
  `Trash::untrash`, which also forgets its log entry; as Drive untrashes)
  unless something took its place; then it stays in the Trash, logged, and
  the error says where. Histories the Trash step closed come back with it:
  a Drive download's Replace goes through `versions::state::VersionedTrash`
  (this computer's transfer session, Keep Versions' decorator, outside a
  local route), which reopens and registers them again; a local copy or
  move runs inside `VersionsState::transfer`, which closed them for the
  whole route.
- A cross-volume move deletes the source only after the copy fully
  succeeded (`engine::local`, `remove_any`, never through a provider's
  `discard`).
- Copy/move refuse: vanished source, same path, destination inside the source,
  existing target without `overwrite`, replacing a folder with its own child,
  a target whose last component is empty, `.` or `..`. Their wording lives in
  `placement::Refusal`, and the mock's `checkTransfer` makes the same ones in
  the same order. Add a test for every new refusal.
- Name clashes always ask (Replace / Skip / Keep Both); Enter picks Keep Both.

**Stay fast with huge folders.**
- Views are virtualized: items are absolutely positioned slots computed by
  `computeLayout` / `itemPos`, and only `renderWindow`'s range is in the DOM.
  Never render all of `shown`, never measure items with `getBoundingClientRect`
  for hit-testing (rubber band and drop targets use layout maths), and use
  `scrollToIndex` instead of `scrollIntoView`.
- Folder item counts and thumbnails are fetched only for rendered rows.
- Commands that touch many files or wait on other programs are `async fn`
  commands in `lib.rs` that run their work through `blocking(…)` (Tauri's
  blocking-thread pool). Don't use `#[tauri::command(async)]` on a plain
  function: that runs it on the few shared async workers, and a slow copy or a
  queue of thumbnails then stalls every other command.
- The thumbnail cache in `src/thumbnails.ts` is capped by bytes (64 MB), not
  just by count: photo thumbnails are ~120 KB at 256 px and ~450 KB at 512 px.

**Look like the theme, behave like GNOME Files 50.**
- Two themes, picked by the platform, with no choice in the app: the GNOME
  theme follows GNOME Files 50 on Linux, the macOS theme follows macOS 26
  Tahoe on a Mac. Only the look differs: wording, behaviour, shortcuts, dialog
  button order and default responses are GNOME's in both.
- Use the tokens (`--accent-bg`, `--view-bg`, `--hover`, `--radius-button`,
  …), not hard-coded colours. A new look-related value is a token with a value
  in each theme (`src/themes/`), and every colour needs a dark-mode value.
  Sizes the views compute in JavaScript go in `LAYOUT_SIZES` (`src/theme.ts`).
- The macOS theme is an original imitation: no Apple fonts, icons or other
  assets (the system font is used through `-apple-system`). File and folder
  icons the OS draws at run time (`file_icons.rs`) are not assets: nothing is
  copied into the repository or the bundle.
- Use GNOME's wording for labels, menus and messages ("Folder is Empty",
  "Move to Trash", "A folder with that name already exists.").
- Widgets: reuse `openMenu`, `openPopover`, `openDialog`, `showToast` from
  `ui.ts` rather than new ad-hoc overlays. A dialog whose response waits on
  the backend uses `openDialog`'s `busy` option (return the promise from
  `onResponse`, or `dialog.run()` for work started elsewhere) rather than
  its own busy flag: it ignores a second Enter, disables the rest except
  `liveWhileBusy`, and puts back what validation wants.
- Shortcuts accept ⌘ on macOS and Ctrl elsewhere (`isMac ? metaKey : ctrlKey`).

**Security.** Escape every file name or path inserted with `innerHTML` via
`escapeHtml`. Icon markup comes from fixed templates only.

## Conventions

- Match the surrounding code: section banners (`// ----`), short doc comments
  on non-obvious functions, no framework, no new runtime dependencies without a
  good reason.
- Persist small UI preferences in `prefs` (localStorage, wrapped in try/catch);
  persist durable settings through `config_store.rs`.
- Ask `src/locations.ts` (or `isRemotePath`) what a location allows rather
  than combining `isCloud`, `isSharePath`, `isSftpPath` and the Trash checks
  at the call site; a new kind of location or rule is a row in
  `tests/locations.test.ts`. What a place itself allows is its provider's
  `capabilities` (`capabilities.ts`, through `capabilityRow`), not a new
  per-kind list: a rule a capability answers reads it, and only the
  frontend's own views (Recent, Starred, Network, the Trash) and rules no
  capability matches keep cells of their own. Code that picks something per kind of place (a
  menu, a rule) is a `Record<LocationKind, …>`, so a new kind can't be
  forgotten.
- An action that menus, shortcuts, the preview, the row star or a drag can
  all reach asks `src/item-actions.ts` inside the action function itself
  (`refuse(action, hereFor(paths))` in `main.ts`), not at each surface; a
  changed rule is a cell in its table and in `tests/item-actions.test.ts`.
  Menus keep their own per-place layouts and may leave an allowed action
  out. `tests/item-actions-matrix.test.ts` pins what every place offers
  through every surface: update it when that changes on purpose.
- Ask `src/paths.ts` how a path breaks up (`pathName`, `parentPath`,
  `childPath`, `pathSegments`) and what holds it, rather than slicing a
  scheme's prefix or writing `p === dir || p.startsWith(dir + "/")` at the
  call site; a new scheme is a row in its `SCHEMES` table and in
  `tests/paths.test.ts`. Containment has two helpers that differ only when
  `dir` is `/` or ends in a slash, so don't swap one for the other:
  `isUnder` ignores a trailing slash on `dir`, and `/` holds everything (a
  versioned folder, a folder that received items); `isWithin` takes `dir`
  as written, so `/` holds only itself (Home, which is `/` when HOME=/,
  the Trash, volumes).
  Never rewrite a path the backend produced: stars, pins and history store
  them as they are, and only `src/path-refs.ts` changes them, to follow a move.
  The one exception is Home: `homeFolder` trims `$HOME`'s trailing slash once
  at start-up, keeping `/` for HOME=/ (#170).
- Code that moves, renames or trashes items reports it to `src/path-refs.ts`
  (`pathRefs.moved(pairs)` with the paths the backend reported, as soon as
  it moved them and only for what really moved; `pathRefs.gone(paths)` after a Move
  to Trash) instead of updating stars, bookmarks, the clipboard or tabs by
  hand. Something new that remembers paths is a holder there, with its rule
  tested in `tests/path-refs.test.ts`.
- In `lib.rs`, a command asks the place's provider in the managed
  `places::Registry` (`provider_for`, `answer_each` for a batch), never a
  `match` on the kind: the reads (listing, existence, path info,
  thumbnails, opening, the preview's file), this computer's writes (New
  Folder, New File, Rename, Batch Rename, Move to Trash, Put Back) and every
  copy and move (the provider's transfer session, `places::with_transfer`,
  which `lib.rs` opens and hands to `engine.rs` for a local or Drive route,
  or uses itself for Arbesk: `Transfer::write` for a local model file into
  a collection, `Transfer::export` to build an asset before the engine
  copies it out). What a provider can't do is its own answer
  (`Error::Unsupported` or `ReadOnly`, in its words), not a fallthrough. A
  command that belongs to one place whatever path it gets (the `arbesk_*`
  commands) takes that place's provider by its scheme
  (`Registry::provider`, `arbesk_place`/`arbesk_ext`), so its parser still
  answers every path. No command looks at `provider.scheme()`: what
  differs is the provider's own answer (`open_target` asks
  `fetch_reports_progress`, false only for this computer, whose file opens
  without registering a progress operation, then fetches from the same
  provider: one lookup on the blocking thread). Still outside the registry,
  each calling its module through `place.rs`' gate: Drive's own writes
  (`places_gdrive::write`), Properties, folder sizes, item counts, search
  and measure (their extensions are empty slots), Delete Permanently and
  Empty Trash (`trash_ops::Trash`). Moving one of them is a change of its
  own, with its extension's methods shaped like the command.
- A copy or move never builds its sequence at the call site or in a
  provider: the engine (`engine.rs`) runs it from the two ends' `Transfer`
  primitives and reads how they behave from `Capabilities`
  (`names_unique`, `put_is_whole`); which pairs may transfer is `place.rs`'
  route table, not the providers'.
- In `lib.rs`, ask `place.rs` which kind a path is and whether it may be
  written, rather than chaining `is_cloud`, `is_share` and `is_sftp`: a
  command that works on this computer only takes its path from
  `place::local` (or `local_write`), a write command checks
  `place::writable` first (copy, move and rename: `place::transfer`), and a
  batch with nothing to say about remote paths answers through
  `place::local_only`; give a new command the gate too. Keep each
  command's local arm a single call into its module; a new kind or refusal
  is a row in `place.rs`'s table test.
- Commit messages: conventional style (`feat:`, `fix:`, `docs:`, `chore:`),
  imperative subject, a body explaining what and why.
- Keep the README's feature list and "Not supported yet" section in sync with
  behaviour changes.

## Adding a provider

A new place (another cloud, SMB, WebDAV) is a provider. Opening a copy route
or offering a new action is a visible change for the human to decide; the
list below only makes the place exist. Each step is a table row the
compiler or a test points at:

1. **The crate**: `src-tauri/crates/places-<name>`, depending on `places`
   and never on the app or Tauri. Its `places::Provider` answers every
   method: what it can't do is `Error::Unsupported` or `Error::ReadOnly` in
   its own words (a read-only place uses `Capabilities::read_only_place` and
   the `read_only_place!` macro). It gives a `Transfer` only if it can be
   copied to or from, and fills only the extensions it has. It never sets
   `Entry.cloud` (the frontend reads that key as Drive's). Add it to
   `members` and `default-members` in `src-tauri/Cargo.toml`, run
   cargo-deny, keep the `Cargo.lock` diff minimal.
2. **The registry line** in `providers::registry` (`.register(…)`), and a
   row in `providers::KINDS` with a path of the new kind, so the
   `capabilities` command answers for it; then its row in
   `tests/capabilities.json` and in `capabilities.ts`
   (`CapabilityRow`, `BUILT_IN_CAPABILITIES`).
3. **The gate**: a `Kind` in `place.rs` (`kind`, and `local`'s and
   `writable`'s words), with rows in its table test. The route table
   (`transfer`, `copy`) refuses the new kind until a route is opened on
   purpose. `lib.rs`' `properties_refusal` and `folder_size_refusal` match
   on `Kind` exhaustively and need their own words for it, and
   `providers.rs`' tests map each `Kind` to its scheme (`scheme_of`).
4. **The frontend's tables**: a `Place` and a row in `paths.ts`' `SCHEMES`
   (and `tests/paths.test.ts`), a `LocationKind` with its `capabilityRow`
   in `locations.ts`, and a cell in each `Record<Place, …>` and
   `Record<LocationKind, …>` table (`locations.ts`, `item-actions.ts`,
   `main.ts`'s menus); a row in `listing-failure.ts` or `item-icons.ts`
   only if it fails or looks unlike the others.
5. **If it signs in**: an `Accounts` extension, its commands in `lib.rs`
   and `api.ts`, a row in `providers.ts`' `PROVIDERS` and in
   `PROVIDER_ACTIONS` (`main.ts`).
6. **The mock**: its commands in `dev/mock-tauri/core.ts`, each with a case
   in `tests/mock-core.test.ts`.
7. **If opening downloads into the app cache**: its folder in
   `downloads::DIRS`, and every file it leaves there marked
   (`places::mark`).
8. **The docs**: its Layout row here, `docs/architecture.md` and its
   diagram, and the README's feature list.

## Life of a task

**Nothing lands on `main` that the human didn't merge.** Every change, docs
and release bumps included, goes through a pull request. `main` is protected
on GitHub (a PR is required, admins included; no force push or deletion), so
a direct push is refused. Agents never merge unless the human says so in the
conversation; GitHub can't enforce that part, since agents use the human's
`gh` account.

0. **Start from fresh data.** At the start of a session, and before answering
   any question about the project's state (what's left, issues, PRs, what
   was merged, what a release contains), run `scripts/session-start.sh`.
   Never answer from memory or from a checkout that may be behind; if the
   fetch fails, say the data may be stale. Act on what it reports: finalize
   merged tasks (step 8), mention open PRs, drafts and branches that can go.
1. **Work in a worktree of your own**, never in the main checkout: it may
   hold the human's uncommitted work. `scripts/new-task.sh <kind> <name>`
   (`feat`, `fix`, `perf`, `refactor`, `docs`, `chore`) creates
   `.worktrees/<name>` on branch `<kind>/<name>` from `origin/main`, with
   `node_modules` linked. Look for an open issue that covers the task.
2. **Test first**, then make the change in small commits, then run the
   checks under [Commands](#commands). For a bug, reproduce it before fixing
   and say how.
3. **Test the real thing, not only the mock.** Mock and unit tests don't run
   the packaged app, its environment or the host's programs (the 0.5.1
   AppImage couldn't open files: host `gio` loaded its bundled GLib). When a
   change touches packaging, launching other programs, the filesystem or the
   platform, run the real app (`npm run tauri dev`, or the built bundle) or
   ask the human to, saying exactly what to try and what they should see.
4. **Take screenshots** of anything visible: before (from `main`) and after,
   with `npm run dev:mock` running: `node scripts/mock-screenshot.mjs <path>
   <file.png>` (`--dark` for dark mode, `--theme gnome|macos`, `--scene
   grid|list|menu|dialog`; the clock is fixed, so `cmp` can prove two
   captures identical). Look at them before using them.
5. **Open the PR** ([Pull requests](#pull-requests)): push the branch,
   upload the screenshots, fill in the template.
6. **Review your own PR** and post the review as a comment, then hand over
   to the human: the link, the verdict, what to look at or test. Stop there.
7. **The human reviews and merges.** If they ask for changes, commit them on
   the same branch, push, and update the review comment.
8. **Finalize after the merge**, from the main checkout:
   `scripts/end-task.sh <name>`. It checks the work is on `origin/main` (or
   in a merged PR, for a squash merge), deletes the worktree, the local
   branch and the branch on GitHub, and fast-forwards the main checkout if
   it is clean. Then reply on the issue if the merge didn't close it, and
   tell the human it's done.

**Record what isn't verified.** Work that can't be checked here (macOS from
Linux and the reverse, a real launch, real network shares) is merged as
done-but-unverified: the PR's "Not verified" section says so, and an open
issue labelled `needs-verification` keeps a checklist line per point, so it
isn't lost in a chat. Remove the label when every point is checked.

Run `gh`, `git` and the scripts as plain commands, without `cd …&&`, `;` or
variables around them: permission rules match whole commands.

## Pull requests

- **Title:** conventional and short, like a commit subject
  (`fix: open files from the Linux AppImage`).
- **Body:** follow `.github/pull_request_template.md`, readable in two
  minutes. Write it to a file outside the repository (the scratchpad) and
  run `gh pr create --base main --title "…" --body-file <file>`.
  In a cloud session, where `gh pr …` (GraphQL) fails, use REST:
  `gh api 'repos/{owner}/{repo}/pulls' -f base=main -f head=<branch> -f title="…" -F body=@<file>`.
- **Issues:** one line each at the top: `Closes #<n>` when the PR resolves
  all of it, `Refs #<n>` when a part or a verification stays open, `No
  issue` otherwise.
- **Screenshots:** `scripts/pr-screenshots.sh before.png after.png` stores
  them on the `captures` branch (never `main`), in a folder named after the
  task branch, and prints the Markdown to paste. Light and dark when colours
  change; "No visible change" otherwise.
- **How it was checked:** tick only what really ran, with the test counts.
  "Real app" names what was tried and by whom, or why it wasn't needed.
- **Self-review comment**, posted with `gh pr comment <n> --body-file <file>`
  (or `gh api 'repos/{owner}/{repo}/issues/<n>/comments' -F body=@<file>`)
  once the PR is open:

  ```markdown
  ## Review

  **Verdict: ✅ ready to merge** (or ⚠️ needs a decision / ❌ blocked)

  | Check | Result |
  |---|---|
  | Mergeable with main, no conflict | ✅ |
  | Checks run locally (tsc, build, npm test, cargo test, CRAP gate) | ✅ N + M passed |
  | Mock preview and screenshots inspected | ✅ / no UI change |
  | Real app, where packaging or other programs are involved | ✅ … / not needed: … |
  | Rules of this file (data safety, huge folders, GNOME look, security) | ✅ |
  | Docs (README, AGENTS.md, HUMANS.md) in sync | ✅ |

  ### Findings
  - Fixed: …
  - For the human to decide: … (options A / B)
  ```

  Fix what you find before posting; anything you can't settle is ⚠️ with
  options, never improvised.

## Releases

Release only when the human asks, and only after something significant.

1. `scripts/session-start.sh`: `main` is what you mean to ship, no open PR
   or worktree holds work meant for it, no draft from an earlier attempt
   lingers.
2. In a `chore` task (`release-X.Y.Z`), bump the version in `package.json`,
   `package-lock.json` (its two root entries), `src-tauri/Cargo.toml`,
   `src-tauri/Cargo.lock` (the `filebrowser` package only),
   `src-tauri/tauri.conf.json` and `docs/linux-appimage.md`, and open the PR
   `chore: release X.Y.Z` listing what's new. The human merges it.
3. Tag `vX.Y.Z` (annotated) on the merged commit of `origin/main` and push
   the tag. The Release workflow
   checks the tag matches the version, runs the tests, builds macOS and Linux
   and creates a **draft** release.
4. **Smoke-test the draft before publishing.** Download its package for this
   machine (verify the size and SHA-256 against the release's digest), install
   it the way users do (`scripts/install-linux-appimage.sh` for the
   AppImage), quit any running copy, and in the installed app: open a folder,
   open a file with its default app and with Open With, search, and show
   thumbnails of a photo folder. Ask the human to do the same on the other
   platform, or note it on a `needs-verification` issue.
5. Publish (`gh release edit vX.Y.Z --draft=false`) only when the human says
   so. The release must contain `latest.json`; without it Check for Updates
   reports "Could not check for updates" to everyone. A draft can be fixed
   by deleting it and moving the tag to the fix; a **published** tag never
   moves: ship a new patch version instead.
6. **Right after publishing, check the update path** (the updater only sees
   published releases): in the previously installed version, Check for
   Updates must offer the new one, install it and restart into it. Ask the
   human to do the same on the other platform. If it fails, the next patch
   release has to fix it, so say so at once.

**The updater signing key is the human's.** Its private half and password
live only in the human's backup and in the `TAURI_SIGNING_PRIVATE_KEY` /
`TAURI_SIGNING_PRIVATE_KEY_PASSWORD` secrets. Agents never generate, read,
print, copy or replace it; the public key in `tauri.conf.json` changes only
when the human says so (installed copies would reject every update signed
with a new key). Only the Release workflow's `sign` job may use the secrets:
never pass them to a job or step that installs or builds anything (npm
packages and crates' build scripts run there; one wrote the whole
environment into `target/`, RUSTSEC-2026-0418). Agents never approve the
`release` environment's deployment, through the web or the API, even though
they use the human's account: the human approves each release.

## Licensing

The project is GPL-3.0-or-later (`LICENSE`). GNOME Files is the design
reference only:
- Don't copy code, icons or other assets from Nautilus, libadwaita or the
  Adwaita icon theme. Draw icons from scratch in the Adwaita style. Showing
  the icons the user's own OS provides at run time (Finder's, the installed
  icon theme) is fine: they are read from the system, never bundled.
- Bundled third-party assets need their license next to them (see
  `src/assets/fonts/AdwaitaSans-LICENSE.txt`) and a line in the README credits.
- Keep the README's independence notice and trademark section intact. The
  project must never suggest it is affiliated with or endorsed by GNOME, Apple
  or anyone else.
