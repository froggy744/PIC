# Wizard tour images

The Discover PIC tour uses these files in order:

1. `wiz-browse.jpg` — Browse
2. `wiz-albums.jpg` — Albums
3. `wiz-edit.jpg` — Edit
4. `wiz-collage.jpg` — Collage

Replace the JPEGs, keep the same filenames, then close and reopen the tour. No code change or rebuild is needed when running from the project folder. Images keep their aspect ratio and scale to fit.

For a standalone build, replace the files in `screenshots/` next to the executable. For an editable or extracted install, replace them in the launcher's runtime resource folder (`share/pic-rs/screenshots/`). For AppImage or Flatpak bundles, replace the source JPEGs before rebuilding the package. A build or package refresh copies the source JPEGs into that folder. PIC includes fallback copies so the tour also works if the external files are unavailable.
