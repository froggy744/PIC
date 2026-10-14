# PIC screensaver expansion — feature/screensaver

Scope: Picasa/iPhoto-inspired slideshow, no original-image mutations and no network I/O on GTK thread.

## Delivery checklist
- [x] Fix Picture Pile: aspect-aware postcard sizes, thin ivory borders, smaller centre print, readable overlaps, deterministic z-order and visible photo swap
- [x] Add Changing Grid: a clean responsive rectangular photo wall with transitions and cycling
- [x] Add Floating Prints: separate near-frameless design, portrait/landscape sizing, balanced spacing, gentle motion and one-at-a-time replacement
- [x] Add Fullscreen Fade: single photo, centered, gentle crossfade, image aspect ratio preserved
- [x] Retain Mosaic and iPhoto Ken Burns; Surprise chooses from all working styles
- [x] Presentation picker: distinct vector previews and correct descriptions, no network access while opening
- [x] Timers: seconds per photo; separate transition duration; pause/resume keyboard control
- [x] Options: optional discreet photo date/name caption, shuffle/sequential, collection source, position shuffle
- [x] UI: disable irrelevant controls per style, preserve saved preferences (last Start), preserve compact timing row
- [x] Keep cache-first workers, NFS isolation, fast application startup, bounded decoding memory
- [x] Clock option: local-time HH:MM bottom-right glass clock (34px, 56% dark backdrop), updates each minute, persists on/off, shares a collision-safe footer with captions
- [x] Preferences: last style, album/folder/source, slide hold duration, transition duration, shuffle, tile count, captions, clock saved in SQLite
- [x] Automated tests: style persistence, clock option, geometry, timing, card cycling and cache/offline regressions — cargo check passed, 23 screensaver tests passed (GitHub Actions run 38430307666)
- [ ] Fedora visual retest after photo-frame redesign: mixed orientations, 1/3/5 Picture Pile photos, 6 Floating Prints, edge spacing and smooth transitions
- [ ] Manual Fedora Wayland test (requires user workstation): all styles, 1 photo, 6 photos, 100 photos, NFS offline/online, 16:9 and portrait display
- [ ] Merge to main only after user confirms visible UI and slideshow behaviour

## Caveats
- Headless GitHub CI cannot validate GPU composition or visual geometry: a Fedora Wayland run is needed before merging into main.
- Picture Pile must be visually checked at 1, 4, 5 and >5 photos; code checks prove layout bounds, not appearance.

## Design limits
- No I/O from tick callbacks or style picker.
- Photo decoding belongs to background worker threads.
- Keep the number of concurrently visible card widgets bounded.
- Do not create arbitrary timers that hide hangs; use monotonic clock.
- Avoid using invalid GTK CSS percent transforms.
