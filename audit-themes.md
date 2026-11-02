
 Verified (read-only):

 1. Contract extracted — src/css/theme_discovery.rs, src/css/mod.rs, src/window/theme.rs, docs/THEMES.md,
    docs/theme-template/theme.css (122 contract selectors), plus src/css/components/*.css (792 lines).
 2. Metadata audit of all 27 theme folders — 26 discoverable stylesheets, all headers parsed.
 3. Real GTK validation — loaded every theme stylesheet through an actual Gtk.CssProvider (GTK 4.22.5 available via gi): 0 parse
    errors in all 26 files. Separately probed which CSS names GTK4 rejects (z-index, position, gap, flex-grow, cursor, user-select,
    appearance, mix-blend-mode, box-sizing, pointer-events, overflow, white-space, will-change, -gtk-outline-radius, -gtk-alpha,
    align-items, justify-content) — since the parse-error signal fires for those, the clean scan proves no shipped theme uses a
    dead property. transition: shorthand is accepted-but-unvalidated (dead weight).
 4. Button/UI baseline — 257 unique add_css_class("…") classes harvested from Rust → coverage probed against each theme.
 5. Hardcoded theme ids found in the engine (src/window/theme.rs:231): "PIC" | "PIC2" | "PIC-Default" | "PIC-Dark" | "Terminal" |
    "Monochrome-Studio" | "windows old" | "Cyberpunk" | "OLED" | "iDark" get APPLICATION + 3; everything else + 1.

 Interim findings (highest signal):

 ┌─────────────────────────────────────────────────────────────────────────────────────────────────────┬──────────────────────────┐
 │ Finding                                                                                             │ Evidence                 │
 ├─────────────────────────────────────────────────────────────────────────────────────────────────────┼──────────────────────────┤
 │ PIC, PIC2, Cyberpunk, OLED in the +3 priority list don't exist on disk — dead entries; Picasa-3     │ src/window/theme.rs:231  │
 │ styles edit-toolbar buttons but is not in the list, so shared CSS outranks it                       │                          │
 ├─────────────────────────────────────────────────────────────────────────────────────────────────────┼──────────────────────────┤
 │ Only 7 themes cover .edit-panel-tabs / .edit-toolbar-actions: iDark, Monochrome-Studio, PIC-Dark,   │ coverage probe           │
 │ PIC-Default, standard, Terminal, windows old                                                        │                          │
 ├─────────────────────────────────────────────────────────────────────────────────────────────────────┼──────────────────────────┤
 │ .traffic-light styled only by apperture + aqua, yet no theme declares window-controls:              │ metadata dump + probe    │
 │ traffic-light → those rules are unreachable                                                         │                          │
 ├─────────────────────────────────────────────────────────────────────────────────────────────────────┼──────────────────────────┤
 │ Template-conformance spread: Picasa-3 58% (51 contract selectors missing), iDark 72%,               │ matrix                   │
 │ PIC-Default/PIC-Dark/Monochrome-Studio/Terminal 70%, bulk of themes 84%,                            │                          │
 │ snowleopard/apperture/aqua/purple 99%                                                               │                          │
 ├─────────────────────────────────────────────────────────────────────────────────────────────────────┼──────────────────────────┤
 │ snowleopard and GbandV2 have no theme.css — they survive only on the single-*.css fallback;         │ dir listing              │
 │ snowleopard also ships a stray theme.css.bak                                                        │                          │
 ├─────────────────────────────────────────────────────────────────────────────────────────────────────┼──────────────────────────┤
 │ version: 1.2 / 1.3 in Monochrome-Studio, PIC-Default, PIC-Dark, Terminal vs documented contract     │ header dump              │
 │ version 1 (parser ignores it)                                                                       │                          │
 ├─────────────────────────────────────────────────────────────────────────────────────────────────────┼──────────────────────────┤
 │ corner-style declared only by Terminal + windows old (both square); window-controls: only ever      │ header dump              │
 │ native; the base: key is parsed but used by nothing                                                 │                          │
 ├─────────────────────────────────────────────────────────────────────────────────────────────────────┼──────────────────────────┤
 │ Ignored tests reference non-existent ids — src/window/theme.rs:404-421 unwraps Cyberpunk, OLED,     │ grep +                   │
 │ Premium Studio, ww → --ignored would panic; no CI workflow runs window::theme:: tests, so it's      │ .github/workflows/*      │
 │ dormant                                                                                             │                          │
 └─────────────────────────────────────────────────────────────────────────────────────────────────────┴──────────────────────────┘

 Still to do: confirm traffic-light/window-controls wiring in src/window/layout.rs; reconcile docs/THEMES.md shipped-list drift
 (docs name GBand, Bright Studio, Premium Studio, RetroPC, macchiato, tokyo night, ww — none on disk; docs omit 9 real folders);
 per-theme button coverage table (context menus, search, sidebar, infobar, crop/filter tiles, swatches, spinbuttons); then the
 final checklist.


 increase your context to 256k
