# Auto Enhance Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Restore Smart Auto V3.1 as a persisted, undoable Auto Enhance action while retaining all existing auto controls and GPU-supported edits.

**Architecture:** Add a separate recipe boolean and engine stage to the shared CPU tone path. Mark the recipe GPU-unsupported so the existing CPU preview fallback is selected only while Auto Enhance is active; exports use the same CPU reference path.

**Tech Stack:** Rust, GTK4, image crate, existing EditRecipe and renderer tests.

**Spec:** `docs/superpowers/specs/2026-10-22-auto-enhance-design.md`

## Global Constraints

- Preserve current Auto Colour, Auto Contrast, and Auto Brightness semantics and callbacks.
- Auto Enhance must not modify source resolution, dimensions, alpha, or stored original files.
- Keep Smart Auto V3.1 sunset protection and low-light recovery while reducing excessive brightening of well-lit portrait-like scenes.
- Use CPU fallback only for recipes that enable Auto Enhance; keep existing GPU eligibility for supported recipes.
- Do not commit, push, merge, or switch branches.

## Review Focus

- Legacy recipe strings must decode with Auto Enhance disabled: test old recipe decoding.
- Portrait-like midtone distributions must not receive excessive shadow lift: add a portrait luminance regression fixture.
- Low-light subjects must remain visibly recovered: add dark portrait/scene test.
- Warm sunset sky and high-key whites must remain stable: preserve V3.1 tests and run them.
- GPU recipes must fall back only when Auto Enhance is active: test both support-predicate outcomes.

### Task 1: Persist Auto Enhance and cover session history

**Files:** Modify `src/edit/model.rs`; tests in its existing module.

- [ ] Add failing encode/decode, legacy-default, non-default/reset, and undo/redo tests for `auto_enhance`.
- [ ] Run the focused model tests and confirm the new cases fail because the field/encoding is missing.
- [ ] Add the boolean to `EditRecipe`, default, encoding/decoding, and `is_default`; use existing `EditSession` history.
- [ ] Run the focused model tests and confirm pass.

### Task 2: Restore V3.1 in shared tone rendering with portrait guard

**Files:** Create `src/edit/auto_develop.rs`; modify `src/edit/mod.rs` and `src/edit/render.rs`; tests in those modules.

- [ ] Add a failing shared-pipeline test showing `auto_enhance` changes low-light pixels while legacy auto fields remain false.
- [ ] Add portrait, sunset, high-key, alpha, dimensions, and deterministic regression tests; confirm tests fail for the intended missing engine/guard.
- [ ] Adapt Smart Auto V3.1 as a standalone engine API `pub fn apply(image: &mut RgbaImage)` and preserve its bounded sampling and tone mapping. Add a conservative well-lit portrait/midtone limiter without changing the existing three controls.
- [ ] Invoke it only when `recipe.auto_enhance`; retain its placement after the existing individual auto stages.
- [ ] Run focused `auto_develop` and renderer tests; confirm all pass.

### Task 3: Wire editor and selective GPU fallback

**Files:** Modify `src/edit/controls.rs`, `src/edit/editor.rs`, and `src/edit/live_gpu.rs`; tests in GPU support module.

- [ ] Add failing support predicate tests proving Auto Enhance disables GPU support and the default/supported recipes remain eligible.
- [ ] Add the Auto Enhance toggle via `connect_toggle`, preserve all existing toggle assignments, and arrange buttons in a two-column/two-row grid.
- [ ] Include Auto Enhance in fallback diagnostics and reject it in `gpu_tone_supported`.
- [ ] Run targeted GPU predicate tests and existing available CPU/GPU parity tests.

### Task 4: Cross-entry-point verification and documentation

**Files:** Tests in `src/edit/render.rs` and `src/edit/preview.rs`; update `docs/guides/edit-mode.md`.

- [ ] Add a test proving persisted Auto Enhance is applied identically by shared preview rendering and export for a deterministic decoded fixture where the test setup permits it.
- [ ] Document Auto Enhance separately from the existing individual controls and note its display-referred RAW limitation.
- [ ] Run `cargo check` and the relevant targeted test filters once; run the full test suite at most once if focused runs indicate broad risk. Attempt GUI verification only if a usable display/runtime exists.
- [ ] Report branch, dirty state, test results, limitations, and leave changes uncommitted.
