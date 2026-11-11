# Linux program actions

These actions are available on Linux for a **single local regular file**.
They are not offered for folders, symbolic links, remote locations or the
Trash. Restore a trashed program before using either action.

## Install an AppImage in GNOME

1. Right-click the downloaded `.AppImage` and choose **Install Application…**.
2. Choose its application name. Only install software you trust: an AppImage
   can access your files when you open it.
3. Choose **Install**. **Cancel** is the default response.
4. Search for the chosen name in GNOME Activities and open it from there.

The browser copies the file to an app-specific directory under
`$XDG_DATA_HOME/filebrowser/appimages/` (normally `~/.local/share/filebrowser/appimages/`),
makes **the copy** executable, and creates a safely quoted `.desktop` launcher
under `$XDG_DATA_HOME/applications/`. The original download's bytes and
permissions are unchanged. No administrator password or `sudo` is used.

The installer checks the AppImage header, but this is **not** a security scan
or signature verification. It does not run the AppImage during installation,
import untrusted launcher settings, or extract its contents. The launcher uses
the desktop's generic application icon. GNOME normally notices it immediately;
if application search has not refreshed, log out and back in.

An existing installation with the same chosen name is never replaced. Use a
different name or manage the old installation manually. This is launcher
integration, not a package manager: no upgrade or uninstall interface is provided.
An AppImage still needs a compatible Linux architecture and its normal runtime
requirements, including FUSE if that image uses it. The browser does not install
FUSE or bypass those requirements.

## Run a Bash script

1. Right-click a local `.sh` or `.bash` file and choose **Run in Terminal…**.
2. Read the warning and choose **Run in Terminal** only if you trust the file.
   Pressing Enter or Escape without changing the choice cancels.
3. Read the terminal output and the script's exit status. Press Enter in the
   terminal when finished to close it.

The file is read by `/bin/bash`, with its containing folder as the working
directory. It does not need to be executable and its permissions are not
changed. This action explicitly uses Bash, even if the script has a different
shebang; use the normal Open/Open With actions for other languages.

The terminal selection is `xdg-terminal-exec` when available, otherwise
GNOME Terminal, GNOME Console (`kgx`), or xterm. If none is installed, install
one through your distribution. The browser does not run the script invisibly
or report success just because a terminal was requested.

Paths are passed as arguments, not inserted into shell code. External terminals
use the existing host-environment cleanup so an AppImage build of File Browser
does not deliberately hand its bundled libraries to the terminal. This feature
does not solve every remaining environment issue tracked by #38.

## Verification checklist

The plain-Rust filesystem and command construction are tested, including
filenames with spaces/punctuation and refused overwrites. A real Linux GNOME
session must additionally verify launcher discovery, AppImage startup, terminal
integration and AppImage-host environment compatibility. See the PR's linked
`needs-verification` issue for the pending host checks.
