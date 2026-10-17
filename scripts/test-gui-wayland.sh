#!/usr/bin/env bash
set -Eeuo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
run_id="$(date -u +%Y%m%dT%H%M%SZ)-$$"
run_dir="${PIC_GUI_WAYLAND_RUN_DIR:-$repo_root/reports/gui-tests/complete-$run_id}"
mkdir -p "$run_dir"
xvfb_dir="$run_dir/xvfb"
native_dir="$run_dir/native"
mkdir -p "$xvfb_dir" "$native_dir"

printf 'Running the complete isolated regression suite (Xvfb), then native Fedora Wayland/Vulkan checks.\n'
set +e
PIC_GUI_SUITE=all PIC_GUI_RUN_DIR="$xvfb_dir" bash "$repo_root/scripts/test-gui.sh"
xvfb_status=$?
PIC_GUI_SUITE=native PIC_GUI_NATIVE_RUN_DIR="$native_dir" bash "$repo_root/scripts/test-gui-native.sh"
native_status=$?
set -e

if [[ "$xvfb_status" == 0 && "$native_status" == 0 ]]; then
    result=PASS
elif [[ "$xvfb_status" == 2 || "$native_status" == 2 ]]; then
    result=BLOCKED
else
    result=FAIL
fi

cat > "$run_dir/report.md" <<EOF
# PIC complete GUI test report

- Overall: **$result**
- Xvfb baseline (41 automated cases): **$( [[ "$xvfb_status" == 0 ]] && printf PASS || { [[ "$xvfb_status" == 2 ]] && printf BLOCKED || printf FAIL; } )**
- Native Fedora Wayland/Vulkan real-photo smoke: **$( [[ "$native_status" == 0 ]] && printf PASS || { [[ "$native_status" == 2 ]] && printf BLOCKED || printf FAIL; } )**
- Xvfb report: [xvfb/report.md](xvfb/report.md)
- Native report: [native/report.md](native/report.md)
- Artifacts: reports/gui-tests/$(basename "$run_dir")/

The 41 baseline cases use an isolated Xvfb display because their input and window-management checks depend on X11. The native section runs in the active Fedora Wayland desktop with Vulkan requested and confirmed from GTK diagnostics.
EOF
mkdir -p "$repo_root/reports/gui-tests"
cp "$run_dir/report.md" "$repo_root/reports/gui-tests/latest-wayland-report.md"
printf 'Complete report: %s\n' "$run_dir/report.md"

if [[ "$xvfb_status" != 0 ]]; then exit "$xvfb_status"; fi
exit "$native_status"
