#!/usr/bin/env bash
set -Eeuo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
run_id="$(date -u +%Y%m%dT%H%M%SZ)-$$"
run_dir="${PIC_GUI_WAYLAND_RUN_DIR:-$repo_root/reports/gui-tests/complete-$run_id}"
mkdir -p "$run_dir"
desktop_dir="$run_dir/wayland-ui"
native_dir="$run_dir/native"
mkdir -p "$desktop_dir" "$native_dir"

printf 'Running all 41 interactive UI regression cases on the Wayland desktop, then native Wayland/Vulkan checks.\n'
set +e
PIC_GUI_DESKTOP_DISPLAY=1 PIC_GUI_SUITE=all PIC_GUI_RUN_DIR="$desktop_dir" bash "$repo_root/scripts/test-gui.sh"
ui_status=$?
PIC_GUI_SUITE=native PIC_GUI_NATIVE_RUN_DIR="$native_dir" bash "$repo_root/scripts/test-gui-native.sh"
native_status=$?
set -e

if [[ "$ui_status" == 0 && "$native_status" == 0 ]]; then
    result=PASS
elif [[ "$ui_status" == 2 || "$native_status" == 2 ]]; then
    result=BLOCKED
else
    result=FAIL
fi

cat > "$run_dir/report.md" <<EOF
# PIC complete GUI test report

- Overall: **$result**
- Full UI regression (41 cases: gallery, filters, ratings, import, viewer, editor/Undo-Redo, collage, screensaver): **$( [[ "$ui_status" == 0 ]] && printf PASS || { [[ "$ui_status" == 2 ]] && printf BLOCKED || printf FAIL; } )**
- Native Fedora Wayland/Vulkan real-photo smoke: **$( [[ "$native_status" == 0 ]] && printf PASS || { [[ "$native_status" == 2 ]] && printf BLOCKED || printf FAIL; } )**
- Full UI report: [wayland-ui/report.md](wayland-ui/report.md)
- Native report: [native/report.md](native/report.md)
- Artifacts: reports/gui-tests/$(basename "$run_dir")/

All 41 interactive UI cases run on the current Fedora Wayland desktop through its XWayland display, including collage, filtering, ratings, editing, and screensaver checks. The native section launches PIC with the GTK Wayland backend and verifies Vulkan initialization, real-photo import, lightbox 1:1 interaction, and persistence.
EOF
mkdir -p "$repo_root/reports/gui-tests"
cp "$run_dir/report.md" "$repo_root/reports/gui-tests/latest-wayland-report.md"
printf 'Complete report: %s\n' "$run_dir/report.md"

if [[ "$ui_status" != 0 ]]; then exit "$ui_status"; fi
exit "$native_status"
