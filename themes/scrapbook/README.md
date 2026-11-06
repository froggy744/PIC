# PIC Scrapbook skin

The `skins` branch introduces a selectable, light, paper-inspired appearance.

**Activate:** Settings → Themes → **PIC Scrapbook**.

This skin is an alternative to the existing PIC interface and leaves all library
and photo-management data unchanged. It styles the existing native GTK4 controls
and adds optional noninteractive decorative overlays to the Library dashboard
and album-cover view.

## Design tokens

| Part | Colour |
| --- | --- |
| Paper | `#FAF4E9` |
| Cream | `#FFFAF0` |
| Ink | `#302722` |
| Cork | `#C6A47C` |
| Lavender | `#D9C4E9` |
| Peach | `#F5C7AE` |
| Sage | `#C5DCB7` |
| Accent | `#76529A` |

Decorations in `assets/` are tiny original SVGs, not photographs or
screenshots. The app continues to render real library thumbnails. The theme
loader resolves asset URLs relative to `theme.css` (see theme engine change).

Optional handwritten fonts include Kalam and Comic Neue; PIC falls back to
Poppins or the system sans-serif when not installed.

## Constraints

- The PNG design reference is aspirational, not a pixel-perfect specification.
- No additional photos are bundled or shown as fake user content.
- Decorative elements do not consume clicks; the album and gallery still use
  the existing GTK widgets and data models.
- Standard, compact, light, and dark skins remain separate.
- Test at 1366 × 768, 1920 × 1080, and 2560 × 1440 before merging.
