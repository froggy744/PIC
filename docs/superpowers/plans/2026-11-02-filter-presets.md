# Filter Presets Implementation Plan

> Execution: implement the task in this session and verify it against the approved design.

**Goal:** Add eight fixed photographic filter presets.

**Architecture:** Extend `FilterPreset` and its existing parameter-driven rendering pipeline in `src/edit/filters.rs`. The existing preset list, labels and keys supply the editor and recipe persistence.

**Tech Stack:** Rust, image, existing PIC edit renderer.

**Spec:** Approved conversation design: eight one-tap presets, shared rendering and targeted persistence and rendering verification.

## Global Constraints

Preserve existing presets and non-destructive recipes; add no dependencies. Verify with targeted tests and `cargo check`.

## Review Focus

- Each new preset appears in the list and its key round-trips.
- Existing recipe keys retain their meaning.
- Rendered presets change representative pixels and preserve alpha.
- Classic Silver produces neutral grayscale.
- Empty images remain safe.

### Task 1: Add and verify presets

**Files:** Modify `src/edit/filters.rs`; inspect renderer and UI consumers for exhaustive matches.

- [ ] Add a failing test for the eight keys, labels, list membership, recipe round trips and representative rendering including grayscale and alpha.
- [ ] Run the targeted test and confirm it fails because the presets are absent.
- [ ] Add the eight variants, labels, keys, decoding and distinct parameter sets using the existing pipeline.
- [ ] Run targeted filter tests, `cargo check`, and formatting verification; inspect the diff.
