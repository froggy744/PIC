# Settings and File Picker TODO

## Settings reorganisation

Current Settings order:

1. File Formats
2. Themes
3. Interface
4. Sidebar
5. Folders
6. Database
7. Albums
8. Library

### Proposed order

1. Interface
2. Themes
3. Sidebar
4. Albums
5. Folders
6. File Formats
7. Library & Storage
8. Libraries & Backups

The goal is to group appearance first, organisation second, and data/storage last.

## Interface

Keep visual thumbnail and UI behaviour here:

### Thumbnails
- Square thumbnail corners
- Show entire photo in thumbnails
- Show filenames below thumbnails
- High quality thumbnails (640 px)

### Effects
- Zoom transition animations

Do not put thumbnail cache/storage maintenance here.

## Sidebar

Keep all sidebar visibility controls together:

### Sections
- Library
- Albums
- Folders
- Network Shares

### Library destinations
- All Photos
- Favourites
- Recently Added
- History — add a new show/hide toggle

The History toggle requested here is for sidebar visibility only. If a future option disables History recording itself, treat that as a separate setting.

## Albums

Move album appearance controls out of Interface and into Albums:

- Bookshelf
- Bookshelf background / Next Background
- Album Covers
- Album cover design / Next Album Covers
- Disable All Themes
- Reset All Theme Settings
- Existing album list/counts can remain on this page, separated from appearance controls.

## Folders

Keep folder-management behaviour here:

- Automatic folder watching
- Individual watched-folder toggles
- Registered-folder status

## File Formats

Keep format-specific behaviour here:

- Enabled/disabled image formats
- RAW + JPEG pair behaviour

## Library & Storage

Move library statistics and thumbnail cache/storage maintenance here:

- Recently Added limit
- Total photos
- Originals available
- Originals unavailable
- Database size
- Total albums
- Total library folders
- Availability stats / Update now

### Thumbnail cache
- Cached thumbnails
- Required thumbnails
- Unused thumbnails
- Thumbnail cache size
- Clean Thumbnail Cache
- Clear thumbnails

### Destructive maintenance
- Clear database
- Clear all

Thumbnail appearance belongs in Interface; thumbnail cache/storage belongs here.

## Libraries & Backups

Rename the current Database page to **Libraries & Backups**.

Keep:
- Current library
- Library name
- Description
- Rename/save library details
- Known libraries
- Create library
- Open existing database
- Backup database
- Restore database

### Responsive layout fixes
- Do not allow the full current database path to force the page wider than the Settings window.
- Ellipsize or wrap long database paths.
- Do not force Create/Open/Backup/Restore into one wide horizontal row.
- Make action buttons wrap or use a compact two-column/vertical layout.
- Keep the Settings window reasonably sized; pages should adapt instead of requiring a huge window.

## Add Folder picker status

The GTK4 `FileDialog::select_folder()` replacement introduced in commit `1b481884c` is working in the tested build.

Previous behaviour:
- `GtkFileChooserNative` opened and selected folders correctly.
- Closing/accepting it emitted repeated GTK criticals:
  `thaw_updates: assertion 'GTK_IS_FILE_SYSTEM_MODEL (model)' failed`.

Current status:
- Local Add Folder picker works with `GtkFileDialog`.
- Keep Network Shares on PIC's own network picker.
- Verify both Accept and Cancel during normal testing.
- Confirm the old repeated `thaw_updates` criticals no longer appear.

## Implementation status — 2026-10-02

Completed:
- Reordered Settings to Interface, Themes, Sidebar, Albums, Folders, File Formats, Library & Storage, Libraries & Backups.
- Moved album appearance controls out of Interface and into Albums.
- Added persisted Show History toggle under Interface → Navigation.
- Added sidebar support for the new History visibility setting.
- Renamed Library to Library & Storage.
- Renamed Database to Libraries & Backups.
- Made the Libraries & Backups page more responsive: wrapping current-library path, ellipsized known-library paths, and 2×2 database action grid.
- Add Folder picker is working and requires no further work at present.

Still to verify manually:
- Settings page layout at narrow and normal window widths.
- History toggle persistence across restart and immediate sidebar refresh.
- Album appearance controls still apply live from their new page.
- Libraries & Backups page no longer clips on the right.
