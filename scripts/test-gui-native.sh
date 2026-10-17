#!/usr/bin/env bash
set -Eeuo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
run_id="$(date -u +%Y%m%dT%H%M%SZ)-$$"
run_dir="${PIC_GUI_NATIVE_RUN_DIR:-$repo_root/reports/gui-tests/native-$run_id}"
report="$run_dir/report.md"
mkdir -p "$run_dir/screenshots"
mkdir -p "$repo_root/reports/gui-tests"

write_blocked_report() {
    local reason="$1"
    cat > "$report" <<EOF
# PIC automated native Fedora GUI smoke report

- Overall: **BLOCKED**
- Commit: $(git -C "$repo_root" rev-parse HEAD 2>/dev/null || printf unknown)
- Suite: automated native Wayland/Vulkan with real-photo import
- Reason: $reason
- Artifact directory: reports/gui-tests/$(basename "$run_dir")

No application interaction was performed.
EOF
    cp "$report" "$repo_root/reports/gui-tests/latest-native-report.md"
}

if [[ -z "${WAYLAND_DISPLAY:-}" ]]; then
    write_blocked_report "WAYLAND_DISPLAY is unset; run this suite from a Wayland desktop session."
    printf 'BLOCKED: no Wayland session. Report: %s\n' "$report" >&2
    exit 2
fi
if [[ -z "${XDG_RUNTIME_DIR:-}" || ! -d "$XDG_RUNTIME_DIR" ]]; then
    write_blocked_report "XDG_RUNTIME_DIR is unavailable for the active Wayland session."
    printf 'BLOCKED: Wayland runtime directory unavailable. Report: %s\n' "$report" >&2
    exit 2
fi
if [[ "$WAYLAND_DISPLAY" = /* ]]; then
    wayland_socket="$WAYLAND_DISPLAY"
else
    wayland_socket="$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY"
fi
if [[ ! -S "$wayland_socket" ]]; then
    write_blocked_report "Wayland socket is unavailable: $wayland_socket"
    printf 'BLOCKED: Wayland socket unavailable. Report: %s\n' "$report" >&2
    exit 2
fi

for tool in python3 ydotool ydotoold gdbus; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        write_blocked_report "Missing required command: $tool"
        printf 'BLOCKED: install %s. Report: %s\n' "$tool" "$report" >&2
        exit 2
    fi
done

photo_folder="${PIC_GUI_NATIVE_PHOTO_DIR:-/mnt/steam/Wickus}"
if [[ ! -d "$photo_folder" ]]; then
    write_blocked_report "Real-photo test folder is unavailable: $photo_folder. Set PIC_GUI_NATIVE_PHOTO_DIR to an existing folder."
    printf 'BLOCKED: real-photo folder not found: %s. Report: %s\n' "$photo_folder" "$report" >&2
    exit 2
fi

if ! python3 -c 'import pyatspi' >/dev/null 2>&1; then
    write_blocked_report "Python AT-SPI module pyatspi is required to drive the native GTK controls."
    printf 'BLOCKED: install python3-pyatspi. Report: %s\n' "$report" >&2
    exit 2
fi

pic_bus_name='io.github.froggy744.PIC'
pic_bus_owner="$(gdbus call --session --dest org.freedesktop.DBus \
    --object-path /org/freedesktop/DBus \
    --method org.freedesktop.DBus.NameHasOwner "$pic_bus_name" 2>/dev/null || true)"
if [[ "$pic_bus_owner" == *true* ]]; then
    write_blocked_report "Another PIC instance owns the desktop application bus name. Close it before running the isolated native test; the harness will not risk connecting to its normal library."
    printf 'BLOCKED: PIC is already running. Close the normal PIC window and rerun. Report: %s\n' "$report" >&2
    exit 2
fi

if [[ -n "${PIC_GUI_APP:-}" ]]; then
    app_bin="$PIC_GUI_APP"
    if [[ ! -x "$app_bin" ]]; then
        write_blocked_report "PIC_GUI_APP is not executable: $app_bin"
        printf 'BLOCKED: PIC binary unavailable. Report: %s\n' "$report" >&2
        exit 2
    fi
else
    app_bin="$repo_root/target/release/pic-rs"
    needs_build=0
    if [[ ! -x "$app_bin" ]]; then
        needs_build=1
    elif [[ "$repo_root/Cargo.toml" -nt "$app_bin" || "$repo_root/Cargo.lock" -nt "$app_bin" || "$repo_root/build.rs" -nt "$app_bin" ]] \
        || find "$repo_root/src" "$repo_root/resources" -type f -newer "$app_bin" -print -quit | rg -q .; then
        needs_build=1
    fi
    if [[ "$needs_build" == 1 ]]; then
        printf 'Building the current PIC release binary for native smoke test...\n'
        if ! cargo build --release --manifest-path "$repo_root/Cargo.toml" >"$run_dir/cargo-build.log" 2>&1; then
            write_blocked_report "PIC release build failed; see cargo-build.log."
            printf 'BLOCKED: PIC release build failed. Report: %s\n' "$report" >&2
            exit 2
        fi
    fi
fi

export PIC_GUI_RUN_DIR="$run_dir"
export PIC_GUI_APP="$app_bin"
export PIC_GUI_SUITE=native
export PIC_GUI_WAYLAND_READY=1
export PIC_GUI_COMMIT="$(git -C "$repo_root" rev-parse HEAD 2>/dev/null || printf unknown)"
export PIC_GUI_NATIVE_PHOTO_DIR="$photo_folder"
export GDK_BACKEND=wayland
export GSK_RENDERER=vulkan
export GSK_DEBUG=renderer
export PICASA_TRACE=1
unset G_MESSAGES_DEBUG
export GTK_USE_PORTAL=0

for directory in home config data cache; do
    mkdir -p "$run_dir/xdg/$directory"
done
export HOME="$run_dir/xdg/home"
export XDG_CONFIG_HOME="$run_dir/xdg/config"
export XDG_DATA_HOME="$run_dir/xdg/data"
export XDG_CACHE_HOME="$run_dir/xdg/cache"

set +e
python3 "$repo_root/scripts/gui_native_test.py" 2>&1 | tee "$run_dir/harness.log"
status=${PIPESTATUS[0]}
set -e
if [[ ! -f "$report" ]]; then
    write_blocked_report "Native driver exited before writing a report (status $status); inspect harness.log."
fi
cp "$report" "$repo_root/reports/gui-tests/latest-native-report.md"
printf 'Report: %s\n' "$report"
exit "$status"
