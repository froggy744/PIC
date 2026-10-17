#!/usr/bin/env bash
# Real GTK allocation checks for every shipped theme, plus Enter activation
# and alignment during live theme switches. Requires Xvfb and xdotool.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."
run_dir=$(mktemp -d /tmp/pic-theme-headers.XXXXXX)
cargo test --no-run --message-format=json > "$run_dir/build.json"
test_binary=$(python3 - "$run_dir/build.json" <<'PY'
import json, sys
for line in open(sys.argv[1]):
    item = json.loads(line)
    if item.get('executable') and item.get('profile', {}).get('test'):
        print(item['executable'])
PY
)
for directory in themes/*/; do
    theme=${directory#themes/}
    theme=${theme%/}
    xvfb-run -a -s '-screen 0 1920x1080x24' dbus-run-session -- \
        env -u WAYLAND_DISPLAY GDK_BACKEND=x11 GSK_RENDERER=cairo PIC_TEST_THEME="$theme" \
        "$test_binary" theme_header_edges_align --ignored --nocapture > "$run_dir/$theme.log" 2>&1
    printf 'PASS %s\n' "$theme"
done
xvfb-run -a -s '-screen 0 1920x1080x24' dbus-run-session -- \
    env -u WAYLAND_DISPLAY GDK_BACKEND=x11 GSK_RENDERER=cairo \
    PIC_TEST_THEME=Terminal PIC_TEST_THEME_SWITCHES=1 \
    "$test_binary" theme_header_edges_align --ignored --nocapture > "$run_dir/switches.log" 2>&1
printf 'PASS live theme switching and Enter selection\nLogs: %s\n' "$run_dir"
