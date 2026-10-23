# Orientation Filter Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let users filter photos by portrait, landscape, square, or unknown orientation.

**Architecture:** Add a computed `Field::Orientation` expression to the existing filter/facet query layer, using catalog width and height. Include Orientation in the default visible metadata filters; existing dropdown and explorer code discover new fields through `Field::ALL`.

**Tech Stack:** Rust, rusqlite, GTK4/libadwaita.

**Spec:** `docs/superpowers/specs/2026-10-23-orientation-filter-design.md`

## Global Constraints

- Derive orientation from catalog dimensions without reading source files or adding schema.
- Missing or zero dimensions resolve to Unknown.
- Preserve existing browsing and filter persistence behavior.

## Review Focus

- Missing or zero dimensions return Unknown.
- Equal dimensions return Square.
- Portrait and landscape use width/height ordering, without relying on EXIF rotation.
- Existing saved filter selections continue to deserialize when the enum grows.

---

### Task 1: Add orientation as a metadata facet

**Files:**
- Modify: `src/db/filters.rs`
- Modify: `src/window/metadata_panel.rs`

**Interfaces:**
- Produces: `Field::Orientation`, selectable through existing filter menus and explorer; its facet/query values are `Portrait`, `Landscape`, `Square`, and `Unknown`.

- [ ] Add the enum variant and label, include it in `Field::ALL`, and implement a SQL CASE expression based on positive `photos.width` and `photos.height`: lower width is Portrait, greater width is Landscape, equal width is Square, and absent/nonpositive dimensions are Unknown.
- [ ] Add `Field::Orientation` to `DEFAULT_VISIBLE_FILTERS` so it is present on a fresh panel by default; existing saved selections are left intact.
- [ ] Review the field labels, generated SQL expression, and saved-selection behavior against the design. No database migration or metadata extraction changes are needed.
