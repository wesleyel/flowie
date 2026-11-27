#!/bin/sh
# Build flowie-ffi for iOS devices and the iOS Simulator and package the static library as
# build/ios/FlowieFFI.xcframework, then generate its Swift bindings into apple/FlowieKit
# (FlowieFFI and its C module). The iOS app (apple/FlowieApp) links both. Needs Xcode.
set -eu
cd "$(dirname "$0")/.."
targets="aarch64-apple-ios aarch64-apple-ios-sim"
# rusqlite's bundled SQLite and the Rust code target the app's minimum iOS.
export IPHONEOS_DEPLOYMENT_TARGET=17.0
rustup target add $targets >/dev/null
for target in $targets; do
    cargo build -p flowie-ffi --locked --release --target "$target"
done

out=build/ios
rm -rf "$out/FlowieFFI.xcframework"
mkdir -p "$out"
xcodebuild -create-xcframework \
    -library target/aarch64-apple-ios/release/libflowie_ffi.a \
    -library target/aarch64-apple-ios-sim/release/libflowie_ffi.a \
    -output "$out/FlowieFFI.xcframework"

pkg=apple/FlowieKit
gen="$out/bindings"
rm -rf "$gen" && mkdir -p "$gen"
cargo run -q -p flowie-ffi --features bindgen --bin uniffi-bindgen --locked -- \
    generate --library target/aarch64-apple-ios-sim/release/libflowie_ffi.a --language swift --out-dir "$gen"
mkdir -p "$pkg/Sources/FlowieFFI" "$pkg/Sources/flowie_ffiFFI/include"
cp "$gen/flowie_ffi.swift" "$pkg/Sources/FlowieFFI/"
cp "$gen/flowie_ffiFFI.h" "$pkg/Sources/flowie_ffiFFI/include/"
cp "$gen/flowie_ffiFFI.modulemap" "$pkg/Sources/flowie_ffiFFI/include/module.modulemap"
echo "Built $out/FlowieFFI.xcframework"
