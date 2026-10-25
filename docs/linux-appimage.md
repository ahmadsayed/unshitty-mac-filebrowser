# Linux AppImage and GNOME launcher

The Linux AppImage is portable: it bundles WebKitGTK, GTK and the codecs it
needs, so it is about 119 MB (331 MB unpacked). If you only use this machine,
the `.rpm`/`.deb` or the release binary is smaller, because it can use the
system WebKitGTK.

## Install

Download `File.Browser_0.4.0_amd64.AppImage` from the
[v0.4.0 release](https://github.com/ahmadsayed/unshitty-mac-filebrowser/releases/tag/v0.4.0),
then run the installer from this repository:

```sh
chmod +x scripts/install-linux-appimage.sh
scripts/install-linux-appimage.sh ~/Downloads/File.Browser_0.4.0_amd64.AppImage
```

It installs:

- `~/.local/bin/file-browser.AppImage`
- `~/.local/share/applications/io.github.ahmadsayed.filebrowser.desktop`
- `~/.local/share/icons/hicolor/512x512/apps/filebrowser.png`

The desktop entry is named **File Browser**, with file-manager keywords and
`StartupWMClass=filebrowser`, so it appears in GNOME application search and
groups under the right window.

Search for **File Browser** in GNOME Activities. If GNOME has not refreshed its
application cache, log out and back in once.

## Manual install

```sh
mkdir -p ~/.local/bin ~/.local/share/applications \
         ~/.local/share/icons/hicolor/512x512/apps
install -m 755 ~/Downloads/File.Browser_0.4.0_amd64.AppImage \
        ~/.local/bin/file-browser.AppImage
```

Extract the bundled icon:

```sh
tmp=$(mktemp -d)
(cd "$tmp" && ~/.local/bin/file-browser.AppImage --appimage-extract filebrowser.png)
install -m 644 "$tmp/squashfs-root/filebrowser.png" \
        ~/.local/share/icons/hicolor/512x512/apps/filebrowser.png
rm -rf "$tmp"
```

Create `~/.local/share/applications/io.github.ahmadsayed.filebrowser.desktop`:

```ini
[Desktop Entry]
Type=Application
Version=1.0
Name=File Browser
GenericName=File Manager
Comment=Browse and manage files
Exec=/home/YOUR-USER/.local/bin/file-browser.AppImage
Icon=filebrowser
Terminal=false
StartupNotify=true
StartupWMClass=filebrowser
Categories=System;FileManager;FileTools;
Keywords=files;file browser;file manager;browser;manager;explorer;finder;
MimeType=inode/directory;
```

Refresh the caches:

```sh
gtk-update-icon-cache -f -t ~/.local/share/icons/hicolor
update-desktop-database ~/.local/share/applications
```

## FUSE

The AppImage normally mounts itself with FUSE. If that is unavailable, run it
with:

```sh
~/.local/bin/file-browser.AppImage --appimage-extract-and-run
```

and use that command in the desktop entry’s `Exec=` line.

## Moving or removing it

The desktop entry points at `~/.local/bin/file-browser.AppImage`. If you move
the file, update the `Exec=` line. To remove the application, delete the
AppImage, the desktop entry and `filebrowser.png`.
