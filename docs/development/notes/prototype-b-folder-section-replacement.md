# Prototype B Folder Section Replacement

Date: 2026-09-27
Branch: `rc6`

This is the new **Prototype B Folder section replacement** tested in the **Compare Claude/Codex Architecture** chat and promoted into the current `rc6` Folder renderer.

The sectioned Folder view keeps the flat photo model and virtualized/recycled tile architecture, while tightening section lookup and zoom anchoring:

- Uses binary search to find visible folder sections and the folder containing a photo, instead of scanning or cloning all section metadata.
- Uses those lookups for tile placement, scrolling, and zoom-anchor selection.
- Adds a top-edge anchor near the top of the gallery to avoid scroll clamping shifting the view during zoom.
- Adds three tests for section lookup boundaries and top-edge anchoring.

This replaces the previous section lookup/scan implementation inside `src/grid/sectioned_folder.rs`. The existing `rc6` sectioned-grid architecture, animations, tile recycling, selection, context menus, lightbox integration, and legacy fallback remain in place.
