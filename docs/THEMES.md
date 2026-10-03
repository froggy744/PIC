# Appearance Themes

Themes are plain folders on disk — nothing is hardcoded in the app. The
appearance picker (Settings → Themes, also reachable from the header-bar
sun/moon button) rescans the theme folder every time it is shown, so a new
theme appears without restarting.

## Location

```
<app folder>/themes/<theme-id>/theme.css
```

The app resolves `themes` relative to its working directory, exactly like
the `images/` runtime folder; if the folder is not found there it falls back
to the path next to the executable, so the binary works from any launch
directory. The shipped themes live in the repository at `themes/` and
`cargo build` (via `build.rs`) and the packaging scripts both stage them
next to the binary.

## Theme folder

| File | Required | Purpose |
|------|----------|---------|
| `theme.css` | preferred | The stylesheet. |
| any single `*.css` | fallback | Used only when `theme.css` is absent and it is the **only** stylesheet in the folder. |
| anything else | optional | Extra files (images, notes, backups) are ignored. |

A folder with several stylesheets and no `theme.css` is rejected on purpose,
so a stray backup copy cannot break discovery.

## Metadata header

The display name and behaviour flags come from a comment at the very top of
the stylesheet:

```css
/* picasa-theme
   version: 1
   name: Deep Ocean
   dark: false
   mode: overlay
*/
```

| Key | Values | Default | Meaning |
|-----|--------|---------|---------|
| `version` | `1` | — | Theme contract version of this file. Unknown keys are ignored; current contract is 1. |
| `name` | any text | folder name, humanized | Label shown in the picker. |
| `dark` | `true`/`false` (also `yes`/`on`/`1`) | `false` | Describes whether the theme palette is dark. It also controls the header icon. |
| `color-scheme` | `theme` or `system` | `system` | `system` follows GNOME's preference for light palettes. `theme` pins light palettes to light GTK/libadwaita chrome. Dark palettes always force dark chrome and a dark lightbox backdrop. |
| `mode` | `overlay` or `base` | `overlay` | `base` marks the one theme that is always loaded beneath every overlay theme (the shipped iDark). |
| `window-controls` | `native` or `traffic-light` | `native` | `traffic-light` replaces the native GTK minimize/maximize/close buttons with the app's gel-style traffic lights while the theme is active; style them via `button.traffic-light-*` (see the template). |

The header only counts as metadata when its first line contains the marker
`picasa-theme`; ordinary licence comments are ignored.

## Identity and persistence

- The **theme id** is the folder name — it is what gets saved to the settings
  database. Renaming a folder makes the saved selection fall back to
  `standard`, then to the first theme found.
- The **display name** is free to change at any time via the metadata header.
- If `themes` is missing entirely the app runs on the stock GTK
  appearance.

## Layering

Lowest to highest priority on the GDK display:

1. `css::install_foundation` — embedded base + platform CSS.
2. The `mode: base` theme (shipped: `iDark`).
3. The selected overlay theme.

When the base theme itself is selected, no overlay provider is added.

## Shipped themes

Theme folder names are stable saved ids. The current shipped folders are
`standard`, `iDark` (base), `teal`, `glass`, `tron`, `archive ledger`,
`retro`, `velvet-dusk`, `GBand`, `GbandV2`, `Bright Studio`, `Premium Studio`,
`RetroPC`, `aurora`, `frappe`, `grey`, `lars`, `latte`, `macchiato`, `nord`,
`tokyo night` and `ww`. The picker reads their display names from metadata.

## Authoring tips

- **Start from the community template: `docs/theme-template/`** — a complete,
  commented theme with every selector the app exercises (surfaces, photo
  tiles, sidebar selected + scroll-follow marker, segmented toggles, info
  bar, context menu). Copy it to `themes/<your-id>/` and recolor the
  palette block. See its README for the three-step install.
- `themes/standard/theme.css` is the shortest real example: it resets the
  dark base back to the stock libadwaita look.
- `themes/teal/theme.css` shows a recolored appearance palette.
- Always style the scroll-follow folder marker
  (`.navigation-sidebar row.sidebar-scroll-location`) and the segmented
  toggles (`.crop-aspect-button:checked`, `.crop-orientation-row button:checked`,
  `.collage-layout-tile:checked`, `.collage-tabs > button:checked`) — they are
  separate mechanisms from `row:selected` and otherwise fall through to the
  built-in base theme's styling.
- **Always neutralize the search entry's own background**: the inner
  `GtkSearchEntry` paints its system view background over the themed wrapper,
  which produces light-on-light or dark-on-dark text depending on the
  system appearance. Keep `.search-field entry { background: transparent;
  color: <your contrast color>; caret-color: <same>; }`.
- **GTK CSS is not web CSS.** Unsupported properties (width, height,
  transform, filter values other than blur(), flex/grid, …) produce parser
  warnings and are silently dropped. Use min-width/min-height, and build
  hover "lifts" from box-shadow instead of transforms.
- The metadata block must be the **first comment in the file**; only that
  block is parsed.
- Themes only style widgets; layout and behaviour live in the app. After
  editing a theme, switch to another theme and back to reload the stylesheet.
  Reopening Settings → Themes rescans discovery; selecting the active theme
  is a no-op.

## Gallery and album components

Photo Wall and Masonry use a provider above the appearance theme to keep tiles
borderless and selection outlines inset. Both use `@accent_bg_color`; Photo
Wall has square corners, while Masonry follows thumbnail corner settings.
The square-corner setting also overrides Grid and album cover radii.

Shared album component styles load above the appearance theme and use
`@theme_fg_color` and `@accent_bg_color` for plain covers. Framed album covers
and bookshelf artwork are discovered independently under `images/theme/`.
See the [template README](theme-template/README.md#album-cover-and-bookshelf-assets)
for current asset paths, row geometry and refresh guidance.
