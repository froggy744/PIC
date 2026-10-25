# Cover Flow and Reflective Bookshelf Design

## Intent

Give PIC's Albums index an iTunes era Cover Flow view and an iPhoto '08 inspired reflective bookshelf treatment. Add Cover Flow as a screensaver style using individual photos from the screensaver's selected source.

## Scope and behavior

- The Albums index gains a third layout, **Cover Flow**, alongside its existing grid and bookshelf. Its items are albums, shown in the current sort order. The focused cover is front facing, adjacent covers angle away, and a short reflection fades below the covers. Clicking the focused cover opens its album; clicking a neighbor focuses it. Keyboard arrows, Home/End, mouse wheel, and touchpad scroll move focus. The focused album's name and photo count remain readable. Empty and single album states remain useful.
- The screensaver gains a **Cover Flow** style. Its items are individual photos from the selected source, respecting existing shuffle, portrait exclusion, duration, caption, clock, decode, and exit behavior. It advances automatically and wraps. The selected photo is centered and the captions refer to that photo.
- The existing bookshelf layout gains an optional reflective treatment inspired by iPhoto '08: subtle cover shadows, a short reflection where the selected shelf art plausibly has a reflective surface, and themed brightness/opacity. Reflection is decoration only and must never displace album labels or obscure shelf art.
- The reflection effect uses the *visible composited cover* where possible, including a selected frame; otherwise it mirrors the photo thumbnail. It must derive its palette and contrast from the active appearance and selected shelf theme instead of assuming a black background.
- Default behavior and saved grid/bookshelf choices remain unchanged. Existing album appearance and cover theme settings keep their meanings; Cover Flow is a layout choice, while bookshelf/reflection are presentation choices.

## Constraints

- Native Rust, GTK4 and GSK implementation in the existing single crate. No web view or new dependency unless a focused prototype shows the current stack cannot meet the effect.
- Use cached preview textures and a bounded visible window of items. Reuse the screensaver's worker and cancellation rules, especially for remote sources. Keep thumbnails and original photo decoding off the GTK thread.
- Keep selection, focus, and opening behavior accessible and functional without animation. Honor reduced motion where GTK exposes that preference, and keep keyboard operation independent of pointer input.
- Reflection and shadow are visual only; original photos, edit recipes, export, and database album membership are untouched.

## Approach

Use one small Cover Flow geometry and drawing module with separate adapters for album covers and screensaver photo textures. The album page owns browsing and album actions; the screensaver retains its existing player and decode pipeline. GSK snapshot transforms provide the card angle, shadow, and mirrored reflection with a fading alpha mask. This avoids per-frame image copies and fits the custom GSK rendering already used by the screensaver.

Alternative approaches considered: a GTK carousel of ordinary card widgets would be simpler for input and accessibility but cannot reliably render the angled, reflected composition; pre-rendering every cover to bitmaps would simplify painting but scales poorly with large libraries and duplicates image work. The shared geometry/drawing approach is recommended, subject to a small rendering proof in the first task.

## Open implementation details

- Exact perspective approximation and reflection opacity should be tuned against light and dark themes using screenshots; the visual target is recognizable Cover Flow with legible covers, not pixel-level recreation of Apple artwork.
- A bookshelf theme can tune reflection via `theme.conf`. At least one shipped shelf theme has a visible reflection by default; themes without suitable reflective surfaces can set the strength to zero. Existing theme files continue to load.
