# macOS DMG installation

Approved: a single local `.dmg` gets **Install Application…** on macOS.
Use the previously agreed real-UI and plain-Rust test boundaries.

## Behavior

Inspect the disk image read-only without executing any contents. Show valid
`.app` bundles from its root or at most two levels below, allowing a choice
when several exist. Offer **My Applications** (`~/Applications`, default)
and **All Users** (`/Applications`); no sudo or automatic privilege escalation.
Installation confirmation defaults to Cancel. Package installers and bundled
scripts are never run by this action. After success, offer Open, but do not
automatically launch the application.

## Isolation and cleanup

Each inspection/installation is a self-contained blocking operation: create
an owned private cache directory, snapshot the DMG, hash the snapshot and mount
it read-only at an owned mount point with automatic opening disabled. Validate
the returned mount point and reject a borrowed/reused mount at another location.
Inspection unmounts before returning the choices, so cancelling the dialog
cannot strand a mount. Installation takes a new snapshot and verifies its hash
against inspection before mounting. Paths from the caller are matched only to
freshly scanned application IDs, never used as arbitrary source paths.

Bound helper subprocess time/output and scanning depth/count. Eject only the
mount created by this operation, never a user's existing mounted image. No
forced ejection. If ejection fails, report it and retain the scratch directory
while mounted; never recursively delete a mounted filesystem. Original DMGs
are never renamed, deleted or modified.

## Applications and replacement

Validate bundle metadata and executable containment without launching code.
Copy with macOS `ditto` into a fresh staging directory beside the destination,
preserving extended attributes, resource forks and quarantine. Carry forward
the image's quarantine attribute when needed; never clear quarantine, ad-hoc
sign a downloaded app or bypass Gatekeeper. Gatekeeper remains authoritative
when the user later opens the installed application.

Publish using macOS's exclusive rename so even a raced-in empty directory
cannot be overwritten. On conflict, ask **Replace / Skip / Keep Both** with
Keep Both as the default. Replace stages the complete new app first, then
sends the old app to the Trash before publication. Refuse replacement of the
currently running File Browser itself and non-bundle targets. Warn the user
to quit the target application first; never automatically quit unrelated apps.
An error after moving the old app to Trash must explain where recovery is
possible. Keep Both chooses a numbered copy name with exclusive publication.
Trash replacement goes through the existing VersionsState guard so tracked
histories are suspended/unregistered safely rather than bypassing their locks.

## Verification

UI: macOS/local/single-file eligibility, Cancel while inspection is pending,
app selection, destinations, conflicts/defaults, busy guards, errors and Open.
Backend: metadata/path refusals, actual read-only mount/eject, original hash
unchanged, quarantine preservation, no-clobber publication, Keep Both and safe
Trash replacement. Use harmless agent-created app bundles/images, never a
user's installed applications for automated tests.

No Linux behavior change. Inspect both themes/light-dark in the mock preview;
record real macOS testing and any remaining Gatekeeper/GUI checks honestly.
