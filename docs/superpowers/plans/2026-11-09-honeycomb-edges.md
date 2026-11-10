# Honeycomb edge correction

Root cause: equal row lengths and a positive half-pitch stagger leave alternating edge holes. Correct the lattice, not its styling.

- Add regression coverage for clipped first/last photographs, alternating row lengths and unique ordered indices.
- Use N full-cell rows alternating with N+1 cells centered on both boundaries. Consume the original collection once, row by row. Preserve section ranges and existing visible-row lookup.
- Clip overhanging widgets at a width-constrained virtual surface; do not expand horizontal scroll extent. Preserve fractional polygon dimensions through integer GTK allocation.
- Add independent 2px (default) / 0px gap styles, saved as a Honeycomb-only preference and exposed on its layout button's context menu. Preserve other layouts' behavior.
- Verify geometry, ordering, picking, virtualization and anchor tests. Run the actual GTK gallery and inspect edge screenshots at multiple sizes and zoom levels for both styles under Cairo and Vulkan.

Keep existing uncommitted implementation and unrelated original-checkout changes. No commits, merges or pushes.

Completed with subsequent user refinements: horizontal policy NEVER plus a width-constrained clipping wrapper around unchanged native GtkFixed; actual GtkViewport width; default visible border 5px, adjustable under Settings → Interface → Thumbnails (None or 2–12px). Positive border changes paint inside existing geometry without changing photo dimensions or indices. The temporary context menu was removed.

Verification: 85 targeted Rust tests; six GTK smoke runs; full PIC application and actual Settings screenshots; requested 1366×768/1920×1080/2560×1440 captures at multiple scroll/zoom positions; Cairo and software Vulkan inspected. Existing formatting differences remain. See reports/gui-tests/honeycomb-edges/verification.md. No outstanding important review findings. Changes remain uncommitted.
