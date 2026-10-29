# Install applications from a DMG (macOS)

1. Right-click a local `.dmg` file and choose **Install Application…**.
2. Choose an application from the disk image. If there are several valid app
   bundles, they appear in the Application selector.
3. Choose **My Applications** (`~/Applications`, default) or **All Users**
   (`/Applications`). All Users requires normal write permission; the browser
   does not request `sudo` or an administrator password.
4. Choose **Install**. Enter/Escape default to **Cancel**.
5. After installation, the toast offers **Open**. Opening is your choice,
   and macOS Gatekeeper still decides whether the application may run.

If an item already exists, choose **Replace**, **Skip**, or **Keep Both**.
Enter chooses **Keep Both**, which creates a numbered copy without overwriting
even an empty directory. Quit the target app before replacing it. Replace
stages the complete new bundle first, then moves the old app to the Trash.
The running File Browser cannot replace itself.

## What this does—and does not do

- The original DMG is never changed, moved or deleted.
- Inspection reads a private snapshot mounted read-only and ejects it before
  showing the selection dialog. Cancel during inspection discards the eventual
  result; it cannot install anything.
- Installation takes a fresh snapshot and compares its content hash with the
  inspected image, refusing a changed image.
- App bundles are discovered within two folder levels; symlink entries and
  bundles with invalid/out-of-bounds metadata or executables are ignored.
- macOS `ditto` preserves resources/extended attributes. Quarantine flags from
  both the image and bundle are retained conservatively; nothing clears
  quarantine, re-signs the app or bypasses Gatekeeper.
- This is not a signature/security scan or a compatibility guarantee. Install
  only applications you trust. The browser never runs `.pkg` installers or
  scripts inside the disk image; use the ordinary OS installer flow for those.
- Helper operations have time/output bounds. No disk is forcibly ejected. If
  ejection fails, the result warns you and retains the private cache while it
  is mounted. Eject that image before removing the reported directory.
- An unexpected process termination can leave private staging data/mounts;
  original downloads and any replaced application in Trash remain recoverable.

Native tests on macOS cover real snapshot/mount/copy/eject operations, quarantine,
conflicts, Keep Both and a raced-in empty target. They use agent-created fixtures,
not a user's installed applications. The packaged UI and real Gatekeeper prompt
for downloaded third-party apps still require the PR's documented smoke test.
