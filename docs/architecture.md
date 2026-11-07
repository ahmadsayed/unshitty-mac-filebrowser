# Architecture

How the app is put together after the deep-module refactors (#136–#183):
which module owns which rule, how a call travels from the window to the
filesystem, and where a change lands. The module list itself, with one line
per file, is the Layout table in [AGENTS.md](../AGENTS.md).

![Block diagram of the app: the frontend's main.ts asks four groups of rule and state modules and calls typed commands in api.ts; Tauri IPC carries them to lib.rs, where every path goes through the place.rs gate before reaching placement.rs, trash_ops::Trash, fs_ops, Drive, shares or SSH; placement uses the Trash for Replace and platform.rs for renames that never replace.](architecture.svg)

Teal boxes are the modules the refactors added or deepened, with their PR
numbers. Dashed boxes are outside the app (the operating system) or stand in
for it in tests (`dev/mock-tauri`).

## How a call travels

**In the window.** `main.ts` handles the DOM, events, views, menus and keys.
It owns no rules: it asks small modules that have no DOM.

- **Where am I, what may run.** `paths.ts` breaks a path into its kind
  (`placeOf`: local, Drive, share, SSH), name, parent and crumbs.
  `locations.ts` turns a path into a `LocationKind` (Recent, Starred,
  Network, the Trash, a folder in it, a share, an SSH server, Drive, a
  folder) and says what that kind of place allows. `item-actions.ts` holds
  one rule per action that several surfaces reach (menus, keys, the preview,
  the row star), asked inside the action itself. `preview-actions.ts` builds
  the Space preview's buttons from those rules.
- **View state and stored paths.** `selection.ts` (selection, cursor,
  anchor), `search.ts` (`SearchSession`: the walk, its pause, its limit,
  late answers), `folder-summaries.ts` (item counts and version badges),
  `listing-failure.ts` (what a folder that can't be listed turns into) and
  `path-refs.ts` (stars, bookmarks, the clipboard and tab history follow a
  moved or renamed item).
- **Pictures.** `item-icons.ts` decides which picture an item shows and when
  it is asked again; `itemIcon` in `icons.ts` composes the picture with its
  emblems. `native-icons.ts` and `thumbnails.ts` are the caches behind them.
- **Widgets.** `ui.ts`: menus, popovers, toasts, and dialogs whose busy mode
  runs one action at a time while the backend works.

**Across the boundary.** `api.ts` has one typed wrapper per Tauri command.
In the tests, `dev/mock-tauri` answers the same commands from an in-memory
filesystem.

**In the backend.** `lib.rs` registers thin commands. Every path a command
receives goes through `place.rs` first: its kind, whether it is a local path
(remote, malformed and relative paths never reach `std::fs`), and whether
the place may be written. Then:

- copies, moves and Drive downloads go through `placement.rs`: the refusals
  and their wording, staging under a hidden name, and Replace;
- Move to Trash, Restore, Delete Permanently and Empty Trash go through
  `trash_ops::Trash`: the trash log, where an item came from (sidecar, log,
  Finder's Put Back), and both Trash layouts;
- listing, counts, opening and reading go to `fs_ops.rs`, `cloud/` (Google
  Drive), `share/` (iroh shares) and `sftp.rs` (SSH, read only).

Replace sends the old item to the Trash through `trash_ops::Trash` and, if
the last rename fails, takes it back out and forgets its log entry. Every
final rename goes through `platform.rs`'s `rename_exclusive`
(`renameat2` on Linux, `renamex_np` on macOS), so it never replaces an item
that took the name meanwhile. `platform.rs` holds the other differences
between macOS and Linux too: XDG paths and volumes.

## Where a change lands

| Change | Lands in |
| --- | --- |
| Allow or refuse an action in a kind of place | one cell in `item-actions.ts` |
| A new kind of location (SMB, WebDAV) | a row in `paths.ts`, a kind in `locations.ts`, a `Kind` in `place.rs` |
| How copy, move or Replace behaves | `placement.rs` |
| The trash log, Put Back, Restore, Empty Trash | `trash_ops::Trash` |
| Something new that remembers paths | one holder in `path-refs.ts` |
| What a folder that can't be listed shows | a row in `listing-failure.ts` |
| Folder counts and version badges | `folder-summaries.ts` |
| Search timing, limits and sections | `search.ts` |
| Which picture or emblem an item shows | `item-icons.ts`, and `itemIcon` in `icons.ts` |
| A dialog that waits on the backend | `openDialog`'s `busy` option in `ui.ts` |

When a module here changes what it owns, update this page and its diagram
(`docs/architecture.svg`) in the same PR.
