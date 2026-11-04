# Photo Drift Screensaver Plan

> **For agentic workers:** implement task by task and check off progress here.

**Goal:** Add a distinct screensaver that resembles the opening three seconds of `examples/m2-res_480p.mp4`: upright photos scattered over a pale field, mostly small with occasional foreground prints, moving gently without converging on one vanishing point.

**Architecture:** Add a `Photo Drift` style to the existing screensaver picker and player. Reuse photo selection, decode, cache, edit recipe, GSK snapshot, and configured start transition. Keep Flying Photos unchanged. Give each photo a stable viewport anchor and independent low amplitude drift/depth scale; preserve aspect ratio and recycle photos through the existing bounded slots.

**Tech Stack:** Rust 2021, GTK4/GSK, existing `Player` and decode worker.

**Spec:** User request in the conversation; reference is video timestamp 0:00–0:03.

## Global Constraints

- Keep originals untouched and use the existing photo decode/cache pipeline.
- Honor Tiles with a bounded cap and existing Flip/Crossfade/Cut start settings.
- Maintain smooth, independent movement, readable coverage, and rare large foreground prints.
- Do not change Flying Photos or unrelated editor changes.

## Review Focus

- Low tile counts still cover the viewport without a grid.
- Portrait and landscape aspects remain unchanged.
- Photos begin with varied sizes and do not converge to a central point.
- Initial Flip/Crossfade/Cut transition still applies when playback starts.
- A real rendered capture resembles the reference in count, scale, and spacing.

## Progress

- [x] Inspect the reference segment and current screensaver architecture.
- [x] Add Photo Drift style and setting wiring.
- [x] Implement anchored gentle motion and varied depth in its own GSK canvas.
- [x] Add focused model tests and capture rendered output at 2, 5, and 10 seconds.
- [x] Compare captures against the video, run checks, and report status.

## Completion Notes

- Added Photo Drift as a separate selectable style; Flying Photos remains unchanged.
- The 20-photo GSK capture uses real showcase images and honors the selected
  Crossfade or Flip start transition. Stable golden-angle anchors spread cards
  across the screen, with several depth scales and low-amplitude independent drift.
- Captures at 2, 5, and 10 seconds are in `reports/photo-drift/`. The reference
  begins with a sparser field; Photo Drift grows into a denser full-screen field
  by five seconds while retaining clear photos and the same pale, borderless look.
- Verified with `cargo check --locked`, the layout/aspect and continuous-motion
  model tests, the tile-cap test, and the GTK GSK real-photo capture test.
- Follow-up: Advanced settings now save an independent 0.25×–3× movement speed.
  Photo Drift uses a fixed six-second base motion cycle; Duration still controls
  photo replacement. Persistence and timing-independence tests pass.
