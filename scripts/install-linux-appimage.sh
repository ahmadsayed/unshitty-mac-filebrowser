#!/usr/bin/env bash
# Install a File Browser AppImage for the current user and register it with
# GNOME (or any freedesktop desktop) so it appears in application search.
set -euo pipefail

if [[ $# -ne 1 ]]; then
  echo "usage: $0 /path/to/File.Browser_<version>_amd64.AppImage" >&2
  exit 2
fi

source_image=$1
if [[ ! -f "$source_image" ]]; then
  echo "not a file: $source_image" >&2
  exit 1
fi

bin_dir="$HOME/.local/bin"
app_dir="$HOME/.local/share/applications"
icon_dir="$HOME/.local/share/icons/hicolor/512x512/apps"
app_image="$bin_dir/file-browser.AppImage"
desktop_file="$app_dir/io.github.ahmadsayed.filebrowser.desktop"

mkdir -p "$bin_dir" "$app_dir" "$icon_dir"
install -m 755 "$source_image" "$app_image"

# The AppImage carries its icon; extract it so the desktop entry can use it.
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
if (cd "$tmp" && "$app_image" --appimage-extract filebrowser.png >/dev/null 2>&1) &&
   [[ -f "$tmp/squashfs-root/filebrowser.png" ]]; then
  install -m 644 "$tmp/squashfs-root/filebrowser.png" "$icon_dir/filebrowser.png"
fi

cat > "$desktop_file" <<EOF
[Desktop Entry]
Type=Application
Version=1.0
Name=File Browser
GenericName=File Manager
Comment=Browse and manage files
Exec=$app_image
Icon=filebrowser
Terminal=false
StartupNotify=true
StartupWMClass=filebrowser
Categories=System;FileManager;FileTools;
Keywords=files;file browser;file manager;browser;manager;explorer;finder;
MimeType=inode/directory;
EOF
chmod 644 "$desktop_file"

if command -v gtk-update-icon-cache >/dev/null 2>&1; then
  gtk-update-icon-cache -f -t "$HOME/.local/share/icons/hicolor" >/dev/null 2>&1 || true
fi
if command -v update-desktop-database >/dev/null 2>&1; then
  update-desktop-database "$app_dir" >/dev/null 2>&1 || true
fi

echo "Installed: $app_image"
echo "Desktop entry: $desktop_file"
echo "Search for “File Browser” in GNOME Activities."
