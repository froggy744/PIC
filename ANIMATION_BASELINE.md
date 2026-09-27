# Animation Baseline (rc7)

This branch intentionally uses a near-zero-animation UI baseline so layout and anchoring bugs can be diagnosed without presentation motion hiding the real geometry.

## Animations intentionally kept

### Lightbox open animation — KEEP EXACTLY AS IS
The lightbox shared-element open transition is the reference-quality animation and must not be changed while this baseline is active.

Current implementation: `src/lightbox/impl.rs`.

Behavior:
- Starts from the clicked thumbnail's real bounds.
- Creates a temporary `gtk::Picture` using the source paintable.
- Interpolates X, Y, width, and height from the thumbnail bounds to the fitted lightbox image.
- Duration: 200 ms.
- Easing: cubic ease-out, `1 - (1 - t)^3`.
- Backdrop opacity fades from 0 to 1 during the same transition.
- The real lightbox picture stays hidden until the transition completes.
- If the full decode is still pending, the source paintable becomes a seamless temporary backstop.
- The temporary transition picture is removed when complete.

This animation is considered correct and should be preserved.

### Sidebar expand/collapse slide — KEEP
Sidebar section reveal/collapse motion is intentional and remains enabled.

Current implementation: `src/sidebar.rs`.

### Smooth scrolling — KEEP
Smooth mouse-wheel/touchpad scrolling is intentional and remains enabled.

Do not remove or disable `src/smooth_scroll.rs` or the smooth gallery scrolling paths in `src/window/navigation.rs`.

## Animations removed for the baseline

- Gallery/GridView resize FLIP/presentation motion.
- Sectioned Folder reflow animation.
- Sectioned Folder animated jump-to-folder/photo movement.
- Library Home horizontal pan-button spring animation.
- Settings page crossfade.
- Cosmetic CSS opacity/border/shadow transitions.

## Rule for future work

Real layout changes must happen deterministically first. Do not animate tile geometry, row wrapping, section headers, or scroll corrections while debugging zoom/reflow.

If animation is added back later, add only presentation-only motion after the final geometry and anchor behavior are proven correct.
