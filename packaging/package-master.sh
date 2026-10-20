#!/usr/bin/env bash
# The standalone master server (the internet server list; no game, no
# window): target/dist/opentt-master-<version>-<os>-<arch>.tar.gz, a .zip
# on Windows, with the binary, LICENSE, NOTICE and README.txt (how to run it).
#
# Usage: packaging/package-master.sh [--no-build]

source "$(dirname "${BASH_SOURCE[0]}")/common.sh"

BUILD=1
case "${1:-}" in
    "") ;;
    --no-build) BUILD=0 ;;
    -h | --help)
        sed -n '2,6p' "$0"
        exit 0
        ;;
    *) die "unknown argument $1" ;;
esac
case "$(uname -s)" in
    MINGW* | MSYS* | CYGWIN*) OS=windows SUFFIX=.exe ;;
    Darwin) OS=macos SUFFIX="" ;;
    *) OS=linux SUFFIX="" ;;
esac
[ "$BUILD" = 0 ] || (cd "$ROOT" && cargo build --release --locked -p tt-master)
BUILT="$ROOT/target/release/tt-master$SUFFIX"
[ -f "$BUILT" ] || die "$BUILT not built (drop --no-build)"

PKG="$NAME-master-$VERSION-$OS-$(uname -m)"
D="$DIST/$PKG"
rm -rf "$D" "$D.tar.gz" "$D.zip"
mkdir -p "$D"
cp "$BUILT" "$D/$NAME-master$SUFFIX"
cp "$(legal_file LICENSE)" "$(legal_file NOTICE)" "$D/"
cat >"$D/README.txt" <<TXT
OpenTT master server $VERSION: lists the internet games and introduces
players to hosts behind NAT. One UDP socket, nothing on disk, no window.

    $NAME-master [--port N] [--status SECONDS]

    --port N          UDP port (default 28100; open it in the firewall)
    --status SECONDS  print the counters this often (default 60, 0 = never)

It prints a line (UTC time) when a game server is added to the list or
dropped from it. Players enter this machine's HOST[:PORT] as the master
server (Play > On-line > Join > Internet); hosts tick "List on the master
server" or start a dedicated server with --master-server HOST[:PORT].
TXT
if [ "$OS" = windows ]; then
    zip_dist "$PKG"
else
    tar -C "$DIST" -czf "$D.tar.gz" "$PKG"
    rm -rf "$D"
    echo "wrote $D.tar.gz"
fi
