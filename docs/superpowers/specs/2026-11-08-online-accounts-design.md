# Online Accounts — Design

**Date:** 2026-11-08
**Status:** implemented 2026-11-08 (branch `feat/online-accounts`); the
"As built" section at the end lists where the code differs from this design
**Tracking:** no issue yet; PR #214 (the Add… menu) ends with the note this
spec answers
**Related:** `2026-10-24-google-drive-design.md` (D3, D15: the one account and
the Add… menu this replaces), `2026-10-23-file-browser-design.md` (the sidebar
and Network), `AGENTS.md` (rules every change must keep)

## Intent and the why

Network's bar has an **Add…** button whose menu holds **Connect to Server…**
and, under an "Online Accounts" heading, **Google Drive…**. The user's words:
*"the current approach of selecting the google drive via custom button is not
that UX elegant, specially if we want to add multiple options in the future."*

Three directions were mocked up
(https://claude.ai/artifact/9jWeJrVqSU5eqz1rLy87TD): the Network list with
Connected and Available sections as in GNOME Files 47+, an Online Accounts
dialog as in GNOME Settings, and a single smart entry. The user chose the
dialog: it reuses a real GNOME pattern (two boxed lists, "Your Accounts" and
"Connect an Account", action rows with chevrons), it keeps account setup out
of the file listing, where GNOME never puts it, and it gives the whole
account lifecycle (connect, expired sign-in, change client, disconnect, a
second account) one home.

Success looks like: the bar is Nautilus's three controls (address entry,
**Connect**, a round info button) with no provider-specific chrome; **Online
Accounts…** is one click away from the primary menu, Network's menus and
Network's empty state; adding Google Drive, and later OneDrive or a second
Google account, happens in that dialog; and a developer adds a provider by
adding one row to a table plus that provider's own sign-in flow, with no new
button, menu item or `if (driveAccount)` anywhere.

## Current state (main at 48ed6e4)

- `networkEntries()` (`src/main.ts`) lists received shares, SSH servers and,
  from the single `driveAccount`, one "Google Drive" row.
- `renderNetworkPaste()` builds the bar: entry ("Server or share link"),
  **Connect**, **Add…** whose menu is built inline and hides **Google
  Drive…** while an account is connected.
- Google Drive's dialogs: `openClientSetup` ("Add Your Google Client"),
  `openConnectDialog` ("Connect to Google Drive" / "Sign In to Google
  Drive"), `confirmDisconnectDrive` ("Disconnect Google Drive?"). A listing
  that fails for lack of a credential goes through `src/drive-auth.ts` and
  `src/listing-failure.ts` to `promptDriveSignIn`.
- **Connect to Server…** is in the primary menu and Network's background and
  location menus.
- The backend (`src-tauri/src/cloud/accounts.rs`) already stores a list and
  `upsert`s by account id; only the frontend keeps one account.

## Goals

1. **One generic surface.** An **Online Accounts** dialog lists connected
   accounts and the providers that can be connected, built entirely from a
   provider table.
2. **GNOME's shape and words.** The dialog follows GNOME Settings → Online
   Accounts; Network's bar follows GNOME Files 47+'s network address bar.
3. **The lifecycle in one place.** Expired sign-in, change client and
   disconnect are reached from the account's row.
4. **Several accounts per provider.** The frontend holds every account the
   backend reports; Network lists each one.
5. **Nothing else changes.** Browsing Drive, the `gdrive://` paths, the
   location rules, the sign-in prompt on a failed listing, Connect to Server
   and pasted share links all behave as today.

## Non-goals

- New providers. OneDrive, Dropbox, WebDAV and SMB are what the table is
  for; none is implemented here.
- A backend change. Multi-account needs none (`upsert` by id); the
  single-account non-goal of the Drive spec's v1 is lifted on the frontend
  only.
- Changing the Google client flow ("Add Your Google Client", the guide).
- Shared Drives, "Shared with me", Drive search (the Drive spec's M4).

## Design

### 1. The provider table (`src/providers.ts`)

A new module, without the DOM, that says what providers exist and what each
one contributes. One row per provider, keyed so a forgotten provider is a
type error:

```ts
export type ProviderId = "gdrive";

export interface Account {
  provider: ProviderId;
  id: string;          // the backend's account id (gdrive: DriveAccount.id)
  label: string;       // the row's subtitle: the email
  rootPath: string;    // Network's entry: gdrive://<id>/…
}

export interface Provider {
  label: string;       // "Google Drive"
  icon: string;        // a symbol id from src/icons.ts, drawn by the app
  /** Start connecting a new account (the provider's own dialogs). */
  connect(): void;
  /** Sign the account in again (expired or revoked token). */
  signIn(account: Account): void;
  /** Ask, then disconnect; resolves once Network no longer lists it. */
  disconnect(account: Account): void;
  /** Extra links on the account dialog ("Change Client…"); may be empty. */
  links(account: Account): { label: string; action: () => void }[];
}

export const PROVIDERS: Record<ProviderId, Provider>;
export const PROVIDER_ORDER: ProviderId[];  // the dialog's order
```

`main.ts` keeps the accounts (`accounts: Account[]`, replacing
`driveAccount`) and gives `providers.ts` the callbacks it needs the same way
`item-actions.ts` and `locations.ts` are wired today: the table's functions
are the existing `connectDrive`, `openConnectDialog`, `confirmDisconnectDrive`
and the client-change link, moved behind the row. Nothing in `main.ts` may
test `provider === "gdrive"`; what a provider does lives in its row.

A new provider is: a `ProviderId`, a row in `PROVIDERS`, its scheme in
`src/paths.ts`'s `SCHEMES` and its kind in `src/locations.ts` (both already
rows), its own dialogs, and a row in `tests/providers.test.ts`.

### 2. The Online Accounts dialog

Opened by `showOnlineAccounts()` with `openDialog` from `src/ui.ts`
(heading, `extra` DOM, no responses but **Close**; Escape closes). Layout,
top to bottom, in GNOME's words:

- Heading: **Online Accounts**.
- Body: "Connect an account to browse its files in Network."
- Group **Your Accounts** (hidden when there is none): a boxed list, one
  action row per account: the provider's icon, the provider's label as
  title, the email as subtitle, a chevron. A row whose token the app knows
  is expired (see §4) shows "Sign in again" in the destructive colour with
  the error icon before the chevron, as Settings shows "Error".
  Activating a row opens the account dialog (§3).
- Group **Connect an Account**: a boxed list, one action row per provider
  in `PROVIDER_ORDER`: icon, label, chevron. Activating a row closes this
  dialog and calls the provider's `connect()`; when that flow ends with an
  account connected, the app navigates to its root as it does today.
  A provider stays listed while it has accounts: that is how a second
  account is added.

Rows are real `<button>`s inside the list: Tab moves between them, Enter and
Space activate, and the dialog's first row has focus on open. The dialog is
460 px wide at most and shrinks with the window (it must fit a 700 px
window). It is never busy itself: connecting and disconnecting happen in the
provider's own dialogs, which already use `openDialog`'s busy mode. The
dialog is rebuilt from `accounts` whenever it is opened; while open, a
disconnect or a new sign-in closes it first, so it never shows stale rows.

### 3. The account dialog

Activating an account row opens a small dialog, also with `openDialog`:

- Heading: the provider's label ("Google Drive"). Body: the email, then the
  state: "Connected" or, when known expired, "The sign-in expired or was
  revoked. Sign in again to keep browsing its files."
- Links (`dialogLinks`): the provider's `links(account)`; for Google Drive,
  **Change Client…** as the connect dialog offers today, and **Open in
  Network**, which navigates to the account's root.
- Responses, in GNOME's order: **Close**; **Sign In Again** (suggested,
  present only when the sign-in is known expired; otherwise the row's main
  action is Close); **Disconnect…** (destructive). Default response Close;
  Escape closes.

**Sign In Again** opens the provider's existing sign-in dialog for that
account (`openConnectDialog(account)`), **Disconnect…** the existing
confirmation ("Disconnect Google Drive?", default Cancel), unchanged.

### 4. Knowing an account needs a sign-in

The app learns a token is dead only when a request fails. Today
`listing-failure.ts` turns such a failure into the sign-in prompt. This
design adds one piece of state: `main.ts` keeps the set of account ids whose
last failure was a `sign-in` or `client` need (cleared when the account
signs in again or is disconnected). The dialog and the account dialog read
it. No polling, no new backend call: an account that expired while the app
never touched it shows "Connected" until the next request, which is what
Settings does too.

### 5. Entry points

- **Primary menu:** **Online Accounts…** directly after **Connect to
  Server…**.
- **Network's background and location menus:** **Connect to Server…**, then
  **Online Accounts…**, then the separator and the rest as today.
- **Network's empty state:** title "No Network Locations", text "Connect to a
  server, paste a share link or connect an online account.", and under it a
  pill button **Online Accounts…** (a status page's suggested action, as
  GNOME status pages have). The button is part of `statusPage`'s markup for
  Network only, wired after render like the bar.
- **The sign-in prompt on a failed listing** stays what it is; it is the
  same dialog the account row's **Sign In Again** opens.

### 6. Network's bar

`renderNetworkPaste` keeps the entry (placeholder "Server or share link",
which is what it accepts) and **Connect**, drops **Add…**, and adds a round,
flat info button (icon `info-outline`, tooltip "Server Address Information")
that opens a popover with `openPopover`:

- Title **Server Addresses**.
- "Server addresses are made up of a protocol prefix and an address.
  Examples:" then `ssh://192.168.0.1, sftp://files.example.org`.
- A two-column grid **Available Protocols** / **Prefix**:
  "SSH File Transfer Protocol" — `sftp:// or ssh://`;
  "Shared folder link" — `share://`.

The popover is text only, built from a small table in `main.ts` so a new
scheme is one row. The bar stays one row and fits a 700 px window; the
popover is positioned above the button and clamps to the window as every
popover does.

### 7. Network's listing

`networkEntries()` lists one row per account from `accounts`, in the order
the backend reports them, after the shares and servers: the provider's label
as the name, the email as the detail (what `networkDetail` already shows),
"Connected" as the status. `parentOf`, `crumbsFor`, `basename` and
`selectionOn` already work by account id (`cloudAccountKey`); with several
accounts each root's Up goes to Network. The row's context menu stays
**Open**, **Copy Location**, **Disconnect…**, with **Disconnect…** calling
the provider's `disconnect(account)`. Nothing in the virtualized views
changes.

### 8. Several accounts, on the frontend

`driveAccount: DriveAccount | null` becomes `accounts: Account[]`, filled at
start-up from `driveAccounts()` and updated when a sign-in finishes (upsert
by id, as the backend does) or a disconnect succeeds (remove by id).
`promptDriveSignIn` already finds the account by `cloudAccountKey(loc)`.
Signing in to a Google account that is already connected replaces its row
(same id) and shows "Signed in to <email>", as reconnecting does today.

### 9. Themes

Everything is built from existing tokens and classes: boxed lists use
`--card-bg`, `--border` and `--radius-popover` (12 px) in the GNOME theme and
the macOS theme's equivalents; action rows are 52 px tall with the dialog's
font; the chevron and the error icon are symbols from `src/icons.ts`. Both
themes get the same markup; only the tokens differ. No new colour: the
"Sign in again" text uses `--destructive`.

### 10. Tests

Frontend only (`npm test`); no Rust change.

- `tests/providers.test.ts` (new): the table has a row for every
  `ProviderId`; each row's `connect`, `signIn`, `disconnect` call what the
  mock expects; `PROVIDER_ORDER` names every provider once.
- `tests/online-accounts.test.ts` (new): the dialog opens from the primary
  menu, Network's menus and the empty-state button; lists no account group
  before a connection and one row per account after; a provider row starts
  the client setup when no client is configured and the connect dialog when
  it is; the account dialog's links and responses; "Sign in again" shows
  after a listing failed for a credential and goes away after signing in;
  Escape and Close; focus on the first row.
- `tests/drive.test.ts`, `tests/dialog-in-flight.test.ts`: reach Google
  Drive through the dialog instead of the Add… menu; a second account lists
  a second Network row and Up from either root goes to Network.
- `tests/network.test.ts`: the info button opens the popover with the two
  protocol rows; the Add… test goes.
- `tests/share.test.ts`: the new empty-state text and button.
- `tests/item-actions-matrix.test.ts`: Network's background and location
  menus gain **Online Accounts…**.
- `tests/mock-core.test.ts`: unchanged unless a mock command is added (none
  is expected).
- `scripts/mock-screenshot.mjs`: scene `network-add-menu` becomes
  `online-accounts` (the dialog open) and `network-address-help` (the
  popover open); PR screenshots light and dark.

### 11. Documentation

- `README.md`: the Google Drive line names **Network → Online Accounts… →
  Google Drive**.
- `docs/google-drive.md` Part 2 and "Using a different client": the same
  path; a note that another Google account is added the same way.
- `AGENTS.md`'s layout table: a row for `src/providers.ts`; the Drive spec's
  D3 and D15 get a line pointing here.
- `docs/architecture.md`: `providers.ts` next to `locations.ts` and
  `item-actions.ts` in the diagram's rules group.

## Decisions

- **D1. A dialog, not list rows.** Account setup lives in an Online
  Accounts dialog shaped like GNOME Settings' panel, not in Network's
  listing: Files never puts setup in a content view, and the dialog gives
  the lifecycle one home.
- **D2. One table drives every surface.** `PROVIDERS` is a
  `Record<ProviderId, Provider>`; the dialog, Network's rows, the account
  dialog and the tests read it. No surface may name a provider.
- **D3. Nautilus's bar.** Entry, **Connect**, info popover; no menu button.
  Connect to Server stays reachable from the menus and from a typed address.
- **D4. Expired sign-ins are learned, not polled.** The dialog shows what
  the last failed request taught the app; nothing asks Google in the
  background.
- **D5. Several accounts per provider, frontend only.** The backend already
  keeps a list; the frontend stops truncating it to one.
- **D6. GNOME's words everywhere.** "Online Accounts", "Your Accounts",
  "Connect an Account", "Server Addresses", "Available Protocols", "Server
  Address Information" are GNOME's own strings; Close / suggested /
  destructive follow GNOME's button order and defaults.

## Out of scope, noted for later

- Providers beyond Google Drive, each its own spec: OneDrive and Dropbox
  (OAuth, a new scheme each), WebDAV/Nextcloud and SMB (address-based: they
  belong in the bar's protocol list more than in the accounts dialog; the
  design leaves room for both).
- A Preferences dialog. If one arrives, **Online Accounts…** moves under it
  and the primary menu item goes.

## As built (2026-11-08)

Where the implementation differs from the design above, and why:

- **One backend change after all.** A fresh sign-in got a new account id
  every time, so connecting a Google account that is already connected would
  have listed it twice. `CloudState::record_sign_in` now reuses the id of the
  account with the same email (ignoring case) when no account is named, with
  a test. The mock keeps a list of accounts and does the same.
- **The table is split in two.** `src/providers.ts` holds what has no DOM
  (label, icon, which paths are the provider's, the account key in a path,
  `Account`, `accountOf`); what a provider does is `PROVIDER_ACTIONS` in
  `main.ts`, a `Record<ProviderId, ProviderActions>`, because its dialogs
  live there. A missing row is still a type error in either. The dialogs are
  their own module, `src/online-accounts.ts`.
- **The account dialog.** Its link to the account's files is **Browse
  Files** (it opens My Drive, not Network). Responses are Close,
  **Disconnect…** (destructive), then **Sign In Again** (suggested, only when
  known expired), so the suggested action is last as in GNOME's dialogs;
  Enter signs in again when that is offered and closes otherwise.
- **Change Client… asks first.** A client change disconnects every Google
  account, so it confirms ("Change Google Client?", default Cancel) unless
  the only account it removes is the one whose sign-in already failed (the
  expired sign-in dialog's Change Client…, which behaves as before).
- **Network's row icon** comes from the provider's row (a cloud drawn by the
  app, in both themes) instead of the network glyph.
- **The protocol popover** lists "Shared folder link" with its real prefix,
  `share://`.
- **Known edge:** signing in again for one account while choosing a
  different Google account in the browser keeps the first account's id and
  takes the new email, as before this change; if that email is another
  connected account, both rows show it until one is disconnected.
