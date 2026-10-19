#!/usr/bin/env bash
# Linux packages of the game, written to target/dist/:
#
#   opentt-<version>-linux-x86_64.tar.gz   portable: the binary, licences,
#                                          example mods (mods/),
#                                          desktop entry and icons
#   opentt_<version>-1_amd64.deb           cargo-deb ([package.metadata.deb]
#                                          of crates/tt-engine/Cargo.toml)
#   opentt-<version>-linux-x86_64.AppImage appimagetool
#
# Usage: packaging/package-linux.sh [--no-build] [tar] [deb] [appimage]
#   (no kinds = all three). Prerequisites: packaging/README.md.

source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

BUILD=1
KINDS=()
for A in "$@"; do
    case "$A" in
        --no-build) BUILD=0 ;;
        tar | deb | appimage) KINDS+=("$A") ;;
        -h | --help)
            sed -n '2,13p' "$0"
            exit 0
            ;;
        *) die "unknown argument $A" ;;
    esac
done
[ ${#KINDS[@]} -gt 0 ] || KINDS=(tar deb appimage)

ARCH="$(uname -m)"
BUILT="$ROOT/target/release/$BIN"
[ "$BUILD" = 0 ] || build_game
[ -x "$BUILT" ] || die "$BUILT not built (drop --no-build)"
mkdir -p "$DIST"

# The desktop integration files under $1/share ($1: a usr/-like prefix).
stage_desktop() {
    local P="$1" S
    install -Dm644 "$ROOT/packaging/linux/$APP_ID.desktop" "$P/share/applications/$APP_ID.desktop"
    install -Dm644 "$ROOT/packaging/linux/$APP_ID.metainfo.xml" "$P/share/metainfo/$APP_ID.metainfo.xml"
    for S in $ICON_SIZES; do
        install -Dm644 "$ICON_DIR/opentt-$S.png" "$P/share/icons/hicolor/${S}x${S}/apps/$APP_ID.png"
    done
}

package_tar() {
    local D="$DIST/$NAME-$VERSION-linux-$ARCH"
    rm -rf "$D"
    install -Dm755 "$BUILT" "$D/$NAME"
    stage_docs "$D"
    stage_desktop "$D"
    stage_mods "$D/mods"
    tar -C "$DIST" -czf "$D.tar.gz" "$(basename "$D")"
    rm -rf "$D"
    echo "wrote $D.tar.gz"
}

package_deb() {
    command -v cargo-deb >/dev/null || die "cargo-deb is missing: cargo install cargo-deb"
    (cd "$ROOT" && cargo deb -p tt-engine --no-build --deb-version "$VERSION-1" -o "$DIST/")
}

package_appimage() {
    local TOOL="${APPIMAGETOOL:-$(command -v appimagetool || true)}"
    [ -n "$TOOL" ] || die "appimagetool is missing: set APPIMAGETOOL or put it on PATH (packaging/README.md)"
    local D="$ROOT/target/AppDir"
    rm -rf "$D"
    install -Dm755 "$BUILT" "$D/usr/bin/$NAME"
    stage_desktop "$D/usr"
    stage_docs "$D/usr/share/doc/$NAME"
    stage_mods "$D/usr/share/$NAME/mods"
    cp "$ROOT/packaging/linux/$APP_ID.desktop" "$D/$APP_ID.desktop"
    cp "$ICON_DIR/opentt-256.png" "$D/$APP_ID.png"
    ln -s "$APP_ID.png" "$D/.DirIcon"
    ln -s "usr/bin/$NAME" "$D/AppRun"
    local OUT="$DIST/$NAME-$VERSION-linux-$ARCH.AppImage"
    # FUSE is not needed to run the tool itself.
    ARCH="$ARCH" APPIMAGE_EXTRACT_AND_RUN=1 "$TOOL" --no-appstream "$D" "$OUT"
    echo "wrote $OUT"
}

for K in "${KINDS[@]}"; do
    "package_$K"
done
