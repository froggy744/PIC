#!/usr/bin/env bash
set -Eeuo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
run_dir="${PIC_HONEYCOMB_BENCH_RUN_DIR:-/tmp/pic-honeycomb-cache-bench-$(date -u +%Y%m%dT%H%M%SZ)-$$}"
photo_db="${PIC_BENCH_LIBRARY_DB:-${XDG_DATA_HOME:-$HOME/.local/share}/pic-rs/library.db}"

if [[ -z "${WAYLAND_DISPLAY:-}" ]]; then
    echo "BLOCKED: run from a native Wayland session (WAYLAND_DISPLAY is unset)." >&2
    exit 2
fi
if [[ ! -r "$photo_db" ]]; then
    echo "BLOCKED: PIC library database is unreadable: $photo_db" >&2
    exit 2
fi
if gdbus call --session --dest org.freedesktop.DBus \
    --object-path /org/freedesktop/DBus \
    --method org.freedesktop.DBus.NameHasOwner io.github.froggy744.PIC 2>/dev/null | rg -q true; then
    echo "BLOCKED: close the running PIC application before starting the native benchmark." >&2
    exit 2
fi

mkdir -p "$run_dir" "$run_dir/cache" "$run_dir/captures"
if [[ -d "$HOME/.cache/pic-rs/thumbs" && ! -e "$run_dir/cache/pic-rs" ]]; then
    echo "Copying the existing PIC thumbnail cache into isolated benchmark storage..."
    cp -a "$HOME/.cache/pic-rs" "$run_dir/cache/pic-rs"
fi
export XDG_CACHE_HOME="$run_dir/cache"
export GDK_BACKEND=wayland
export GSK_RENDERER=vulkan
export PIC_BENCH_LIBRARY_DB="$photo_db"

run_mode() {
    local mode="$1"
    local extra="${2:-}"
    local log="$run_dir/${mode}${extra:+-$extra}.log"
    local capture_dir="$run_dir/captures/${mode}${extra:+-$extra}"
    mkdir -p "$capture_dir"
    echo "RUN mode=$mode phase=${extra:-measured} log=$log"
    if [[ "$mode" == "cached" ]]; then
        env PIC_BENCH_LAYOUT="$mode" PIC_HONEYCOMB_RENDER_MODE=cached \
            PIC_BENCH_CAPTURE_DIR="$capture_dir" \
            cargo test --locked grid::photo_wall_tests::actual_library_honeycomb_cached_scroll_benchmark \
                -- --ignored --test-threads=1 --nocapture 2>&1 | tee "$log"
    else
        env -u PIC_HONEYCOMB_RENDER_MODE PIC_BENCH_LAYOUT="$mode" \
            PIC_BENCH_CAPTURE_DIR="$capture_dir" \
            cargo test --locked grid::photo_wall_tests::actual_library_honeycomb_cached_scroll_benchmark \
                -- --ignored --test-threads=1 --nocapture 2>&1 | tee "$log"
    fi
}

# Warm disk thumbnails once, then interleave renderer modes twice. Each test
# process starts with a cold in-memory mask cache and runs the same down/reverse/
# down path, separating cold and warm Honeycomb passes in its JSON records.
run_mode grid prewarm
run_number=0
for mode in gsk cached grid cached gsk grid; do
    run_number=$((run_number + 1))
    run_mode "$mode" "run-$run_number"
done

python3 - "$run_dir" <<'PY'
import json
import math
import re
import statistics
import sys
from pathlib import Path

root = Path(sys.argv[1])
records = []
for log in sorted(root.glob("*.log")):
    for line in log.read_text(errors="replace").splitlines():
        if line.startswith("BENCH_JSON "):
            records.append(json.loads(line.removeprefix("BENCH_JSON ")))

print("\nFrame pacing summary; intervals are aggregated across interleaved runs")
print("mode    phase  frames  median  p95  p99  max  >16.7  >33.3  CPU%  RSS-MiB  visible-ms")
for mode in ("gsk", "cached", "grid"):
    for phase in ("cold", "warm"):
        group = [r for r in records if r["mode"] == mode and r["phase"] == phase]
        if not group:
            continue
        intervals = sorted(value for record in group for value in record["intervals_ms"])
        q = lambda p: intervals[min(len(intervals) - 1, math.ceil((len(intervals) - 1) * p))]
        cpu = statistics.median(record["cpu_pct"] for record in group)
        rss = max(record["rss_hwm_kib"] for record in group) / 1024
        visible = statistics.median(record["visible_settle_ms"] for record in group)
        print(f"{mode:7} {phase:5} {len(intervals):6} {q(.50):7.2f} {q(.95):5.2f} {q(.99):5.2f} {intervals[-1]:5.2f} {sum(x>16.7 for x in intervals):6} {sum(x>33.3 for x in intervals):6} {cpu:5.1f} {rss:8.1f} {visible:10.1f}")

print("\nCache/worker summaries are in BENCH_JSON records and mode logs under", root)
print("Captured tiles:", root / "captures")
PY

if command -v python3 >/dev/null && python3 -c 'import numpy, PIL' >/dev/null 2>&1; then
python3 - "$run_dir/captures/gsk-run-5/gsk.png" "$run_dir/captures/cached-run-4/cached.png" <<'PY'
import sys
from pathlib import Path
import numpy as np
from PIL import Image, ImageChops
normal_path, cached_path = map(Path, sys.argv[1:])
if not normal_path.exists() or not cached_path.exists():
    print("Image comparison skipped: one of the native tile captures is missing.")
    raise SystemExit(0)
a = np.asarray(Image.open(normal_path).convert("RGBA"), dtype=np.int16)
b = np.asarray(Image.open(cached_path).convert("RGBA"), dtype=np.int16)
if a.shape != b.shape:
    print("Image comparison failed: sizes differ", a.shape, b.shape)
    raise SystemExit(1)
d = np.abs(a - b)
interior = (a[..., 3] >= 250) & (b[..., 3] >= 250)
edge = (a[..., 3] < 250) | (b[..., 3] < 250)
metric = lambda values: (float(values.mean()) if values.size else 0.0,
                         int(values.max()) if values.size else 0)
all_mae, all_max = metric(d)
colour_mae, colour_max = metric(d[interior, :3])
edge_mae, edge_max = metric(d[edge])
print(f"\nGSK/cached tile diff size={a.shape[1]}x{a.shape[0]} all_rgba_mae={all_mae:.3f}/255 all_max={all_max}/255 interior_rgb_mae={colour_mae:.3f}/255 interior_max={colour_max}/255 edge_rgba_mae={edge_mae:.3f}/255 edge_max={edge_max}/255 edge_pixels={int(edge.sum())}")
ImageChops.difference(Image.open(normal_path).convert("RGBA"), Image.open(cached_path).convert("RGBA")).save(normal_path.parent.parent / "normal-cached-difference.png")
PY
fi

echo "Benchmark artifacts: $run_dir"
