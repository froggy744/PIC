#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
REPO_ROOT="$(cd -- "$SCRIPT_DIR/.." && pwd -P)"
if [[ $# -ne 1 || ! -f "$1" ]]; then
    echo "Usage: $0 SOURCE_IMAGE (existing original artwork)" >&2
    exit 2
fi
SOURCE="$1"
OUTPUT="$REPO_ROOT/icon"

mkdir -p "$OUTPUT"

# Generate PNG sizes
for size in 16 24 32 48 64 128 256 512 1024; do

    magick "$SOURCE" \
        -resize "${size}x${size}" \
        "$OUTPUT/pic-${size}.png"

done

# Windows ICO
magick \
    "$OUTPUT/pic-16.png" \
    "$OUTPUT/pic-24.png" \
    "$OUTPUT/pic-32.png" \
    "$OUTPUT/pic-48.png" \
    "$OUTPUT/pic-64.png" \
    "$OUTPUT/pic-128.png" \
    "$OUTPUT/pic-256.png" \
    "$OUTPUT/pic.ico"

echo "PIC icons generated successfully!"
