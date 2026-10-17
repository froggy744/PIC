#!/usr/bin/env bash
set -Eeuo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
run_id="$(date -u +%Y%m%dT%H%M%SZ)-$$"
run_dir="${PIC_GUI_RUN_DIR:-$repo_root/reports/gui-tests/$run_id}"
report="$run_dir/report.md"

show_test_menu() {
    local choice
    if [[ -n "${PIC_GUI_SUITE:-}" ]]; then
        printf '%s\n' "$PIC_GUI_SUITE"
        return
    fi
    cat >&2 <<'EOF'
Choose a GUI test run (each focused run performs the setup and earlier steps it depends on):
  1) Full suite
  2) Startup only
  3) Library and ratings
  4) Import
  5) Gallery and viewer
  6) Editing and collage
  7) Screensaver
  8) Complete Wayland desktop suite (41 UI checks + native Vulkan)
  9) Wayland/Vulkan native smoke only
Selection [1]:
EOF
    read -r choice || choice=1
    case "${choice:-1}" in
        1) printf 'all\n' ;;
        2) printf 'startup\n' ;;
        3) printf 'library\n' ;;
        4) printf 'import\n' ;;
        5) printf 'viewer\n' ;;
        6) printf 'editing\n' ;;
        7) printf 'screensaver\n' ;;
        8) printf 'wayland\n' ;;
        9) printf 'native\n' ;;
        *) printf 'Invalid selection: %s\n' "$choice" >&2; exit 2 ;;
    esac
}

gui_suite="$(show_test_menu)"
case "$gui_suite" in all|startup|library|import|viewer|editing|screensaver|native|wayland) ;;
    *) printf 'Invalid PIC_GUI_SUITE: %s\n' "$gui_suite" >&2; exit 2 ;;
esac

if [[ "$gui_suite" == "native" ]]; then
    export PIC_GUI_SUITE="$gui_suite"
    exec "$repo_root/scripts/test-gui-native.sh"
fi
if [[ "$gui_suite" == "wayland" ]]; then
    export PIC_GUI_SUITE="$gui_suite"
    exec "$repo_root/scripts/test-gui-wayland.sh"
fi

mkdir -p "$run_dir/screenshots"
desktop_display="${PIC_GUI_DESKTOP_DISPLAY:-0}"

write_blocked_report() {
    local reason="$1"
    cat > "$report" <<EOF
# PIC GUI test report

- Result: **BLOCKED**
- Commit: 
- Reason: $reason

No application interaction was performed.
EOF
    cp "$report" "$repo_root/reports/gui-tests/latest-report.md"
}

required_tools=(xdotool identify python3)
if [[ "$desktop_display" == 1 ]]; then
    required_tools+=(gdbus wmctrl /usr/bin/python3)
else
    required_tools+=(import dbus-run-session)
fi
for tool in "${required_tools[@]}"; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        case "$tool" in
            xdotool) package_hint="Fedora: xdotool; Ubuntu: xdotool" ;;
            import|identify) package_hint="Fedora: ImageMagick; Ubuntu: imagemagick" ;;
            python3) package_hint="Fedora: python3; Ubuntu: python3" ;;
            dbus-run-session) package_hint="Fedora: dbus-daemon; Ubuntu: dbus-daemon" ;;
            gdbus) package_hint="Fedora: glib2; Ubuntu: libglib2.0-bin" ;;
            wmctrl) package_hint="Fedora: wmctrl; Ubuntu: wmctrl" ;;
            /usr/bin/python3) package_hint="Fedora: python3-gobject; Ubuntu: python3-gi" ;;
        esac
        write_blocked_report "Missing required command: $tool"
        printf 'BLOCKED: missing %s (install %s)\nReport: %s\n' "$tool" "$package_hint" "$report" >&2
        exit 2
    fi
done

if ! python3 -c 'import PIL' >/dev/null 2>&1; then
    write_blocked_report "Python module Pillow is required"
    printf 'BLOCKED: install Python Pillow. Report: %s\n' "$report" >&2
    exit 2
fi

if ! python3 -c 'from PIL import Image' >/dev/null 2>&1; then
    write_blocked_report "Pillow is installed but its Image module could not be imported"
    printf 'BLOCKED: Python Pillow import failed. Report: %s\n' "$report" >&2
    exit 2
fi

if [[ "$desktop_display" == 1 ]]; then
    if [[ "${XDG_SESSION_TYPE:-}" != wayland || -z "${WAYLAND_DISPLAY:-}" || -z "${DISPLAY:-}" ]]; then
        write_blocked_report "The full UI suite needs an active Wayland desktop with its XWayland DISPLAY."
        printf 'BLOCKED: run from the Fedora Wayland desktop session. Report: %s\n' "$report" >&2
        exit 2
    fi
    if ! command -v gdbus >/dev/null 2>&1; then
        write_blocked_report "gdbus is required to check that the normal PIC instance is closed."
        printf 'BLOCKED: install glib2. Report: %s\n' "$report" >&2
        exit 2
    fi
    bus_owner="$(gdbus call --session --dest org.freedesktop.DBus --object-path /org/freedesktop/DBus --method org.freedesktop.DBus.NameHasOwner io.github.froggy744.PIC 2>/dev/null || true)"
    if [[ "$bus_owner" == *true* ]]; then
        write_blocked_report "Another PIC instance is open. Close it before testing so only the isolated temporary library is used."
        printf 'BLOCKED: close the normal PIC window and rerun. Report: %s\n' "$report" >&2
        exit 2
    fi
    display_geometry="$(xdotool getdisplaygeometry 2>/dev/null || true)"
    read -r desktop_width desktop_height <<<"$display_geometry"
    if [[ "${desktop_width:-0}" -lt 1600 || "${desktop_height:-0}" -lt 1000 ]]; then
        write_blocked_report "The XWayland desktop is smaller than the 1600x1000 geometry required by the visual checks: $display_geometry"
        printf 'BLOCKED: set the desktop display to at least 1600x1000. Report: %s\n' "$report" >&2
        exit 2
    fi
    export PIC_GUI_DISPLAY_WIDTH="$desktop_width"
    export PIC_GUI_DISPLAY_HEIGHT="$desktop_height"
    if ! /usr/bin/python3 -c 'from gi.repository import Gio, GLib' >/dev/null 2>&1; then
        write_blocked_report "Python GObject bindings are required for Wayland desktop screenshots."
        printf 'BLOCKED: install python3-gobject. Report: %s\n' "$report" >&2
        exit 2
    fi
    xvfb_bin=""
else
    xvfb_bin="${XVFB_BIN:-$(command -v Xvfb || true)}"
    if [[ -z "$xvfb_bin" || ! -x "$xvfb_bin" ]]; then
        write_blocked_report "Xvfb was not found. Install xorg-x11-server-Xvfb; no live display was used."
        printf 'BLOCKED: Xvfb not found (Fedora package: xorg-x11-server-Xvfb). Report: %s\n' "$report" >&2
        exit 2
    fi
fi

if [[ -n "${PIC_GUI_APP:-}" ]]; then
    app_bin="$PIC_GUI_APP"
else
    app_bin="$repo_root/target/release/pic-rs"
    printf 'Building PIC release binary...\n'
    cargo build --release --manifest-path "$repo_root/Cargo.toml" 2>&1 | tee "$run_dir/cargo-build.log"
fi
if [[ ! -x "$app_bin" ]]; then
    write_blocked_report "PIC release binary is unavailable: $app_bin"
    printf 'BLOCKED: PIC release binary unavailable. Report: %s\n' "$report" >&2
    exit 2
fi

xvfb_pid=""
wm_pid=""
if [[ "$desktop_display" == 1 ]]; then
    display="$DISPLAY"
    printf 'Using the Wayland desktop XWayland display %s for the full GUI regression suite.\n' "$display"
else
    display_file="$run_dir/xvfb-display"
    "$xvfb_bin" -displayfd 3 -screen 0 1600x1000x24 -nolisten tcp -ac -noreset \
        >"$run_dir/xvfb.log" 2>&1 3>"$display_file" &
    xvfb_pid=$!
fi
cleanup() {
    if [[ -n "${wm_pid:-}" ]] && kill -0 "$wm_pid" 2>/dev/null; then
        kill "$wm_pid" 2>/dev/null || true
        wait "$wm_pid" 2>/dev/null || true
    fi
    if [[ -n "$xvfb_pid" ]] && kill -0 "$xvfb_pid" 2>/dev/null; then
        kill "$xvfb_pid" 2>/dev/null || true
        wait "$xvfb_pid" 2>/dev/null || true
    fi
}
trap cleanup EXIT INT TERM

if [[ "$desktop_display" != 1 ]]; then
    for _ in $(seq 1 100); do
        [[ -s "$display_file" ]] && break
        kill -0 "$xvfb_pid" 2>/dev/null || break
        sleep 0.1
    done
    if [[ ! -s "$display_file" ]]; then
        write_blocked_report "Xvfb failed to start; see xvfb.log"
        printf 'BLOCKED: Xvfb failed to start. Report: %s\n' "$report" >&2
        exit 2
    fi
    display=":$(tr -d '\n' < "$display_file")"
fi

export DISPLAY="$display"
export GDK_BACKEND=x11
export GSK_RENDERER=cairo
export LIBGL_ALWAYS_SOFTWARE=1
export GTK_A11Y=none
export PIC_GUI_RUN_DIR="$run_dir"
export PIC_GUI_APP="$app_bin"
export PIC_GUI_SUITE="$gui_suite"
export PIC_GUI_COMMIT="$(git -C "$repo_root" rev-parse HEAD)"
export PIC_GUI_XVFB_LOG="$run_dir/xvfb.log"
export PIC_GUI_DISPLAY_DESCRIPTION="Xvfb $display (1600x1000x24), GTK X11/Cairo"
if [[ "$desktop_display" == 1 ]]; then
    if ! command -v wmctrl >/dev/null 2>&1 || ! wmctrl -m >/dev/null 2>&1; then
        write_blocked_report "An active XWayland-compatible window manager is required for the desktop UI checks."
        printf 'BLOCKED: desktop window manager is unavailable. Report: %s\n' "$report" >&2
        exit 2
    fi
    export PIC_GUI_WINDOW_MANAGER="$(wmctrl -m | sed -n 's/^Name: //p')"
    export PIC_GUI_DISPLAY_DESCRIPTION="Wayland desktop via XWayland $display ($(printf '%s' "$display_geometry" | tr ' ' 'x')), GTK X11/Cairo"
elif [[ -n "${PIC_GUI_WM_BIN:-$(command -v openbox || true)}" ]]; then
    wm_bin="${PIC_GUI_WM_BIN:-$(command -v openbox || true)}"
    if ! command -v wmctrl >/dev/null 2>&1; then
        write_blocked_report "An X11 window manager was requested, but wmctrl is missing"
        printf 'BLOCKED: install wmctrl to validate managed fullscreen windows. Report: %s\n' "$report" >&2
        exit 2
    fi
    if [[ ! -x "$wm_bin" ]]; then
        write_blocked_report "Requested X11 window manager is not executable: $wm_bin"
        printf 'BLOCKED: X11 window manager is not executable: %s\nReport: %s\n' "$wm_bin" "$report" >&2
        exit 2
    fi
    mkdir -p "$run_dir/window-manager-home/config" "$run_dir/window-manager-home/data"
    wm_config_dirs="${PIC_GUI_WM_CONFIG_DIRS:-/etc/xdg}"
    wm_data_dirs="${PIC_GUI_WM_DATA_DIRS:-/usr/local/share:/usr/share}"
    HOME="$run_dir/window-manager-home" \
        XDG_CONFIG_HOME="$run_dir/window-manager-home/config" \
        XDG_CONFIG_DIRS="$wm_config_dirs" \
        XDG_DATA_HOME="$run_dir/window-manager-home/data" \
        XDG_DATA_DIRS="$wm_data_dirs" \
        "$wm_bin" --sm-disable >"$run_dir/window-manager.log" 2>&1 &
    wm_pid=$!
    for _ in $(seq 1 50); do
        wmctrl -m >/dev/null 2>&1 && break
        kill -0 "$wm_pid" 2>/dev/null || break
        sleep 0.1
    done
    if ! wmctrl -m >/dev/null 2>&1; then
        write_blocked_report "X11 window manager failed to become active; see window-manager.log"
        printf 'BLOCKED: X11 window manager failed to start. Report: %s\n' "$report" >&2
        exit 2
    fi
    export PIC_GUI_WINDOW_MANAGER="$(wmctrl -m | sed -n 's/^Name: //p')"
else
    export PIC_GUI_WINDOW_MANAGER="none"
fi
if [[ "${PIC_GUI_REQUIRE_WM:-1}" == "1" && "$PIC_GUI_WINDOW_MANAGER" == "none" ]]; then
    write_blocked_report "A window manager is required for this test run. Install Openbox and set PIC_GUI_WM_BIN if needed."
    printf 'BLOCKED: window manager required; install Openbox. Report: %s\n' "$report" >&2
    exit 2
fi
if [[ "$desktop_display" != 1 ]]; then
    mkdir -p "$run_dir/runtime"
    chmod 700 "$run_dir/runtime"
    export XDG_RUNTIME_DIR="$run_dir/runtime"
fi
export PIC_GUI_WM_PID="$wm_pid"
portal_config="$run_dir/portal-config"
portal_home="$run_dir/xdg/home"
mkdir -p "$portal_config/xdg-desktop-portal"
mkdir -p "$portal_home/data" "$portal_home/cache"
cat > "$portal_config/xdg-desktop-portal/portals.conf" <<'EOF'
[preferred]
default=gtk
EOF

set +e
if [[ "$desktop_display" == 1 ]]; then
    HOME="$portal_home" XDG_CONFIG_HOME="$portal_config" XDG_DATA_HOME="$portal_home/data" \
        XDG_CACHE_HOME="$portal_home/cache" XDG_CURRENT_DESKTOP=PIC_GUI \
        python3 "$repo_root/scripts/gui_test.py" >"$run_dir/harness.log" 2>&1
else
    HOME="$portal_home" XDG_CONFIG_HOME="$portal_config" XDG_DATA_HOME="$portal_home/data" \
        XDG_CACHE_HOME="$portal_home/cache" XDG_CURRENT_DESKTOP=PIC_GUI XDG_SESSION_TYPE=x11 \
        dbus-run-session -- python3 "$repo_root/scripts/gui_test.py" >"$run_dir/harness.log" 2>&1
fi
status=$?
cat "$run_dir/harness.log"
set -e
if [[ ! -f "$report" ]]; then
    write_blocked_report "GUI driver exited before writing a report (status $status); inspect harness.log and application.log."
fi
cp "$report" "$repo_root/reports/gui-tests/latest-report.md"
exit "$status"
