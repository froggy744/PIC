# Docked Properties inspector — review and QA checklist

Branch: `feature/docked-photo-properties` · PR #168

## Scope

Properties is a right-hand *gallery inspector*, not a modal dialog. It is mounted within the gallery/viewer band, above the persistent bottom infobar. `main` is unchanged until reviewed and merged.

## Code review (implemented)

- [x] Context menu Properties opens a docked panel rather than an adw::Dialog.
- [x] Panel has an explicitly labelled/tooltip-equipped close button.
- [x] Closing clears inspector context and hides the panel; grid width returns.
- [x] A different selected photo refreshes metadata while the panel is active.
- [x] Clearing selection closes the inspector instead of leaving stale details.
- [x] A lightbox visibility transition hides the panel and restores it on return if still active.
- [x] Invoking Properties while in lightbox does not force the inspector visibly over the lightbox.
- [x] Inspector is a sibling of the grid/viewer area, **not** of the infobar.
- [x] The inspector content has its own scroll container for overflow.
- [x] Compact key/value rows and a smaller preview use less vertical space.
- [x] File path and other long text can wrap and be selected.
- [x] Camera section stays hidden when there is no camera metadata.

## User-verified on 2K display (2026-10-12)

- [x] Metadata scrolling works.
- [x] Closing Properties works.
- [x] Opening lightbox hides metadata; returning restores it.
- [x] Infobar is visible below the inspector after gallery-band fix.
- [ ] Confirm new draggable divider actually resizes inspector and grid.
- [ ] Confirm the compact 300px initial width on a 2K screen.
- [ ] Confirm only one Close button and no duplicate minimize/maximize controls.

## Manual GUI regression checks — outstanding

- [ ] `cargo fmt --all -- --check` and `cargo check` pass on Fedora with GTK4/libadwaita installed.
- [ ] Open Properties on one grid photo; verify thumbnail, filename, size, date, dimensions, rotation, favourites.
- [ ] Select another photo and check preview and all text update (no stale values).
- [ ] Close Properties with the X and reopen from context menu; no empty/stuck panel.
- [ ] Clear selection: panel closes; choose another image and reopen normally.
- [ ] Open lightbox with panel open: no metadata drawn over viewer. Exit: previous inspector is restored without obscuring infobar.
- [ ] From lightbox context menu, select Properties: viewer remains unobstructed.
- [ ] Check panel at 1920×1080, 1440×900, 1024×768 and narrow/minimized widths.
- [ ] Check a very tall portrait and a very wide image preview; no stretching.
- [ ] Check very long paths, missing EXIF metadata, offline originals and missing cached thumbnails.
- [ ] Scroll a long metadata panel: bottom info/actions toolbar stays fully visible, usable and full-width.
- [ ] Test grid, masonry, folder, album and search views.
- [ ] Verify light/dark themes and keyboard/mouse accessibility.

## Release gate

Do **not** merge this PR until the build and manual checks pass. In particular verify that closing Properties releases grid width, that no inspector can cover the infobar, and that lightbox never displays the inspector.
