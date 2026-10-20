#!/usr/bin/env bash
# The Windows package: target/dist/opentt-<version>-windows-x86_64.zip
# with the game executable (renamed to opentt), LICENSE, NOTICE, README.md,
# the font licences and the example mods (mods/).
#
# Usage: packaging/package-windows.sh [--no-build] [--target TRIPLE]
#   On Windows (Git Bash) the default target is the native one (MSVC);
#   from Linux use packaging/cross-windows.sh. Needs zip, 7z or python3.

source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

BUILD=1
TARGET=""
while [ $# -gt 0 ]; do
    case "$1" in
        --no-build) BUILD=0 ;;
        --target)
            shift
            TARGET="${1:-}"
            ;;
        -h | --help)
            sed -n '2,9p' "$0"
            exit 0
            ;;
        *) die "unknown argument $1" ;;
    esac
    shift
done

if [ -n "$TARGET" ]; then
    OUT_DIR="$ROOT/target/$TARGET/release"
    [ "$BUILD" = 0 ] || build_game --target "$TARGET"
else
    OUT_DIR="$ROOT/target/release"
    [ "$BUILD" = 0 ] || build_game
fi
BUILT="$OUT_DIR/${BIN}.exe"
[ -f "$BUILT" ] || die "$BUILT not built"

PKG="$NAME-$VERSION-windows-x86_64"
D="$DIST/$PKG"
rm -rf "$D" "$D.zip"
mkdir -p "$D"
cp "$BUILT" "$D/${NAME}.exe"
stage_docs "$D"
stage_mods "$D/mods"
zip_dist "$PKG"
