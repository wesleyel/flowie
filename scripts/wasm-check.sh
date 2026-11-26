#!/bin/sh
# Run flowie-wasm's tests compiled to wasm, under Node: the outline, SQLite and sync engine as
# the browser runs them. Needs Node, the wasm32-unknown-unknown target and rustup's
# llvm-tools (SQLite's C code is archived with llvm-ar; the system ar can't index wasm). The
# wasm-bindgen test runner matching Cargo.lock is installed under target/ when missing.
set -eu
cd "$(dirname "$0")/.."
version=$(awk '/^name = "wasm-bindgen"$/ { getline; gsub(/version = |"/, ""); print; exit }' Cargo.lock)
tools="$PWD/target/wasm-tools/$version"
if [ ! -x "$tools/bin/wasm-bindgen-test-runner" ]; then
    cargo install wasm-bindgen-cli --version "$version" --locked --root "$tools"
fi
host=$(rustc -vV | sed -n 's/^host: //p')
ar="$(rustc --print sysroot)/lib/rustlib/$host/bin/llvm-ar"
[ -x "$ar" ] || { echo "llvm-ar missing: rustup component add llvm-tools" >&2; exit 1; }
AR_wasm32_unknown_unknown="$ar" CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER="$tools/bin/wasm-bindgen-test-runner" \
    cargo test -p flowie-wasm --target wasm32-unknown-unknown --locked
