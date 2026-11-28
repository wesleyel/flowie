#!/usr/bin/env bash
# Build the iOS app for the App Store: archive apple/FlowieApp (the app and its Control Center
# and share extensions, Release configuration: iCloud Production, push production) and export a signed
# .ipa. Uploading to App Store Connect (TestFlight) only happens with --upload.
#
# usage:
#   scripts/package-ios.sh             build/ios/appstore/Flowie-<version>-<build>.ipa
#   scripts/package-ios.sh --upload    the same, then upload it to App Store Connect
#
# Signing is automatic with the team in apple/FlowieApp/Project.swift: Xcode makes or updates
# the App Store profiles for flowie.spokey.cn, flowie.spokey.cn.JournalControl and
# flowie.spokey.cn.Share. The three App IDs and the App Group group.flowie.spokey.cn (on the app
# and the share extension) must be registered first (appstore/README.md).
#
# environment:
#   FLOWIE_BUILD   build number (CFBundleVersion); default: git commit count, like the Mac build
#   FLOWIE_TEAM    default 5D26F33GNM
#   ASC_KEY_ID, ASC_ISSUER_ID, ASC_KEY_PATH
#                  App Store Connect API key (read from appstore/fastlane/.env when set there).
#                  Lets xcodebuild manage profiles and upload without an Xcode account; without
#                  it, the account signed in to Xcode is used.
set -euo pipefail
cd "$(dirname "$0")/.."

UPLOAD=0
case "${1:-}" in
    "") ;;
    --upload) UPLOAD=1 ;;
    *) echo "usage: $0 [--upload]" >&2; exit 2 ;;
esac

NAME=Flowie
BID=flowie.spokey.cn
TEAM="${FLOWIE_TEAM:-5D26F33GNM}"
# The same version as the desktop (the workspace's, in Cargo.toml) and the same build number scheme.
VERSION=$(cargo metadata --no-deps --format-version 1 | python3 -c 'import json,sys; print(next(p["version"] for p in json.load(sys.stdin)["packages"] if p["name"]=="flowie-app"))')
BUILD="${FLOWIE_BUILD:-$(git rev-list --count HEAD)}"
OUT=build/ios/appstore
ARCHIVE="$OUT/$NAME-$VERSION-$BUILD.xcarchive"
IPA="$OUT/$NAME-$VERSION-$BUILD.ipa"

# The build number is the commit count, so an upload must be of a committed tree.
if [ $UPLOAD = 1 ] && [ -n "$(git status --porcelain)" ]; then
    echo "the working tree has changes; commit them before uploading" >&2
    exit 1
fi

# App Store Connect API key, as the fastlane lanes use it.
if [ -f appstore/fastlane/.env ]; then
    set -a
    # shellcheck disable=SC1091
    . appstore/fastlane/.env
    set +a
fi
AUTH=()
if [ -n "${ASC_KEY_ID:-}" ] && [ -n "${ASC_ISSUER_ID:-}" ] && [ -n "${ASC_KEY_PATH:-}" ]; then
    KEY_PATH="${ASC_KEY_PATH/#\~/$HOME}"
    [ -f "$KEY_PATH" ] || { echo "ASC_KEY_PATH points at nothing: $ASC_KEY_PATH" >&2; exit 1; }
    AUTH=(-authenticationKeyPath "$KEY_PATH" -authenticationKeyID "$ASC_KEY_ID" -authenticationKeyIssuerID "$ASC_ISSUER_ID")
fi

echo "== $NAME for iOS $VERSION ($BUILD), $BID, team $TEAM"

# ---------------------------------------------------------------- build
scripts/build-ios-ffi.sh
(cd apple/FlowieApp && tuist generate --no-open)
rm -rf "$ARCHIVE" "$IPA"
mkdir -p "$OUT"
xcodebuild archive \
    -workspace apple/FlowieApp/Flowie.xcworkspace -scheme Flowie -configuration Release \
    -destination 'generic/platform=iOS' -archivePath "$ARCHIVE" \
    -derivedDataPath build/ios/DerivedData-appstore \
    -allowProvisioningUpdates ${AUTH[@]+"${AUTH[@]}"} \
    MARKETING_VERSION="$VERSION" CURRENT_PROJECT_VERSION="$BUILD" DEVELOPMENT_TEAM="$TEAM"

# ---------------------------------------------------------------- export
export_options() { # destination: export (an .ipa here) or upload (to App Store Connect)
    local plist="$OUT/ExportOptions-$1.plist"
    rm -f "$plist"
    pb() { /usr/libexec/PlistBuddy -c "$1" "$plist" >/dev/null; }
    pb "Add :method string app-store-connect"
    pb "Add :destination string $1"
    pb "Add :teamID string $TEAM"
    pb "Add :signingStyle string automatic"
    pb "Add :uploadSymbols bool true"
    # The version and build number are this script's; App Store Connect mustn't change them.
    pb "Add :manageAppVersionAndBuildNumber bool false"
    echo "$plist"
}
EXPORT_DIR=$(mktemp -d)
xcodebuild -exportArchive -archivePath "$ARCHIVE" -exportPath "$EXPORT_DIR" \
    -exportOptionsPlist "$(export_options export)" -allowProvisioningUpdates ${AUTH[@]+"${AUTH[@]}"}
mv "$EXPORT_DIR/$NAME.ipa" "$IPA"
rm -rf "$EXPORT_DIR"

# ---------------------------------------------------------------- check
# What App Review and iCloud need of the signed app: production environments, the versions
# above in the app and its extensions, the privacy manifest and the icon.
CHECK=$(mktemp -d)
unzip -q "$IPA" -d "$CHECK"
APP="$CHECK/Payload/$NAME.app"
fail() { echo "check failed: $*" >&2; exit 1; }
plist() { /usr/libexec/PlistBuddy -c "Print :$2" "$1" 2>/dev/null; }
for bundle in "$APP" "$APP/PlugIns/JournalControl.appex" "$APP/PlugIns/Share.appex"; do
    [ "$(plist "$bundle/Info.plist" CFBundleShortVersionString)" = "$VERSION" ] || fail "$bundle isn't version $VERSION"
    [ "$(plist "$bundle/Info.plist" CFBundleVersion)" = "$BUILD" ] || fail "$bundle isn't build $BUILD"
done
ENT="$CHECK/app.entitlements"
codesign -d --entitlements - --xml "$APP" > "$ENT" 2>/dev/null
[ "$(plist "$ENT" aps-environment)" = production ] || fail "aps-environment isn't production"
[ "$(plist "$ENT" com.apple.developer.icloud-container-environment)" = Production ] || fail "the iCloud environment isn't Production"
plist "$ENT" com.apple.developer.icloud-container-identifiers | grep -q iCloud.flowie.spokey.cn || fail "no iCloud container"
plist "$ENT" com.apple.security.application-groups | grep -q group.flowie.spokey.cn || fail "no App Group"
codesign -d --entitlements - --xml "$APP/PlugIns/Share.appex" > "$ENT" 2>/dev/null
plist "$ENT" com.apple.security.application-groups | grep -q group.flowie.spokey.cn || fail "no App Group in the share extension"
for bundle in "$APP" "$APP/PlugIns/JournalControl.appex" "$APP/PlugIns/Share.appex"; do
    [ -f "$bundle/PrivacyInfo.xcprivacy" ] || fail "no privacy manifest in $bundle"
done
[ "$(plist "$APP/Info.plist" ITSAppUsesNonExemptEncryption)" = false ] || fail "ITSAppUsesNonExemptEncryption isn't false"
[ "$(plist "$APP/Info.plist" CFBundleIcons:CFBundlePrimaryIcon:CFBundleIconName)" = AppIcon ] || fail "no app icon"
codesign --verify --strict --deep "$APP" || fail "the signature doesn't verify"
rm -rf "$CHECK"
echo "built $IPA"

# ---------------------------------------------------------------- upload
if [ $UPLOAD = 1 ]; then
    xcodebuild -exportArchive -archivePath "$ARCHIVE" -exportPath "$OUT/upload" \
        -exportOptionsPlist "$(export_options upload)" -allowProvisioningUpdates ${AUTH[@]+"${AUTH[@]}"}
    echo "uploaded $VERSION ($BUILD); App Store Connect is processing it, then it shows in TestFlight"
else
    echo "to upload it to App Store Connect (TestFlight): $0 --upload"
fi
