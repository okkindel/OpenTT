#!/usr/bin/env bash
# The macOS package (run on a Mac): OpenTT.app in a disk image,
# target/dist/opentt-<version>-macos-<arch>.dmg. The app is not signed with
# a developer certificate (ad-hoc signature only): packaging/README.md says
# how to open it.
#
# Usage: packaging/package-macos.sh [--no-build] [--universal]
#   --universal  one binary for Apple silicon and Intel (aarch64 + x86_64,
#                joined with lipo; both Rust targets must be installed);
#                else the native architecture.

source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

BUILD=1
UNIVERSAL=0
for A in "$@"; do
    case "$A" in
        --no-build) BUILD=0 ;;
        --universal) UNIVERSAL=1 ;;
        -h | --help)
            sed -n '2,10p' "$0"
            exit 0
            ;;
        *) die "unknown argument $A" ;;
    esac
done
[ "$(uname -s)" = Darwin ] || die "this script runs on macOS (it needs lipo, codesign and hdiutil)"

if [ "$UNIVERSAL" = 1 ]; then
    ARCH=universal
    BUILT="$ROOT/target/universal/$BIN"
    if [ "$BUILD" = 1 ]; then
        build_game --target aarch64-apple-darwin
        build_game --target x86_64-apple-darwin
    fi
    mkdir -p "$(dirname "$BUILT")"
    lipo -create -output "$BUILT" "$ROOT/target/aarch64-apple-darwin/release/$BIN" "$ROOT/target/x86_64-apple-darwin/release/$BIN"
else
    ARCH="$(uname -m)"
    BUILT="$ROOT/target/release/$BIN"
    [ "$BUILD" = 0 ] || build_game
fi
[ -x "$BUILT" ] || die "$BUILT not built"

STAGE="$DIST/macos-dmg"
APP="$STAGE/OpenTT.app"
rm -rf "$STAGE"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
sed "s/@VERSION@/$VERSION/g" "$ROOT/packaging/macos/Info.plist" >"$APP/Contents/Info.plist"
printf 'APPL????' >"$APP/Contents/PkgInfo"
install -m755 "$BUILT" "$APP/Contents/MacOS/$NAME"
cp "$ICON_DIR/opentt.icns" "$APP/Contents/Resources/opentt.icns"
stage_docs "$APP/Contents/Resources"
stage_mods "$APP/Contents/Resources/mods"
codesign --force --deep --sign - "$APP"

stage_docs "$STAGE"
ln -s /Applications "$STAGE/Applications"
OUT="$DIST/$NAME-$VERSION-macos-$ARCH.dmg"
rm -f "$OUT"
hdiutil create -volname OpenTT -srcfolder "$STAGE" -ov -format UDZO "$OUT"
rm -rf "$STAGE"
echo "wrote $OUT"
