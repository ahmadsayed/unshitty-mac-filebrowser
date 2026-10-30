# Check for Updates

**Date:** 2026-10-29

**Status:** Implemented; updating from one signed release to the next not yet verified.

## Goal

Users can find out about a new version from inside the app, and, where the
app can safely replace itself, install it after they approve.

- **Manual only.** The app contacts GitHub only when the user picks **Check
  for Updates…** in the main menu. No check at startup or in the background.
- **AppImage and the macOS app:** download, verify and install after the user
  approves; the new version starts after the user agrees to restart.
- **`.deb` and `.rpm`:** the app reports the new version and points to the
  release page; the system's package manager does the update.
- Only **published** releases are offered, never drafts.
- Updates are **signed** with a key only the maintainer holds; an update that
  doesn't verify is never installed, even if it came from GitHub.

## Non-goals

- Automatic or scheduled checks, update notifications, a "check on startup"
  setting.
- Installing `.deb` / `.rpm` packages, downloading them, or detecting the
  distribution's package manager.
- Release channels (beta), delta updates, rolling back to an older version.
- Updating 0.5.3 and earlier: they have no updater; users install the first
  version with one by hand.

## What the user sees

The main menu gains **Check for Updates…** just above "About File Browser".
Everything below is built with `openDialog`; wording is GNOME's style.

**Checking.** Dialog "Check for Updates" with a spinner, "Checking for
updates…" and [Cancel]. Then:

| Result | Dialog |
|---|---|
| Up to date | "File Browser 0.5.3 is the latest version." [Close] |
| Offline or GitHub unreachable | "Could not check for updates." + the reason (e.g. "No internet connection.") [Close] [Try Again] |
| Newer, AppImage or Mac app | "File Browser 0.5.4 is available. You have 0.5.3." + a **What's New** link to the release page. [Not Now] [Download and Install]. **Not Now is the default** (Enter and Esc). |
| Newer, `.deb` / `.rpm` | "File Browser 0.5.4 is available. You have 0.5.3. Update it with your system's package manager." [Close] [Open Release Page] |
| Newer, dev build | As AppImage, but instead of the install button: "Updates are installed only in release builds." [Close] [Open Release Page] |

The release notes aren't shown in the dialog: today they are generic ("See
the README…"); the release page has what changed.

**Installing.** A progress bar, "Downloading… 23.4 MB of 91.4 MB", and
[Cancel]. Cancel stops the download and changes nothing. After the download
the plugin verifies the signature, then installs. On success:

> **File Browser 0.5.4 is installed.** Restart to use it.
> [Later] [Restart Now]

- **Later** keeps the session; the new version starts the next time the app
  is opened. Checking again before then offers this same dialog (the
  restart), not the same download a second time; a newer release is offered
  as usual.
- **Restart Now** is disabled while copies, moves or other operations run,
  with the hint "Waiting for operations to finish…", and enables itself when
  they end. Restarting closes every window, like Quit.

**Failures** return to a safe state; the installed app is never left
half-replaced:

| Failure | Message | Buttons |
|---|---|---|
| Signature doesn't verify, or was made for another version | "The update could not be verified and was not installed." | [Close] |
| Download failed | "The download failed." + reason | [Close] [Try Again] |
| Downloaded and verified, but installing failed | "The update was downloaded but could not be installed." + reason | [Close] [Open Release Page] |
| Another window is installing it | "Update in Progress": "Another window is already installing this update." | [Close] |
| Can't replace the app (no write access to its folder) | "File Browser can't replace itself in this folder. Download the new version from the release page." | [Close] [Open Release Page] |

## Backend

**Crate:** `tauri-plugin-updater` (v2, maintained by the Tauri team).
`Cargo.lock` gains only its new entries.

**Configuration** (`tauri.conf.json`):

- `plugins.updater.pubkey`: the maintainer's public key (public, committed).
- `plugins.updater.endpoints`:
  `["https://github.com/ahmadsayed/unshitty-mac-filebrowser/releases/latest/download/latest.json"]`.
  GitHub's `latest` never points at a draft.
- `bundle.createUpdaterArtifacts` stays **off** in the file; CI turns it on
  (see Releases), so local `npm run tauri build` needs no key.

**`src-tauri/src/updates.rs`**: plain functions, no Tauri types (testable in
the scratch crate like the other modules):

- `enum InstallKind { AppImage, MacApp, LinuxPackage, DevBuild }` and
  `install_kind(debug: bool, os: &str, appimage: Option<&str>) -> InstallKind`:
  debug build → `DevBuild`; macOS → `MacApp`; Linux with `$APPIMAGE` set and
  non-empty → `AppImage`; other Linux → `LinuxPackage`.
- `can_self_update(kind) -> bool`: true for `AppImage` and `MacApp` only.
- Mapping updater errors to the messages above (offline, verification
  failed, permission denied, not installed, busy, other download failure),
  keeping the original error text for the details line. Any failure of the
  install step that isn't about verification or permissions is "not
  installed" (`while_installing`).
- `InstallGuard::try_start` returns a ticket that frees the guard when
  dropped, so no early return can leave installs refused until restart.

Progress carries bytes only (`downloaded`, `total: Option<u64>`); the
frontend formats them with `formatSize` from `format.ts`: "23.4 MB of
91.4 MB", or "23.4 MB" when the total is unknown.

**Commands** in `lib.rs`. `check_update` and `install_update` are `async
fn`: they wait on the network and on long work (the download, and its
signature check, run on the blocking pool). `cancel_update`,
`operations_running` and `restart_app` are plain commands: each only reads
or sets a flag, or hands over to `restart()`, so there is nothing to wait
for.

- `check_update() -> UpdateInfo { current: String, latest: Option<String>, kind: InstallKind, release_url: String, restart_pending: bool }`.
  `latest` is `None` when up to date. The release URL is
  `…/releases/tag/v<latest>`. `restart_pending` is true when `latest` was
  installed earlier in this session and waits for a restart.
- `install_update(progress: Channel<Progress>)`: refused unless
  `can_self_update(kind)`; refused while another install runs. Downloads,
  verifies, installs; progress messages carry downloaded and total bytes.
- `cancel_update()`: stops a running download; a download cancelled before
  install changes nothing on disk. Only the window that started the install
  can cancel it; another window's Cancel does nothing.
- `restart_app()`: `AppHandle::restart()`.

No updater permissions are granted to the webview: the frontend only calls
these commands.

## Frontend

- `src/updates.ts`: the dialog flow above. Uses `openDialog`, `api.ts` and
  `hasRunningOperations()`.
- `src/api.ts`: typed wrappers for the four commands.
- `src/operations.ts`: `hasRunningOperations(): boolean` and a way to be
  told when it changes (a callback registration, matching the module's
  style).
- `src/main.ts`: the menu item only.

## Releases

**Signing key**, generated once by the maintainer, never seen by an agent:

```sh
npm run tauri signer generate -- -w ~/.tauri/filebrowser.key
gh secret set TAURI_SIGNING_PRIVATE_KEY < ~/.tauri/filebrowser.key
gh secret set TAURI_SIGNING_PRIVATE_KEY_PASSWORD
```

The public key (`~/.tauri/filebrowser.key.pub`) goes into
`tauri.conf.json`. The private key needs a backup: without it, installed
copies can't take updates until users reinstall by hand.

**`release.yml`:**

- Build step: the two secrets as environment variables, and
  `--config '{"bundle":{"createUpdaterArtifacts":true}}'` when the secret is
  present. Without it (a fork, a manual run without secrets) the build skips
  the updater files and still succeeds.
- Kept packages also include `*.AppImage.sig` and, for macOS,
  `bundle/macos/*.app.tar.gz` and its `.sig`.
- Release job: `scripts/updater-manifest.mjs` writes `latest.json` from the
  `.sig` files and uploads it with the other assets:
  `version`, `pub_date`, `notes`, and `platforms` entries `linux-x86_64`
  (AppImage) and `darwin-aarch64` (the Apple Silicon
  `.app.tar.gz`), each with `signature` (the `.sig` file's contents) and
  `url` (`https://github.com/<repo>/releases/download/<tag>/<asset name>`).
  Asset names use GitHub's renaming: spaces become dots
  (`File Browser.app.tar.gz` → `File.Browser.app.tar.gz`). Without `.sig`
  files the script writes nothing.

This adds about a minute (signing, packing the Mac app); no extra build.

## Verification

**Frontend tests** (`tests/updates.test.ts`, mock backend with a
`__mockUpdate({ latest, kind, fail })` control):

- the menu item exists, above "About File Browser";
- each row of both tables: wording and buttons;
- Not Now is the default response when an update is offered;
- Cancel during download shows nothing installed and closes;
- Restart Now is disabled while an operation runs and enables when it ends;
- the progress line reads "Downloading… 23.4 MB of 91.4 MB", and only the
  downloaded size when the total is unknown;
- Try Again after an offline check checks again.

`tests/mock-core.test.ts` gets a case per new mock command.

**Rust tests** (`updates.rs`): `install_kind` for each combination,
including an empty `$APPIMAGE`; `can_self_update`; the error mapping.

**Script test:** `scripts/updater-manifest.mjs` against a fixture folder of
fake packages and `.sig` files: the platform keys, the renamed asset URLs,
and no output without `.sig` files.

**Real app, before merge:** a local release build on Linux runs Check for
Updates against the published `latest.json` and reports correctly for how it
was started (installed AppImage vs. `.rpm`).

**Needs two signed releases** (a `needs-verification` issue, one line each):
updating N → N+1 on the AppImage and on macOS; Restart Now relaunches the
new version; a tampered `latest.json` or signature is refused; Gatekeeper
doesn't block the updated Mac app; an app in a folder the user can't write
shows the "can't replace itself" message.

## Docs

- **AGENTS.md:** `updates.rs` and `src/updates.ts` in the Layout table; the
  release checklist's smoke test gains "update the previously installed
  version with Check for Updates"; the signing key is the maintainer's,
  agents never handle it.
- **HUMANS.md:** key generation, the two secrets, keeping a backup.
- **README:** Check for Updates in the feature list; under "Not supported
  yet": no automatic checks, no self-update for `.deb`/`.rpm`.
