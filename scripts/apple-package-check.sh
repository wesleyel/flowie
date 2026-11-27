#!/bin/sh
# Build flowie-ffi, generate its Swift bindings into apple/FlowieKit, build the package and
# run its checks: CloudKit's record round trip (no iCloud needed) and the SwiftUI layer's store.
# macOS with the Swift toolchain only (the Command Line Tools are enough).
set -eu
cd "$(dirname "$0")/.."
pkg=apple/FlowieKit
cargo build -p flowie-ffi --locked
lib="$PWD/target/debug/libflowie_ffi.dylib"
gen="${TMPDIR:-/tmp}/flowie-ffi-bindings"
rm -rf "$gen" && mkdir -p "$gen"
cargo run -q -p flowie-ffi --features bindgen --bin uniffi-bindgen --locked -- generate --library "$lib" --language swift --out-dir "$gen"
mkdir -p "$pkg/Sources/FlowieFFI" "$pkg/Sources/flowie_ffiFFI/include"
cp "$gen/flowie_ffi.swift" "$pkg/Sources/FlowieFFI/"
cp "$gen/flowie_ffiFFI.h" "$pkg/Sources/flowie_ffiFFI/include/"
cp "$gen/flowie_ffiFFI.modulemap" "$pkg/Sources/flowie_ffiFFI/include/module.modulemap"
(cd "$pkg" && swift build -Xlinker -L"$PWD/../../target/debug")
DYLD_LIBRARY_PATH=target/debug "$pkg/.build/debug/cloudkit-check"
DYLD_LIBRARY_PATH=target/debug "$pkg/.build/debug/ui-check"
