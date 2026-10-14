# PIC — Personal Image Catalogue — Release Candidate 5

*Inspired by Picasa and classic iPhoto*

**RC5 focuses on a more polished everyday photo experience:** a new screensaver, more capable editing tools, cleaner navigation, and improvements to gallery responsiveness.

PIC is a native **Rust + GTK4** photo manager for Linux. It keeps photos in their existing folders, and ordinary browsing, organisation, ratings and editing recipes do not modify original image files. Exporting an edit creates a new image.

## What's new since RC4

### New photo screensaver

Enjoy your library outside the normal gallery with PIC's new screensaver. The screensaver includes multiple presentation styles, with additional configuration and visual refinements.

### Stronger photo editing and filters

Edit Mode has gained further adjustments to exposure, contrast, colour and tonal balance, including improvements to fill light and shadows. New **clarity** and **grain** controls offer more creative flexibility.

Automatic contrast and colour tools have been refined, alongside new **newspaper** and **old-photo** effects. Filter strength can be adjusted rather than applying every look at full intensity.

Work on the editor also improves preview updates at 1:1 zoom, so adjustments feel more immediate and the image stays in place.

### Refreshed Library Home and photo organisation

The Library Home has a more structured, polished layout with clearer collection information and easier access to photos, albums and folders.

Album presentation, sidebar navigation, the photo InfoBar and metadata controls have also received usability improvements. Photo properties can be managed more conveniently without interrupting browsing.

### Smoother browsing and viewing

RC5 continues the performance work on **Grid, Photo Wall and Masonry** layouts, especially around resizing, scrolling and zooming.

Viewer controls, transitions and interactions have been refined. The photo overlay rendering path has also been optimised to avoid unnecessary repeated work.

### Rust compiler warning cleanup

The Rust codebase has undergone a compiler-warning cleanup, with outstanding warnings addressed for a cleaner build and easier maintenance.

### Interface and reliability refinements

- Cleaner toolbars, photo actions and InfoBar presentation.
- Better interaction between photo properties, Edit Mode and Collage Mode.
- Further thumbnail, folder-view and gallery behaviour fixes.
- Additional regression tests and internal cleanup.

## Core PIC features

- Browse large local photo libraries using Grid, Photo Wall and Masonry.
- Organise photos with folders, albums, favourites, ratings and search.
- Edit non-destructively, apply filters, export and create collages.
- Import photos from cameras, SD cards and local folders.
- Browse supported SMB/NFS network shares.
- Use multiple library databases and switch between them quickly.
- View photos full-screen and enjoy the screensaver.

## Downloads and testing

**RC5 release packages will be published on [GitHub Releases](https://github.com/froggy744/PIC/releases).** Linux builds are offered as **AppImage** and **Flatpak**, subject to final packaging checks.

RC5 is still release-candidate software. Back up both your original photos and the PIC library database before testing. A database backup does not include your original image files.

Feedback is particularly welcome on screensaver behaviour, editing previews, large galleries, folder navigation and Linux package installation. When reporting an issue, please include your Linux distribution, package format, steps to reproduce and any useful logs.

See the [getting started guide](../guides/getting-started.md), [Edit Mode guide](../guides/edit-mode.md) and [RC4 release notes](rc4.md).

PIC is an independent open-source project and is not affiliated with Google or Apple.
