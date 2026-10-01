# PIC Guide 2 — Folders and network shares

*Part of the [PIC user guides](../../README.md).*

PIC indexes the folders you choose. Importing and removing a folder from the catalogue do not reorganise or delete its original files.

## Add and browse a folder

Click **+** beside **Folders** to choose a local or mounted folder. Indexing reports progress and updates counts while photos become available; thumbnails continue in the background. Refresh checks for changes without needlessly reprocessing unchanged images.

<img src="../../screenshots/folders%20view.jpg" alt="Folder view showing photos beneath their folder heading" width="900">

Expand the sidebar tree to reach subfolders. Selecting a folder navigates to its section in the photo stream. Folder headings help keep the location clear, and the virtualised layout keeps only the required tiles active rather than building the whole library's widgets.

Use the shared zoom slider or Ctrl + wheel to change photo size. The regular grid and Photo Wall are both available. Arrow keys move through the photos; Ctrl+A selects the active folder's scope. **Open in Folder** from a photo's context menu returns to its location.

## Watch, refresh or remove

<img src="../../screenshots/watch%20folders.jpg" alt="Watched folder menu with refresh, stop watching, statistics and library removal" width="900">

Right-click a folder for its available actions:

- **Refresh Folder** checks it for changes.
- **Watch Folder / Stop Watching Folder** changes automatic monitoring for that location.
- **Folder Statistics** shows information about the folder.
- **Remove from Library** removes its catalogue membership, leaving original files on storage.
- Folder-wide favourite actions add or remove its photos from Favourites.

After a complete successful scan, Refresh removes catalogue records for deleted photos and subfolders. Existing empty folders remain. If the imported folder itself is missing, PIC removes it only when storage information saved by a previous scan confirms its filesystem is still connected. Unknown or offline roots remain; use **Remove from Library** to remove a confirmed deleted root that has no saved storage information.

Monitoring depends on storage availability. Use Refresh to request a rescan when needed, particularly after reconnecting storage or changing files outside PIC.

## Browse SMB and NFS

Open **Browse Network Photos** to discover servers, scan the local network, or enter a share URL. Expand the network tree to choose the folder to import. Registered shares are available in the **Network Shares** sidebar section.

<img src="../../screenshots/browse%20network.jpg" alt="Browse Network Photos dialog with discovery, scanning and URL controls" width="900">

The Linux packages include direct SMB and NFS transports, so manually mounting a share is not required for those paths. An existing mount can still be imported as a local folder. Access depends on the server, export/share permissions and connectivity. See [Portable NFS](../build/portable-nfs.md) for NFS details.

Local and network JPEGs use the same colour pipeline: valid embedded ICC profiles are converted to sRGB before thumbnail resizing and viewer display, with EXIF orientation preserved. The colour fix does not require an extra network read just to inspect the profile.

## Offline storage

When a drive or share is unreachable, its catalogue entries and available cached thumbnails remain. Availability markers indicate that the original cannot currently be read. Reconnect the storage and refresh or allow PIC's availability checks to run. Full-resolution viewing, editing and export require an accessible original.

Removing something in your file manager changes the underlying storage. PIC's catalogue and thumbnail cache are not backups of those files.

## Folders and albums

Folders describe physical locations. Albums collect photos within PIC without moving them or making copies. A photo can appear in several albums while its original remains in one folder.

**Previous:** [← Navigation](navigation.md) · **Next:** [Library →](library.md)
