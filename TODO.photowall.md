TASK: Add a "Photo Wall" / justified mosaic view to PIC

You are working on my Rust GTK4 photo-management application, PIC, which is inspired by Picasa/iPhoto.

IMPORTANT:
Before changing code, inspect the existing project architecture carefully.

Do NOT rewrite the existing photo grid.
Do NOT replace the current virtualization system.
Do NOT duplicate thumbnail loading/caching logic.
Do NOT break the existing Grid view.
Reuse the existing photo model, thumbnail cache, selection system, scrolling, folder sections, and virtualization wherever possible.

The new feature is an ADDITIONAL VIEW MODE called:

    Photo Wall

============================================================
1. CURRENT APPLICATION
============================================================

The application currently has:

- All Photos
- Favourites
- Recently Added
- History
- Albums
- Folder views
- Network Shares

Photos are currently displayed using the existing regular grid.

The existing grid supports:

- thumbnail loading
- virtualization
- scrolling
- photo selection
- multi-selection
- opening photos/lightbox
- folder sections
- zoom/thumbnail sizing
- RAW/NEF images
- cached thumbnails

All of those behaviours must continue working.

============================================================
2. NEW VIEW MODES
============================================================

Introduce a clean layout/view abstraction such as:

    enum PhotoLayout {
        Grid,
        PhotoWall,
    }

Use whatever architecture fits the existing project best.

Do NOT force this exact enum if the project already has a suitable
view/layout abstraction.

The important requirement is that Grid and Photo Wall use the SAME
underlying photo collection and existing photo infrastructure.

============================================================
3. PHOTO WALL DESIGN
============================================================

Photo Wall must be a JUSTIFIED MOSAIC layout.

This is NOT:

- Pinterest masonry
- fixed square thumbnails
- CSS-style columns
- a normal GTK Grid
- random-sized thumbnails

Photos must preserve their original aspect ratios.

Example:

+----------------+--------+----------------------+
|                |        |                      |
|   Landscape    |Portrait|      Landscape       |
|                |        |                      |
+----------------+--------+----------------------+

+--------+------------------------+--------------+
|        |                        |              |
|Portrait|       Landscape        |    Square    |
|        |                        |              |
+--------+------------------------+--------------+

Each row normally has one common height.

Photo widths are determined from their aspect ratios.

Photos should be accumulated into a row until the row can reasonably
fill the available viewport width.

Then calculate the row height so that the row fills the available width
while maintaining every photo's aspect ratio.

Conceptually:

    aspect_ratio = image_width / image_height

For a row containing N photos:

    available_photo_width =
        viewport_width - total_horizontal_gaps

    row_height =
        available_photo_width / sum(aspect_ratios)

Then:

    photo_width = row_height * aspect_ratio

Account properly for padding, spacing and rounding.

The row should visually fill the available width.

============================================================
4. TARGET ROW HEIGHT / ZOOM
============================================================

The existing bottom zoom/thumbnail-size slider must work in Photo Wall.

Grid mode:
    slider controls the existing grid thumbnail size.

Photo Wall:
    slider controls TARGET ROW HEIGHT.

Smaller target height:
    more photos per row / denser wall.

Larger target height:
    fewer photos per row / larger photos.

Do NOT require a separate Photo Wall zoom control.

Changing the slider should immediately recalculate the Photo Wall
layout.

============================================================
5. SPACING
============================================================

Photo Wall should be visually dense.

Use approximately:

    3-6 px

between photos, or derive an appropriate small value from the existing
theme/layout constants.

Do not use the large spacing of the existing normal grid.

Photos should still have enough separation to remain visually distinct.

============================================================
6. LAST ROW
============================================================

Do NOT aggressively stretch the final incomplete row.

For example:

    [ photo ][ photo ][ photo ]

is preferable to making three photographs enormous merely to fill the
entire width.

Use the target row height for an incomplete final row and LEFT ALIGN it.

A reasonable tolerance around the target row height is acceptable.

============================================================
7. FOLDER VIEW -- VERY IMPORTANT
============================================================

Photo Wall must work with the existing folder-section behaviour.

Folder boundaries MUST remain visible.

Example:

    SPAR 2026                         138 photos
    ------------------------------------------------

    [photo][ portrait ][      landscape       ]
    [   landscape   ][photo][ portrait ]

    R4C                               243 photos
    ------------------------------------------------

    [ portrait ][ landscape ][ landscape ]
    [      landscape      ][photo][photo]

Each folder is an INDEPENDENT mosaic section.

NEVER allow a justified row to cross a folder boundary.

When Folder A ends:

    Folder A
    [photo][photo][       photo       ]
    [photo][photo]

    ------------------------------------------
    Folder B

    [photo][     photo     ][photo][photo]

The last row of Folder A should be handled as an incomplete final row.

Then render the Folder B header.

Then begin a completely new Photo Wall row calculation for Folder B.

Existing folder headers, names, counts, collapsing/expanding behaviour,
and continuous scrolling must continue to work.

============================================================
8. OTHER COLLECTIONS
============================================================

Photo Wall should eventually work with the same photo sources as Grid:

- All Photos
- Favourites
- Recently Added
- Albums
- individual folders
- parent folders containing multiple folder sections
- Network Shares where supported

Do NOT create separate Photo Wall data-loading code for every source.

The layout should operate on the photo items supplied by the existing
view/data layer.

============================================================
9. VIEW MODE UI
============================================================

Look at the existing bottom-right toolbar.

There is already a grid/view-related icon.

Prefer extending this control instead of adding another unrelated
button elsewhere in the application.

The desired UI is approximately:

    View
    ----------------
    Grid
    Photo Wall

A GTK Popover/MenuButton is acceptable if appropriate for the current
architecture.

Do NOT put "Photo Wall" in the left Library sidebar.

Photo Wall is a VIEW MODE, not a library/source.

Switching:

    Grid -> Photo Wall

must keep the same:

- selected library/folder/album
- scroll context where reasonably possible
- search/filter
- photo collection

Only the layout changes.

============================================================
10. SELECTION
============================================================

Existing selection behaviour must continue working.

Selected photos in Photo Wall must show the application's normal
selection indication.

Support existing:

- single selection
- multi-selection
- Ctrl selection
- Shift selection if currently supported
- keyboard behaviour where applicable
- opening/lightbox behaviour
- context menu
- drag/drop if currently supported

Do NOT create a separate selection model.

============================================================
11. VIRTUALIZATION -- CRITICAL
============================================================

My library can contain many thousands of photographs.

Example:

    All Photos: 14,361+

Photo Wall MUST NOT create a GTK widget for every photograph.

Reuse or extend the existing virtualization architecture.

The layout engine should be able to calculate geometry such as:

    PhotoWallItem {
        photo_index,
        x,
        y,
        width,
        height,
        row,
        section,
    }

Only photographs intersecting:

    viewport + overscan

should need active thumbnail widgets/rendering.

Scrolling through 10,000+ photographs must remain smooth.

Do NOT solve this by creating thousands of GTK Picture widgets inside a
FlowBox.

============================================================
12. LAYOUT CALCULATION
============================================================

Separate GEOMETRY from RENDERING.

Prefer an architecture similar to:

    photo data
        |
        v
    PhotoWallLayout
        |
        v
    calculated rectangles
        |
        v
    virtualization
        |
        v
    visible thumbnail tiles

PhotoWallLayout should not decode images.

It should only need metadata such as:

    width
    height
    aspect ratio
    photo ID/index
    section/folder boundary

The layout algorithm should therefore be cheap enough to recalculate
when the window is resized or the zoom slider changes.

============================================================
13. IMAGE DIMENSIONS
============================================================

First inspect how PIC currently stores image dimensions.

Do NOT repeatedly decode JPEG/RAW files just to obtain aspect ratios.

If dimensions already exist in the database/photo model, use them.

If some images do not have dimensions yet, implement a sensible
fallback without blocking scrolling or repeatedly reading full RAW
files.

Do not introduce expensive synchronous filesystem operations into the
layout pass.

============================================================
14. WINDOW RESIZING
============================================================

Photo Wall must respond correctly when the content area changes width.

Example:

Wide window:

    [----][--][------][---]
    [--][------][---][----]

Narrow window:

    [----][--][------]
    [---][--][------]
    [---][----]

Recalculate rows for the new available width.

Do not simply scale the entire old layout.

Do not allow thumbnails to overflow the right edge.

Avoid resize loops / GTK allocation loops.

Debounce layout recalculation only if actually necessary.

============================================================
15. ASPECT RATIO
============================================================

NEVER distort photographs.

A portrait photograph remains portrait.

A landscape photograph remains landscape.

A square photograph remains square.

Use cropping only if the existing thumbnail rendering architecture
requires it for another reason.

The Photo Wall itself should be based on the ORIGINAL PHOTO ASPECT
RATIO.

Respect EXIF orientation.

For example, a portrait NEF whose stored pixel dimensions are landscape
because of EXIF rotation must appear with the correct visual aspect
ratio.

============================================================
16. PERFORMANCE
============================================================

Performance is important.

Do not:

- decode originals during layout
- load every thumbnail
- create widgets for the entire library
- recalculate the complete layout on every tiny GTK event unnecessarily
- perform filesystem I/O inside the layout calculation
- clone large photo collections unnecessarily

The geometry calculation itself should be lightweight.

============================================================
17. IMPLEMENTATION APPROACH
============================================================

FIRST:

1. Inspect the current project.
2. Find the existing grid implementation.
3. Find virtualization.rs and related code.
4. Find how folder sections are represented.
5. Find how photo dimensions/orientation are stored.
6. Find the bottom toolbar/view button.
7. Find how zoom level is currently represented.
8. Find how thumbnail widgets are recycled/reused.
9. Find selection and activation handling.

THEN provide a short implementation plan.

After understanding the architecture, IMPLEMENT the feature.

Do not stop after writing the plan.

Do not build an unrelated prototype application.

Modify the real PIC application.

============================================================
18. KEEP CHANGES FOCUSED
============================================================

Do not refactor unrelated parts of the application.

Do not replace working systems simply because another architecture
would be cleaner.

Prefer extending the existing architecture.

Keep Grid working exactly as it currently does.

Photo Wall should be an additional layout strategy.

============================================================
19. TESTING
============================================================

After implementation:

Run:

    cargo fmt --check
    cargo check
    cargo test

Fix errors caused by your changes.

Also test logically/visually where possible:

1. All Photos -> Grid
2. All Photos -> Photo Wall
3. switch Grid -> Photo Wall -> Grid
4. resize narrow -> wide
5. move zoom slider
6. portrait + landscape mixture
7. select a photo
8. multi-select photos
9. open a photo
10. Folder view with several folder sections
11. verify rows NEVER cross folder boundaries
12. verify final folder row remains sensible
13. scroll through a large collection
14. verify thumbnails are still virtualized
15. verify RAW/NEF orientation produces correct aspect ratio

============================================================
20. ACCEPTANCE CRITERIA
============================================================

The feature is complete when:

[ ] Existing Grid mode still works.
[ ] Photo Wall can be selected from the view control.
[ ] Photo Wall uses justified rows.
[ ] Original aspect ratios are preserved.
[ ] EXIF orientation is respected.
[ ] Rows fill the available width.
[ ] Incomplete final rows are not excessively stretched.
[ ] Zoom slider changes target row height.
[ ] Window resizing recalculates the wall correctly.
[ ] Folder headers remain intact.
[ ] Rows NEVER cross folder boundaries.
[ ] Selection still works.
[ ] Lightbox/open still works.
[ ] Existing thumbnail caching is reused.
[ ] Existing virtualization is reused or appropriately extended.
[ ] 10,000+ photos do not create 10,000+ GTK widgets.
[ ] No full-resolution image decoding occurs during layout.
[ ] Grid <-> Photo Wall switching does not change the selected source.
[ ] cargo check passes.
[ ] existing tests pass.

============================================================
IMPORTANT FINAL INSTRUCTION
============================================================

Do not guess how the existing PIC architecture works.

READ THE EXISTING CODE FIRST.

Adapt Photo Wall to the architecture that already exists.

The desired architecture is:

    SAME DATA
       |
       +---- Grid layout
       |
       +---- Photo Wall layout

NOT:

    Grid application
    +
    separate Photo Wall application

Implement this as a production feature of the existing PIC application.
