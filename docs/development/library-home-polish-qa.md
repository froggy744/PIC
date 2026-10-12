# Library Home polish — validation checklist

Branch: `feature/library-home-polish`

## Behaviour
- [ ] Library Home header is clear, with a subtitle and three summary stats.
- [ ] Stats show **All Photos**, **Favourites**, **Folders**, in that order. The third stat **does not** display Recently Added.
- [ ] Folders stat counts imported root folders (`folders.imported_root=1`) for the current library, not every discovered subdirectory.
- [ ] Stats refresh when library content changes, and when switching libraries.
- [ ] Recently Added, Recently Edited, Favourites and Albums previews still render.
- [ ] Single-click on a photo preview highlights/selects the card but **does not** launch the lightbox.
- [ ] Double-click on a photo preview opens the right photo in the viewer (including rapid mouse double-click); single-click alone never opens.
- [ ] Horizontal scrollbars are hidden beneath all Library Home sections; touchpad and arrow navigation still scroll.
- [ ] Left/right buttons animate over ~280 ms with ease-in/out (including repeated quick clicks and window resizing).
- [ ] Album card click still navigates into the album.
- [ ] Arrow buttons and View All/View History links still work.
- [ ] Empty-state text works when no photos, favourites, edits or albums exist.
- [ ] Panel works at 1280px and narrower widths; summary doesn't conceal toolbar or content.
- [ ] Light and dark themes show appropriate contrast.

## Regression / build gates
- [ ] `cargo fmt --all -- --check`
- [ ] `cargo check` with no new warnings
- [ ] `cargo test`
- [ ] Library photo preview double-click tested using a mouse and touchpad.
- [ ] Normal All Photos/grid selection and double-click behavior unchanged.
- [ ] Large photo libraries remain smooth (no synchronous database reads on the UI thread).
- [ ] Ensure folder count semantics match product preference (imported roots vs all discovered folders).

Do not merge to main until compiled and visually tested on Fedora.
