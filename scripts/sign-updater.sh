#!/usr/bin/env bash
# Sign the packages Check for Updates installs, as Tauri's own signer would,
# with plain minisign: the release workflow runs this in a job of its own that
# has the signing key and nothing else (no npm packages, no Rust build), so a
# compromised dependency in the build jobs can never read the key.
#
#   TAURI_SIGNING_PRIVATE_KEY=… TAURI_SIGNING_PRIVATE_KEY_PASSWORD=… \
#     scripts/sign-updater.sh <version> <updater pubkey> <package>...
#
# The key and the pubkey are Tauri's format: a minisign key file, base64-
# encoded. Each <package> gets <package>.sig, the base64 of a minisign
# signature whose trusted comment carries `version:<version>`, which the app's
# updater requires (requireSignedVersion). Every signature is then verified
# against <updater pubkey> (from tauri.conf.json), so a key that doesn't match
# the one installed copies trust fails here, not on users' machines. Nothing
# is left behind on failure. Tested by src-tauri/tests/release_signing.rs.
set -euo pipefail

version=${1:-}
pubkey=${2:-}
shift 2 || true
if [[ -z $version || -z $pubkey || $# -eq 0 ]]; then
  echo "usage: $0 <version> <updater pubkey> <package>..." >&2
  exit 2
fi
if [[ -z ${TAURI_SIGNING_PRIVATE_KEY:-} ]]; then
  echo "sign-updater: TAURI_SIGNING_PRIVATE_KEY is not set." >&2
  exit 1
fi
command -v minisign >/dev/null || { echo "sign-updater: minisign is not installed." >&2; exit 1; }

# The key lives only in a private temporary directory, removed on any exit.
work=$(umask 077 && mktemp -d "${RUNNER_TEMP:-${TMPDIR:-/tmp}}/sign-updater.XXXXXX")
signed=()
cleanup() {
  status=$?
  rm -rf "$work"
  # On failure, no partial set of signatures survives.
  if [[ $status -ne 0 ]]; then
    for sig in "${signed[@]}"; do rm -f "$sig"; done
  fi
}
trap cleanup EXIT
(umask 077 && printf '%s' "$TAURI_SIGNING_PRIVATE_KEY" | base64 -d >"$work/key")
printf '%s' "$pubkey" | base64 -d >"$work/key.pub"

for package in "$@"; do
  [[ -f $package ]] || { echo "sign-updater: no package $package" >&2; exit 1; }
  name=$(basename "$package")
  comment=$(printf 'timestamp:%s\tfile:%s\tversion:%s' "$(date +%s)" "$name" "$version")
  # minisign reads the password from stdin when it isn't a terminal.
  printf '%s\n' "${TAURI_SIGNING_PRIVATE_KEY_PASSWORD:-}" |
    minisign -S -s "$work/key" -m "$package" -x "$work/sig" -t "$comment" >/dev/null
  if ! minisign -V -q -p "$work/key.pub" -m "$package" -x "$work/sig" >/dev/null; then
    echo "sign-updater: the signature of $name does not verify with the app's updater public key." >&2
    echo "sign-updater: the signing key secret doesn't match plugins.updater.pubkey in tauri.conf.json." >&2
    exit 1
  fi
  base64 -w0 "$work/sig" >"$package.sig"
  signed+=("$package.sig")
  echo "Signed $name for version $version."
done
