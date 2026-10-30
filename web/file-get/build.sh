#!/usr/bin/env bash
# Build the page a phone opens from a QR code into web/file-get/dist:
#   dist/index.html             a short landing page
#   dist/get/                   the download page, its script and the wasm
# Needs the wasm32-unknown-unknown target (rustup target add
# wasm32-unknown-unknown), a clang that can target WebAssembly and
# wasm-bindgen-cli at the version in Cargo.lock (cargo install --locked
# wasm-bindgen-cli --version <it>).
set -euo pipefail
cd "$(dirname "$0")"
# Apple's clang can't target WebAssembly; Homebrew's LLVM can
# (`brew install llvm`). ring compiles a little C for the TLS code.
if [[ $(uname) == Darwin && -z ${CC_wasm32_unknown_unknown:-} ]] && command -v brew >/dev/null; then
  llvm=$(brew --prefix llvm 2>/dev/null || true)
  if [[ -x $llvm/bin/clang ]]; then
    export CC_wasm32_unknown_unknown=$llvm/bin/clang AR_wasm32_unknown_unknown=$llvm/bin/llvm-ar
  else
    echo "On a Mac, building the page needs Homebrew's LLVM: brew install llvm" >&2
    exit 1
  fi
fi
cargo build --release
rm -rf dist
mkdir -p dist/get
wasm-bindgen --target web --no-typescript --out-dir dist/get \
  target/wasm32-unknown-unknown/release/file_get.wasm
cp static/get/* dist/get/
cp static/index.html dist/
echo "Built $(du -sh dist | cut -f1) in web/file-get/dist"
