# Honeycomb photo layout

## Intent and scope

Add Honeycomb as the fourth layout in every supported photo section, following the supplied photographic reference. Keep existing Grid, Photo Wall and Masonry rendering unchanged. The latest repeated specification restores layout support wherever the existing layouts are supported, with independent per-section preferences. Do not revive Timeline or modify originals. Preserve the unrelated date_browser.rs changes in the original checkout.

## Approach

Extend SectionedFolderView's viewport-recycled photo tiles, PhotoLayout switching, thumbnail presentation cache and shared MultiSelection. This is the recommended engineering choice: a separate canvas would duplicate activation, selection, context menus, availability states and accessibility. A widget for every photo would violate the large-library requirement. A CPU-masked thumbnail cache would add memory and duplicate existing presentation assets.

## Geometry

The reference settles the contradictory orientation wording: use regular pointy-top hexagons, with vertical sides, top and bottom vertices, and alternating rows shifted horizontally. For circumradius R, width is sqrt(3)*R, height is 2*R. The photograph polygon vertices are (w/2,0), (w,h/4), (w,3h/4), (w/2,h), (0,3h/4), (0,h/4). Use a five-logical-pixel perpendicular gap: center spacing horizontally is w+g and vertically is sqrt(3)*(w+g)/2. This yields constant spacing on vertical and angled edges. Fit whole cells within the viewport, adjusting column count with zoom; narrow windows fall back to one column. Keep original photo ordering row by row. Final rows contain only real photographs.

Store geometry separately and expose it through existing wall item/row/section lookup contracts where practical. Visible row search must include overlapping polygon bounds. Geometry generation may be O(n) on membership/reflow changes; scrolling and realization must depend on visible rows, not total library size. Keep floats through geometry and clipping; GTK integer allocations must not deform the polygon.

## Rendering

Use renderer-independent GTK/GSK composition supported by GTK 4.12. Inspect API availability before implementation: newer GSK path APIs cannot be assumed. Prefer a cached vector alpha-mask render node composed with the existing photo snapshot, rather than per-frame pixel masks or raw decoding. Cache masks by allocated size. Draw a theme-aware hexagonal selection outline and subtle hover inside the same polygon; remove rectangular decorations only on Honeycomb tiles. Preserve missing/offline states within the polygon. Reuse 320/640 thumbnail requests and Cover fitting. Inspect screenshots under Cairo and Vulkan; do not claim parity if either renderer cannot be exercised.

## Interaction and lifecycle

Override shape containment and check event coordinates where child picking could bypass it. Empty corners must not select or activate a photo. Keep existing click modifiers, viewer activation, menu actions, shared selection, keyboard focus and navigation. Directional navigation should use neighboring geometry rather than rectangular column assumptions. Recycled tiles must clear Honeycomb-only state when rebound to other layouts. Invalidate pending geometry/restoration work on layout/filter/model changes.

Use existing photo-ID plus viewport-offset restoration for switching, viewer return, resizing and zoom. Capture the pointer photo where Ctrl-wheel currently provides pointer anchoring; otherwise retain viewport anchor. Avoid scaling old geometry on resize if that changes the fixed gap or prevents responsive column changes. Preserve existing layout code paths.

## Preferences and grouping

Add the honeycomb persisted value using every existing photo_layout section key and the existing legacy fallback. Keep the picker and existing preferences available in all supported sections. All Photos supports None, Day and Month today; add Year grouping through its existing grouping preference machinery to satisfy the requested Year behavior. Honeycomb renders full-width non-sticky headings and restarts its geometry per date group. Keep other layouts' existing grouping presentation intact.

## Validation

Add focused inline tests for six equal edges, regular proportions, perpendicular gaps, narrow widths, partial rows, sorted membership, section bounds, visible-row lookup, shape hit testing, saved layout scope, and photo anchor restoration. Add ignored GTK tests for tile clipping/selection, recycling and layout switching. Run cargo fmt --all -- --check, cargo check --locked, and relevant serial Rust tests. Run GTK smoke tests using isolated HOME/XDG state and inspect actual screenshots in light/dark themes, multiple viewport widths and zoom levels. Exercise thousands of model photos with bounded realized tile counts. Capture limitations honestly, especially Vulkan availability and missing real-photo fixtures. Do not build release artifacts, package, merge or push.

## Workspace and delivery

Use /home/peet/.codex/worktrees/1d23/PIC, created from 3f57bf31adad7c2f4587b28c87fe9dbb949f63c1. The managed worktree currently has detached HEAD. Create a feature branch as authorized by the task before product changes. Leave changes uncommitted unless committing is explicitly authorized; report base SHA distinctly from a feature commit. No source code has been modified during design.
