# Share files with a QR code

**Date:** 2026-10-30

**Issue:** #73

**Status:** Implemented. Checked end to end on Linux with a local relay and
Chromium as the phone; several files (D12) also through n0's public relays.
Not yet on real phones.

**Related:** [iroh sharing](2026-10-25-iroh-sharing-design.md), whose
non-goal "sharing single files" this covers for phones only. Folder sharing is
unchanged.

## Goal

Someone at the computer picks one or more files. Someone else scans a QR
code with a phone and the files download straight to the phone:

- no app on the phone;
- any network (Wi-Fi or mobile data);
- encrypted end to end;
- nothing stored anywhere in between.

## What the user sees

- **Menu:** right-click one or more local files (no folder, no remote file,
  not in the Trash) → **Share with QR Code…**, right after Open With….
- **The dialog,** "Share “name”" (or "Share 3 Files"), shows in order:
  - **Preparing… 40%** while the file is hashed, then **Connecting…** while
    the computer reaches its relay;
  - the QR code, the hint "Scan with a phone's camera to download the file
    (files). Works on any network while this window is open.", and the link with
    **Copy**;
  - a status line: **Waiting for a phone…**, **Sending… 42%**, **Downloaded by
    1 device**.
- **Stopping:** **Stop Sharing** (the only button, also on Escape) ends the
  share, and so does closing the window.
- **A failure** replaces the code and the hint with its reason, e.g. "Could
  not reach the relay server. Check the internet connection and try again."
- **The phone's page** shows the file name and size, **Download**, a progress
  bar, then **Downloaded and checked.** with **Save File** (and **Share…**
  where the phone can share files). A picture the phone's browser can show
  (JPEG, PNG, GIF, WebP, and HEIC/AVIF where supported) replaces the file
  icon once it has arrived; the link carries no name or type, so nothing
  shows before Download. Every refusal says what to do: "Sharing has
  stopped. Ask for a new QR code." The page follows the phone's light or
  dark mode.
- **Several files on the phone:** "3 Files" and the total size, **Download**,
  one progress bar for all of them, then a row per file (a small picture for
  photos, the name, the size, **Save**) with **Save All** and **Share…**
  below. Share… hands every file to the phone at once (on an iPhone, Save to
  Photos takes them all).

## Decisions

- **D1. The share lasts while the dialog is open.** This is easy to
  understand and hard to forget; nothing keeps serving in the background.
- **D2. The phone runs iroh in its browser.** The page is WebAssembly built
  from `web/file-get`. Browsers can't send UDP, so iroh in a browser only
  connects through a relay. The page uses the sharer's home relay, named in
  the link.
- **D3. A protocol of our own, not iroh-blobs.** Only core iroh officially
  supports browsers. `filebrowser/file/2` is tiny: one stream per request.
  - The phone sends the token (16 bytes), a file index (u32; `u32::MAX`
    asks for the list) and an offset (u64).
  - The computer answers a status byte; with `OK`, then either the list
    (u32 length, then the list) or the file's bytes from the offset to the
    end.
  - The list: the count (u32), then per file its size (u64), BLAKE3 hash and
    name (u16 length + UTF-8). At most 1,000 files, names cut at 1,024 bytes.
  - Statuses: `OK`, `NotShared` (stopped or unknown token), `Changed`,
    `BadOffset`, `NoSuchFile`.
  - Version 1 (`filebrowser/file/1`, 0.7.0) shared one file, answering its
    name before its bytes. The app no longer makes version 1 links; the page
    still downloads them.
- **D4. The wire format lives in one file.** `src-tauri/src/share/file_link.rs`
  is pure code, compiled into the app and into the page (`#[path]`), so the
  two can't drift.
- **D5. The link is `<page>#<ticket>`.** Browsers never send the part after
  `#` to the page's server. The ticket (base64url) holds:
  - a version byte, the sharer's endpoint id and a random 128-bit token (the
    permission to download);
  - the BLAKE3 hash of the list, the total size and the number of files
    (version 1: the one file's hash and size);
  - the relay URL.

  The phone checks the list against the link, then every file's size and
  hash against the list, before offering to save.
- **D6. The computer hashes before showing the code.** A file that changes
  afterwards (size or modification time) is refused (`Changed`), never
  served stale. A change during hashing fails the share ("Try again").
- **D7. A separate endpoint.** Folder sharing's endpoint has its own router
  and identity. The QR endpoint is started on the first QR share, with an
  ephemeral key, and stays up while the app runs.
- **D8. n0's public relays by default.** They are free, rate-limited and meant
  for hobby use with no uptime guarantee. That suits this project; a
  self-hosted relay removes the limits.
  - `FILEBROWSER_RELAY` sets the relay, for tests or a self-hosted relay.
  - `FILEBROWSER_FILE_PAGE` sets the page, for forks or testing before the
    page is published.
- **D9. The page lives on GitHub Pages.** It is published from this repo by
  `.github/workflows/file-page.yml`: built on pull requests, deployed from
  `main` or by hand. Pages has to be turned on once.
- **D10. Downloads are held in memory, then saved.** Saving from a tap works
  everywhere, including iPhones. The page warns when a file is over 1 GB.
  Streaming to disk is a later improvement.
- **D11. The QR code** is drawn by the `qrcode` crate (SVG, error correction
  M, black on white whatever the theme, since scanners need the contrast).
  It is shown as an `<img>` from a data URL, never inserted as markup.
- **D12. Several files share one link.** The link names the list, not the
  files, so the QR code is the same size for one file or a thousand. One
  file is a list of one, so there is a single path. The phone downloads the
  files one after another over one connection and keeps them in memory
  (D10); a phone that received every file counts as one download. Folders
  aren't shared yet: they would need walking, and names with their paths.

## Testing

- **Rust (`share::file_link`, `share::file_share`).**
  - The ticket round trip (both versions), damaged and newer tickets,
    requests, the list round trip, damaged or oversized lists, names, safe
    file names.
  - Loopback downloads of the list and every file, whole and from an
    offset, with the hashes checked against the list and the list against
    the ticket; one download counted once every file arrived.
  - Refusals: unknown token, no such file, offset past the end, file
    changed (the other files still download), share stopped.
  - Folders, missing files, no files and too many files refused; hashing
    progress and cancelling; progress views across files.
- **App (`tests/qr-share.test.ts`, mock in `dev/mock-tauri`).**
  - The menu item (one or more files, never with a folder).
  - Several files: the heading, the hint and the paths sent.
  - Preparing, code and link, Sending…, downloads, Copy.
  - Stop Sharing, Escape, a failure, a share stopped elsewhere.
- **End to end, run here.**
  - Setup: a local `iroh-relay --dev`, the real app (debug build) on Xvfb
    with `FILEBROWSER_RELAY`, the page served locally, and Chromium at
    phone size as the phone. The QR code on screen was decoded from a
    screenshot (jsQR), so the code itself scans.
  - An 8 MB file arrived byte-identical (SHA-256), light and dark. The app
    then showed "Downloaded by 1 device".
  - After Stop Sharing, the phone got "This file is no longer shared."
  - The page also works from a plain-HTTP LAN address (not a secure
    context), which `web/file-get/serve.sh` relies on.
  - Without a reachable relay, the dialog shows its error.
  - Several files (D12), through n0's public relays: a scratch program
    running the app's own `file_share.rs` and `file_link.rs` shared a PNG,
    an 8 MB binary and a text file; Chromium at phone size downloaded all
    three in about 5 s, and Save All gave byte-identical files (SHA-256),
    light and dark. One photo arrived the same way as a list of one. After
    stopping, the phone got "Sharing has stopped." A sharer built from the
    v0.7.0 sources (a version 1 link) still downloaded with the new page.
  - Not run for several files: the app's own menu and dialog (covered by
    the mock tests), and the QR code scanned from the screen.
- **Not verified:**
  - real phones (iPhone Safari, Android Chrome), on Wi-Fi and on mobile data;
  - n0's public relays for a single file from the app itself;
  - the published Pages site;
  - macOS (the app side is the same Rust; a Mac firewall isn't involved,
    since all connections are outgoing).
