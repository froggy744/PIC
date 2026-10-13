# PIC screensaver expansion — feature/screensaver

Scope: Picasa/iPhoto-inspired slideshow, no original-image mutations and no network I/O on GTK thread.

## Delivery checklist
- [ ] Fix Picture Pile: bounded postcard stack with overlapping but readable images, deterministic z-order, visible photo swap, and no broken six-card/offscreen geometry
- [ ] Add Changing Grid: a clean responsive rectangular photo wall with transitions and cycling
- [ ] Add Floating Prints: scattered independent prints with different positions/tilts and one-at-a-time replacement
- [ ] Add Fullscreen Fade: single photo, centered, gentle crossfade, image aspect ratio preserved
- [ ] Retain Mosaic and iPhoto Ken Burns; Surprise chooses from all working styles
- [ ] Presentation picker: distinct vector previews and correct descriptions, no network access while opening
- [ ] Timers: seconds per photo; separate transition duration; pause/resume keyboard control
- [ ] Options: optional discreet photo date/name caption, shuffle/sequential, collection source, position shuffle
- [ ] UI: show only relevant controls per style, preserve saved preferences
- [ ] Keep cache-first workers, NFS isolation, fast application startup, bounded decoding memory
- [ ] Automated tests for style persistence, geometry, timing, card cycling, offline behaviour; compile in GitHub CI
- [ ] Manual Fedora Wayland test: all styles, 1 photo, 6 photos, 100 photos, NFS offline/online, 16:9 and portrait display
- [ ] Merge to main only after user confirms visible UI and slideshow behaviour

## Design limits
- No I/O from tick callbacks or style picker.
- Photo decoding belongs to background worker threads.
- Keep the number of concurrently visible card widgets bounded.
- Do not create arbitrary timers that hide hangs; use monotonic clock.
- Avoid using invalid GTK CSS percent transforms.
