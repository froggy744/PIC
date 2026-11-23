#!/usr/bin/env bash
# Front-surface interaction checks: drive PIC (started with `cargo run`)
# through the gallery, photo viewer, editor and date overview, and check
# that the zoom slider, Space, 1:1, Escape and sort reach the surface in
# front (PICASA_TRACE route lines, the slider's range/value/tooltip through
# AT-SPI, screenshots). Written for candidates 12, 13 and 6
# (docs/architecture/STATUS.md).
#
# Usage, from the repository root:
#   scripts/front-surface-checks/run.sh
#   PIC_CHECK_REPO=/path/to/main-worktree scripts/front-surface-checks/run.sh
#
# It never touches the real library: it runs on a private Xvfb display with
# its own D-Bus and AT-SPI bus, and copies the temporary library of a GUI
# harness run (PIC_CHECK_LIBRARY=reports/gui-tests/<run>, else the newest
# one; run `PIC_GUI_SUITE=viewer scripts/test-gui.sh` once to create it).
# Output (app.log, screenshots, checks.log) goes to
# reports/front-surface-checks/<time>/. Needs Xvfb, openbox, xdotool,
# ImageMagick `import`, dbus-run-session, at-spi2-core, python3 with pyatspi
# and Pillow. Exits non-zero if any check fails.
set -u
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$(cd "$here/../.." && pwd)"
out="$repo/reports/front-surface-checks/$(date -u +%Y%m%dT%H%M%SZ)"
mkdir -p "$out"
export PIC_CHECK_OUT="$out"

display_number=77
while [[ -e "/tmp/.X${display_number}-lock" ]]; do display_number=$((display_number + 1)); done
display=":$display_number"
Xvfb "$display" -screen 0 1600x1000x24 -nolisten tcp >"$out/xvfb.log" 2>&1 &
xvfb=$!
sleep 1
export DISPLAY="$display" GDK_BACKEND=x11 GSK_RENDERER=cairo LIBGL_ALWAYS_SOFTWARE=1 GTK_A11Y=atspi
mkdir -p "$out/wm/config" "$out/wm/data" "$out/portal"
HOME="$out/wm" XDG_CONFIG_HOME="$out/wm/config" XDG_DATA_HOME="$out/wm/data" \
    openbox --sm-disable >"$out/wm.log" 2>&1 &
wm=$!
sleep 1
HOME="$out/portal" XDG_CONFIG_HOME="$out/portal/config" XDG_DATA_HOME="$out/portal/data" \
    XDG_CACHE_HOME="$out/portal/cache" XDG_CURRENT_DESKTOP=PIC_GUI XDG_SESSION_TYPE=x11 \
    dbus-run-session -- bash -c '
        /usr/libexec/at-spi-bus-launcher --launch-immediately & launcher=$!
        for _ in {1..50}; do
            gdbus call --session --dest org.a11y.Bus --object-path /org/a11y/bus \
                --method org.a11y.Bus.GetAddress >/dev/null 2>&1 && break
            sleep 0.1
        done
        /usr/libexec/at-spi2-registryd & registry=$!
        sleep 1
        python3 "$1"; status=$?
        kill "$registry" "$launcher" 2>/dev/null
        exit "$status"
    ' _ "$here/checks.py" 2>&1 | tee "$out/checks.log" | grep -E '^(PASS|FAIL|SUMMARY)|Error'
status=${PIPESTATUS[0]}
kill "$wm" "$xvfb" 2>/dev/null
echo "Output: $out"
exit "$status"
