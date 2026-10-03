#!/usr/bin/env bash
# Builds Gitr.app (and optionally a .dmg) for macOS.
#
#   ./bundle.sh               release build for this Mac's architecture -> dist/Gitr.app
#   ./bundle.sh --universal   Apple Silicon + Intel universal binary
#   ./bundle.sh --dmg         also create dist/Gitr-<version>.dmg
#
# Environment:
#   BUNDLE_ID      bundle identifier (default: io.github.gitr)
#   SIGN_IDENTITY  codesign identity (default: "-" = ad-hoc; use your
#                  "Developer ID Application: ..." identity for distribution)
set -euo pipefail

cd "$(dirname "$0")"

if [[ "$(uname)" != "Darwin" ]]; then
    echo "bundle.sh only runs on macOS" >&2
    exit 1
fi

UNIVERSAL=0
DMG=0
for arg in "$@"; do
    case "$arg" in
        --universal) UNIVERSAL=1 ;;
        --dmg) DMG=1 ;;
        -h|--help) sed -n '2,12p' "$0"; exit 0 ;;
        *) echo "unknown option: $arg" >&2; exit 1 ;;
    esac
done

APP_NAME="Gitr"      # bundle / display name
EXEC_NAME="gitr"     # executable (cargo binary) name
BUNDLE_ID="${BUNDLE_ID:-io.github.gitr}"
SIGN_IDENTITY="${SIGN_IDENTITY:--}"
VERSION="$(sed -n 's/^version *= *"\(.*\)"/\1/p' Cargo.toml | head -1)"
DIST="dist"
APP="$DIST/$APP_NAME.app"

echo "==> Building $APP_NAME $VERSION"
rm -rf "$DIST/gitr.app"  # bundle name before the rename
if [[ $UNIVERSAL == 1 ]]; then
    rustup target add aarch64-apple-darwin x86_64-apple-darwin >/dev/null
    cargo build --release --target aarch64-apple-darwin
    cargo build --release --target x86_64-apple-darwin
    BINARY="$DIST/$EXEC_NAME-universal"
    mkdir -p "$DIST"
    lipo -create -output "$BINARY" \
        "target/aarch64-apple-darwin/release/$EXEC_NAME" \
        "target/x86_64-apple-darwin/release/$EXEC_NAME"
else
    cargo build --release
    BINARY="target/release/$EXEC_NAME"
fi

echo "==> Creating icon"
ICONSET="$(mktemp -d)/AppIcon.iconset"
mkdir -p "$ICONSET"
for size in 16 32 128 256 512; do
    sips -z "$size" "$size" assets/icon-1024.png --out "$ICONSET/icon_${size}x${size}.png" >/dev/null
    double=$((size * 2))
    sips -z "$double" "$double" assets/icon-1024.png --out "$ICONSET/icon_${size}x${size}@2x.png" >/dev/null
done

echo "==> Assembling $APP"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$BINARY" "$APP/Contents/MacOS/$EXEC_NAME"
iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/AppIcon.icns"
rm -rf "$(dirname "$ICONSET")"

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key>                  <string>$APP_NAME</string>
    <key>CFBundleDisplayName</key>           <string>$APP_NAME</string>
    <key>CFBundleIdentifier</key>            <string>$BUNDLE_ID</string>
    <key>CFBundleExecutable</key>            <string>$EXEC_NAME</string>
    <key>CFBundleIconFile</key>              <string>AppIcon</string>
    <key>CFBundlePackageType</key>           <string>APPL</string>
    <key>CFBundleVersion</key>               <string>$VERSION</string>
    <key>CFBundleShortVersionString</key>    <string>$VERSION</string>
    <key>CFBundleInfoDictionaryVersion</key> <string>6.0</string>
    <key>LSMinimumSystemVersion</key>        <string>11.0</string>
    <key>NSHumanReadableCopyright</key>      <string>Copyright © $(date +%Y) ozkl. MIT License.</string>
    <key>LSApplicationCategoryType</key>     <string>public.app-category.developer-tools</string>
    <key>NSHighResolutionCapable</key>       <true/>
    <key>NSSupportsAutomaticGraphicsSwitching</key> <true/>
    <key>NSRequiresAquaSystemAppearance</key> <false/>
</dict>
</plist>
PLIST

echo "==> Signing ($SIGN_IDENTITY)"
codesign --force --deep --options runtime --sign "$SIGN_IDENTITY" "$APP"
codesign --verify --strict "$APP"

if [[ $DMG == 1 ]]; then
    DMG_PATH="$DIST/$APP_NAME-$VERSION.dmg"
    echo "==> Creating $DMG_PATH"
    STAGE="$(mktemp -d)"
    cp -R "$APP" "$STAGE/"
    ln -s /Applications "$STAGE/Applications"
    rm -f "$DMG_PATH"
    hdiutil create -volname "$APP_NAME" -srcfolder "$STAGE" -ov -format UDZO "$DMG_PATH" >/dev/null
    rm -rf "$STAGE"
fi

echo "==> Done: $APP"
