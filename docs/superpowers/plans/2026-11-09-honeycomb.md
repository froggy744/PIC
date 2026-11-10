# Honeycomb Implementation Plan

Goal: functional, viewport-recycled Honeycomb in all supported photo sections.
Architecture: reuse SectionedFolderView and cached thumbnails; independent regular-hex geometry and GTK 4.12 alpha mask.
Spec: ../specs/2026-11-09-honeycomb-design.md
Constraints: no dependencies, no existing-layout rendering changes, no commits/pushes, no release builds.

- [x] Add geometry/hit-test unit tests; run to observe missing implementation, implement regular-hex layout and verify.
- [x] Integrate fourth layout and per-section preferences/picker; test persistence and scope.
- [x] Add cached GSK alpha-mask rendering, theme-aware outlines and shape containment to recycled tiles; verify GTK screenshots.
- [x] Connect date grouping including Year; preserve anchors and shared interactions; test switching and bounded realization.
- [x] Run format, locked compile check and relevant serial tests; inspect Cairo/Vulkan light/dark screenshots; final independent review.

Review focus: narrow width; partial last row; recycled tile shape reset; grouping headers; asynchronous anchor restoration.
Execution: inline, user requested continuous completion. Ledger: this checklist plus verification notes below.

Verification: 83 scoped Rust tests pass; locked compile check passes; six explicit GTK smoke tests pass including Cairo and software Vulkan screenshots and Honeycomb lightbox return. See reports/gui-tests/honeycomb/verification.md.

Rulings: reference image resolves orientation as pointy-top staggered rows. Latest repeated specification supersedes the earlier All Photos-only restriction; all existing sections retain independent layout preferences. New render APIs remain compatible with GTK 4.12. Keep branch uncommitted under AGENTS.md; do not merge or push. Preserve baseline formatting rather than reformat unrelated code.

Independent review: fixed badge clipping, shape-aware marquee selection and pointer-anchor reuse; added renderer/picking/anchor tests. Integer allocations now fit a regular polygon without stretching. No unresolved important review findings.

Follow-up: restore original section preference behavior and picker callback, add Honeycomb per-section persistence and GTK Favorites restoration coverage.
