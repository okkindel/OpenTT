#!/usr/bin/env bash
# Builds the Windows package from Linux: the x86_64-pc-windows-gnu target
# with mingw-w64, then packaging/package-windows.sh (target/dist/).
#
# Prerequisites (packaging/README.md):
#   rustup target add x86_64-pc-windows-gnu
#   mingw-w64 (Debian/Ubuntu: sudo apt install mingw-w64): the linker
#   x86_64-w64-mingw32-gcc and x86_64-w64-mingw32-windres (the icon and
#   version information; without it the executable has neither)
#
# Usage: packaging/cross-windows.sh

source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

TARGET=x86_64-pc-windows-gnu
MISSING=()
rustup target list --installed 2>/dev/null | grep -qx "$TARGET" || MISSING+=("the Rust target: rustup target add $TARGET")
command -v x86_64-w64-mingw32-gcc >/dev/null || MISSING+=("the mingw-w64 linker x86_64-w64-mingw32-gcc: sudo apt install mingw-w64")
if [ ${#MISSING[@]} -gt 0 ]; then
    echo "missing prerequisites:" >&2
    printf '  %s\n' "${MISSING[@]}" >&2
    exit 1
fi
command -v x86_64-w64-mingw32-windres >/dev/null || echo "warning: no x86_64-w64-mingw32-windres: the executable gets no icon or version information" >&2

export CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER="${CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER:-x86_64-w64-mingw32-gcc}"
exec "$ROOT/packaging/package-windows.sh" --target "$TARGET"
