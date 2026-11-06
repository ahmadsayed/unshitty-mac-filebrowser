#!/usr/bin/env bash
# Refuses a macOS binary that loads a library from outside macOS itself.
# The release builds on GitHub's macOS image, where Homebrew is installed: a
# crate that links whatever pkg-config finds (lzma-sys did, in 0.7.5) ends
# up loading /opt/homebrew/..., and the app then fails to open on every Mac
# without that exact file. Allowed: /usr/lib, /System/Library, and paths
# inside the app bundle (@rpath, @executable_path, @loader_path).
#
#   scripts/check-macos-libraries.sh "File Browser.app/Contents/MacOS/filebrowser"
set -euo pipefail

if [[ $# -ne 1 || ! -f $1 ]]; then
  echo "usage: $0 <macOS binary>" >&2
  exit 2
fi

# otool on a Mac; llvm-objdump prints the same list elsewhere.
if command -v otool >/dev/null; then
  listing=$(otool -L "$1")
else
  listing=$(llvm-objdump --macho --dylibs-used "$1")
fi

# The first line names the binary; each other one is "<path> (compatibility …)".
libraries=$(sed -n '2,$s/^[[:space:]]*\(.*\) (compatibility.*/\1/p' <<<"$listing")
if [[ -z $libraries ]]; then
  echo "check-macos-libraries: no libraries listed for $1" >&2
  exit 1
fi

foreign=$(grep -vE '^(/usr/lib/|/System/Library/|@rpath/|@executable_path/|@loader_path/)' <<<"$libraries" || true)
if [[ -n $foreign ]]; then
  echo "check-macos-libraries: $1 loads libraries that aren't part of macOS:" >&2
  sed 's/^/  /' <<<"$foreign" >&2
  echo "Link them statically (for example the crate's \"static\" feature)." >&2
  exit 1
fi
echo "check-macos-libraries: $(wc -l <<<"$libraries") libraries, all from macOS"
