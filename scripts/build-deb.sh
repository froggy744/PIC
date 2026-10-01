#!/usr/bin/env bash
set -euo pipefail

# Resolve the checkout independently of the invocation directory.
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

command -v cargo >/dev/null 2>&1 || { echo "ERROR: cargo is required."; exit 1; }
command -v python3 >/dev/null 2>&1 || { echo "ERROR: python3 is required."; exit 1; }

# Read package metadata from Cargo instead of hard-coding the version or binary.
readarray -t META < <(cargo metadata --no-deps --format-version 1 | python3 -c '
import json, sys
m=json.load(sys.stdin)
root=m["resolve"]["root"] if m.get("resolve") else m["packages"][0]["id"]
p=next(p for p in m["packages"] if p["id"] == root)
bins=[t["name"] for t in p["targets"] if "bin" in t["kind"]]
if not bins:
    raise SystemExit("No Cargo binary target found")
print(p["name"])
print(p["version"])
print(bins[0])
')
PACKAGE_NAME="${PACKAGE_NAME:-${META[0]}}"
VERSION="${VERSION:-${META[1]}}"
BIN_NAME="${BIN_NAME:-${META[2]}}"

APP_NAME="${APP_NAME:-PIC}"
APP_ID="${APP_ID:-io.github.you.PicRs}"
MAINTAINER="${MAINTAINER:-Petrus Rademeyer}"
DESCRIPTION="${DESCRIPTION:-PIC - a fast Picasa/iPhoto-inspired photo manager for Linux}"
HOMEPAGE="${HOMEPAGE:-https://github.com/froggy744/PIC}"

DIST="$ROOT/dist"
mkdir -p "$DIST"

echo "==> Building release binary"
cargo build --release --locked 2>/dev/null || cargo build --release

BINARY="$ROOT/target/release/$BIN_NAME"
[[ -x "$BINARY" ]] || { echo "ERROR: release binary not found: $BINARY"; exit 1; }

find_icon() {
    local size="$1"
    local candidate
    for candidate in         "$ROOT/icon/pic-${size}.png"         "$ROOT/icons/${size}x${size}.png"         "$ROOT/icons/${size}.png"         "$ROOT/data/icons/${size}x${size}/apps/${APP_ID}.png"         "$ROOT/data/icons/hicolor/${size}x${size}/apps/${APP_ID}.png"         "$ROOT/assets/icons/${size}x${size}.png"         "$ROOT/assets/icon-${size}.png"         "$ROOT/icon-${size}.png"; do
        [[ -f "$candidate" ]] && { printf '%s\n' "$candidate"; return 0; }
    done
    return 1
}

write_desktop() {
    local path="$1"
    cat > "$path" <<EOF
[Desktop Entry]
Type=Application
Name=${APP_NAME}
Comment=${DESCRIPTION}
Exec=${BIN_NAME}
Icon=${APP_ID}
Terminal=false
Categories=Graphics;Photography;
StartupNotify=true
StartupWMClass=${APP_ID}
EOF
}

command -v dpkg-deb >/dev/null 2>&1 || {
    echo "ERROR: dpkg-deb is required."
    echo "On Debian/Ubuntu: sudo apt install dpkg-dev"
    echo "On Fedora, build the DEB in a Debian/Ubuntu container or install dpkg tools."
    exit 1
}

ARCH="$(dpkg --print-architecture 2>/dev/null || echo amd64)"
STAGE="$ROOT/target/package-deb"
rm -rf "$STAGE"
mkdir -p     "$STAGE/DEBIAN"     "$STAGE/usr/bin"     "$STAGE/usr/share/applications"     "$STAGE/usr/share/metainfo"

install -Dm755 "$BINARY" "$STAGE/usr/bin/$BIN_NAME"

DESKTOP_SOURCE=""
for f in     "$ROOT/${APP_ID}.desktop"     "$ROOT/data/${APP_ID}.desktop"     "$ROOT/data/applications/${APP_ID}.desktop"     "$ROOT/linux/${APP_ID}.desktop"; do
    if [[ -f "$f" ]]; then DESKTOP_SOURCE="$f"; break; fi
done

if [[ -n "$DESKTOP_SOURCE" ]]; then
    install -Dm644 "$DESKTOP_SOURCE" "$STAGE/usr/share/applications/${APP_ID}.desktop"
else
    write_desktop "$STAGE/usr/share/applications/${APP_ID}.desktop"
fi

METAINFO_SOURCE=""
for f in     "$ROOT/${APP_ID}.metainfo.xml"     "$ROOT/data/${APP_ID}.metainfo.xml"     "$ROOT/data/metainfo/${APP_ID}.metainfo.xml"     "$ROOT/linux/${APP_ID}.metainfo.xml"; do
    if [[ -f "$f" ]]; then METAINFO_SOURCE="$f"; break; fi
done
if [[ -n "$METAINFO_SOURCE" ]]; then
    install -Dm644 "$METAINFO_SOURCE" "$STAGE/usr/share/metainfo/${APP_ID}.metainfo.xml"
fi

for size in 16 32 48 64 128 256 512; do
    if icon="$(find_icon "$size")"; then
        install -Dm644 "$icon"             "$STAGE/usr/share/icons/hicolor/${size}x${size}/apps/${APP_ID}.png"
    fi
done

# Native GTK/libadwaita runtime requirements. Rust-only crates are linked into
# the executable by Cargo; these are the main host GUI/runtime libraries.
cat > "$STAGE/DEBIAN/control" <<EOF
Package: ${PACKAGE_NAME}
Version: ${VERSION}
Section: graphics
Priority: optional
Architecture: ${ARCH}
Maintainer: ${MAINTAINER}
Depends: libgtk-4-1, libadwaita-1-0, libglib2.0-0
Homepage: ${HOMEPAGE}
Description: ${DESCRIPTION}
 Fast local photo browsing, albums, favourites, metadata, RAW support,
 thumbnail caching and fullscreen viewing.
EOF

INSTALLED_SIZE="$(du -sk "$STAGE" | awk '{print $1}')"
printf '\nInstalled-Size: %s\n' "$INSTALLED_SIZE" >> "$STAGE/DEBIAN/control"

OUT="$DIST/${PACKAGE_NAME}_${VERSION}_${ARCH}.deb"
rm -f "$OUT"
echo "==> Building $OUT"
dpkg-deb --root-owner-group --build "$STAGE" "$OUT"

echo
echo "DONE: $OUT"
echo "Test install:"
echo "  sudo apt install \"$OUT\""
echo
echo "Runtime linkage:"
ldd "$BINARY" | sed 's/^/  /' || true
