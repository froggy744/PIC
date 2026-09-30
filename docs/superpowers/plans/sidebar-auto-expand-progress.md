# Sidebar automatic expansion — resume checkpoint

User requested saving progress before tokens run out. Preserve this isolated worktree and branch. Do not merge into rc8 or push this feature without a later instruction.

Worktree: /home/peet/picasa-iphoto-clone/.worktrees/sidebar-auto-expand
Branch: sidebar-auto-expand
Base: 2b4505c (rc8 menu-only change, already pushed to origin/rc8).
Primary checkout /home/peet/picasa-iphoto-clone remains on rc8.

## Approved behavior

Expanding local folders with double-click, disclosure arrow, or Reveal All should grow the folder pane into available vertical space. Keep Network Shares heading reachable; reserve up to three share rows when populated and expanded. Empty/collapsed/hidden shares release space. Overflow scrolls the expanded parent and first child into view. Preserve selection, counts, navigation, passive location marker, network behavior. No Photo Wall implementation changes.

## Implemented; verification completed after fixing the GTK test wait

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

## Latest verification — completed

- The product layout code is unchanged from efce336. Current src/sidebar.rs edits are confined to cfg(test).
- The GTK regression previously assumed 400 milliseconds implied completed allocation. Test-only settle_sidebar_layout now waits for completed paints after both section animations, with a five-second timeout. It reports display/frame readiness separately from sizing assertions.
- Frame diagnostics showed successful Wayland growth across two layout frames. Three diagnostic runs passed. The original failing run did not record frame counts, so its exact compositor timing cannot be retrospectively proven.
- Updated GTK regression passed on default Wayland: 1 passed, zero failures, 4.19s. /tmp/sidebar-frame-barrier-wayland-final.log
- Updated GTK regression passed on X11: 1 passed, zero failures, 4.09s. /tmp/sidebar-frame-barrier-x11-final.log
- Verified the revised test still detects a real bug: temporarily omitting automatic pane growth caused failure (old=70, new=70, desired target=220). /tmp/sidebar-frame-barrier-mutation.log. Temporary mutation removed before final passing runs.
- Final full suite passed: 442 passed, zero failures, 44 ignored. /tmp/sidebar-verified-suite.log
- Existing network-share GTK test and cargo check passed earlier; no product code changed afterward. Diff whitespace check passed.
- User said the amount of testing was excessive. Stop further testing: essential verification is complete, no further diagnostics scheduled.

## Search-result sidebar focus follow-up

User manually confirmed automatic expansion works but reported that search navigates the grid correctly without focusing the sidebar's target folder.

- Reproduced with collapsed Folders and a nested destination: selected target row was y=1099, height=36, viewport=389 after expansion; the target remained outside the visible sidebar. /tmp/sidebar-search-red.log
- Root cause: scroll_to_folder used immediate focus/scroll plus fixed 100 ms recursive retries, which ran before pane animation/allocation and could be overwritten by scroll restoration. It also read FOLDER_PANE_SAVED_KEY from the paned instead of the outer sidebar.
- Fixed only src/sidebar.rs: expand/switch Tree mode synchronously, restore saved height from sidebar, then schedule a Destination reveal through the same generation-protected frame-completion path used for automatic growth. focus_folder_destination selects/focuses and scrolls the allocated row without navigation callbacks. Removed fixed retry timers. Newer search requests supersede old pending reveals.
- New GTK regression search_folder_reveal_waits_for_layout_and_expands_collapsed_section passed; covers collapsed section, nested target visibility/focus, imported-only to Tree mode, and successive destinations. /tmp/sidebar-search-final.log
- Existing growth regression passed after fix. /tmp/sidebar-growth-after-search.log
- Full suite passed: 442 passed, zero failures, 45 ignored. /tmp/sidebar-search-suite.log
- No Photo Wall, grid, or window/search code changed. No further automated testing needed absent a new failure.
- These search fix/test changes remain uncommitted in sidebar-auto-expand. User should restart the sidebar build to verify search-result focus manually before integration.

## Resume next

No required implementation or automated testing remains. Test synchronization improvement, search-result focus fix, and these notes are saved in the worktree but not committed. Product implementation checkpoint is efce336; prior progress commit is 78377b2. User can request a final commit/push or integration later. Keep sidebar-auto-expand separate; do not automatically merge into rc8 or push.

User manually tested and reported it seems to work. Database identity remains unverified. Do not disturb the running app or reindexing. Primary rc8 checkout remains unchanged.

## Integration authorized by user

User reports search focus still does not always work and explicitly requested pushing to rc8, with further fixes to continue on rc8. Treat intermittent search focus as OPEN despite passing synthetic tests. Automatic expansion was manually confirmed working. No more tests for this integration step. Commit remaining changes, merge sidebar-auto-expand into rc8, and push origin/rc8. Preserve the sidebar worktree.
