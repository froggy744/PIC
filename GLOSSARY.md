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
| **Sidebar mode** | Which left navigation the user chose: the **full sidebar** inside the split view, or the **compact rail**. Persisted as `compact-sidebar-enabled`. Code: `SidebarMode`. |
| **Compact rail** | The narrow icon navigation (44-300 px) beside the split view, in its own dock. Labels appear from 90 px. |
| **Drawer** | The full sidebar while the split is collapsed by the responsive breakpoint (window ≤ 1050 px, back at ≥ 1080 px): it slides over the photos instead of sitting beside them. |
| **Hover peek** | Opening the hidden full sidebar by resting the pointer on the left-edge strip. It does not change the pin, and closes again when the pointer leaves. |
| **Immersive overlay** | In Immersive Mode, the existing sidebar shell moved onto the gallery as a presentation-only overlay, showing the full or compact page per the sidebar mode, so the gallery never reflows. |
| **Sidebar presentation** | Deciding how the left navigation is shown for a sidebar mode, breakpoint, pin/hover and Immersive Mode: which widgets are visible, where, and how wide. One module: `window/sidebar_presentation.rs`. |
| **InfoBar presentation** | Deciding what the bottom InfoBar shows: nothing, one photo's metadata, or the **selection strip** (count, thumbnails and grouped actions for a multi-selection), and which of its buttons are enabled. Callers report events; the lightbox and the date overview **cover** the gallery, so gallery events wait until they close. One module: `infobar_presentation.rs` (`infobar::presentation`), applied by `InfoBar`. |
| **Properties inspector** | The photo details panel: the **gallery panel** docked beside the photos, or the **viewer panel** over the lightbox. Each has its own open state and scroll position. |
| **Photo marks** | A photo's favourite flag and 0-5 star rating. Changed only through **photo mark commands**: a control reports an intent (toggle favourite, toggle stars, set rating) for its **anchor** photo, and the plan picks the targets (the lightbox photo, or the gallery selection, or the collage ticks), the value, and whether photos leave the view. Pure plan: `window/photo_marks.rs`; adapter: `window/photo_mark_commands.rs`. |
| **Properties presentation** | Deciding when each Properties panel is shown: the Photos-page gate, the viewer, and Immersive Mode, which **suspends** the panels (hidden, not closed; they resume afterwards). One module: `window/properties_presentation.rs`. |
