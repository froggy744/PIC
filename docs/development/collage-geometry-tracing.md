# PIC collage geometry / preview-to-export trace

Set `PIC_COLLAGE_TRACE=1` before running PIC to log **only when preview
geometry changes**, not on every GTK frame tick. Preview and export use a shared
pixel-alignment function (`collage::model::tile_geometry`) and relative canvas
stroke widths.

## Reproducing preview / export mismatches

From the **feature/collage-sidebar-redesign** branch:

```bash
git pull --ff-only
PIC_COLLAGE_TRACE=1 cargo run --release 2>&1 | tee /tmp/pic-collage.log
```

With PIC open: choose 8+ photos with mixed orientations, Grid, and a nonzero
black or grey border. Make sure the high-resolution thumbnails have loaded,
then create **Save Collage…** and keep the application open until export
completes. Close PIC and compare the run:

```bash
python3 scripts/compare-collage-trace.py /tmp/pic-collage.log
grep PIC_COLLAGE_TRACE /tmp/pic-collage.log | tail -80
```

The comparison normalises coordinates and stroke widths from preview canvas
pixels to exported image pixels. `OK` means centre, size and border width all
agree within 2 preview pixels. `MISMATCH` shows per-photo coordinate
differences. The comparison freezes the preview snapshot when export begins.

### Trace events

- `preview_canvas`: actual GTK canvas dimensions, layout, Fit/Fill, border
  colour and stroke width, radius, spacing, item count
- `preview_tile`: photo ID, original slot rect, visible rect, centre,
  roundness, border stroke and rotation
- `preview_content`: thumbnail source aspect, tile aspect and GTK photo
  rectangle inside that tile (detects fit/letterbox issues)
- `export_tile`: full-resolution canvas rect, slot, visible rect, centre,
  border thickness and intended scaling mode
- `export_content`: decoded photo dimensions, edited dimensions and target
  dimensions (detects source decoder/crop differences)
- `export_composite`: bitmap dimensions and final overlay destination

Trace logging does not print image filenames or full paths and does not
decode additional images. Normal runs have the flag disabled.

## Root cause and fix

The previous preview added a CSS `border` to each `GtkFrame`. In GTK this
consumed content box space, shifting/scaling the photo inside a fixed tile.
Export independently drew an inset border over the image pixels. The apparent
photo centre and edges therefore differed, even with a uniform stroke width.
Additionally, preview rounded tile geometry while export used a different
ceil/truncation sequence.

The new implementation draws border strokes in a pointer-transparent GTK
drawing overlay after the photo. The overlay is not used for layout
measurements; the same `tile_geometry` drives preview and export placement.
The stroke is centred inside the existing visible bounds, without changing
the photo placement. All border colours and thicknesses use this path.
```
