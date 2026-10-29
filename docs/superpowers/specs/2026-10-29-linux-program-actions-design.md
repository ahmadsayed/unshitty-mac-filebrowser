# Linux program context actions

Approved in conversation: AppImage **Install Application…** and Bash script
**Run in Terminal…**, each with confirmation. Agreed test seams are the real
frontend UI and public plain-Rust install/run interfaces.

## Installation

Only a single local regular `.AppImage` file, outside Trash and `.versions/`,
offers installation on Linux. Confirm that applications can access the user's
files; ask for a display name. Cancel is the default. Copy—not move—the source
to a fresh owned directory under `$XDG_DATA_HOME/filebrowser/appimages/`, using
`~/.local/share` when unset. Make only the copy executable. Publish a new
`.desktop` entry under `$XDG_DATA_HOME/applications/` using safely escaped Exec
arguments and a generic `application-x-executable` icon.

Validate the AppImage ELF/AI header before writing. Do not execute it to read
metadata, blindly import its desktop file, extract arbitrary assets or grant
elevated permissions. A chosen name has a stable hashed installation ID;
existing installations/launchers are refused, never overwritten. Failed
installation removes only the staging artifacts created by that call. Keep
the source bytes and permissions unchanged. Cache refresh is best-effort.

This is initial user-local integration, not a package manager: no upgrades,
uninstall UI, embedded icons or FUSE installation. The installed AppImage uses
its normal runtime and may still need the host's FUSE compatibility package.

## Scripts

Only a single local regular `.sh` or `.bash` outside Trash/`.versions/` is
eligible. A dialog warns that Bash will execute its contents with the user's
permissions. Cancel is the default. Run `/bin/bash -- <absolute-path>` in the
user's terminal, with the script's parent as working directory. Keep the
terminal open to display exit status/output; don't claim the script succeeded
merely because the terminal was started. No chmod is needed or performed.

Prefer `xdg-terminal-exec`, then GNOME Terminal, GNOME Console, then xterm.
Only fixed command templates are used: paths travel as positional arguments,
never interpolated into shell text. External programs use the existing host
environment cleanup so the file browser AppImage's libraries are not inherited.
Report a missing terminal before executing a script. No sudo or remote execution.

## Integration and validation

- New async Tauri commands do file/process work on blocking workers; no Tauri
  types in platform logic. Frontend wrappers and mocks accompany every command.
- Existing menus/dialog widgets and theme tokens are reused; no new runtime
  dependencies. Escape every file name rendered as HTML.
- UI: Linux-only eligibility, single selection, confirmation, Cancel, invalid
  names, error retry, busy guard, successful launcher feedback.
- Rust: unchanged source, executable copy, escaped desktop fields, invalid
  AppImage, name clash, rollback and existing-target preservation, script path
  injection refusal, no permission changes and actual fake-terminal invocation.
- Real Linux desktop smoke test: install an AppImage, find/launch it in GNOME
  Activities, and run a harmless script to inspect its working directory/output.
  A macOS host cannot verify GNOME. Unverified Linux behavior must be tracked
  on a `needs-verification` issue before handover.
