#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"

cache="${PIC_BUILD_CACHE:-${XDG_CACHE_HOME:-$HOME/.cache}/pic-linux-build}"
arch="$(uname -m)"

tools_dir="$cache/tools"
sources_dir="$cache/flatpak-sources"

mkdir -p "$tools_dir"
mkdir -p "$sources_dir"

download_file() {
    local url="$1"
    local dest="$2"

    local dir
    local name

    dir="$(dirname "$dest")"
    name="$(basename "$dest")"

    mkdir -p "$dir"

    if command -v aria2c >/dev/null 2>&1; then
        aria2c \
            --continue=true \
            --max-connection-per-server=16 \
            --split=16 \
            --min-split-size=1M \
            --file-allocation=none \
            --max-tries=5 \
            --retry-wait=2 \
            --timeout=30 \
            --connect-timeout=15 \
            --auto-file-renaming=false \
            --allow-overwrite=true \
            --dir="$dir" \
            --out="$name" \
            "$url"
    else
        echo "WARNING: aria2c not found. Falling back to curl."
        echo "Install aria2 for faster downloads:"
        echo "  sudo dnf install aria2"
        echo

        curl \
            -fL \
            --retry 5 \
            --retry-delay 2 \
            --retry-all-errors \
            --connect-timeout 15 \
            --continue-at - \
            "$url" \
            -o "$dest"
    fi
}

echo
echo "=========================================="
echo "PIC Linux build dependency downloader"
echo "=========================================="
echo "Cache: $cache"
echo "Architecture: $arch"
echo

#
# linuxdeploy
#

linuxdeploy_name="linuxdeploy-${arch}.AppImage"
linuxdeploy_dest="$tools_dir/$linuxdeploy_name"
linuxdeploy_url="https://github.com/linuxdeploy/linuxdeploy/releases/download/continuous/$linuxdeploy_name"

echo "=========================================="
echo "Downloading: $linuxdeploy_name"
echo "URL: $linuxdeploy_url"
echo "=========================================="

if [[ -s "$linuxdeploy_dest" ]]; then
    echo "Cached: $linuxdeploy_dest"
else
    download_file "$linuxdeploy_url" "$linuxdeploy_dest"
fi

chmod +x "$linuxdeploy_dest"

echo
echo "linuxdeploy ready:"
echo "  $linuxdeploy_dest"
echo

#
# Flatpak module sources
#

while IFS='|' read -r name sha url; do
    [[ -z "${name:-}" ]] && continue
    [[ -z "${sha:-}" ]] && continue
    [[ -z "${url:-}" ]] && continue

    dest="$sources_dir/$sha/$name"

    echo
    echo "=========================================="
    echo "Downloading: $name"
    echo "URL: $url"
    echo "=========================================="

    #
    # Already downloaded and checksum is valid.
    #
    if [[ -f "$dest" ]] &&
       printf '%s  %s\n' "$sha" "$dest" | sha256sum --check --status
    then
        echo "Cached OK: $name"
        continue
    fi

    #
    # Remove a corrupt completed file.
    #
    # aria2 may still have a .aria2 control file for resumable downloads,
    # so only remove the destination itself here.
    #
    if [[ -f "$dest" ]]; then
        echo "Cached file failed SHA256 check."
        echo "Removing corrupt file: $dest"
        rm -f "$dest"
    fi

    download_file "$url" "$dest"

    echo
    echo "Verifying SHA256..."

    if ! printf '%s  %s\n' "$sha" "$dest" | sha256sum --check -
    then
        echo
        echo "ERROR: SHA256 verification failed:"
        echo "  $dest"
        rm -f "$dest"
        exit 1
    fi

done < <(
    sed -n '/^FLATPAK_MODULE_SOURCES=(/,/^)/p' "$SCRIPT_DIR/PIC-build-linux-one-script.sh" |
    sed -n 's/^[[:space:]]*"\([^"]*\)".*/\1/p'
)

echo
echo "=========================================="
echo "All downloads completed successfully."
echo "=========================================="
echo
echo "Cache directory:"
echo "  $cache"
echo
