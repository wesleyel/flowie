#!/usr/bin/env bash
# Build the Mac App Store package: universal binary (arm64 + x86_64), App
# Sandbox, privacy manifest, third-party notices, signed app and signed .pkg
# ready for Transporter.
#
# usage:
#   scripts/package-appstore.sh              signed build/appstore/Flowie.pkg
#   scripts/package-appstore.sh --local      sandboxed, ad-hoc signed app for testing (no .pkg)
#
# environment (signed build):
#   FLOWIE_BUNDLE_ID   default dev.flowie.app (must match the App ID in App Store Connect)
#   FLOWIE_PROFILE     Mac App Store provisioning profile (.provisionprofile); default packaging/macos/embedded.provisionprofile
#   FLOWIE_BUILD       build number (CFBundleVersion); default: git commit count
#   APP_IDENTITY       default: the first "Apple Distribution" / "3rd Party Mac Developer Application" identity
#   INSTALLER_IDENTITY default: the first "3rd Party Mac Developer Installer" identity
set -euo pipefail
cd "$(dirname "$0")/.."

LOCAL=0
[ "${1:-}" = "--local" ] && LOCAL=1

NAME=Flowie
BID="${FLOWIE_BUNDLE_ID:-dev.flowie.app}"
VERSION=$(cargo metadata --no-deps --format-version 1 | python3 -c 'import json,sys; print(next(p["version"] for p in json.load(sys.stdin)["packages"] if p["name"]=="flowie-app"))')
BUILD="${FLOWIE_BUILD:-$(git rev-list --count HEAD)}"
OUT=build/appstore
PKG_DIR=packaging/macos
APP="$OUT/$NAME.app"

echo "== $NAME $VERSION ($BUILD), $BID"

# ---------------------------------------------------------------- signing
if [ $LOCAL = 0 ]; then
    find_identity() { security find-identity -v "$@" | grep -E "$PATTERN" | head -1 | sed -E 's/^.*"(.*)"$/\1/'; }
    PATTERN='"(Apple Distribution|3rd Party Mac Developer Application): '
    APP_IDENTITY="${APP_IDENTITY:-$(find_identity -p codesigning)}"
    PATTERN='"3rd Party Mac Developer Installer: '
    INSTALLER_IDENTITY="${INSTALLER_IDENTITY:-$(find_identity)}"
    PROFILE="${FLOWIE_PROFILE:-$PKG_DIR/embedded.provisionprofile}"
    [ -n "$APP_IDENTITY" ] || { echo "no Apple Distribution signing identity in the keychain" >&2; exit 1; }
    [ -n "$INSTALLER_IDENTITY" ] || { echo "no 3rd Party Mac Developer Installer identity in the keychain" >&2; exit 1; }
    [ -f "$PROFILE" ] || { echo "missing provisioning profile: $PROFILE (Mac App Store distribution profile for $BID; or set FLOWIE_PROFILE)" >&2; exit 1; }
    TEAM=$(echo "$APP_IDENTITY" | sed -E 's/.*\(([A-Z0-9]+)\)$/\1/')
    # The profile must be for this team and bundle id.
    PROFILE_PLIST=$(mktemp)
    security cms -D -i "$PROFILE" > "$PROFILE_PLIST"
    PROFILE_APPID=$(/usr/libexec/PlistBuddy -c 'Print :Entitlements:com.apple.application-identifier' "$PROFILE_PLIST")
    rm -f "$PROFILE_PLIST"
    [ "$PROFILE_APPID" = "$TEAM.$BID" ] || { echo "profile is for $PROFILE_APPID, expected $TEAM.$BID" >&2; exit 1; }
    echo "   app: $APP_IDENTITY"
    echo "   pkg: $INSTALLER_IDENTITY"
fi

# ---------------------------------------------------------------- build
# Own target dir: cross builds next to stale host artifacts in target/ can
# fail to load proc macros (E0463).
TD=target/appstore
for t in aarch64-apple-darwin x86_64-apple-darwin; do
    CARGO_TARGET_DIR=$TD cargo build --release -p flowie-app --target "$t"
done
mkdir -p "$OUT"
lipo -create -output "$OUT/flowie" \
    $TD/aarch64-apple-darwin/release/flowie $TD/x86_64-apple-darwin/release/flowie
scripts/bundle-macos.sh "$OUT/flowie" "$NAME" "$BID" "$OUT" >/dev/null
rm -f "$OUT/flowie"

# ---------------------------------------------------------------- Info.plist
PL="$APP/Contents/Info.plist"
pb() { /usr/libexec/PlistBuddy -c "$1" "$PL"; }
pb "Set :CFBundleShortVersionString $VERSION"
pb "Set :CFBundleVersion $BUILD"
pb "Add :LSApplicationCategoryType string public.app-category.productivity"
pb "Add :ITSAppUsesNonExemptEncryption bool false"
pb "Add :NSHumanReadableCopyright string Copyright © $(date +%Y) Wesley Yang. All rights reserved."
pb "Add :CFBundleDevelopmentRegion string en"
pb "Add :CFBundleLocalizations array"
pb "Add :CFBundleLocalizations:0 string en"
pb "Add :CFBundleLocalizations:1 string zh-Hans"
pb "Add :CFBundleInfoDictionaryVersion string 6.0"
pb "Add :LSArchitecturePriority array"
pb "Add :LSArchitecturePriority:0 string arm64"
pb "Add :LSArchitecturePriority:1 string x86_64"
plutil -lint "$PL" >/dev/null

# ---------------------------------------------------------------- resources
cp "$PKG_DIR/PrivacyInfo.xcprivacy" "$APP/Contents/Resources/"
NOTICES="$APP/Contents/Resources/ThirdPartyNotices.txt"
cargo about generate -c "$PKG_DIR/about.toml" -m crates/flowie-app/Cargo.toml "$PKG_DIR/about.hbs" -o "$NOTICES" 2>/dev/null
FONTS_DIR=$(dirname "$(cargo metadata --format-version 1 | python3 -c 'import json,sys; print(next(p["manifest_path"] for p in json.load(sys.stdin)["packages"] if p["name"]=="ratex-katex-fonts"))')")/fonts
{
    printf '\n================================================================================\nKaTeX fonts (SIL Open Font License 1.1)\n--------------------------------------------------------------------------------\n'
    cat "$FONTS_DIR/FONT_NOTICE.txt" "$FONTS_DIR/OFL.txt"
    printf '\n================================================================================\nLucide icons\n--------------------------------------------------------------------------------\n'
    cat crates/flowie-app/assets/icons/LICENSE
} >> "$NOTICES"

# ---------------------------------------------------------------- sign
ENT="$(mktemp -d)/Flowie.entitlements"
cp "$PKG_DIR/Flowie.entitlements" "$ENT"
if [ $LOCAL = 1 ]; then
    codesign --force --options runtime --entitlements "$ENT" --sign - "$APP"
    rm -f "$ENT"
    codesign --verify --strict "$APP"
    echo "local sandboxed build: $APP"
    exit 0
fi
/usr/libexec/PlistBuddy -c "Add :com.apple.application-identifier string $TEAM.$BID" "$ENT"
/usr/libexec/PlistBuddy -c "Add :com.apple.developer.team-identifier string $TEAM" "$ENT"
cp "$PROFILE" "$APP/Contents/embedded.provisionprofile"
codesign --force --timestamp --options runtime --entitlements "$ENT" --sign "$APP_IDENTITY" "$APP"
rm -f "$ENT"
codesign --verify --strict --verbose=1 "$APP"

PKG="$OUT/$NAME-$VERSION-$BUILD.pkg"
productbuild --component "$APP" /Applications --sign "$INSTALLER_IDENTITY" "$PKG"
pkgutil --check-signature "$PKG" | head -3
echo "upload with Transporter: $PKG"
