# Getting started with PIC

[Main documentation](../../README.md) · [Navigation](navigation.md) · [Folders](folders.md) · [Library](library.md)

PIC — Personal Image Catalogue is a local-first photo manager for Linux. It indexes your existing photo folders, stores organisation and edits in a local database, and caches thumbnails for browsing. Importing does not move your originals.

## Install and open

Flatpak is the primary Linux package. Follow the [Linux build and installation guide](../build/linux.md) to build or install it, then launch PIC from your desktop menu or run:

```sh
flatpak run io.github.you.PicRs
```

Native development and optional AppImage builds are also covered there. Windows builds remain experimental.

## Add your first folder

1. Click **+** beside **Folders** and select a photo folder.
2. Let PIC index the collection. Photos and counts appear progressively while thumbnail work continues in the background.
3. Select **All Photos** for the combined collection, or choose a folder in the sidebar.

You can also import from **Browse Network Photos** using SMB or NFS. A disconnected drive or share can still have cached previews; the original must be reachable for full viewing, editing or export.

<img src="../../screenshots/all%20photos.jpg" alt="All Photos with imported folders in the sidebar" width="900">

## Choose your layout

Use the bottom bar's view toggle to switch between the regular grid and **Photo Wall**, which arranges photos in rows using their natural proportions. Change thumbnail size with the zoom slider or **Ctrl + mouse wheel**.

<img src="../../screenshots/photo%20wall%20sidebar%20hidden.jpg" alt="Photo Wall with the sidebar hidden" width="900">

In **Settings → Interface**, choose whether filenames are shown, how portrait thumbnails are presented, and whether Photo Wall uses optional **640 px high-quality thumbnails**. Those previews are generated as needed. [Library guide →](library.md)

## Open and organise photos

- Click to select a photo and show its details in the bottom bar.
- Double-click, or use Enter or Space, to open the selected photo.
- Use Left / Right or the mouse wheel to browse; **Space** toggles fit and 1:1, and **Ctrl + wheel** zooms. Press Esc to return.
- Heart a photo for **Favourites**, give it a star rating, or add it to an album.
- **1–5** assign ratings and **0** clears them; these shortcuts are inactive while typing in a text field.

Search matches photo and folder names. Select a folder suggestion and press Enter to jump directly to it. Esc clears search. [Navigation guide →](navigation.md)

## Make an edit

Select a photo and choose the pencil button or **Edit** from its context menu. Explore **Tools, Filters, Crop, Overlays and Text**. Changes are recipes saved in PIC, with Undo, Redo and Reset. Use **Export** to create an edited image file.

<img src="../../screenshots/effects%20screen.jpg" alt="Filters in PIC Edit Mode" width="900">

[Edit Mode guide →](edit-mode.md) · [Crop Mode guide →](crop-mode.md)

## Keep your collection safe

**Settings → Database** manages separate libraries and database backups. These backups preserve catalogue information, not the original photo folders. Back up the originals separately.

PIC's current JPEG thumbnail and viewer pipeline applies valid embedded ICC profiles, including ProPhoto RGB and Adobe RGB, before resizing. Unprofiled JPEGs retain their existing sRGB treatment; malformed colour metadata falls back without hiding the photo.

For disconnected storage, watched folders and network shares, see the [Folders guide](folders.md).
