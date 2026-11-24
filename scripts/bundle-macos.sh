#!/usr/bin/env bash
# Wrap a built binary into a minimal macOS .app bundle.
# usage: scripts/bundle-macos.sh <binary> <AppName> <bundle.id> [outdir]
set -euo pipefail
BIN="$1"; NAME="$2"; BID="$3"; OUT="${4:-build}"
APP="$OUT/$NAME.app"
# The marketing version is the workspace version in Cargo.toml, the same one the app shows
# in Settings → About and App Store Connect lists; package-appstore.sh sets the build number.
VERSION="${FLOWIE_VERSION:-$(cargo metadata --no-deps --format-version 1 \
  | python3 -c 'import json,sys; print(next(p["version"] for p in json.load(sys.stdin)["packages"] if p["name"]=="flowie-app"))')}"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$BIN" "$APP/Contents/MacOS/$NAME"
[ -f assets/AppIcon.icns ] && cp assets/AppIcon.icns "$APP/Contents/Resources/"
# Flowie opens share packages (.flowie, a zip): a double-click goes to the running app.
DOCUMENT_TYPES=""
if [ "$NAME" = Flowie ]; then
DOCUMENT_TYPES='  <key>CFBundleDocumentTypes</key><array><dict>
    <key>CFBundleTypeName</key><string>Flowie Package</string>
    <key>CFBundleTypeRole</key><string>Viewer</string>
    <key>LSHandlerRank</key><string>Owner</string>
    <key>LSItemContentTypes</key><array><string>cn.spokey.flowie.package</string></array>
  </dict></array>
  <key>UTExportedTypeDeclarations</key><array><dict>
    <key>UTTypeIdentifier</key><string>cn.spokey.flowie.package</string>
    <key>UTTypeDescription</key><string>Flowie Package</string>
    <key>UTTypeConformsTo</key><array><string>public.zip-archive</string><string>public.data</string></array>
    <key>UTTypeTagSpecification</key><dict>
      <key>public.filename-extension</key><array><string>flowie</string></array>
      <key>public.mime-type</key><array><string>application/x-flowie-package</string></array>
    </dict>
  </dict></array>'
fi
cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleName</key><string>$NAME</string>
  <key>CFBundleDisplayName</key><string>$NAME</string>
  <key>CFBundleIdentifier</key><string>$BID</string>
  <key>CFBundleExecutable</key><string>$NAME</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>CFBundleVersion</key><string>1</string>
  <key>CFBundleIconFile</key><string>AppIcon</string>
  <key>LSMinimumSystemVersion</key><string>13.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSSupportsAutomaticGraphicsSwitching</key><true/>
$DOCUMENT_TYPES
</dict></plist>
PLIST
codesign --force --deep --sign - "$APP" >/dev/null 2>&1 || true
echo "$APP"
