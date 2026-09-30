# Photo Wall thumbnail quality

## Goal

Improve sharpness of large visible Photo Wall photographs while preserving the existing 320 px previews as an immediate fallback. Keep scrolling responsive and avoid increasing the resolution of the whole library.

## Proposed behaviour

- Use the existing 320 px cached presentation immediately, including offline photos.
- When the Wall viewport has been stationary for 500 ms, request a 640 px preview only for a visible photo whose allocated longest edge, multiplied by GTK's scale factor, exceeds 320 px.
- Use one additional resolution tier, 640 px. Never enlarge a smaller original merely to produce that size.
- Generate larger previews through the existing thumbnail generation code and worker pool. Give ordinary thumbnail requests precedence and allow at most two quality upgrades to decode concurrently.
- Cancel quality requests that are still queued when their photos leave the viewport. Already running decodes may finish and populate the cache.
- Preserve the existing availability and supported network decoding policies. A missing or unavailable original leaves the smaller presentation usable; a quality-upgrade failure must not change selection, badges, or the original's availability.
- Previously generated larger previews remain usable offline. Layout calculation and GTK callbacks perform no original reads or filesystem probes.

## Cache and presentation

Keep the normal thumbnail paths, generation policy, and PhotoObject cached-thumbnail field unchanged. Derive a deterministic `-wall640.jpg` sibling from the existing fingerprinted cache filename, including compatibility with legacy cache locations. Resolution must participate in presentation identity so small and large results cannot overwrite each other's pending requests.

Extend the existing generation functions with an explicit maximum edge. Their current callers retain 320 px. Reuse orientation, RAW preview handling, rotation and edit presentation; do not add a second decoder implementation.

Store larger presentations in the existing RAM cache infrastructure, with a separate maximum of 32 larger entries and a 64 MiB budget. Preserve the existing capacity for ordinary presentations. A tile that loses a larger cached entry continues to display its smaller preview.

Include the larger cache variants in maintenance's valid-path calculation so normal cleanup does not discard useful previews. Source fingerprint changes invalidate both resolutions through the existing cache identity rules.

Completions must match the tile's current photo, rotation, edit recipe, and requested quality. Late results must not paint a recycled tile. Switching back to Grid restores normal presentation requests and releases Wall-only quality state.

## Preserved behaviour

- Grid layout, styling, normal thumbnail resolution and prefetch.
- Photo Wall row-height zoom, Ctrl+wheel, +/- controls, reflow, viewport anchors and lightbox return focus.
- Photo ordering, section boundaries, folder headers, selection, badges and activation.
- Existing viewport loading, virtualization and overscan ranges.

## Validation

1. Existing 320 px requests keep their cache identity and output size.
2. A 640 px request produces additional detail from a sufficiently large JPEG and the supplied rotated NEF; it does not upscale a small input.
3. Both tiers apply orientation, user rotation and edits correctly.
4. Larger previews are requested only for oversized visible tiles after the stationary delay, never for overscan or prefetch.
5. Missing originals, failed upgrades and offline operation retain the usable smaller preview.
6. Request deduplication, queue cancellation, concurrency limits, late-result rejection and RAM bounds hold.
7. Cache maintenance retains current larger variants and removes stale ones normally.
8. Existing Wall geometry, anchors, toggle, selection and lightbox-return regressions pass.
9. Record synthetic large-library tile counts and frame intervals; distinguish these from real RAW decoding performance.

## Scope

This is a separate cache and rendering change following the committed hover/selection polish. The design is ready for review; no production thumbnail-quality changes have been made.
