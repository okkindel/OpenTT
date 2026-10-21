# Shared settings of the packaging scripts (sourced, not run).
#
# ROOT       the repository root
# BIN        the game binary's cargo name ([[bin]] of crates/tt-engine)
# NAME       the installed name of the binary and the package name: opentt
# APP_ID     the desktop / AppStream / macOS bundle id
# VERSION    $VERSION, else the workspace version of Cargo.toml
# DIST       where the packages are written: target/dist/

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
NAME=opentt
APP_ID=io.github.okkindel.OpenTT
BIN="$(sed -n '/^\[\[bin\]\]/,/^\[/ s/^name *= *"\(.*\)"/\1/p' "$ROOT/crates/tt-engine/Cargo.toml" | head -n1)"
if [ -z "${VERSION:-}" ]; then
    VERSION="$(sed -n '/^\[workspace.package\]/,/^\[/ s/^version *= *"\(.*\)"/\1/p' "$ROOT/Cargo.toml" | head -n1)"
fi
DIST="$ROOT/target/dist"
ICON_DIR="$ROOT/crates/tt-engine/icon"
ICON_SIZES="16 32 48 64 128 256"

die() {
    echo "error: $*" >&2
    exit 1
}

[ -n "$BIN" ] || die "no [[bin]] name in crates/tt-engine/Cargo.toml"
[ -n "$VERSION" ] || die "no version in the [workspace.package] of Cargo.toml"

# A legal file of the repository root (LICENSE, NOTICE); in a checkout that
# keeps them with the repository templates, from there.
legal_file() {
    local f
    for f in "$ROOT/$1"; do
        if [ -f "${f}" ]; then
            echo "${f}"
            return
        fi
    done
    die "$1 not found"
}

# Copies the documents every package ships into $1: LICENSE, NOTICE,
# README.md and the licence of each embedded font (fonts/<font>/OFL.txt).
stage_docs() {
    local out="$1" ofl font
    mkdir -p "${out}"
    cp "$(legal_file LICENSE)" "${out}/LICENSE"
    cp "$(legal_file NOTICE)" "${out}/NOTICE"
    cp "$ROOT/README.md" "${out}/README.md"
    for ofl in "$ROOT"/crates/tt-gui/assets/fonts/*/OFL.txt; do
        font="$(basename "$(dirname "${ofl}")")"
        mkdir -p "${out}/fonts/${font}"
        cp "${ofl}" "${out}/fonts/${font}/OFL.txt"
    done
}

# The example mods that ship with the game (mods/ of the repository,
# docs/modding.md § Example mods), copied into $1: the game lists them in
# Options → Mods, not enabled. Each package puts them where the game looks
# (tt_assets::layout::installed_mod_dirs): next to the binary (tar.gz,
# zip), share/opentt/mods (AppImage; the .deb lists them in
# [package.metadata.deb]), Contents/Resources/mods (.app).
EXAMPLE_MODS="carpet-bombing desert-camo"
stage_mods() {
    local out="$1" m
    mkdir -p "${out}"
    for m in $EXAMPLE_MODS; do
        [ -f "$ROOT/mods/${m}/mod.ron" ] || die "the example mod mods/${m} is missing"
        cp -R "$ROOT/mods/${m}" "${out}/${m}"
    done
}

# Packs the folder $DIST/$1 into $DIST/$1.zip (zip, 7z or python3) and
# removes the folder.
zip_dist() {
    if command -v zip >/dev/null; then
        (cd "$DIST" && zip -qr "$1.zip" "$1")
    elif command -v 7z >/dev/null; then
        (cd "$DIST" && 7z a -tzip -bso0 "$1.zip" "$1")
    elif command -v python3 >/dev/null; then
        (cd "$DIST" && python3 -m zipfile -c "$1.zip" "$1")
    else
        die "no zip, 7z or python3 to write the archive"
    fi
    rm -rf "${DIST:?}/$1"
    echo "wrote $DIST/$1.zip"
}

# The cargo build of the game (release, the lock file as it is, without the
# developer tooling of the `dev` feature).
build_game() {
    (cd "$ROOT" && cargo build --release --locked -p tt-engine --no-default-features --bin "$BIN" "$@")
}
