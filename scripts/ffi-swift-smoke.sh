#!/bin/sh
# Build flowie-ffi, generate its Swift bindings, and compile and run a Swift program over them
# (crates/flowie-ffi/swift/main.swift). Needs only the Swift toolchain (Command Line Tools
# are enough), on macOS.
set -eu
cd "$(dirname "$0")/.."
out="${TMPDIR:-/tmp}/flowie-ffi-swift"
rm -rf "$out" && mkdir -p "$out"
cargo build -p flowie-ffi --locked
lib="target/debug/libflowie_ffi.dylib"
cargo run -q -p flowie-ffi --features bindgen --bin uniffi-bindgen --locked -- \
    generate --library "$lib" --language swift --out-dir "$out"
swiftc -O "$out/flowie_ffi.swift" crates/flowie-ffi/swift/main.swift \
    -I "$out" -Xcc -fmodule-map-file="$out/flowie_ffiFFI.modulemap" \
    -L target/debug -lflowie_ffi -o "$out/smoke"
DYLD_LIBRARY_PATH=target/debug "$out/smoke"
