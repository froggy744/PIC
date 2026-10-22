# Auto Enhance Design

## Goal

Restore Smart Auto V3.1 as a separate non-destructive Auto Enhance option in PIC V1.01 while preserving the existing Auto Colour, Auto Contrast, and Auto Brightness behavior.

## Design

Add `auto_enhance` to `EditRecipe`, its version 1 encoding, default/reset semantics, and UI history via the existing recipe toggle path. Auto Enhance runs as a new stage in `apply_tone`, after the existing individual auto adjustments, using Smart Auto V3.1's bounded regional analysis and tonal mapping. It never rewrites the source or changes dimensions. Preserve the V3.1 low-light and sunset behavior, and conservatively reduce broad and selective lifting on already well-lit portrait-like midtone distributions.

The editor's existing GPU support predicate will reject recipes with Auto Enhance. Those requests use the existing CPU preview worker and shared `apply_recipe` pipeline. Supported recipes remain on the current GPU path. Export and all other CPU rendering entry points already use the shared render pipeline, so the persisted option applies consistently to FIT, 1:1, JPEG, RAW, and export.

Place four Quick Fixes in a two-column grid: Auto Enhance and Auto Contrast on row one; Auto Colour and Auto Brightness on row two. Preserve each existing control's recipe field and callback unchanged.

## Verification

Add focused engine tests for dark-scene recovery, a midtone portrait guard, sunset sky preservation, high-key highlight preservation, deterministic output, and alpha/dimension preservation. Add recipe persistence/legacy decoding and undo/redo/reset tests, shared renderer tests, and GPU support predicate tests proving Auto Enhance alone selects CPU fallback while supported edits remain eligible for GPU. Run targeted Rust tests and `cargo check`; run GUI verification if the environment supports launching PIC.

## Limits

The recovered algorithm works on PIC's decoded display-referred RGBA pixels, including decoded RAW output. It cannot recover sensor data clipped by the upstream RAW decoder or replace scene-linear/profile-managed RAW development. The CPU fallback can be slower on large NEFs; analysis remains bounded by the engine's two-dimensional sample grid.
