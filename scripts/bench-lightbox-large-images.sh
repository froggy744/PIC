#!/usr/bin/env bash
# Lightbox large-image benchmark: every scenario of
# lightbox::bench_tests::bench_lightbox_large_images, REPS times (default 5),
# one process per run. Detailed logs (benchmark lines + PICASA_TRACE) go to
# reports/bench-lightbox/<stamp>-<mode>/ (gitignored); stdout is the summary
# only: median, p95 and max latency, peak memory, threads, cancellations.
#
# Usage: scripts/bench-lightbox-large-images.sh [xvfb|desktop]
#   xvfb     private Xvfb display, cairo renderer (default)
#   desktop  the current Wayland session with the Vulkan renderer: test
#            windows open on the real desktop for the whole run
#
# Required environment (see src/lightbox/bench_tests.rs):
#   PIC_BENCH_LIGHTBOX_PNG PIC_BENCH_LIGHTBOX_RGBA_PNG PIC_BENCH_LIGHTBOX_JPEG
#   PIC_BENCH_LIGHTBOX_RAW PIC_BENCH_LIGHTBOX_SET PIC_BENCH_LIGHTBOX_SUSTAINED
# Optional: REPS (default 5), PIC_BENCH_BIN (prebuilt test binary).
set -euo pipefail

mode=${1:-xvfb}
reps=${REPS:-5}
cd "$(dirname "$0")/.."

for var in PIC_BENCH_LIGHTBOX_PNG PIC_BENCH_LIGHTBOX_RGBA_PNG PIC_BENCH_LIGHTBOX_JPEG \
    PIC_BENCH_LIGHTBOX_RAW PIC_BENCH_LIGHTBOX_SET PIC_BENCH_LIGHTBOX_SUSTAINED; do
    [[ -n ${!var:-} ]] || { echo "$var is not set" >&2; exit 2; }
done

bin=${PIC_BENCH_BIN:-}
if [[ -z $bin ]]; then
    bin=$(cargo test --locked --no-run 2>&1 |
        sed -n 's/.*Executable unittests src\/main.rs (\(.*\))/\1/p' | head -n1)
fi
[[ -x $bin ]] || { echo "test binary not found" >&2; exit 2; }

logs="reports/bench-lightbox/$(date +%Y%m%d-%H%M%S)-$mode"
mkdir -p "$logs"

run_one() { # scenario rep [png]
    local scenario=$1 rep=$2 png=${3:-$PIC_BENCH_LIGHTBOX_PNG} label=$1
    [[ $# -eq 3 ]] && label="${1}_rgba"
    local log="$logs/$label-$rep.log"
    local env_args=(PICASA_TRACE=1 PIC_BENCH_LIGHTBOX_SCENARIO="$scenario"
        PIC_BENCH_LIGHTBOX_PNG="$png")
    local test_args=(--exact lightbox::bench_tests::bench_lightbox_large_images
        --ignored --nocapture --test-threads=1)
    if [[ $mode == desktop ]]; then
        env "${env_args[@]}" GDK_BACKEND=wayland GSK_RENDERER=vulkan \
            "$bin" "${test_args[@]}" >"$log" 2>&1 || echo "RUN_FAILED" >>"$log"
    else
        xvfb-run -a -s "-screen 0 1920x1080x24" env -u WAYLAND_DISPLAY \
            "${env_args[@]}" GDK_BACKEND=x11 GSK_RENDERER=cairo \
            "$bin" "${test_args[@]}" >"$log" 2>&1 || echo "RUN_FAILED" >>"$log"
    fi
}

for rep in $(seq 1 "$reps"); do
    for scenario in cold_fit fit_100 toggle_repeat png_jpeg_raw png_cached \
        rapid sustained close_decoding; do
        run_one "$scenario" "$rep"
    done
    run_one fit_100 "$rep" "$PIC_BENCH_LIGHTBOX_RGBA_PNG"
    run_one toggle_repeat "$rep" "$PIC_BENCH_LIGHTBOX_RGBA_PNG"
done

python3 -I - "$logs" "$mode" "$reps" <<'PY'
import glob, os, re, statistics, sys

logs, mode, reps = sys.argv[1], sys.argv[2], int(sys.argv[3])
runs = {}
for path in sorted(glob.glob(os.path.join(logs, "*.log"))):
    label = os.path.basename(path).rsplit("-", 1)[0]
    runs.setdefault(label, []).append(open(path, errors="replace").read())

def fields(line):
    return dict(part.split("=", 1) for part in line.split()[1:] if "=" in part)

def bench(text):
    return [fields(line) for line in text.splitlines() if line.startswith("BENCH ")]

def num(value):
    return None if value in (None, "-") else float(value)

def pct(values, q):
    values = sorted(values)
    return values[min(len(values) - 1, max(0, round(q * (len(values) - 1))))]

rows, anomalies = [], []

def latency(name, values):
    values = [v for v in values if v is not None]
    if values:
        rows.append((name, len(values), statistics.median(values), pct(values, 0.95), max(values)))

def peak(label):
    return max((num(f.get("peak_mb")) or 0) for text in runs.get(label, []) for f in bench(text))

for label, texts in sorted(runs.items()):
    for index, text in enumerate(texts, 1):
        if "RUN_FAILED" in text or "panicked" in text:
            anomalies.append(f"{label} run {index}: failed (see log)")

def steps(label, step):
    return [num(f.get("ms")) for t in runs.get(label, []) for f in bench(t) if f.get("step") == step]

latency("cold FIT open", steps("cold_fit", "fit"))
latency("FIT -> 100% (RGB PNG)", steps("fit_100", "100"))
latency("FIT -> 100% (RGBA PNG)", steps("fit_100_rgba", "100"))
for label, kind in (("toggle_repeat", "RGB"), ("toggle_repeat_rgba", "RGBA")):
    toggles = [f for t in runs.get(label, []) for f in bench(t) if f.get("kind") == "toggle"]
    latency(f"first 100% present ({kind})",
            [num(f["present_ms"]) for f in toggles if f["round"] == "0" and f["to"] == "100"])
    latency(f"repeat 100% present ({kind})",
            [num(f["present_ms"]) for f in toggles if f["round"] != "0" and f["to"] == "100"])
    latency(f"repeat FIT present ({kind})",
            [num(f["present_ms"]) for f in toggles if f["to"] == "fit"])
latency("PNG -> JPEG", steps("png_jpeg_raw", "jpeg"))
latency("JPEG -> RAW", steps("png_jpeg_raw", "raw"))
latency("PNG -> cached photo", steps("png_cached", "cached_jpeg"))

nav = {}
for label in ("rapid", "sustained"):
    presented = total = 0
    present, cancelled, requests, drop_max, threads_max, sweeps = [], [], [], 0, 0, []
    for text in runs.get(label, []):
        records = bench(text)
        navs = [f for f in records if f.get("kind") == "nav"]
        total += len(navs)
        presented += sum(1 for f in navs if f["present_ms"] != "-")
        present += [num(f["present_ms"]) for f in navs]
        for f in records:
            if f.get("kind") == "threads":
                drop_max = max(drop_max, int(f["max_drop_threads"]))
                threads_max = max(threads_max, int(f["max_threads"]))
            if f.get("kind") == "threads_after_close" and int(f["drop_threads"]) > 0:
                anomalies.append(f"{label}: {f['drop_threads']} drop threads alive after close")
            if f.get("kind") == "sweep":
                sweeps.append((int(f["sweep"]), float(f["rss_mb"])))
        cancelled.append(len(re.findall(r"decode_done .*outcome=error", text))
                         + len(re.findall(r"display_discard lane=", text)))
        requests.append(len(re.findall(r"request lane=\S+ action=new", text)))
    latency(f"{label} navigation present", present)
    nav[label] = (presented, total, statistics.median(cancelled) if cancelled else 0,
                  statistics.median(requests) if requests else 0, drop_max, threads_max, sweeps)
    if total and presented == 0:
        anomalies.append(f"{label}: no photo presented")

print(f"Lightbox large-image benchmark: mode={mode}, {reps} repetitions, logs: {logs}")
print()
print(f"{'latency (ms)':34} {'n':>4} {'median':>8} {'p95':>8} {'max':>8}")
for name, n, median, p95, maximum in rows:
    print(f"{name:34} {n:4d} {median:8.0f} {p95:8.0f} {maximum:8.0f}")
print()
print(f"{'peak memory (MB, max over runs)':34} {'peak':>8}")
for label in ("cold_fit", "fit_100", "fit_100_rgba", "toggle_repeat", "toggle_repeat_rgba",
              "png_jpeg_raw", "png_cached", "rapid", "sustained", "close_decoding"):
    if label in runs:
        print(f"{label:34} {peak(label):8.0f}")
print()
for label, (presented, total, cancelled, requests, drop_max, threads_max, sweeps) in nav.items():
    first = [rss for sweep, rss in sweeps if sweep == 0]
    last = [rss for sweep, rss in sweeps if sweep == max(s for s, _ in sweeps)]
    print(f"{label}: presented {presented}/{total} steps; per run median {requests:.0f} decode "
          f"requests, {cancelled:.0f} cancelled; max threads {threads_max}, max drop threads "
          f"{drop_max}; RSS after first/last sweep (median) "
          f"{statistics.median(first):.0f}/{statistics.median(last):.0f} MB")
print()
print("anomalies: " + ("; ".join(anomalies) if anomalies else "none"))
PY
