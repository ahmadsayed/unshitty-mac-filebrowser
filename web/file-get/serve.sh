#!/usr/bin/env bash
# Try "Share with QR Code…" before the page is on GitHub Pages: build the
# page and serve it from this computer to phones on the same Wi-Fi, then
# start the app pointing its links here. The download itself still goes
# through iroh's public relays, like the real thing.
#
#   web/file-get/serve.sh            # then, in another terminal, the
#                                    # command it prints
set -euo pipefail
cd "$(dirname "$0")"
./build.sh
port=${PORT:-8766}
if [[ $(uname) == Darwin ]]; then
  ip=$(ipconfig getifaddr en0 || ipconfig getifaddr en1 || echo 127.0.0.1)
else
  ip=$(hostname -I 2>/dev/null | awk '{print $1}')
  ip=${ip:-127.0.0.1}
fi
cat <<MSG

Serving the page at http://$ip:$port/get/ (Ctrl+C stops it).
Start the app with its links pointing here:

  FILEBROWSER_FILE_PAGE=http://$ip:$port/get/ npm run tauri dev

MSG
exec python3 -m http.server "$port" --bind 0.0.0.0 --directory dist
