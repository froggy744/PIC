# Wall and Masonry zoom — verification checklist

Branch: `fix/gallery-zoom-animation-sync`

## Method: final layout + paint-only settle
- [x] Preserve existing Photo Wall justified rows and Masonry column-placement logic.
- [x] Compute and apply the new tile positions/sizes **once** per discrete zoom step; never animate X/Y, scroll position, or row/column membership.
- [x] Reuse SquareTile's centre-origin, clipped snapshot scaling: **0.95 → 1.0** when zooming in and **1.05 → 1.0** when zooming out.
- [x] Add a subtle tile opacity fade **0.88 → 1.0** over **180 ms**, using the existing cubic ease-in/out.
- [x] Carry the current visual scale/opacity into a rapid subsequent zoom rather than restarting the fade from the beginning.
- [x] Keep in-flight scale/opacity during scroll/GTK refresh and on newly mapped/recycled tiles.
- [x] Cancel cleanly when animations are disabled, the view changes, or a replacement model is installed. Respect GTK reduced-animation settings.
- [x] Leave Grid, Folder, lightbox and photo decoding geometry unchanged, apart from the prior shared-input zoom routing already on this branch.
- [x] Add an ignored GTK regression test for both layouts, stationary tile coordinates, finish and cancellation.

## Validation before merge
- [ ] Run `cargo check`.
- [ ] Run `cargo test wall_and_masonry_zoom_fade_without_flying_tiles -- --ignored --test-threads=1` on a real GTK desktop.
- [ ] Run `cargo test photo_wall_ctrl_wheel_zoom_allocations_match_every_painted_frame -- --ignored --test-threads=1`.
- [ ] Test Ctrl+wheel **in and out** in Photo Wall and Masonry, including changing row/column membership and large collections.
- [ ] Test the zoom slider and rapid repeated/reversed wheel input: no flying tiles, vertical jump, prolonged dimming or blank placeholders.
- [ ] Test switching between Grid ↔ Wall ↔ Masonry mid-animation and disabling Settings → Zoom transition animations.
- [ ] Obtain user confirmation before merging into `main`.

**Non-goal:** Animating photo positions between different wall layouts (FLIP position travel). Such motion is visually distracting and can damage scroll-anchor stability. The destination geometry is authoritative from the first frame.
