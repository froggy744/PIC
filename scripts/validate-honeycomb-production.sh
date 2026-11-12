#!/usr/bin/env bash
set -Eeuo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."
repo="$PWD"
if [[ "$(git branch --show-current)" != 'feature/honeycomb' ]]; then
    echo 'ERROR: run on feature/honeycomb; refusing to run on another branch' >&2
    exit 2
fi
if [[ -z "${WAYLAND_DISPLAY:-}" ]]; then
    echo 'ERROR: native Wayland session required' >&2
    exit 2
fi
export GDK_BACKEND=wayland GSK_RENDERER=vulkan
export PIC_BENCH_LIBRARY_DB="${PIC_BENCH_LIBRARY_DB:-$HOME/.local/share/pic-rs/library.db}"
if [[ ! -r "$PIC_BENCH_LIBRARY_DB" ]]; then
    echo "ERROR: library database not found: $PIC_BENCH_LIBRARY_DB" >&2
    exit 2
fi
out="${PIC_BENCH_OUTPUT:-$HOME/pic-honeycomb-final-$(date +%Y%m%d-%H%M%S)}"
mkdir -p "$out"
printf '%s\n' "$repo" > "$out/repository.txt"
git rev-parse HEAD > "$out/commit.txt"
git status --short > "$out/worktree-status.txt"
git diff -- src/grid/tile.rs src/grid/sectioned_folder.rs src/grid/photo_wall_tests.rs > "$out/changes.patch"
echo "Results: $out"
echo 'Building the release test binary ONCE; this can take several minutes.'
if ! cargo test --release --locked grid::photo_wall_tests::actual_library_honeycomb_cached_scroll_benchmark --no-run > "$out/build.log" 2>&1; then
    tail -n 70 "$out/build.log"
    echo "Build failed; full log: $out/build.log" >&2
    exit 1
fi
TEST=grid::photo_wall_tests::actual_library_honeycomb_cached_scroll_benchmark
run() {
    local mode="$1" idx="$2"
    local logfile="$out/${mode}-${idx}.log"
    echo "Running $mode trial $idx"
    local code=0
    local render_mode="cached"
    [[ "$mode" == "gsk" ]] && render_mode="live"
    PIC_HONEYCOMB_RENDER_MODE="$render_mode" PIC_BENCH_LAYOUT="$mode" \
      cargo test --release --locked "$TEST" -- --ignored --test-threads=1 --nocapture > "$logfile" 2>&1 || code=$?
    echo "$code" > "$out/${mode}-${idx}.exit"
    grep -E 'BENCH_SETUP|BENCH_UNREADY_SUMMARY|invalid benchmark|test result:|FAILED' "$logfile" || true
    return "$code"
}
failed=0
for idx in 1 2 3; do
    for mode in grid gsk cached; do
        run "$mode" "$idx" || failed=1
    done
done
python3 - "$out" <<'PY_ANALYZE'
import csv,json,statistics,sys
from pathlib import Path
root=Path(sys.argv[1]); records=[]
for logfile in sorted(root.glob('*.log')):
    if logfile.name=='build.log':continue
    for line in logfile.read_text(errors='replace').splitlines():
        if line.startswith('BENCH_JSON '):
            try:
                row=json.loads(line.removeprefix('BENCH_JSON '))
                row['run']=logfile.stem
                row['valid']=((root/(logfile.stem+'.exit')).read_text().strip()=='0')
                records.append(row)
            except (ValueError,OSError):pass
fields=['run','mode','phase','kind','valid','frames','median_ms','p95_ms','p99_ms','max_ms','over_16_7','over_33_3','scroll_ms','visible_settle_ms','visible_ready','cpu_pct','rss_hwm_kib','cache_hits','cache_misses','cache_fallbacks','cache_bytes','worker_us']
with (root/'summary.csv').open('w',newline='') as f:
    w=csv.DictWriter(f,fieldnames=fields,extrasaction='ignore');w.writeheader();w.writerows(records)
with (root/'frames.csv').open('w',newline='') as f:
    w=csv.writer(f);w.writerow(['run','mode','phase','frame_index','interval_ms'])
    for r in records:
        for i,v in enumerate(r.get('intervals_ms',[])):w.writerow([r['run'],r['mode'],r['phase'],i,v])
lines=['# Native Honeycomb validation', '', 'All values are measured by the feature-branch GTK/Vulkan test. Invalid runs are not included in conclusions.', '', '| Mode | Valid runs | Warm p95 median (ms) | Warm p99 median (ms) | Peak RSS MiB | Cached-worker activity |', '|---|---:|---:|---:|---:|---|']
for mode in ('grid','gsk','cached'):
    valid_runs=[]
    for index in (1,2,3):
        run=f'{mode}-{index}'
        if (root/(run+'.exit')).exists() and (root/(run+'.exit')).read_text().strip()=='0':
            valid_runs.append(run)
    group=[r for r in records if r['run'] in valid_runs and r['phase']=='warm']
    p95=f'{statistics.median(r["p95_ms"] for r in group):.2f}' if group else 'N/A'
    p99=f'{statistics.median(r["p99_ms"] for r in group):.2f}' if group else 'N/A'
    rss=f'{max(r["rss_hwm_kib"] for r in group)/1024:.1f}' if group else 'N/A'
    worker='yes' if any(r.get('worker_us',0)>0 and r.get('cache_bytes',0)>0 for r in group) else 'no/not shown'
    lines.append(f'| {mode} | {len(valid_runs)}/3 | {p95} | {p99} | {rss} | {worker} |')
lines += ['', '## Production gate', '', '- All three Grid and GSK runs must pass.', '- Cached mode must pass three runs and show nonzero worker processing/cache bytes.', '- Inspect rendered image parity and scroll/interactions manually before enabling cache by default.', '- This script does not provide a main-branch comparator. Do not claim main-vs-feature speedups from these results.', '']
(root/'report.md').write_text('\n'.join(lines))
print('\n'.join(lines))
print('Raw frames:',root/'frames.csv')
print('Summary:',root/'summary.csv')
print('Report:',root/'report.md')
PY_ANALYZE
if [[ "$failed" != 0 ]]; then
    echo "Some trials failed. See $out/report.md and individual logs." >&2
    exit 1
fi
if ! grep -q '"worker_us":[1-9]' "$out"/cached-*.log; then
    echo 'Cached worker not confirmed active. Keep cached disabled in production.' >&2
    exit 1
fi
echo "Trials complete. Read $out/report.md"
