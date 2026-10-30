#!/usr/bin/env bash
# Build the page a phone opens from a QR code into web/file-get/dist:
#   dist/index.html             a short landing page
#   dist/get/                   the download page, its script and the wasm
# Needs the wasm32-unknown-unknown target, clang (ring's C code) and
# wasm-bindgen-cli at the version in Cargo.lock.
set -euo pipefail
cd "$(dirname "$0")"
cargo build --release
rm -rf dist
mkdir -p dist/get
wasm-bindgen --target web --no-typescript --out-dir dist/get \
  target/wasm32-unknown-unknown/release/file_get.wasm
cp static/get/* dist/get/
cp static/index.html dist/
echo "Built $(du -sh dist | cut -f1) in web/file-get/dist"
