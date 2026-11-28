#!/usr/bin/env bash
# Package a built Linux binary as a tar.gz and an AppImage.
# usage: scripts/bundle-linux.sh <binary> [outdir]
#
# The tar.gz holds an install prefix (bin/, share/): extract it into ~/.local with
# `tar -xzf Flowie-*.tar.gz -C ~/.local --strip-components=1`. The AppImage runs as is.
# Both carry the desktop entry, hicolor icons and the .flowie MIME type, all named after the
# app id the window reports (flowie.spokey.cn), so Wayland matches the window to its icon.
#
# The AppImage needs appimagetool (on PATH, or APPIMAGETOOL=/path/to/appimagetool); without
# it only the tar.gz is made. It also needs patchelf: the AppImage carries the xkbcommon
# libraries (BUNDLED_LIBS below) that light X11 desktops may lack. Icon sizes below 256 px
# need ImageMagick (magick or convert).
# FLOWIE_VERSION overrides the version in the file names (e.g. 1.0.0-rc.7 for a candidate);
# ARCH overrides the architecture (default: uname -m). Prints the paths it made.
set -euo pipefail
BIN="$1"
OUT="${2:-build}"
APP_ID=flowie.spokey.cn
ARCH="${ARCH:-$(uname -m)}"
VERSION="${FLOWIE_VERSION:-$(cargo metadata --no-deps --format-version 1 \
  | python3 -c 'import json,sys; print(next(p["version"] for p in json.load(sys.stdin)["packages"] if p["name"]=="flowie-app"))')}"
NAME="Flowie-$VERSION-linux-$ARCH"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"

[ -x "$BIN" ] || { echo "bundle-linux: $BIN is not an executable" >&2; exit 1; }
mkdir -p "$OUT"
OUT="$(cd "$OUT" && pwd)"

# Fill an install prefix ($1) with the binary and its desktop integration.
install_prefix() {
  local prefix="$1"
  install -Dm755 "$BIN" "$prefix/bin/flowie"
  install -Dm644 "$ROOT/assets/linux/$APP_ID.desktop" "$prefix/share/applications/$APP_ID.desktop"
  install -Dm644 "$ROOT/assets/linux/$APP_ID.xml" "$prefix/share/mime/packages/$APP_ID.xml"
  install -Dm644 "$ROOT/assets/icon.png" "$prefix/share/icons/hicolor/256x256/apps/$APP_ID.png"
  local resize=""
  if command -v magick >/dev/null; then resize=magick; elif command -v convert >/dev/null; then resize=convert; fi
  if [ -n "$resize" ]; then
    for size in 16 24 32 48 64 128; do
      local dir="$prefix/share/icons/hicolor/${size}x${size}/apps"
      mkdir -p "$dir"
      "$resize" "$ROOT/assets/icon.png" -resize "${size}x${size}" "$dir/$APP_ID.png"
    done
  else
    echo "bundle-linux: no ImageMagick, so only the 256 px icon" >&2
  fi
  install -Dm644 "$ROOT/LICENSE" "$prefix/share/doc/flowie/LICENSE"
}

# --- tar.gz: an install prefix in a top-level folder
STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT
install_prefix "$STAGE/$NAME"
TARBALL="$OUT/$NAME.tar.gz"
tar -C "$STAGE" --owner=0 --group=0 -czf "$TARBALL" "$NAME" 2>/dev/null \
  || tar -C "$STAGE" -czf "$TARBALL" "$NAME" # bsdtar (macOS) has no --owner
echo "$TARBALL"

# --- AppImage: the same prefix as usr/, with the entry and icon at the top
TOOL="${APPIMAGETOOL:-$(command -v appimagetool || true)}"
if [ -z "$TOOL" ]; then
  echo "bundle-linux: appimagetool not found, skipping the AppImage" >&2
  exit 0
fi
APPDIR="$STAGE/Flowie.AppDir"
install_prefix "$APPDIR/usr"
# Libraries the AppImage brings along, from the build machine. Everything else Flowie links
# is on AppImage's excludelist (fontconfig, freetype, wayland-client, X11-xcb…), but these
# aren't base libraries: Debian and Ubuntu split libxkbcommon-x11-0 out, and light X11
# desktops (i3, Openbox, XFCE) may not have it. libxcb-xkb is what libxkbcommon-x11 needs.
BUNDLED_LIBS=(libxkbcommon.so.0 libxkbcommon-x11.so.0 libxcb-xkb.so.1)
command -v patchelf >/dev/null || { echo "bundle-linux: the AppImage needs patchelf" >&2; exit 1; }
mkdir -p "$APPDIR/usr/lib"
for lib in "${BUNDLED_LIBS[@]}"; do
  path="$(ldd "$BIN" | awk -v l="$lib" '$1 == l { print $3 }')"
  [ -f "$path" ] || { echo "bundle-linux: $BIN doesn't resolve $lib" >&2; exit 1; }
  cp -L "$path" "$APPDIR/usr/lib/$lib"
done
# Look in usr/lib first. DT_RPATH (--force-rpath), not RUNPATH, so it also covers
# libxcb-xkb, which only libxkbcommon-x11 asks for. No LD_LIBRARY_PATH in AppRun: that
# would leak into the processes Flowie starts.
# shellcheck disable=SC2016 # $ORIGIN is for the dynamic loader, not the shell
patchelf --force-rpath --set-rpath '$ORIGIN/../lib' "$APPDIR/usr/bin/flowie"
# shellcheck disable=SC2016
patchelf --force-rpath --set-rpath '$ORIGIN' "$APPDIR/usr/lib/libxkbcommon-x11.so.0"
cp "$ROOT/assets/linux/$APP_ID.desktop" "$APPDIR/$APP_ID.desktop"
cp "$ROOT/assets/icon.png" "$APPDIR/$APP_ID.png"
ln -s "$APP_ID.png" "$APPDIR/.DirIcon"
ln -s usr/bin/flowie "$APPDIR/AppRun"
APPIMAGE="$OUT/$NAME.AppImage"
# Extract-and-run: CI runners have no FUSE to mount appimagetool's own AppImage.
APPIMAGE_EXTRACT_AND_RUN=1 ARCH="$ARCH" "$TOOL" --no-appstream "$APPDIR" "$APPIMAGE" >&2
echo "$APPIMAGE"
