#!/usr/bin/env python3
"""Compare PIC_COLLAGE_TRACE GTK preview geometry with exported tile geometry.

Usage:
    PIC_COLLAGE_TRACE=1 cargo run --release 2>&1 | tee /tmp/pic-collage.log
    python3 scripts/compare-collage-trace.py /tmp/pic-collage.log

Only geometry is compared. Photo contents can also differ because the preview
uses cached thumbnails before high-resolution originals finish loading.
"""
import argparse
import re
from pathlib import Path

RE_PREVIEW_CANVAS = re.compile(r"PIC_COLLAGE_TRACE preview_canvas=(\d+)x(\d+)")
RE_PREVIEW_TILE = re.compile(
    r"PIC_COLLAGE_TRACE preview_tile id=(-?\d+) .*?"
    r"visible=\((-?\d+),(-?\d+),(\d+)x(\d+)\) "
    r"center=\(([\d.]+),([\d.]+)\).*?stroke=(\d+)"
)
RE_EXPORT_TILE = re.compile(
    r"PIC_COLLAGE_TRACE export_tile id=(-?\d+) canvas=(\d+)x(\d+) .*?"
    r"visible=\((-?\d+),(-?\d+),(\d+)x(\d+)\) "
    r"center=\(([\d.]+),([\d.]+)\).*?stroke=(\d+)"
)


def parse(log: str):
    current_canvas = None
    preview = {}
    export = {}
    preview_at_export = None
    for line in log.splitlines():
        if match := RE_PREVIEW_CANVAS.search(line):
            current_canvas = tuple(map(int, match.groups()))
            preview = {}  # latest complete preview snapshot
        if match := RE_PREVIEW_TILE.search(line):
            if current_canvas:
                id_, x, y, w, h, cx, cy, stroke = match.groups()
                preview[int(id_)] = (current_canvas, float(cx), float(cy),
                                     int(w), int(h), int(stroke))
        if match := RE_EXPORT_TILE.search(line):
            if preview_at_export is None:
                # Export can run on a worker while the user keeps editing.
                # Freeze the preview snapshot associated with the export.
                preview_at_export = preview.copy()
            id_, cw, ch, x, y, w, h, cx, cy, stroke = match.groups()
            export[int(id_)] = ((int(cw), int(ch)), float(cx), float(cy),
                                int(w), int(h), int(stroke))
    return (preview_at_export if preview_at_export is not None else preview), export


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("log", type=Path)
    args = parser.parse_args()
    preview, export = parse(args.log.read_text(errors="replace"))
    if not preview or not export:
        parser.error("No matched preview/export geometry. Set PIC_COLLAGE_TRACE=1, create an export, then retry.")
    overlap = sorted(preview.keys() & export.keys())
    print(f"Preview photos: {len(preview)} | Export photos: {len(export)} | Compared: {len(overlap)}")
    missing = sorted(export.keys() - preview.keys())
    if missing:
        print(f"Export photo IDs missing in the latest preview snapshot: {missing}")
    if not overlap:
        raise SystemExit(2)
    failure = False
    for id_ in overlap:
        (pw, ph), pcx, pcy, pwidth, pheight, pst = preview[id_]
        (ew, eh), ecx, ecy, ewidth, eheight, est = export[id_]
        # Compare normalized geometry in GTK preview pixels. This allows for
        # a 900x675 GTK canvas and a 1920x1440 JPEG export.
        dx = pcx - ecx / ew * pw
        dy = pcy - ecy / eh * ph
        dw = pwidth - ewidth / ew * pw
        dh = pheight - eheight / eh * ph
        ds = pst - est / min(ew, eh) * min(pw, ph)
        worst = max(map(abs, [dx, dy, dw, dh, ds]))
        status = "OK" if worst <= 2 else "MISMATCH"
        failure |= status != "OK"
        print(f"id={id_:>6} {status:<8} offset=({dx:+.2f},{dy:+.2f}) "
              f"size=({dw:+.2f},{dh:+.2f}) border={ds:+.2f} px")
    if failure:
        print("Geometry mismatch detected. Attach the log to your bug report.")
        raise SystemExit(1)
    print("Geometry matches within 2 GTK preview pixels. Inspect photo crop/decoding if contents still differ.")


if __name__ == "__main__":
    main()
