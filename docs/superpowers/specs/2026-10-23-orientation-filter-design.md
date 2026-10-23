# Orientation filter

- **Intent:** Let photographers narrow the current library view by image orientation.
- **Behavior:** Add an Orientation filter field, deriving Portrait, Landscape, or Square by comparing catalogued width and height; missing or zero dimensions are Unknown. Keep this as a catalog query, without reading original files or adding stored metadata.
- **Scope:** Integrate with existing facets, saved filter selections, explorer columns, and SQL filtering. No changes to photo presentation or EXIF rotation behavior.
- **Approaches:** (1) Add a computed facet from width/height (chosen: consistent with current catalog, no migration); (2) persist a new orientation column (unneeded duplication); (3) show orientation only as descriptive metadata (does not satisfy filtering).
- **Verification:** Add targeted filter/facet coverage for portrait, landscape, square, and unknown dimensions; run the relevant filter tests and `cargo check` if feasible.
