# TRON glow preview captures

`tron-neon-before/` contains the original TRON stylesheet rendered by the same GTK preview fixture. `tron-neon-after/` contains the updated theme in Subtle, Standard, and Cinematic modes. The fixture shows unselected, hovered, and selected photo tiles, a shipped album frame over generated photo pixels, and a zoom scale.

The captures were rendered on the host Wayland display with GTK4 and the Cairo renderer. The GTK regression test parses `themes/tron/theme.css`, switches all three persisted presets, writes the screenshots, and confirms a sample from the photo area remains pixel-identical across presets.

Regenerate the updated captures with:

```sh
PIC_TRON_SCREENSHOT_DIR="$PWD/screenshots/tron-neon-after" \
  GDK_BACKEND=wayland GSK_RENDERER=cairo \
  cargo test tron_glow_presets_persist_and_theme_css_parses -- --ignored --test-threads=1
```
