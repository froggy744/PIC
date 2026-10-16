# PIC GUI test report

- Overall: **COMPLETE**
- Commit: `3ce4f39890b25087f6ab5c2d5c5e067af0384c07`
- Display: Xvfb `:2` (1600×1000×24), GTK backend X11, Cairo renderer
- Application binary: `/home/peet/PIC/target/release/pic-rs`
- Artifact directory: `reports/gui-tests/merge-audit-3ce4f39`
- PIC process: exited during cleanup
- Counts: 38 PASS, 0 FAIL, 0 BLOCKED, 0 NOT TESTED
- Data/config: generated test photos and isolated XDG directories under this run directory

## Regression fixes verified

- **Edit Undo/Redo:** slider drags changed the recipe while Undo remained disabled until pointer release. The session now exposes changed active slider actions to Undo and refreshes history controls during slider changes. Single and multiple-tool Undo/Redo GUI sequences restored the expected rendered pixels (all center-pixel deltas were 0); reset, save/reopen, and original-file hash checks also passed.
- **Collage preview/export:** the preview's fitted-image offset inside `GtkFixed` was not consistently reflected in its allocation, while export centered the fitted image. The preview now fits content within the full tile, and preview/export share aspect-fit geometry. The measured normalized geometry error was `0.0118` (below the `0.02` tolerance) on a 683×683 canvas; the export was a nonblank 3840×3840 JPEG.

## Test matrix

| Feature | Action performed | Expected | Actual | Result |
|---|---|---|---|---|
| PIC startup and Library | Launch PIC on a new Xvfb display and capture the visible main window | PIC opens and renders its Library window | window 4194309; screenshot=01-library-home.png | PASS |
| Isolated sample gallery | Load the generated temporary photo fixture and query its catalog rows | Generated photos load into the temporary library | 15 generated photos indexed; screenshot=02-library-loaded.png | PASS |
| Photo rating persistence | Select landscape.jpg, press 3, and query the SQLite rating | Pressing 3 rates the selected image and saves that rating | database photo rating=3; screenshot=02a-rating-three-stars.png | PASS |
| Folder navigation and photo membership | Click the imported fixture folder and compare catalog membership and visible photos | Opening the generated folder shows its 15 catalogued photos | folder database count=15; visible generated tiles=12; screenshot=02b-folder-view.png | PASS |
| Folder rating filter membership | Open the folder filter, select three stars, and verify the visible filename and stored filter | Three-star folder filter shows exactly the rated photo | setting=3; visible fixture tiles=[(518, 178, 'landscape.jpg')]; screenshots=02c-rating-filter-menu.png, 02d-folder-rated-filter.png | PASS |
| Folder rating filter reset | Click Clear and verify the full fixture membership returns | Clearing the rating filter restores the folder photos | visible fixture tiles=12; result=02e-folder-rating-filter-cleared.png | PASS |
| Import chooser cancellation | Open the real folder picker, press Escape, and compare database counts | Escape closes the picker without changing indexed photos | photo count before/after=15/15; screenshots=02f-import-cancel-dialog.png, 02g-import-cancelled.png | PASS |
| GUI local folder import | Choose the generated folder through the GTK chooser and query imported paths | Selecting the temporary import directory indexes both photos | photos indexed from GUI-selected path=2; total library photos=17; screenshots=02h-import-source-chooser.png, 02h1-import-home-listing.png, 02h2-import-folder-selected.png, 02i-import-complete.png | PASS |
| Duplicate GUI import | Repeat the folder-picker import and verify row count stays unchanged | Importing the same source folder again does not duplicate photo rows | photo count after repeat import=17; screenshots=02j-duplicate-import-chooser.png, 02k-duplicate-import-result.png | PASS |
| Gallery scrolling | Scroll the gallery with the mouse wheel and compare visible filenames | Mouse wheel scrolls the populated gallery | visible photo sequence before/after=['landscape.jpg', 'sample-01.jpg', 'sample-04.jpg', 'sample-07.jpg', 'portrait.jpg', 'sample-02.jpg', 'sample-05.jpg', 'sample-08.jpg', 'sample-00.jpg', 'sample-03.jpg', 'sample-06.jpg', 'sample-09.jpg']/['sample-04.jpg', 'sample-07.jpg', 'sample-10.jpg', 'sample-05.jpg', 'sample-08.jpg', 'sample-11.jpg', 'sample-06.jpg', 'sample-09.jpg']; screenshot=03-gallery-scrolled.png | PASS |
| Gallery drag selection | Drag a selection rectangle across thumbnails and count selected badges | Mouse drag changes gallery selection | visible selected-thumbnail badges=4; screenshot=03b-gallery-after-drag.png | PASS |
| Gallery double-click opens lightbox | Double-click the selected thumbnail and verify fixture image pixels | Double-click displays the selected landscape photo | landscape pixels displayed=727110; screenshot=04-after-double-click.png | PASS |
| Lightbox next-photo navigation | Press Right and verify portrait pixels in the lightbox viewport | Arrow Right displays the next sample photo | Arrow Right displayed portrait.jpg color in 405695 pixels; screenshot=05-lightbox-next-photo.png | PASS |
| Lightbox zoom and 1:1 | Drag the zoom slider, press Space, and measure native image dimensions | Zoom slider enlarges the image and Space selects native 1:1 scale | landscape image bounds: Fit=(695, 102, 1218, 886), plus=(417, 102, 1497, 886), 1:1=(657, 102, 1257, 886); screenshots=06-lightbox-zoom.png, 07-lightbox-one-to-one.png | PASS |
| Lightbox 1:1 panning | Drag the native image and measure the fixture edge marker displacement | Dragging the 1:1 image moves its rendered position | native image bounds before/after=(657, 102, 1257, 886)/(657, 102, 1257, 886); clipped edge marker pixels before/after=2268/6; screenshot=07a-lightbox-panned.png | PASS |
| Lightbox Escape dismissal | Press Escape and verify the gallery returns | Escape returns to the gallery | Gallery screenshot=08-gallery-after-viewer.png | PASS |
| Gallery right-click opens photo context menu | Right-click the photo and capture the open context menu | Right-click displays photo actions | Context menu visible; screenshot=09-photo-context-menu.png | PASS |
| Photo editor opens | Choose Edit Photo and verify editor controls and source-image canvas | Edit Photo action opens the selected image and adjustment controls | PIC window title='PIC'; editor split-divider pixels=700; source image bounds=(779, 317, 1502, 739); screenshot=10-photo-editor.png | PASS |
| Editor exposure adjustment | Move the Exposure slider and compare rendered image pixels | Exposure interaction changes preview pixels | preview center RGB delta=203; screenshot=11-editor-exposure-adjusted.png | PASS |
| Editor Undo/Redo sequences | Undo and redo single and multiple adjustment tool changes and compare rendered pixels | Single and multiple tool edits undo and redo independently in order | single undo delta=0, single redo delta=0; contrast undo delta=0, exposure undo delta=0, two-step redo delta=0; sequence_errors=[]; screenshots=11a-editor-undo.png, 11a-editor-redo.png, 11a-editor-multiple-adjustments.png, 11a-editor-undo-contrast.png, 11a-editor-undo-exposure.png, 11a-editor-redo-multiple.png | PASS |
| Editor Reset | Change Exposure, click Reset, and compare preview pixels with the original render | Reset all edits restores the original rendered pixels | preview center RGB delta from original=0; screenshot=11b-editor-reset.png | PASS |
| Editor filters panel | Click Filters and capture the resulting controls | Filters tab displays filter controls | Filters screenshot=12-editor-filters.png | PASS |
| Editor save, reopen data and original safety | Save a nonzero recipe, query SQLite, and compare source SHA-256 | Done persists nonzero edits in the catalog while preserving the source file | saved recipe='v=1/crop=0.00000,0.00000,1.00000,1.00000/straighten=0.0000/exposure=0.9221/contrast=0.0000/fill=0.0000/highlights=0.0000'; original SHA-256 unchanged=True; screenshot=13-gallery-after-editor.png | PASS |
| Editor persistence after reopening | Reopen the photo and compare preview pixels and saved recipe | Reopening the photo restores the saved adjustment preview | preview RGB delta from original=203; catalog recipe unchanged=True; screenshot=13a-editor-reopened-persisted.png | PASS |
| Gallery multi-photo selection | Ctrl-click a second thumbnail and count selected badges | Ctrl-click selects a second generated photo | visible selected-thumbnail badges=2; screenshot=14-gallery-multiselection.png | PASS |
| Collage action from context menu | Right-click the multiselection and invoke Create Collage | Multiple selection enables Create Collage | Context menu screenshot=15-collage-context-menu.png | PASS |
| Collage preview opens | Capture the live collage editor preview | Create Collage opens the collage preview/editor | Collage screenshot=16-collage-preview.png | PASS |
| Collage export options | Click Create Collage and inspect the export settings dialog | Export options are shown before choosing a destination | Options dialog screenshot=16a-collage-export-options.png | PASS |
| Collage export destination chooser | Click Export and choose the temporary output path | Save chooser accepts an isolated output path | Save chooser screenshot=16b-collage-export-destination.png | PASS |
| Collage export image validation | Open the saved image and verify format, dimensions, and pixel variance | Exported image opens, is nonblank, and has expected dimensions | rc6-collage.jpg: JPEG (3840, 3840), expected (3840, 3840) | PASS |
| Collage preview/export consistency | Compare normalized preview canvas pixels with exported pixels | Each photo has matching normalized x/y position and width/height | normalized max geometry error=0.01171303074670571; preview boxes=[(102, 0, 583, 280), (0, 0, 683, 683)]; export boxes=[(107, 8, 575, 281), (0, 0, 683, 683)]; canvas=(683, 683); screenshots=16-collage-preview.png, export=rc6-collage.jpg | PASS |
| Collage Exit action | Exit the collage editor and verify gallery return | Exit returns from collage editor to gallery | Post-collage screenshot=17-gallery-after-collage.png | PASS |
| Gallery layout control interaction | Open the layout control and capture its changed state | Layout button visibly changes gallery layout or presents layout choices | Layout control response screenshot=19-gallery-layout-control.png | PASS |
| Screensaver setup | Open style controls and start playback through the GUI | Play control opens screensaver style settings | Setup controls shown; screenshot=20-screensaver-setup.png | PASS |
| Screensaver window and fullscreen geometry | Query the visible screensaver X11 window geometry | Screensaver window is visible and covers the X11 display | window=4195090; geometry=1600x1000; screenshot=21-screensaver-running.png; wm=Openbox | PASS |
| Screensaver photo playback and transition | Capture the slideshow before and after its configured interval | A different generated image appears after the configured interval | Transition screenshot=21a-screensaver-transition.png | PASS |
| Screensaver keyboard exit | Press Escape and capture return to the main window | Escape exits screensaver and returns to PIC | Returned to PIC; screenshot=22-screensaver-keyboard-exit.png | PASS |
| Screensaver mouse exit | Restart playback, move/click the mouse, and verify dismissal | Mouse input exits screensaver and returns to PIC | Mouse input dismissed fullscreen screensaver and returned to PIC; setup=23-screensaver-setup-again.png; screenshot=24-screensaver-mouse-exit.png | PASS |

## Startup diagnostics

GTK/resource/startup scan: no matched critical errors

## Artifacts

- `application.log`
- `harness.log`
- `xvfb.log`
- `screenshots/`

## Coverage limitations

- Screensaver fullscreen assertions used X11 window manager: `Openbox`; geometry and transitions are measured only when its test cases pass.
- Original-file mutation is checked by SHA-256 against generated fixture photos; RAW decoding and external-device behavior were not tested.
- The isolated session reports portal backend and PipeWire availability warnings in `harness.log`; these did not block the exercised views.
