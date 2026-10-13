# Editor: tonal-quality and filter validation (2026-10-13)

Branch: `feature/editor-tone-clarity-grain`; PR: #177. No change to originals or edit database schema.

## Findings / decisions

1. Existing Fill Light and Shadows were additive RGB offsets using nearly the same shadow mask. They could flatten shadows, shift saturation, and amplify black noise.
2. Replace them with different *luminance-masked parametric gamma curves*: Fill Light covers the lower-midrange, Shadows is concentrated in darker tones, and Highlights targets upper luminance. Apply a common RGB multiplier to retain relative channel balance.
3. Replace aggressive percentile auto-stretch with 1%-trimmed luminance bounds, capped slope, bounded shoulder/headroom and an 0.75 blend. A narrow histogram must not be mapped to hard black and white.
4. Auto Colour estimates channel gains from low-chroma midtone pixels only, blending modestly towards neutrality. An image containing only highly saturated/warm colours remains as shot instead of being forced grey.
5. Add signed midtone local contrast (Clarity, -1..+1) using a downsampled Gaussian luminance guide, with a midtone-weighted residual. Keep Sharpen separate.
6. Add deterministic monochrome Grain (0..1); same recipe and source dimensions give identical results across repeated renders.
7. Add Old Photo (desaturation, sepia/warm tone, fade, vignette, grain) and Newspaper (warm paper and deterministic 4x4 ordered black-ink halftone) to the existing non-destructive filter palette.
8. Add Filter Amount immediately below Auto Contrast/Auto Colour. Blend the selected creative look at 0–100% before manual tonal operations; leave automatic and manual corrections unaffected. Legacy recipes decode to full 100%. Keep filter chooser thumbnails at full strength.
9. Do not imply Lightroom's proprietary RAW process is reproduced. PIC operates on its decoded display RGB pixels and cannot recreate sensor highlight data already clipped before decoding.

## Project checklist

- [x] Add Clarity and Grain to EditRecipe encode/decode/default/undo/sync.
- [x] Add sliders under Edit > Tools > Detail.
- [x] Separate Fill Light, Shadows, Highlights using targeted tonal curves.
- [x] Rework Auto Contrast/Auto Colour to avoid colour/contrast exaggeration.
- [x] Apply Clarity/Grain in the shared viewer/export/render path.
- [x] Add Old Photo and Newspaper filter tiles and stable keys.
- [x] Add undoable Filter Amount slider (0–100%) for all filters, B&W and Sepia; store in recipe with backwards-compatible defaults.
- [x] Add intensity tests (zero, partial, full), including alpha and preview-thumbnail behaviour.
- [x] Add unit tests for tone, colour, clarity, grain, filter persistence, alpha.
- [x] Add GitHub Actions editor test workflow for the branch/PR.
- [ ] Automated `cargo check --locked` completed successfully on CI.
- [ ] CPU render, recipe, and filter regression suites completed successfully.
- [ ] Visual review of difficult Nikon outdoor shadow/highlight and group images.
- [ ] Preview/export parity, undo, filter switch, copy/paste and large-photo responsiveness confirmed.
- [ ] Merge only after validation and approval.

## 1:1 effects preview and pan regression

- [x] Root cause verified in editor: every completed 1:1 render reapplied `apply_canvas_one_to_one` and enqueued a fallback scroll-to-centre tick, even for an unchanged filter-only image.
- [x] Preserve the native canvas size when a quick 1600×1200 draft texture replaces the displayed pixels.
- [x] Re-anchor only on initial entry into 1:1 or when the result's image geometry actually changes; never on tone/filter pixel replacement.
- [x] Keep native full-quality refinement in a separate worker so slow 24MP work cannot block the quick draft lane.
- [x] Refine after 240 ms of no further edits, rather than requesting native pixels on every slider tick; abort an obsolete result when 1:1 is disabled.
- [x] Fix lightbox recipe-only refresh: compare the old/new crop and straightening recipe from the cached native texture; unchanged photo geometry swaps pixels without resetting 1:1 scroll.
- [x] Add pure unit tests for draft canvas dimensions and re-anchor policy.
- [x] Add ignored real-GTK regression test: pan in lightbox 1:1, paste an effect, verify scroll adjustments remain unchanged.
- [x] Headless CI passed compilation, editor render/model/filter suites, pure native-preview geometry tests, and the Xvfb lightbox 1:1 pan regression (GitHub Actions run 38385214392).
- [ ] Verify effects response, zoom/pan stability and final sharpness visually with real 24MP Nikon JPEG/NEF on Fedora.
- [ ] Confirm no regressions in Fit mode, crop/rotate geometry changes, undo/redo, and export.

## Research references
- Adobe: [Tone controls and tonal ranges](https://helpx.adobe.com/uk/lightroom-classic/desktop/help/tone-control-adjustment.html)
- Adobe: [Highlights, Shadows, Clarity and image adjustments](https://helpx-origin.aws116.adobeitc.com/ie/lightroom-classic/help/apply-local-adjustments.html)
- Adobe: [Clarity increases midtone local contrast](https://www.adobe.com/learn/lightroom-cc/web/apply-clarity-dehaze-sharpen-lightroom-cc)
- Adobe: [Texture, Clarity and Grain editing controls](https://helpx.adobe.com/ie/lightroom/web/edit-photos/apply-effects/adjust-effects.html)
