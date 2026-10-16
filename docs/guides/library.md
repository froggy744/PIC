# PIC Guide 3 — Library, ratings, albums and search

> **PIC V1 guide.** Screenshots are being refreshed for the updated interface; the older images have been hidden until replacements are ready.

*Part of the [PIC user guides](../../README.md).*

## Library destinations

The sidebar provides **All Photos**, **Favourites**, **Recently Added** and **History**. The Library heading opens the overview; choose a destination to browse that collection. Folder and Network Shares sections navigate the physical locations you imported.

<!-- V1 SCREENSHOT: add current UI capture for this section; older gallery image withheld. -->

## Grid and Photo Wall

The regular grid uses consistent thumbnail slots. **Photo Wall** arranges photos in rows at their natural proportions, while **Masonry** offers a varied-height tiled layout. Switch using the gallery layout controls. Both use the same collection and selection.

<!-- V1 SCREENSHOT: add current UI capture for this section; older gallery image withheld. -->

Resize previews with the zoom slider or **Ctrl + wheel**. **Settings → Interface** controls filenames below thumbnails, portrait presentation and optional **High quality thumbnails (640 px)** for Photo Wall. Normal thumbnails remain 320 px. High-quality previews are generated as needed rather than regenerating the whole library at startup.

<!-- V1 SCREENSHOT: add current UI capture for this section; older gallery image withheld. -->

Portrait collections can also be browsed in the regular grid. Choose portrait-thumbnail presentation in Interface settings to suit your collection.

<!-- V1 SCREENSHOT: add current UI capture for this section; older gallery image withheld. -->

## Group, sort and filter

Group library photos by **Day** or **Month**, or use an ungrouped view. Folder browsing uses folder headings. Sort by date taken, name, file size, dimensions, date added or **Rating**, with ascending and descending choices.

The rating filter is independent of sorting: choose **Clear**, **All Stars**, **Unrated**, or an individual star level.

## Favourites and ratings

Heart a photo from the bottom bar or context menu to include it in Favourites. Ctrl+click several photos to apply actions to a selection.

<!-- V1 SCREENSHOT: add current UI capture for this section; older gallery image withheld. -->

Assign **1–5 stars** with the star control or the number keys; **0** clears a rating. In the gallery, shortcuts rate the selection; in the viewer, they rate the current photo. Ratings appear on thumbnails and are stored in the database rather than the original file. These shortcuts do not run while you are typing in a text field.

## Albums

Use **+** beside Albums to create a collection. Select photos and choose **Add to album** from the context menu. A photo can belong to several albums without duplication. Removing a photo from an album does not delete its original.

Album context actions include renaming and cover customisation. An album is a virtual collection; it does not create a matching folder on your disk.

## Search and return to a folder

Search matches photo and folder names. Choose a folder suggestion with the keyboard and press Enter to navigate there; Esc clears search. **Open in Folder** on a photo returns to its folder section. Closing the viewer restores the photo selection and browsing position.

History shows recently edited photos and recorded operations, including saved edits and collage work. It is distinct from Recently Added, which reflects catalogue additions.

## Create from your selection

Select multiple photos and open the collage workspace. Choose Grid, Mosaic or Smart AI, adjust the layout and appearance, then create an exported JPEG. Save a project when you want to resume work later; saved collage work also appears in History.

<!-- V1 SCREENSHOT: add current UI capture for this section; older gallery image withheld. -->

## Separate libraries and backups

Open **Settings → Database** to manage catalogue databases:

- Create a library or open an existing `.db` file.
- Choose from known libraries and change the library name and description.
- Back up the database or restore a database backup.

Separate libraries can represent different projects or archives. Photo files and the shared thumbnail cache are not copied by database management. A database backup protects organisation and edit recipes, not the original photographs; keep separate photo backups. Imported text/overlay assets also need to remain available for their edits.

## Thumbnail colour

For standalone JPEGs, thumbnails and the viewer use the embedded ICC profile when it is valid, converting source RGB to sRGB before resizing. ProPhoto RGB and Adobe RGB files therefore receive actual profile conversion rather than a saturation adjustment. JPEGs without a profile retain their previous treatment, and bad profiles fall back to usable decoded pixels.

Existing JPEG thumbnails from before this fix are regenerated through normal lazy/background caching. The library does not synchronously recolour every photo on startup.

**Previous:** [← Folders](folders.md) · **Next:** [Edit Mode →](edit-mode.md)

## V1 albums and bookshelf display

The Albums page supports album-cover presentation and a bookshelf-style layout. Cover and bookshelf artwork is configured separately from the app's appearance theme. Use the albums page's available presentation controls to change the look; photo membership remains virtual and original folders are untouched.

**Three gallery layouts:** Grid, Photo Wall and Masonry. Rating controls and rating filtering work in the catalogue and folder browsing views; filter by an individual star level, all rated pictures or unrated pictures.
