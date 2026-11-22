# PIC Glossary

Domain terms used in code, docs and reviews. Use these names exactly; code paths are in `AUDIT-INDEX.md`.

| Term | Meaning |
|---|---|
| **Gallery surface** | One of the widgets that shows photo tiles: the All Photos `GtkGridView`, the sectioned Folder view (`gtk::Fixed`, also used by every wall layout), or the legacy Folder list. Only one is on screen at a time. |
| **Gallery scroller** | The scrolled window a per-frame thumbnail pump belongs to: `Photos` (All Photos) or `Folder`. A pump acts only while its own surface is on screen. Code: `GalleryScroller`. |
| **Thumbnail presentation** | Getting the right thumbnail, at the right size, onto every visible tile: visibility, request queueing, prefetch, the decoded-thumbnail RAM cache and delivery. One module: `grid/presentation.rs`. |
| **Visible tiles** | A surface's visible band plus where each tile is *placed* (not where GTK last allocated it). The seam every surface answers through. Code: `visible_presentation_tiles`. |
| **Presentation target** | The short-edge size, in device pixels, a gallery thumbnail is decoded at for the current zoom (128/192/256/384 px buckets, or full size). Cached thumbnails remember the target they were decoded for. |
| **Stand-in** | A cached thumbnail decoded for a smaller zoom, shown while the sharper one loads, so a zoom-in never blanks a tile. |
| **Settled tile** | A tile showing its thumbnail at a size good enough for the current presentation target. Pumps skip settled tiles. |
| **Scrub** | A direct scrollbar drag that moves more than about a page per event; it switches the pumps to model-derived targets. |
| **Settle** | The moment scrolling stops (about 90-110 ms of quiet), when visible tiles are refreshed and prefetch runs within the RAM cache budget. |
| **Recipe** | `EditRecipe`: the non-destructive edit description applied at display and export time. Originals are never modified. |
