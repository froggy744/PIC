# Sidebar automatic expansion — resume checkpoint

User requested saving progress before tokens run out. Preserve this isolated worktree and branch. Do not merge into rc8 or push this feature without a later instruction.

Worktree: /home/peet/picasa-iphoto-clone/.worktrees/sidebar-auto-expand
Branch: sidebar-auto-expand
Base: 2b4505c (rc8 menu-only change, already pushed to origin/rc8).
Primary checkout /home/peet/picasa-iphoto-clone remains on rc8.

## Approved behavior

Expanding local folders with double-click, disclosure arrow, or Reveal All should grow the folder pane into available vertical space. Keep Network Shares heading reachable; reserve up to three share rows when populated and expanded. Empty/collapsed/hidden shares release space. Overflow scrolls the expanded parent and first child into view. Preserve selection, counts, navigation, passive location marker, network behavior. No Photo Wall implementation changes.

## Implemented; intermittent GTK verification remains open

Only src/sidebar.rs product code changed:
- folder_expansion_target measures the entire list (including theme spacing), avoids double-counting the heading outside the Paned, bounds growth using max_position and reserved share viewport.
- schedule_folder_expansion uses frame callbacks, generation cancellation, and weak sidebar references. Waits for both revealer and pane-position animations, sets position, then scrolls after allocation.
- toggle_folder_branch unifies disclosure/double-click. Tree rebuild also schedules growth for keyboard/ancestor reveals.
- Reveal All uses the bounded target and preserves section reattachment/animation.
- Startup placement now waits for allocated bounds.
- Corrected outer ancestor lookup: GtkWidget.get_ancestor includes self; walk from inner scroller's parent to find outer sidebar.
- sync_filter_selection restores tree selection without clearing the passive scroll-location marker. Public set_active_filter retains navigation semantics.

## Test evidence

Build cache shared through CARGO_TARGET_DIR=/home/peet/picasa-iphoto-clone/target.
- Baseline cargo test --offline: 442 passed, 43 ignored, zero failures. /tmp/sidebar-baseline-tests.log
- New real GTK test folder_expansion_grows_without_hiding_shares_and_scrolls_overflow failed against base at missing growth. /tmp/sidebar-expansion-red.log
- Main scenarios passed before review fixes: double-click, arrow, overflow parent/first child visibility, selection, populated/empty/collapsed/hidden shares, Reveal All reattachment, rapid clicks, short window. /tmp/sidebar-small-red.log (name predates successful result)
- Independent review by sidebar_review found passive marker clearing and pane-animation race.
- Marker regression reproduced failure; fixed by separating selection sync. /tmp/sidebar-review-red.log
- Animation race reproduced with an instant revealer and interrupted native collapse/reopen: folder height reset to 70. /tmp/sidebar-animation-red.log
- Fixed animation race by storing FOLDER_PANE_ANIMATING_KEY and waiting for it to clear.
- Final expanded GTK regression PASSED: 1 passed, zero failures, 485 filtered out; includes both review-fix regressions. /tmp/sidebar-final-gtk.log. Compilation succeeded with warnings. Subsequent final full suite passed; see latest checkpoint below.
- git diff --check passed.
- Formatted changed code; preserved two pre-existing formatting discrepancies (shift_tab wrapping and extra blank before refresh).

## Latest checkpoint — user requested saving again

Implementation checkpoint: efce336. No product code changed since that commit.

- User ran the separate sidebar worktree and reported: "it seems to work". This is positive manual feedback, not proof that every scenario was tested.
- User saw offline photos and reindexing and suggested the temporary/cache database may explain it. Database identity was not verified; do not record this as confirmed. Sidebar code does not change source availability or database selection. Avoid interrupting the user's scan/app.
- Final full suite: 442 passed, zero failures, 44 ignored. /tmp/sidebar-final-suite.log
- Existing GTK network-share test: 1 passed, zero failures. /tmp/sidebar-final-network.log
- cargo check --offline: exit 0, warnings. /tmp/sidebar-final-check.log
- Final diff whitespace check passed; product scope remains src/sidebar.rs only.
- rustfmt check reports only the two pre-existing discrepancies intentionally preserved.
- New GTK regression passed previously, but the default-display rerun FAILED intermittently: shares=true old viewport height=94, new=94, requested pane position=70, max=479, actual paned height=572, calculated target=220, rows=5, revealer=true. Assertion: expansion must grow into spare space. /tmp/sidebar-final-gtk.log
- Diagnostic rerun with GDK_BACKEND=x11 PASSED: 1 passed, zero failures, 18.41s. /tmp/sidebar-final-gtk-x11.log
- The differing results suggest a display/frame timing issue but DO NOT establish its cause. Intermittent failure is unresolved; do not claim all verification is complete merely because X11 passed.
- Primary checkout rc8 clean and synchronized with origin/rc8. Feature branch remains local, unmerged and unpushed.

## Resume next

1. Investigate the intermittent default-backend GTK failure. Compare mapping, frame callbacks and allocation readiness against X11; determine whether the test waits too briefly or production scheduling needs correction. Do not hide the failure or blindly rerun until green.
2. If changing code/tests, follow regression-first debugging and rerun affected GTK test, existing network-share test, full suite, and cargo check. Use CARGO_TARGET_DIR=/home/peet/picasa-iphoto-clone/target. GTK tests should run in separate processes to avoid thread affinity.
3. Update plan results once the timing issue is understood/resolved. The independent review's two material findings already have passing regression coverage; no further review has been requested.
4. Report ready only with verified evidence. Preserve the separate sidebar worktree and rc8. No automatic merge or push without user instruction.

User asked to save progress, not discard the work. No remote feature branch published.
