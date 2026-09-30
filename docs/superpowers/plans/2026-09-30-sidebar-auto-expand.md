# Sidebar automatic expansion implementation plan

Approved design: folder expansion uses spare space beneath Folders, preserves Network Shares' heading and a usable viewport when shares are populated and expanded, and scrolls the expanded branch into view when space runs out. No Photo Wall changes. Work stays on sidebar-auto-expand, separate from rc8.

## Task: bounded automatic allocation and branch reveal

- [x] Run baseline cargo test using the shared build cache.
- [x] Add a GTK regression test exercising real double-click and disclosure handlers: grow into unused space, preserve populated shares, scroll an overflowing branch into view, preserve selection, and handle collapsed/hidden shares and Reveal All.
- [x] Run the regression against existing code and confirm failure.
- [x] Add one allocation helper measuring folder rows without double-counting the heading, reserving the shares heading and up to three rows when populated and expanded. Clamp to GTK's allocated bounds; never shrink an existing usable folder pane on branch collapse.
- [x] Route local tree expansion and Reveal All through the helper; wait for layout before scrolling the parent and first revealed child into view. Cancel stale requests on subsequent changes.
- [ ] Verify full cargo test, the GTK regression, compilation, and diff scope.
- [x] Obtain independent code review and resolve material findings with regression coverage.

Review focus: unallocated/hidden panes, interrupted expansion animations, manual splits, large trees, selection/navigation preservation. No automatic commit/push of this feature until requested.

Checkpoint: full suite, network-share GTK test, cargo check and diff scope passed. Default-backend growth regression failed intermittently while X11 passed; verification checkbox stays open pending investigation. User manually tested and reported it seems to work. See sidebar-auto-expand-progress.md.
