# Sidebar automatic expansion — resume checkpoint

User requested saving progress before tokens run out. Preserve this isolated worktree and branch. Do not merge into rc8 or push this feature without a later instruction.

Worktree: /home/peet/picasa-iphoto-clone/.worktrees/sidebar-auto-expand
Branch: sidebar-auto-expand
Base: 2b4505c (rc8 menu-only change, already pushed to origin/rc8).
Primary checkout /home/peet/picasa-iphoto-clone remains on rc8.

## Approved behavior

Expanding local folders with double-click, disclosure arrow, or Reveal All should grow the folder pane into available vertical space. Keep Network Shares heading reachable; reserve up to three share rows when populated and expanded. Empty/collapsed/hidden shares release space. Overflow scrolls the expanded parent and first child into view. Preserve selection, counts, navigation, passive location marker, network behavior. No Photo Wall implementation changes.

## Implemented, verification still in progress

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
- Final expanded GTK regression PASSED: 1 passed, zero failures, 485 filtered out; includes both review-fix regressions. /tmp/sidebar-final-gtk.log. Compilation succeeded with warnings. Full suite on final code remains pending.
- git diff --check passed.
- Formatted changed code; preserved two pre-existing formatting discrepancies (shift_tab wrapping and extra blank before refresh).

## Resume next

1. Read final GTK log and confirm test result. If needed rerun:
   CARGO_TARGET_DIR=/home/peet/picasa-iphoto-clone/target cargo test --offline folder_expansion_grows_without_hiding_shares_and_scrolls_overflow -- --ignored --test-threads=1
2. Run full cargo test --offline on final code. Run existing ignored saved_network_shares_populate_at_startup_and_use_the_remaining_height separately (GTK tests in separate processes avoid GTK thread affinity).
3. Run cargo check --offline and git diff --check; inspect diff vs 2b4505c.
4. Update plan checkboxes/results. Independent review completed; its two material findings now have regression coverage and fixes, awaiting final green.
5. Report feature ready on isolated branch; leave rc8 untouched. Do not automatically merge/push.

User asked to save progress, not abandon or discard the work. No remote feature branch published.
