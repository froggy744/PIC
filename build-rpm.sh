#!/usr/bin/env bash
set -euo pipefail

# Run from the repository root.
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
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
APP_ID="${APP_ID:-io.github.you.PicasaRs}"
MAINTAINER="${MAINTAINER:-Petrus Rademeyer}"
DESCRIPTION="${DESCRIPTION:-PIC - a fast Picasa/iPhoto-inspired photo manager for Linux}"
HOMEPAGE="${HOMEPAGE:-https://github.com/froggy744/PIC}"

DIST="$ROOT/dist"
mkdir -p "$DIST"

if [[ -x "$ROOT/build-linux-icons.sh" ]]; then
    echo "==> Building Linux icons"
    "$ROOT/build-linux-icons.sh"
fi

echo "==> Building release binary"
cargo build --release --locked 2>/dev/null || cargo build --release

BINARY="$ROOT/target/release/$BIN_NAME"
[[ -x "$BINARY" ]] || { echo "ERROR: release binary not found: $BINARY"; exit 1; }

find_icon() {
    local size="$1"
    local candidate
    for candidate in         "$ROOT/icons/${size}x${size}.png"         "$ROOT/icons/${size}.png"         "$ROOT/data/icons/${size}x${size}/apps/${APP_ID}.png"         "$ROOT/data/icons/hicolor/${size}x${size}/apps/${APP_ID}.png"         "$ROOT/assets/icons/${size}x${size}.png"         "$ROOT/assets/icon-${size}.png"         "$ROOT/icon-${size}.png"; do
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

command -v rpmbuild >/dev/null 2>&1 || {
    echo "ERROR: rpmbuild is required."
    echo "On Fedora: sudo dnf install rpm-build"
    exit 1
}

case "$(uname -m)" in
    x86_64) RPM_ARCH="x86_64" ;;
    aarch64|arm64) RPM_ARCH="aarch64" ;;
    *) RPM_ARCH="$(uname -m)" ;;
esac

TOPDIR="$ROOT/target/rpmbuild"
rm -rf "$TOPDIR"
mkdir -p "$TOPDIR"/{BUILD,BUILDROOT,RPMS,SOURCES,SPECS,SRPMS}

PAYLOAD="$TOPDIR/SOURCES/payload"
mkdir -p     "$PAYLOAD/usr/bin"     "$PAYLOAD/usr/share/applications"     "$PAYLOAD/usr/share/metainfo"

install -Dm755 "$BINARY" "$PAYLOAD/usr/bin/$BIN_NAME"

DESKTOP_SOURCE=""
for f in     "$ROOT/${APP_ID}.desktop"     "$ROOT/data/${APP_ID}.desktop"     "$ROOT/data/applications/${APP_ID}.desktop"     "$ROOT/linux/${APP_ID}.desktop"; do
    if [[ -f "$f" ]]; then DESKTOP_SOURCE="$f"; break; fi
done

if [[ -n "$DESKTOP_SOURCE" ]]; then
    install -Dm644 "$DESKTOP_SOURCE" "$PAYLOAD/usr/share/applications/${APP_ID}.desktop"
else
    write_desktop "$PAYLOAD/usr/share/applications/${APP_ID}.desktop"
fi

METAINFO_SOURCE=""
for f in     "$ROOT/${APP_ID}.metainfo.xml"     "$ROOT/data/${APP_ID}.metainfo.xml"     "$ROOT/data/metainfo/${APP_ID}.metainfo.xml"     "$ROOT/linux/${APP_ID}.metainfo.xml"; do
    if [[ -f "$f" ]]; then METAINFO_SOURCE="$f"; break; fi
done
if [[ -n "$METAINFO_SOURCE" ]]; then
    install -Dm644 "$METAINFO_SOURCE" "$PAYLOAD/usr/share/metainfo/${APP_ID}.metainfo.xml"
fi

for size in 16 32 48 64 128 256 512; do
    if icon="$(find_icon "$size")"; then
        install -Dm644 "$icon"             "$PAYLOAD/usr/share/icons/hicolor/${size}x${size}/apps/${APP_ID}.png"
    fi
done

SPEC="$TOPDIR/SPECS/${PACKAGE_NAME}.spec"
cat > "$SPEC" <<EOF
Name:           ${PACKAGE_NAME}
Version:        ${VERSION}
Release:        1%{?dist}
Summary:        ${DESCRIPTION}
License:        GPL-3.0-or-later
URL:            ${HOMEPAGE}
BuildArch:      ${RPM_ARCH}

Requires:       gtk4
Requires:       libadwaita
Requires:       glib2

%description
${DESCRIPTION}

%install
rm -rf %{buildroot}
cp -a ${PAYLOAD}/. %{buildroot}/

%files
/usr/bin/${BIN_NAME}
/usr/share/applications/${APP_ID}.desktop
%{_datadir}/icons/hicolor/*/apps/${APP_ID}.png
EOF

if [[ -f "$PAYLOAD/usr/share/metainfo/${APP_ID}.metainfo.xml" ]]; then
    echo "/usr/share/metainfo/${APP_ID}.metainfo.xml" >> "$SPEC"
fi

cat >> "$SPEC" <<EOF

%changelog
* $(LC_ALL=C date '+%a %b %d %Y') ${MAINTAINER} - ${VERSION}-1
- Automated local build
EOF

echo "==> Building RPM"
rpmbuild     --define "_topdir $TOPDIR"     --define "_build_id_links none"     -bb "$SPEC"

RPM_FILE="$(find "$TOPDIR/RPMS" -type f -name '*.rpm' | head -n1)"
[[ -n "$RPM_FILE" ]] || { echo "ERROR: RPM was not produced."; exit 1; }

OUT="$DIST/$(basename "$RPM_FILE")"
cp -f "$RPM_FILE" "$OUT"

echo
echo "DONE: $OUT"
echo "Test install:"
echo "  sudo dnf install \"$OUT\""
echo
echo "Runtime linkage:"
ldd "$BINARY" | sed 's/^/  /' || true
