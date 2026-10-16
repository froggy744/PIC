# TRON Cinematic preview

TRON Cinematic is the single `tron` appearance theme. The GTK preview fixture
shows photo tiles, an album cover, sidebar rows, and a zoom scale. It parses the
theme CSS and can save one `tron-cinematic.png` capture.

Generate the capture with:

```sh
PIC_TRON_SCREENSHOT_DIR="$PWD/screenshots/tron-neon" \
  GSK_RENDERER=cairo \
  cargo test tron_cinematic_theme_css_parses_and_previews -- --ignored --test-threads=1
```
