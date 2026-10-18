# Bookshelf background template

Use [`bookshelf-row-template.svg`](bookshelf-row-template.svg) as the master for every bookshelf theme.

## Required geometry

- Source canvas: **2172 × 724 px**.
- App rendering size: **full available width × 288 logical px** per album row.
- Shelf surface: **y = 590 px** in the source, rendered at **y = 235 px**.
- Album-safe region: **x = 136–2036 px, y = 20–590 px**.
- The outer **136 px** on each side may be cropped, shifted, or stretched as the window changes width.
- Keep the shelf continuous across the entire width. Do not place fixed uprights or unique ornaments near either side.
- Make the top and bottom edges visually compatible so rows can repeat vertically without a visible seam.

PIC measures the shelf line in exported artwork and uses it when it differs from
the guide. That measurement finds the strongest full-width edge with a flat band
below it, which on a photographed board is the board's **underside**, a few
pixels below the front edge the albums should stand on. Ship a `theme.conf`
beside the image (`row_height`, `surface_y`) to pin the values instead — do that
for artwork whose shelf edge is not a clean full-width line, and for every
photographic theme. Measure the front edge of the board, where its lit top
surface stops and its vertical face begins, then convert it to rendered pixels:
`surface_y = source_y * row_height / 724`. The shipped themes pin that line:

| Theme | Source edge | `surface_y` |
| --- | --- | --- |
| concrete | 571 | 227 |
| darkwood | 566 | 225 |
| darkwood-small | 584 | 232 |
| frontwall-white | 589 | 234 |
| light-wood | 566 | 225 |
| marble | 594 | 236 |
| shelf | 571 | 227 |
| single | 620 | 247 |

## Export

1. Replace the placeholder artwork on the `ARTWORK - replace this` layer.
2. Preserve the canvas size and shelf baseline.
3. Hide or delete the `GUIDES - hide before export` layer.
4. Export as PNG for transparency or high-quality JPEG for an opaque background.
5. Use the same dimensions and shelf baseline for every background variant.

The app should scale the background horizontally while keeping its 288-pixel row height fixed. Window resizing will then change the number of album columns without shifting albums away from their shelves.
