# PIC community theme template

A starting point for building your own PIC appearance theme. Copy the folder,
rename it, recolor the palette — done.

## Install in three steps

1. **Copy** this folder into your PIC installation's theme folder and give it
   your theme's id (this is the stable name PIC saves; lowercase with hyphens is convenient):

   ```
   themes/my-theme/              <- the copy, folder renamed
   ```

   The `themes/` folder sits next to the PIC executable (same place as the
   bundled `themes/standard/`, `themes/teal/`, …).

2. **Rename** `theme.css` stays `theme.css` — instead, edit the metadata at
   the top of the file:

   ```css
   /* picasa-theme
      version: 1
      name: My Theme
      dark: false
      color-scheme: theme
      mode: overlay
      window-controls: native
   */
   ```

3. **Open PIC → Settings → Themes.** Your theme is already in the Appearance
   list — no restart needed. The list rescans the folder every time the page
   opens.

## Rules of the road

- **The metadata block must be the first comment in the file.** Only the
  first `/* … */` block is parsed; a marker in a later block is ignored.
- **Folder name = theme id.** It is what PIC stores in its settings, so never
  rename a folder that people already use — bump the `name:` instead.
- **`theme.css` preferred.** If it is absent, exactly one `*.css` file is
  accepted. Multiple stylesheets without `theme.css` make the folder invisible.
  Extra files (previews, notes) are allowed.
- **Keep valid GTK CSS.** PIC loads themes through GTK's CSS parser;
  malformed rules and unsupported properties (width, height, transform,
  filter values other than blur(), flex/grid, …) are dropped with a parser
  warning, not a crash — but the effect simply won't appear.
- **Neutralize the search entry's background** (the template already does):
  the inner `GtkSearchEntry` paints its system view background over your
  wrapper, causing light-on-light or dark-on-dark text.
- **Unstyled surfaces fall through to the dark base theme.** Light themes
  should keep the Surfaces section; dark themes should recolor it.
- **Badges and favourites** ship with conventional colors (amber offline,
  red favourites). Recolor them if your palette disagrees.

## Metadata reference

| Key | Values | Meaning |
|-----|--------|------------------|
| `version` | `1` | Theme contract version of this file. Current is 1. |
| `name` | any text | Label in Settings → Themes. Defaults to the folder name. |
| `dark` | `true` / `false` | Dark themes prefer the dark color scheme and the dark lightbox backdrop; the header icon shows a moon. |
| `color-scheme` | `theme` / `system` | `theme` pins a light palette to light GTK chrome; `system` (default) follows the desktop for light palettes. Dark palettes always force dark chrome. |
| `mode` | `overlay` / `base` | Leave `overlay`. `base` identifies the theme loaded beneath all overlays, above the embedded foundation — advanced use only. |
| `window-controls` | `native` / `traffic-light` | `traffic-light` swaps the native GTK minimize/maximize/close buttons for PIC's gel-style traffic lights while your theme is active (style them via the section 10 block in `theme.css`). |

Unknown keys are ignored, so adding `version: 1` today is forward-compatible.

## Testing your theme

- After editing CSS, switch to another theme and back. Reopening the picker
  rescans files, but selecting the already active theme does not reload it.
- Toggle between your theme and **Standard GTK4** to spot unstyled surfaces
  (they will look dark).
- Check the three selection states: a selected sidebar row (Favourites), the
  scroll-follow folder marker (browse into a folder), and multi-selected
  photo tiles.
- Check the segmented toggles: Edit → Crop (Free / Landscape) and the Collage
  panel (Mosaic / Smart / Grid).
- Check Grid, Photo Wall and Masonry, including hover and multi-selection.
  Photo Wall stays borderless with square corners; Masonry is borderless and
  follows the thumbnail corner setting. Their inset highlights use your accent.
- Check plain albums, framed albums and bookshelf views, plus context-menu
  submenus, disabled actions and inactive/active favourite icons.
- Invalid metadata values are dropped with a warning on the console; the
  theme still loads with defaults.

## Album cover and bookshelf assets

These are independent of appearance CSS and live in the runtime `images/`
folder, resolved from the working directory or next to the executable:

```text
images/theme/album-covers/my-cover/design-frame.png
images/theme/bookshelf/my-shelf/row.png
images/theme/bookshelf/my-shelf/theme.conf
```

Each top-level cover folder is a design family; nested `*-frame.png` files are
its variants. PIC measures the transparent photo opening from each frame's
alpha channel. Keep the opening enclosed by opaque frame pixels. Albums use a
stable variant, and can choose their own frame and cover photo from their menus.

Each bookshelf folder supplies one repeating row image. Preferred names are
`row.png`, `row.jpg`, `bookshelf.png`, `bookshelf.jpg`,
`single-row-bookshelf.png` or `single-row-bookshelf.jpg`; otherwise the first
sorted PNG/JPG is used. Optional `theme.conf` sets pixel geometry:

```ini
row_height=288
surface_y=235
```

`row_height` must be positive; `surface_y` must be nonnegative and is clamped
to the row height. Omitted values use the defaults above. Legacy
`*-bookshelf.png` / `*-bookshelf.jpg` files directly in the bookshelf directory
remain supported. Folder-based row themes sort first, with `single` first.
See [bookshelf artwork guidance](../../images/bookshelf-row-template.md).

Use the albums page menu to enable Bookshelf or Album Covers and cycle their
families. Per-album choices persist independently. Settings → Themes → Reset
All Theme Settings clears the appearance customizations and album cover choices.

See also [Themes](../THEMES.md) for the full discovery and layering model.
