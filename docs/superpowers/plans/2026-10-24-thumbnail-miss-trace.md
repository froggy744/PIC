# Thumbnail Miss Lifecycle Trace Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add an opt-in URI-filtered trace, use it to identify missing-thumbnail stalls, and keep visible missing tiles eligible for generation with a focused regression test.

**Architecture:** A small shared helper reads `PIC_THUMBNAIL_TRACE_URI`, matches it as a substring against the source URI, and emits consistent event records. The editor, thumbnail and grid modules call the helper at the editor, worker and GTK lifecycle boundaries. Visible grid scans retain missing, non-corrupt placeholders in the generation wanted-set so retries for still-visible remote photos are not treated as stale work.

**Tech Stack:** Rust, GTK4, existing inline unit tests.

**Spec:** `docs/superpowers/specs/2026-10-24-thumbnail-miss-trace-design.md`

## Global Constraints

- The trace is opt-in and URI-filtered.
- Cache the environment filter on first use to avoid repeated environment lookups in tile-bind hot paths.
- Keep existing trace switches and unrelated rendering behavior unchanged.
- The focused test must not require GTK or network access.
- Wall-quality generation has separate admission and presentation queues; this trace targets the ordinary grid-thumbnail path.

## Review Focus

- URI substrings with spaces and percent encoding — matching uses the literal URI observed at the call site.
- Empty or unset filter — helper emits no records when the filter is empty or absent.
- Nonmatching URI — unrelated photos must not appear in the filtered trace.
- Queue rejection, deduplication, or eviction — events must distinguish lost requests from accepted work.
- Missing-to-refresh lifecycle — event names must make stage progression and a stalled boundary clear.

---

### Task 1: Shared URI-filtered trace helper

**Files:**
- Create: `src/thumbnail_trace.rs`
- Modify: `src/main.rs`
- Test: `src/thumbnail_trace.rs`

**Interfaces:**
- Produces: `crate::thumbnail_trace::event(uri: &str, stage: &str, outcome: &str)` and a pure filter matcher used by its unit test.

- [x] **Step 1: Write the failing URI filter test**

Test `uri_filter_matches_only_the_selected_substring` for matching, nonmatching, and empty filters.

- [x] **Step 2: Run the focused test and confirm the expected failure**

Run: `cargo test --locked thumbnail_trace::tests::uri_filter_matches_only_the_selected_substring -- --test-threads=1`

- [x] **Step 3: Implement the helper and register the module**

Read and cache `PIC_THUMBNAIL_TRACE_URI` on first use; emit one `PIC_THUMBNAIL_FLOW` line only when the non-empty filter occurs in the URI.

- [x] **Step 4: Run the focused test and confirm it passes**

Run: `cargo test --locked thumbnail_trace::tests::uri_filter_matches_only_the_selected_substring -- --test-threads=1`

### Task 2: Trace thumbnail lifecycle boundaries

**Files:**
- Modify: `src/thumbnail_display.rs`
- Modify: `src/thumbnail.rs`
- Modify: `src/thumbnail/cache.rs`
- Modify: `src/grid/virtualization.rs`
- Modify: `src/grid/tile.rs`
- Modify: `src/window/build.rs`

**Interfaces:**
- Consumes: `crate::thumbnail_trace::event(uri, stage, outcome)`.

- [x] **Step 1: Trace display queue admission, worker outcome, and retry**

Emit events for queued, duplicate, capacity rejection, eviction, worker start, and loaded/missing/failed completion, carrying URI and visible priority where available.

- [x] **Step 2: Trace source-generation disposition and result**

Emit events for already-cached, decode-failed suppression, unavailable suppression, queued generation, generation start, cache write, and generation error.

- [x] **Step 3: Trace editor transitions and completion delivery/tile refresh**

Emit editor open/save/close events. Also emit events when a generated path is handed to the UI, when a matching tile is refreshed, and when a paintable is applied or a missing placeholder is settled.

- [x] **Step 4: Run the targeted test and compile check**

Run the URI-filter test above, then `cargo check --locked`.

- [x] **Step 5: Run the relevant GUI harness if available**

Use `PIC_GUI_SUITE=library scripts/test-gui.sh`; report an unavailable display or blocked harness without attributing it to the change.

### Task 3: Keep visible missing placeholders eligible for generation

**Files:**
- Modify: `src/grid/virtualization.rs`

- [x] Add a focused test that missing, non-corrupt placeholders stay eligible while loaded and corrupt tiles do not.
- [x] Apply the candidate rule to both regular and Folder visible scans. Route Folder visible requests through `replace_visible_requests` to update the generation wanted-set.
- [x] Verify with the focused test, `cargo check --locked`, and the album URI trace.
