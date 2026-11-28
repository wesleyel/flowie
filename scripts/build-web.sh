#!/bin/sh
# Build the browser version: web/pkg (flowie-wasm compiled to wasm, with its JS glue and
# types), then web/dist (the page, its worker and the wasm, type-checked against those types
# and bundled by Vite, with a service worker listing the built files).
# Serve it with: flowie-server serve --web web/dist
# Needs the wasm32-unknown-unknown target and rustup's llvm-tools (see scripts/wasm-check.sh).
set -eu
cd "$(dirname "$0")/.."
version=$(awk '/^name = "wasm-bindgen"$/ { getline; gsub(/version = |"/, ""); print; exit }' Cargo.lock)
tools="$PWD/target/wasm-tools/$version"
if [ ! -x "$tools/bin/wasm-bindgen" ]; then
    cargo install wasm-bindgen-cli --version "$version" --locked --root "$tools"
fi
host=$(rustc -vV | sed -n 's/^host: //p')
AR_wasm32_unknown_unknown="$(rustc --print sysroot)/lib/rustlib/$host/bin/llvm-ar" \
    cargo build -p flowie-wasm --target wasm32-unknown-unknown --release --locked
"$tools/bin/wasm-bindgen" --target web --out-dir web/pkg target/wasm32-unknown-unknown/release/flowie_wasm.wasm
(cd web && npm ci --no-audit --no-fund && npm run check && npm run build)
echo "built web/pkg and web/dist"
