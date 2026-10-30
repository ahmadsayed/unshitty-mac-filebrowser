# Share a file with a QR code

**Date:** 2026-10-30

**Issue:** #73

**Status:** Implemented. Checked end to end on Linux with a local relay and
Chromium as the phone; not yet on real phones or through n0's public relays.

**Related:** [iroh sharing](2026-10-25-iroh-sharing-design.md), whose
non-goal "sharing single files" this covers for phones only. Folder sharing is
unchanged.

## Goal

Someone at the computer picks one file. Someone else scans a QR code with a
phone and the file downloads straight to the phone:

- no app on the phone;
- any network (Wi-Fi or mobile data);
- encrypted end to end;
- nothing stored anywhere in between.

## What the user sees

- **Menu:** right-click one local file (not a folder, not a remote file, not
  in the Trash) → **Share with QR Code…**, right after Open With….
- **The dialog,** "Share “name”", shows in order:
  - **Preparing… 40%** while the file is hashed, then **Connecting…** while
    the computer reaches its relay;
  - the QR code, the hint "Scan with a phone's camera to download the file.
    Works on any network while this window is open.", and the link with
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
  shows before Download. Every refusal says what to do: "This file
  is no longer shared. Ask for a new QR code." The page follows the phone's
  light or dark mode.

## Decisions

- **D1. The share lasts while the dialog is open.** This is easy to
  understand and hard to forget; nothing keeps serving in the background.
- **D2. The phone runs iroh in its browser.** The page is WebAssembly built
  from `web/file-get`. Browsers can't send UDP, so iroh in a browser only
  connects through a relay. The page uses the sharer's home relay, named in
  the link.
- **D3. A protocol of our own, not iroh-blobs.** Only core iroh officially
  supports browsers. `filebrowser/file/1` is tiny: one stream per download.
  - The phone sends the token (16 bytes) and an offset (u64).
  - The computer answers a status byte; with `OK`, then the name (u16 length
    + UTF-8) and the bytes from the offset to the end.
  - Statuses: `OK`, `NotShared` (stopped or unknown token), `Changed`,
    `BadOffset`.
- **D4. The wire format lives in one file.** `src-tauri/src/share/file_link.rs`
  is pure code, compiled into the app and into the page (`#[path]`), so the
  two can't drift.
- **D5. The link is `<page>#<ticket>`.** Browsers never send the part after
  `#` to the page's server. The ticket (base64url) holds:
  - a version byte, the sharer's endpoint id and a random 128-bit token (the
    permission to download);
  - the file's BLAKE3 hash and size;
  - the relay URL.

  The phone checks the size and hash before offering to save.
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

## Testing

- **Rust (`share::file_link`, `share::file_share`).**
  - The ticket round trip, damaged and newer tickets, requests, names, safe
    file names.
  - Loopback downloads, whole and from an offset, with the hash checked
    against the ticket and downloads counted.
  - Refusals: unknown token, offset past the end, file changed, share
    stopped.
  - Folders and missing files refused; hashing progress and cancelling;
    progress views.
- **App (`tests/qr-share.test.ts`, mock in `dev/mock-tauri`).**
  - The menu item (files only, one item).
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
- **Not verified:**
  - real phones (iPhone Safari, Android Chrome), on Wi-Fi and on mobile data;
  - n0's public relays (not reachable from the build machine);
  - the published Pages site;
  - macOS (the app side is the same Rust; a Mac firewall isn't involved,
    since all connections are outgoing).
