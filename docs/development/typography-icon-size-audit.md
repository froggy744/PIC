# Typography and icon sizing audit

Snapshot: 2026-11-08. This is a static source audit to help assess uniformity; it is not a claim that every screen, theme, scale factor or locale was visually tested. No application code was changed. Values below are GTK CSS pixels / GTK logical dimensions unless otherwise stated; actual rendered sizes depend on GTK, fonts, display scaling, widget allocation, inheritance and the selected theme.

## How to use this report

Compare **the same role** across screens and themes, rather than forcing decorative titles, photo placeholders and compact badges into one size. The inventory below is a searchable list of source lines, not computed styles: it includes inactive/fallback theme files, comments, tests, multiline expressions and photo-rendering fonts where matched. A declaration can be overridden or its selector may not apply. GTK built-in defaults, icon assets and declarations constructed in ways that do not match the search are not fully captured. Line numbers refer to this snapshot.

## Current contracts and findings

| Area | Evidence | Assessment / action to verify |
| --- | --- | --- |
| Linux body font | `src/css/linux.css:1` leaves family and base size to GTK/desktop; theme CSS may set them. | Native sizing is intentional; record the actual font and size at runtime before choosing a baseline. |
| Other platforms | `src/css/macos.css:2` sets 13px; `src/css/windows.css:2` sets 14px and additional role sizes. | Platform differences are explicit. Check whether platform providers overriding theme typography is intended; Linux does not use these providers. |
| Provider precedence | `src/css/mod.rs:45`, `src/window/theme.rs:166`, `src/window/theme.rs:255` | Base is at application priority; overlays are +1, +3 or scrapbook +7; platform +10, screensaver +11, toolbar +12, sidebar +14, quick filters +15. Other component providers have their own priorities. This differs from the simplified layering in `docs/THEMES.md`; inspect selectors and the active widget before inferring the winning value. |
| Album titles | `src/css/components/albums.css:2`, `:31`, `:13`, `:72` | Identical selectors are repeated: index title 22px/700 then 24px/750, standard album name 14px/600 then 15px/650. Later declarations in this stylesheet supersede earlier ones when the same provider is loaded; verify the resulting visual hierarchy before consolidating. |
| Info title | `themes/iDark/theme.css:59`, `src/css/components/metadata_filter_layout.css:259` | Base theme specifies 13px and the scoped metadata rule specifies 12px at +2. Depending on selected theme priority and selector matching, this can vary. Compare the actual InfoBar title under representative themes. |
| Theme identity | `themes/scrapbook/theme.css:43`, `:66`; `themes/Terminal/theme.css:815` | Scrapbook uses handwritten fonts and oversized decorative headings; Terminal specifies monospace in a scoped rule. Treat these as deliberate theme identity until reviewed, and check fallbacks when fonts are unavailable. |
| Small text | `themes/iDark/theme.css:48`, `src/css/base.css:14`, `src/css/components/metadata_filter_layout.css:203` | 10–11px labels/badges exist alongside 12px controls. Review readability, clipping and contrast at the desktop text scale and high DPI; size alone does not prove a defect. |
| Header / InfoBar actions | `src/css/components/toolbar_action_consistency.css:1`, `:14`, `:42` | Shared tagged controls specify a 16px glyph and 32px CSS minimum with a 1px border; the comment describes a 34px target. Measure actual allocated bounds, including borders, rather than assuming CSS minimum equals allocation. |
| Sidebar actions and rows | `src/sidebar.rs:177`, `:184`, `:3636`; `src/css/components/sidebar_action_consistency.css:2`, `:17` | Header action glyph 14px and requested control size 28px; row icons 18px, shared/recent 16px and disclosure slot 12px. These are different roles. Measure actual hit targets and compare same-role actions. |
| Grid badges and placeholders | `src/grid/view.rs:331`, `:337`; `src/albums/card.rs:572`, `:611` | Grid badges use 18px; placeholders span 24–64px in different contexts. Compare like-for-like placeholders and badges, not all icons together. |
| Text on photos / screensaver | `src/edit/text_render.rs:27`, `src/screensaver/matrix.rs:136` | Pango sizing here serves rendered photo content or screensaver animation, not ordinary UI typography. Keep those sizes independent of an interface uniformity pass. |

## Runtime verification checklist

Record theme, desktop font, display scale, desktop text scale, window size and locale for each run. Use GTK Inspector to capture the effective font, CSS selector/provider, image size and allocated widget bounds; distinguish glyph size from the clickable area. Test a baseline theme (standard/iDark), PIC-Default or PIC-Dark, Scrapbook and Terminal; add other shipped themes where inventory differences are relevant. Repeat at normal and enlarged text scaling and narrow window widths.

- [ ] Main header, InfoBar and menus: compare action glyphs, targets, text labels, tooltips and overflow.
- [ ] Sidebar: compare section headings, row labels, counts, disclosures, shared/watch badges and actions across Library, Ratings, Folders, Shared and Albums.
- [ ] Library dashboard, grid, photo wall and albums (grid, bookshelf, cover flow): compare titles, counts, filenames, badges and placeholders within each role.
- [ ] Viewer, photo properties, metadata filters, search and context menus: check text hierarchy, truncation and controls at increased text size.
- [ ] Editor, crop, drawing, overlays, filters and collage: check section headings, values, controls, icons and narrow panel behavior.
- [ ] Import, onboarding, settings, dialogs and screensaver: check headings, descriptions, buttons, large icons and clipping.
- [ ] Check theme font fallbacks and translated/long labels. Record observed mismatches with screen, theme, effective size, expected role, screenshot and reproduction steps before changing code.

## Source inventory

Generated by scanning all `.css` and `.rs` files below `src/` and `themes/` for font declarations, icon sizing APIs/CSS and selected Pango/markup APIs. Paths and line numbers are provided for follow-up; source lines may include multiple declarations. This is a broad candidate index, not a CSS parser or a runtime measurement.

### CSS typography declarations (including inline Rust CSS) (559 matching lines)

**src/collage/editor.rs**

```text
98: button.collage-layout-tile label { font-size: 11px; font-weight: normal; }\
100: button.collage-compact-choice label { font-weight: normal; font-size: 12px; }\
```

**src/css/base.css**

```text
2: .edit-panel-tabs button { min-height: 36px; padding: 0 13px; font-weight: 600; }
8: .edit-section-label { font-size: 12px; font-weight: 600; color: alpha(@theme_fg_color, 0.62); }
11: .edit-adjustment-value { font-variant-numeric: tabular-nums; min-width: 42px; }
14: .thumbnail-filename { font-size: 11px; }
28: .edit-fit-row button { min-width: 0; padding: 3px 5px; font-size: 11px; }
30: .overlay-anchor-row button { min-height: 30px; padding: 3px 4px; font-size: 11px; }
49: .filter-tile-label { font-size: 12px; font-weight: 600; }
110: font-weight: normal;
119: font-size: 11px;
135: .tour-heading { font-size: 22px; font-weight: 700; }
136: .onboarding-welcome-heading { font-size: 20px; font-weight: 600; }
138: font-size: 15px;
147: font-weight: 500;
151: font-size: 12px;
184: .infobar-selection-count { font-weight: 700; font-size: 12px; }
264: .metadata-filter-controls button label { font-weight: 500; }
275: font-size: 12px;
276: font-weight: 700;
282: .edit-adjustment-value { font-weight: 600; }
316: font-weight: normal;
324: .compact-sidebar-option label { min-width: 0; font-weight: normal; }
337: font-size: 14px;
338: font-weight: 650;
372: font-weight: 700;
```

**src/css/components/albums.css**

```text
2: .albums-index-title { font-size: 22px; font-weight: 700; text-shadow: none; }
13: .albums-standard .album-name { font-weight: 600; font-size: 14px; }
14: .albums-standard .album-photo-count { font-size: 12px; }
31: .albums-index-title { font-size: 24px; font-weight: 750; }
72: .albums-standard .album-name { font-size: 15px; font-weight: 650; }
78: .albums-coverflow-name { font-size: 21px; font-weight: 700; color: @theme_fg_color; }
```

**src/css/components/metadata_filter_layout.css**

```text
37: font-size: 12px;
38: font-weight: 600;
41: font-size: 12px;
45: font-size: 12px;
51: font-size: 12px;
55: font-size: 12px;
66: font-size: 12px;
75: font-size: 12px;
76: font-weight: 700;
94: font-weight: 500;
98: font-size: 12px;
102: font-size: 12px;
125: font-weight: 600;
159: font-weight: 600;
190: font-size: 12px;
191: font-weight: 700;
196: font-size: 12px;
200: font-size: 12px;
203: font-size: 11px;
206: font-size: 11px;
229: font-size: 12px;
230: font-weight: 500;
257: .photo-info-bar .metric-key { font-size: 11px; }
259: .photo-info-bar .info-title { font-size: 12px; }
286: .photo-info-bar .info-title { font-weight: 650; }
```

**src/css/components/photo_context_menu.css**

```text
19: font-weight: 400;
32: font-weight: 400;
50: font-weight: 400;
55: .photo-context-submenu label { font-weight: 400; }
```

**src/css/components/search_status.css**

```text
34: font-size: 12px;
```

**src/css/components/sidebar_action_consistency.css**

```text
65: font-variant-numeric: tabular-nums;
```

**src/css/edit_mode.css**

```text
24: font-weight: 600;
102: font-size: 12px;
103: font-weight: 600;
114: font-variant-numeric: tabular-nums;
172: font-weight: 700;
227: .draw-adjustment-value { font-weight: 600; }
```

**src/css/macos.css**

```text
2: window, button, entry, label, text, .navigation-sidebar { font-family: -apple-system, "SF Pro Text", "Helvetica Neue", sans-serif; font-size: 13px; }
3: .app-title { font-family: -apple-system, "SF Pro Display", "Helvetica Neue", sans-serif; font-size: 15px; font-weight: 700; }
```

**src/css/theme_discovery.rs**

```text
354: assert!(skin.css.contains("font-family: \"Caveat\""));
355: assert!(skin.css.contains("font-family: \"Patrick Hand\""));
356: assert!(!skin.css.contains("font-family: \"Poppins\""),
```

**src/css/windows.css**

```text
2: window, button, entry, label, text, .navigation-sidebar { font-family: "Segoe UI"; font-size: 14px; }
3: .app-title { font-size: 15px; font-weight: 700; }
4: .folder-section-title, .section-heading { font-size: 14px; font-weight: 700; }
5: .folder-section-count, .album-photo-count { font-size: 12px; }
6: .info-title, .metric-val, .section-count { font-size: 13px; }
7: .metric-key, .navigation-sidebar .heading { font-size: 11px; }
8: .edit-section-label, .filter-tile-label { font-size: 12px; }
```

**src/library_home.rs**

```text
36: .home-card-caption { font-size: 12px; }
37: .home-card-caption-secondary { font-size: 11px; }
50: .home-summary-value { font-size: 18px; font-weight: 700; }
51: .home-summary-label { font-size: 11px; }
52: .home-subtitle { font-size: 13px; }
```

**src/screensaver/gpu.rs**

```text
731: font-size: 13px;
739: font-size: 34px;
740: font-weight: 600;
```

**src/sidebar.rs**

```text
2982: "popover.pic-folder-context-menu > contents { padding: 4px; border-radius: 10px; }                  popover.pic-folder-context-menu button.folder-context-action {                  min-height: 28px; padding: 3px 10px; font-size: 12px;                  border-radius: 6px; font-weight: normal; }                  popover.pic-folder-context-menu button.folder-context-action label {                  font-size: 12px; font-weight: normal; }"
```

**themes/GbandV2/Aqua-GarageBand-theme-v2.css**

```text
78: font-weight: 700;
158: font-weight: 700;
162: .section-heading { font-size: 15px; }
163: .folder-section-title { font-size: 14px; }
164: .folder-section-count { color: #A8ABB0; font-size: 12px; }
324: .navigation-sidebar row:selected { font-weight: 700; }
325: .navigation-sidebar row.sidebar-scroll-location { font-weight: 600; }
331: font-weight: 700;
339: font-variant-numeric: tabular-nums;
350: .section-count { font-size: 13px; }
520: font-weight: 700;
571: font-weight: 700;
```

**themes/Monochrome-Studio/theme.css**

```text
115: .app-title { color: @pic_bar_text; font-weight: 700; text-shadow: none; }
152: .navigation-sidebar .sidebar-section-heading-title { font-weight: 700; }
153: .sidebar-count { font-variant-numeric: tabular-nums; }
183: font-weight: 700;
260: .edit-section-label { color: @pic_text_muted; font-weight: 700; }
455: font-weight: 600;
514: font-size: 12px;
515: font-weight: 400;
```

**themes/PIC-Dark/theme.css**

```text
112: .app-title { color: @pic_bar_text; font-weight: 700; text-shadow: none; }
149: .navigation-sidebar .sidebar-section-heading-title { font-weight: 700; }
150: .sidebar-count { font-variant-numeric: tabular-nums; }
180: font-weight: 700;
257: .edit-section-label { color: @pic_text_muted; font-weight: 700; }
450: font-weight: 600;
509: font-size: 12px;
510: font-weight: 400;
```

**themes/PIC-Default/theme.css**

```text
120: .app-title { color: @pic_bar_text; font-weight: 700; text-shadow: none; }
157: .navigation-sidebar .sidebar-section-heading-title { font-weight: 700; }
158: .sidebar-count { font-variant-numeric: tabular-nums; }
188: font-weight: 700;
265: .edit-section-label { color: @pic_text_muted; font-weight: 700; }
460: font-weight: 600;
519: font-size: 12px;
520: font-weight: 400;
778: font-weight: 700;
```

**themes/Picasa-3/theme.css**

```text
42: .app-title { color: #3D3D3D; font-size: 14px; font-weight: 600; text-shadow: none; }
89: .navigation-sidebar .heading { color: #777777; font-size: 11px; font-weight: 700; }
90: .navigation-sidebar .sidebar-section-heading-title { color: #555555; font-weight: 700; }
99: .section-heading { color: #4A4A4A; font-size: 14px; font-weight: 600; text-shadow: none; }
102: .folder-section-title { color: #4A4A4A; font-size: 13px; font-weight: 600; text-shadow: none; }
103: .folder-section-count { color: #777777; font-size: 11px; }
252: font-weight: 700;
```

**themes/Terminal/theme.css**

```text
114: .app-title { color: @pic_bar_text; font-weight: 700; text-shadow: none; }
151: .navigation-sidebar .sidebar-section-heading-title { font-weight: 700; }
152: .sidebar-count { font-variant-numeric: tabular-nums; }
186: font-weight: 700;
263: .edit-section-label { color: @pic_text_muted; font-weight: 700; }
483: font-weight: 600;
543: font-size: 12px;
544: font-weight: 400;
815: font-family: monospace;
```

**themes/apperture/theme.css**

```text
62: .app-title { color: #E6E6E6; text-shadow: 0 -1px 0 alpha(#000000, 0.6); font-weight: 700; }
135: .section-heading { color: #E0E0E0; font-size: 14px; font-weight: 700; text-shadow: 0 -1px 0 alpha(#000000, 0.6); }
136: .section-count { font-size: 12px; color: @ap_dim; }
170: font-weight: 700;
188: .folder-section-title { color: #E0E0E0; font-weight: 700; font-size: 13px; text-shadow: 0 -1px 0 alpha(#000000, 0.6); }
189: .folder-section-count { color: @ap_dim; font-size: 12px; }
231: .albums-index-title { font-size: 22px; font-weight: 700; text-shadow: 0 -1px 0 alpha(#000000, 0.6); }
239: .albums-standard .album-name { font-weight: 600; font-size: 13px; }
240: .albums-standard .album-photo-count { font-size: 12px; color: @ap_dim; }
257: font-weight: 700;
273: .navigation-sidebar .sidebar-section-heading-title { color: #9A9A9A; font-size: inherit; font-weight: 700; letter-spacing: 0.4px; text-shadow: 0 -1px 0 alpha(#000000, 0.6); }
278: .sidebar-count { min-width: 38px; font-variant-numeric: tabular-nums; color: @ap_dim; }
405: font-weight: 700;
```

**themes/aqua/theme.css**

```text
65: .app-title { color: #2A2D34; text-shadow: 0 1px 0 @ap_emboss; font-weight: 700; }
139: .section-heading { color: #2A2D34; font-size: 14px; font-weight: 700; text-shadow: 0 1px 0 @ap_emboss; }
140: .section-count { font-size: 12px; color: @ap_dim; }
174: font-weight: 700;
192: .folder-section-title { color: #2A2D34; font-weight: 700; font-size: 13px; text-shadow: 0 1px 0 @ap_emboss; }
193: .folder-section-count { color: @ap_dim; font-size: 12px; }
235: .albums-index-title { font-size: 22px; font-weight: 700; text-shadow: 0 1px 0 @ap_emboss; }
243: .albums-standard .album-name { font-weight: 600; font-size: 13px; }
244: .albums-standard .album-photo-count { font-size: 12px; color: @ap_dim; }
261: font-weight: 700;
279: .navigation-sidebar .sidebar-section-heading-title { color: #6F7480; font-size: inherit; font-weight: 700; letter-spacing: 0.4px; text-shadow: 0 1px 0 @ap_emboss; }
282: .sidebar-count { min-width: 38px; font-variant-numeric: tabular-nums; color: @ap_dim; }
410: font-weight: 700;
```

**themes/archive ledger/theme.css**

```text
49: .folder-section-title { color: @theme_fg_color; font-weight: 700; font-size: 14px; text-shadow: none; }
50: .folder-section-count { color: alpha(@theme_fg_color, 0.58); font-size: 12px; }
94: .offline-badge { min-width: 20px; min-height: 20px; padding: 0; border-radius: 9999px; color: #2a1a00; background: #B08A3E; font-weight: 700; }
95: .sidebar-offline-badge { min-width: 16px; min-height: 16px; padding: 0; border-radius: 9999px; color: #2a1a00; background: #B08A3E; font-weight: 700; font-size: 10px; }
101: .section-heading { color: @theme_fg_color; font-size: 15px; text-shadow: none; }
121: .sidebar-count { min-width: 38px; font-variant-numeric: tabular-nums; }
122: .section-count { font-size: 13px; }
128: font-weight: 700;
138: .navigation-sidebar .sidebar-section-heading-title { color: alpha(@theme_fg_color, 0.88); font-size: inherit; font-weight: 700; }
```

**themes/aurora/theme.css**

```text
64: .folder-section-title { color: @nord6; font-weight: 700; font-size: 14px; text-shadow: none; }
65: .folder-section-count { color: alpha(@nord4, 0.58); font-size: 12px; }
111: .offline-badge { min-width: 20px; min-height: 20px; padding: 0; border-radius: 9999px; color: @nord0; background: @nord13; font-weight: 700; }
112: .sidebar-offline-badge { min-width: 16px; min-height: 16px; padding: 0; border-radius: 9999px; color: @nord0; background: @nord13; font-weight: 700; font-size: 10px; }
118: .section-heading { color: @nord6; font-size: 15px; text-shadow: none; }
138: .sidebar-count { min-width: 38px; font-variant-numeric: tabular-nums; }
139: .section-count { font-size: 13px; }
145: font-weight: 700;
155: .navigation-sidebar .sidebar-section-heading-title { color: alpha(@nord6, 0.88); font-size: inherit; font-weight: 700; }
```

**themes/frappe/theme.css**

```text
64: .folder-section-title { color: @nord6; font-weight: 700; font-size: 14px; text-shadow: none; }
65: .folder-section-count { color: alpha(@nord4, 0.58); font-size: 12px; }
111: .offline-badge { min-width: 20px; min-height: 20px; padding: 0; border-radius: 9999px; color: @nord0; background: @nord13; font-weight: 700; }
112: .sidebar-offline-badge { min-width: 16px; min-height: 16px; padding: 0; border-radius: 9999px; color: @nord0; background: @nord13; font-weight: 700; font-size: 10px; }
118: .section-heading { color: @nord6; font-size: 15px; text-shadow: none; }
138: .sidebar-count { min-width: 38px; font-variant-numeric: tabular-nums; }
139: .section-count { font-size: 13px; }
145: font-weight: 700;
155: .navigation-sidebar .sidebar-section-heading-title { color: alpha(@nord6, 0.88); font-size: inherit; font-weight: 700; }
```

**themes/glass/theme.css**

```text
72: .folder-section-title { color: @theme_fg_color; font-weight: 700; font-size: 14px; text-shadow: none; }
73: .folder-section-count { color: alpha(@theme_fg_color, 0.58); font-size: 12px; }
119: .offline-badge { min-width: 20px; min-height: 20px; padding: 0; border-radius: 9999px; color: #2a1a00; background: #f2c14e; font-weight: 700; }
120: .sidebar-offline-badge { min-width: 16px; min-height: 16px; padding: 0; border-radius: 9999px; color: #2a1a00; background: #f2c14e; font-weight: 700; font-size: 10px; }
126: .section-heading { color: @theme_fg_color; font-size: 15px; text-shadow: none; }
147: .sidebar-count { min-width: 38px; font-variant-numeric: tabular-nums; }
148: .section-count { font-size: 13px; }
154: font-weight: 700;
164: .navigation-sidebar .sidebar-section-heading-title { color: alpha(@theme_fg_color, 0.88); font-size: inherit; font-weight: 700; }
```

**themes/grey/theme.css**

```text
21: .app-title { color: #D8D8D8; text-shadow: none; font-weight: 600; }
47: .folder-section-title { color: #D8D8D8; font-weight: 600; font-size: 14px; text-shadow: none; }
48: .folder-section-count { color: #8A8A8A; font-size: 12px; }
94: .offline-badge { min-width: 20px; min-height: 20px; padding: 0; border-radius: 9999px; color: #1E1E1E; background: #C4C4C4; font-weight: 700; }
95: .sidebar-offline-badge { min-width: 16px; min-height: 16px; padding: 0; border-radius: 9999px; color: #1E1E1E; background: #C4C4C4; font-weight: 700; font-size: 10px; }
101: .section-heading { color: #D8D8D8; font-size: 15px; text-shadow: none; font-weight: 600; }
121: .sidebar-count { min-width: 38px; font-variant-numeric: tabular-nums; color: #8A8A8A; }
122: .section-count { font-size: 13px; color: #8A8A8A; }
128: font-weight: 600;
138: .navigation-sidebar .sidebar-section-heading-title { color: #9A9A9A; font-size: inherit; font-weight: 700; }
```

**themes/iDark/theme.css**

```text
14: .app-title { font-weight: 700; font-size: 15px; margin-left: 4px; color: #f5f5f5; text-shadow: 0 1px #111; }
27: .folder-section-title { color: #f0f0f0; font-weight: 700; font-size: 14px; text-shadow: 0 1px #151515; }
28: .folder-section-count { color: #a8a8a8; font-size: 12px; }
47: .offline-badge { min-width: 20px; min-height: 20px; padding: 0; border-radius: 9999px; color: #2a1a00; background: #f2c14e; font-weight: 700; }
48: .sidebar-offline-badge { min-width: 16px; min-height: 16px; padding: 0; border-radius: 9999px; color: #2a1a00; background: #f2c14e; font-weight: 700; font-size: 10px; }
54: .section-heading { color: #f0f0f0; font-weight: 700; font-size: 14px; text-shadow: 0 1px #151515; }
59: .info-title { font-weight: 700; font-size: 13px; color: #f4f4f4; }
60: .metric-key { font-size: 11px; color: #bcbcbc; font-weight: 600; }
61: .metric-val { font-size: 13px; font-weight: 500; color: #eeeeee; }
75: .sidebar-count { min-width: 38px; font-variant-numeric: tabular-nums; color: #a9a9a9; }
76: .section-count { font-size: 13px; color: #bcbcbc; }
86: .navigation-sidebar .sidebar-section-heading-title { color: #f0f0f0; font-size: inherit; font-weight: 700; }
88: .navigation-sidebar .heading { color: #a9a9a9; font-size: 11px; font-weight: 700; margin-top: 14px; margin-bottom: 4px; }
115: .edit-toolbar-actions button.edit-toolbar-done { min-height: 40px; padding: 0 20px; border-radius: 999px; background: #3584e4; color: #ffffff; font-weight: 700; }
```

**themes/lars/theme.css**

```text
22: .app-title { color: #FF9C00; text-shadow: none; font-weight: 800; letter-spacing: 0.5px; }
48: .folder-section-title { color: #FF9C00; font-weight: 800; font-size: 14px; text-shadow: none; letter-spacing: 0.5px; }
49: .folder-section-count { color: #CC99CC; font-size: 12px; }
95: .offline-badge { min-width: 20px; min-height: 20px; padding: 0; border-radius: 9999px; color: #000000; background: #FFCC66; font-weight: 800; }
96: .sidebar-offline-badge { min-width: 16px; min-height: 16px; padding: 0; border-radius: 9999px; color: #000000; background: #FFCC66; font-weight: 800; font-size: 10px; }
102: .section-heading { color: #FF9C00; font-size: 15px; text-shadow: none; font-weight: 800; letter-spacing: 0.5px; }
109: .photo-action-button { border-radius: 9999px; color: #000000; background: #99CCFF; border: none; box-shadow: none; font-weight: 700; }
115: button.clear-action-button { color: #000000; background: #CC99CC; border: none; border-radius: 9999px; font-weight: 700; }
122: .sidebar-count { min-width: 38px; font-variant-numeric: tabular-nums; color: #99CCFF; }
123: .section-count { font-size: 13px; color: #CC99CC; }
129: font-weight: 800;
139: .navigation-sidebar .sidebar-section-heading-title { color: #CC99CC; font-size: inherit; font-weight: 800; letter-spacing: 0.5px; }
```

**themes/latte/theme.css**

```text
65: .folder-section-title { color: @nord6; font-weight: 700; font-size: 14px; text-shadow: none; }
66: .folder-section-count { color: alpha(@nord4, 0.58); font-size: 12px; }
112: .offline-badge { min-width: 20px; min-height: 20px; padding: 0; border-radius: 9999px; color: @nord0; background: @nord13; font-weight: 700; }
113: .sidebar-offline-badge { min-width: 16px; min-height: 16px; padding: 0; border-radius: 9999px; color: @nord0; background: @nord13; font-weight: 700; font-size: 10px; }
119: .section-heading { color: @nord6; font-size: 15px; text-shadow: none; }
139: .sidebar-count { min-width: 38px; font-variant-numeric: tabular-nums; }
140: .section-count { font-size: 13px; }
146: font-weight: 700;
156: .navigation-sidebar .sidebar-section-heading-title { color: alpha(@nord6, 0.88); font-size: inherit; font-weight: 700; }
```

**themes/neomorphism/theme.css**

```text
66: .folder-section-title { color: @theme_fg_color; font-weight: 700; font-size: 14px; text-shadow: none; }
67: .folder-section-count { color: alpha(@theme_fg_color, 0.58); font-size: 12px; }
113: .offline-badge { min-width: 20px; min-height: 20px; padding: 0; border-radius: 9999px; color: #4A5056; background: #D3D9E0; font-weight: 700; }
114: .sidebar-offline-badge { min-width: 16px; min-height: 16px; padding: 0; border-radius: 9999px; color: #4A5056; background: #D3D9E0; font-weight: 700; font-size: 10px; }
120: .section-heading { color: @theme_fg_color; font-size: 15px; text-shadow: none; }
141: .sidebar-count { min-width: 38px; font-variant-numeric: tabular-nums; }
142: .section-count { font-size: 13px; }
148: font-weight: 700;
158: .navigation-sidebar .sidebar-section-heading-title { color: alpha(@theme_fg_color, 0.88); font-size: inherit; font-weight: 700; }
```

**themes/nord/theme.css**

```text
64: .folder-section-title { color: @nord6; font-weight: 700; font-size: 14px; text-shadow: none; }
65: .folder-section-count { color: alpha(@nord4, 0.58); font-size: 12px; }
111: .offline-badge { min-width: 20px; min-height: 20px; padding: 0; border-radius: 9999px; color: @nord0; background: @nord13; font-weight: 700; }
112: .sidebar-offline-badge { min-width: 16px; min-height: 16px; padding: 0; border-radius: 9999px; color: @nord0; background: @nord13; font-weight: 700; font-size: 10px; }
118: .section-heading { color: @nord6; font-size: 15px; text-shadow: none; }
138: .sidebar-count { min-width: 38px; font-variant-numeric: tabular-nums; }
139: .section-count { font-size: 13px; }
145: font-weight: 700;
155: .navigation-sidebar .sidebar-section-heading-title { color: alpha(@nord6, 0.88); font-size: inherit; font-weight: 700; }
```

**themes/purple/theme.css**

```text
90: .app-title { color: @theme_fg_color; font-weight: 700; text-shadow: none; }
136: .section-heading { color: @theme_fg_color; font-size: 17px; font-weight: 800; text-shadow: none; }
164: font-weight: 700;
185: .folder-section-title { color: @theme_fg_color; font-weight: 800; font-size: 15px; text-shadow: none; }
186: .folder-section-count { color: alpha(@theme_fg_color, 0.55); font-size: 12px; }
236: .albums-index-title { font-size: 24px; font-weight: 800; text-shadow: none; }
249: .albums-standard .album-name { font-weight: 700; font-size: 14px; }
250: .albums-standard .album-photo-count { font-size: 12px; }
268: font-weight: 700;
278: .navigation-sidebar .sidebar-section-heading-title { color: alpha(@theme_fg_color, 0.88); font-weight: 700; }
281: .sidebar-count { min-width: 38px; font-variant-numeric: tabular-nums; color: alpha(@theme_fg_color, 0.55); }
282: .section-count { font-size: 13px; color: alpha(@theme_fg_color, 0.60); }
367: font-weight: 700;
```

**themes/retro/theme.css**

```text
50: .folder-section-title { color: @theme_fg_color; font-weight: 700; font-size: 14px; text-shadow: none; }
51: .folder-section-count { color: alpha(@theme_fg_color, 0.58); font-size: 12px; }
95: .offline-badge { min-width: 20px; min-height: 20px; padding: 0; border-radius: 9999px; color: #2a1a00; background: #E3A94C; font-weight: 700; }
96: .sidebar-offline-badge { min-width: 16px; min-height: 16px; padding: 0; border-radius: 9999px; color: #2a1a00; background: #E3A94C; font-weight: 700; font-size: 10px; }
102: .section-heading { color: @theme_fg_color; font-size: 15px; text-shadow: none; }
136: .sidebar-count { min-width: 38px; font-variant-numeric: tabular-nums; }
137: .section-count { font-size: 13px; }
143: font-weight: 700;
153: .navigation-sidebar .sidebar-section-heading-title { color: alpha(@theme_fg_color, 0.88); font-size: inherit; font-weight: 700; }
```

**themes/scrapbook/theme.css**

```text
43: font-family: "Patrick Hand", "Cantarell", sans-serif;
44: font-size: 13px;
66: .app-title { font-weight: 900; font-family: "Caveat", "Patrick Hand", cursive; font-size: 22px; }
97: font-weight: 800;
99: font-family: "Caveat", "Patrick Hand", cursive;
100: font-size: 15px;
109: font-variant-numeric: tabular-nums;
143: font-size: 15px;
144: font-weight: 800;
145: font-family: "Caveat", "Patrick Hand", cursive;
150: font-size: 12px;
192: .thumbnail-filename { color: #675549; font-size: 11px; }
198: font-family: "Caveat", "Patrick Hand", cursive;
199: font-size: 29px; font-weight: 800; color: #60417b;
237: font-size: 16px;
238: font-weight: 800;
239: font-family: "Caveat", "Patrick Hand", cursive;
243: font-weight: 700;
244: font-size: 12px;
252: font-weight: 900;
253: font-size: 25px;
278: font-family: "Caveat", "Patrick Hand", cursive;
279: font-weight: 900;
280: font-size: 35px;
289: font-family: "Caveat", "Patrick Hand", cursive;
298: font-family: "Caveat", "Patrick Hand", cursive;
310: color: #b24e8a; font-size: 22px;
313: .scrapbook-hero-flourish { color: #ffe1a1; font-size: 21px; }
316: font-family: "Caveat", "Patrick Hand", cursive;
317: font-weight: 800;
319: .home-main-header .title-1 { font-size: 27px; }
344: .home-card-caption { color: #57453e; font-weight: 700; }
361: .info-title, .metric-val { color: #4b3837; font-weight: 650; }
411: font-size: 38px;
458: font-family: "Caveat", "Patrick Hand", cursive;
459: font-weight: 800;
484: font-family: "Patrick Hand", "Cantarell", sans-serif;
485: font-size: 15px;
496: font-family: "Caveat", "Patrick Hand", cursive;
497: font-size: 23px;
498: font-weight: 700;
515: .navigation-sidebar row { font-size: 16px; }
523: font-size: 12px;
532: font-family: "Caveat", cursive;
533: font-size: 30px;
534: font-weight: 700;
567: font-family: "Caveat", "Patrick Hand", cursive;
568: font-size: 64px;
569: font-weight: 700;
579: font-family: "Patrick Hand", sans-serif;
580: font-size: 19px;
598: font-size: 24px;
603: font-family: "Patrick Hand", sans-serif;
604: font-size: 16px;
617: font-size: 29px;
626: font-family: "Caveat", cursive;
627: font-size: 31px;
628: font-weight: 700;
636: font-family: "Patrick Hand", sans-serif;
637: font-size: 15px;
675: font-family: "Patrick Hand", sans-serif;
676: font-size: 22px;
677: font-weight: 600;
684: font-family: "Patrick Hand", sans-serif;
685: font-size: 14px;
697: font-family: "Patrick Hand", sans-serif;
698: font-size: 17px;
699: font-weight: 900;
704: font-size: 30px;
705: font-weight: 900;
725: font-family: "Patrick Hand", sans-serif;
726: font-size: 74px;
727: font-weight: 400;
731: font-family: "Caveat", cursive;
732: font-size: 27px;
751: font-family: "Patrick Hand", sans-serif;
752: font-size: 23px;
753: font-weight: 600;
758: font-family: "Patrick Hand", sans-serif;
759: font-size: 17px;
761: .scrapbook-gallery-actions button { font-size: 13px; }
788: font-size: 12px;
789: font-family: "Patrick Hand", sans-serif;
793: font-size: 25px;
801: font-family: "Patrick Hand", sans-serif;
802: font-size: 13px;
808: font-family: "Patrick Hand", sans-serif;
809: font-size: 18px;
825: font-family: "Patrick Hand", sans-serif;
826: font-size: 20px;
844: .photo-info-bar .info-title { font-family: "Patrick Hand", sans-serif; font-size: 15px; }
860: font-family: "Caveat", cursive;
861: font-size: 27px;
862: font-weight: 700;
873: font-family: "Caveat", cursive;
874: font-weight: 700;
875: font-size: 25px;
895: font-family: "Patrick Hand", sans-serif;
896: font-size: 15px;
903: font-family: "Patrick Hand", sans-serif;
913: font-family: "Patrick Hand", sans-serif;
914: font-size: 15px;
929: font-size: 16px;
930: font-family: "Patrick Hand", sans-serif;
938: font-family: "Patrick Hand", sans-serif;
940: font-size: 14px;
944: .scrapbook-note-status { color: #628552; font-size: 12px; }
954: font-family: "Patrick Hand", sans-serif;
955: font-size: 14px;
977: font-family: "Caveat", cursive;
978: font-weight: 700;
979: font-size: 29px;
989: font-size: 16px;
990: font-weight: 600;
992: font-family: "Patrick Hand", sans-serif;
996: font-family: "Patrick Hand", sans-serif;
997: font-size: 16px;
1007: font-size: 26px;
1008: font-weight: 700;
1009: font-family: "Caveat", cursive;
1034: font-family: "Patrick Hand", sans-serif;
1035: font-size: 16px;
1044: font-family: "Patrick Hand", sans-serif;
1045: font-size: 15px;
1066: font-family: "Patrick Hand", "Cantarell", sans-serif;
1067: font-size: 16px;
1081: font-family: "Caveat", "Patrick Hand", cursive;
1082: font-weight: 700;
1085: font-size: 27px;
1094: font-family: "Caveat", "Patrick Hand", cursive;
1095: font-weight: 700;
1097: font-size: 25px;
1099: .scrapbook-gallery-title { font-size: 29px; }
1101: font-family: "Caveat", "Patrick Hand", cursive;
1102: font-weight: 700;
1103: font-size: 24px;
1114: font-family: "Patrick Hand", "Cantarell", sans-serif;
1115: font-size: 18px;
1116: font-weight: 400;
1128: font-family: "Patrick Hand", "Cantarell", sans-serif;
1129: font-size: 15px;
1131: font-weight: 500;
1134: font-family: "Patrick Hand", "Cantarell", sans-serif;
1135: font-size: 15px;
1141: font-family: "Patrick Hand", "Cantarell", sans-serif;
1142: font-size: 16px;
1154: font-family: "Patrick Hand", "Cantarell", sans-serif;
1155: font-size: 15px;
1156: font-weight: 400;
1159: .scrapbook-photo-caption { font-size: 16px; }
1160: .home-card-caption { font-size: 17px; }
1161: .home-card-caption-secondary { font-size: 14px; color: #79685e; }
1163: font-family: "Patrick Hand", "Cantarell", sans-serif;
1164: font-size: 16px;
1175: font-family: "Patrick Hand", "Cantarell", sans-serif;
1176: font-size: 18px;
1177: font-weight: 400;
1189: font-family: "Patrick Hand", "Cantarell", sans-serif;
1193: font-size: 16px;
1194: font-weight: 650;
1197: font-size: 15px;
1198: font-weight: 600;
1201: font-size: 13px;
1202: font-weight: 400;
1205: .photo-info-bar .infobar-selection-count { font-size: 16px; }
1207: font-family: "Patrick Hand", "Cantarell", sans-serif;
1208: font-size: 15px;
1221: font-family: "Patrick Hand", "Cantarell", sans-serif;
1222: font-size: 16px;
1225: .scrapbook-home-inspector-name { font-size: 18px; font-weight: 600; color: #362b2a; }
1227: .scrapbook-inspector-location label { font-size: 14px; }
1233: font-family: "Patrick Hand", "Cantarell", sans-serif;
1234: font-size: 16px;
1236: .scrapbook-note-status { font-size: 13px; }
1244: font-family: "Patrick Hand", "Cantarell", sans-serif;
1245: font-size: 17px;
1250: font-family: "Patrick Hand", "Cantarell", sans-serif;
1251: font-size: 19px;
1252: font-weight: 800;
1257: font-family: "Patrick Hand", "Cantarell", sans-serif;
1258: font-size: 17px;
1261: font-family: "Patrick Hand", "Cantarell", sans-serif;
1262: font-size: 15px;
1275: font-family: "Caveat", "Patrick Hand", cursive;
1276: font-size: 29px;
1277: font-weight: 700;
1334: font-family: "Patrick Hand", "Cantarell", sans-serif;
1335: font-size: 16px;
1406: font-family: "Patrick Hand", "Cantarell", sans-serif;
1407: font-size: 14px;
1411: font-family: "Patrick Hand", "Cantarell", sans-serif;
1412: font-size: 17px;
```

**themes/snowleopard/theme-v2.css**

```text
92: font-weight: 700;
312: font-size: 14px;
313: font-weight: 700;
316: .section-count { font-size: 12px; color: @ap_dim; }
356: font-weight: 700;
381: font-weight: 700;
382: font-size: 13px;
385: .folder-section-count { color: @ap_dim; font-size: 12px; }
435: font-size: 22px;
436: font-weight: 700;
468: .albums-standard .album-name { font-weight: 600; font-size: 13px; }
470: font-size: 12px; color: @ap_dim;
518: font-size: inherit;
519: font-weight: 700;
527: font-variant-numeric: tabular-nums;
749: font-weight: 700;
```

**themes/standard/theme.css**

```text
39: .folder-section-title { color: @theme_fg_color; font-weight: 700; font-size: 14px; text-shadow: none; }
40: .folder-section-count { color: alpha(@theme_fg_color, 0.58); font-size: 12px; }
86: .offline-badge { min-width: 20px; min-height: 20px; padding: 0; border-radius: 9999px; color: #2a1a00; background: #f2c14e; font-weight: 700; }
87: .sidebar-offline-badge { min-width: 16px; min-height: 16px; padding: 0; border-radius: 9999px; color: #2a1a00; background: #f2c14e; font-weight: 700; font-size: 10px; }
93: .section-heading { color: @theme_fg_color; font-size: 15px; text-shadow: none; }
114: .sidebar-count { min-width: 38px; font-variant-numeric: tabular-nums; }
115: .section-count { font-size: 13px; }
121: font-weight: 700;
131: .navigation-sidebar .sidebar-section-heading-title { color: alpha(@theme_fg_color, 0.88); font-size: inherit; font-weight: 700; }
```

**themes/teal/theme.css**

```text
51: .folder-section-title { color: @theme_fg_color; font-weight: 700; font-size: 14px; text-shadow: none; }
52: .folder-section-count { color: alpha(@accent_bg_color, 0.60); font-size: 12px; }
98: .offline-badge { min-width: 20px; min-height: 20px; padding: 0; border-radius: 9999px; color: #2a1a00; background: #f2c14e; font-weight: 700; }
99: .sidebar-offline-badge { min-width: 16px; min-height: 16px; padding: 0; border-radius: 9999px; color: #2a1a00; background: #f2c14e; font-weight: 700; font-size: 10px; }
105: .section-heading { color: @theme_fg_color; font-size: 15px; text-shadow: none; }
125: .sidebar-count { min-width: 38px; font-variant-numeric: tabular-nums; }
126: .section-count { font-size: 13px; }
132: font-weight: 700;
142: .navigation-sidebar .sidebar-section-heading-title { color: alpha(@accent_bg_color, 0.85); font-size: inherit; font-weight: 700; }
```

**themes/tron/theme.css**

```text
62: font-weight: 800;
137: font-size: 15px;
138: font-weight: 800;
143: .section-more-btn { color: @accent_color; font-weight: 700; }
189: font-weight: 800;
199: font-weight: 800;
225: font-weight: 800;
226: font-size: 14px;
231: .folder-section-count { color: alpha(@theme_fg_color, 0.50); font-size: 12px; }
321: font-weight: 700;
331: font-weight: 600;
337: font-size: 11px;
338: font-weight: 800;
342: .navigation-sidebar .folder-count { color: alpha(@theme_fg_color, 0.60); font-variant-numeric: tabular-nums; }
344: .sidebar-count { min-width: 38px; font-variant-numeric: tabular-nums; color: alpha(@theme_fg_color, 0.55); }
345: .section-count { font-size: 13px; color: alpha(@theme_fg_color, 0.55); }
360: font-weight: 800;
390: .info-title, .metric-val { color: @theme_fg_color; font-weight: 700; }
489: font-weight: 800;
```

**themes/velvet-dusk/theme.css**

```text
47: .app-title { color: @theme_fg_color; font-weight: 700; letter-spacing: 0.02em; text-shadow: none; }
78: .section-heading { color: @theme_fg_color; font-size: 15px; font-weight: 700; text-shadow: none; }
107: font-weight: 700;
129: .folder-section-title { color: @theme_fg_color; font-weight: 700; font-size: 14px; text-shadow: none; }
130: .folder-section-count { color: alpha(@theme_fg_color, 0.55); font-size: 12px; }
195: font-weight: 700;
201: font-weight: 600;
207: font-size: 11px;
208: font-weight: 800;
211: .navigation-sidebar .folder-count { color: alpha(@theme_fg_color, 0.60); font-variant-numeric: tabular-nums; }
213: .sidebar-count { min-width: 38px; font-variant-numeric: tabular-nums; color: alpha(@theme_fg_color, 0.55); }
214: .section-count { font-size: 13px; color: alpha(@theme_fg_color, 0.55); }
242: .info-title, .metric-val { color: @theme_fg_color; font-weight: 600; }
278: font-weight: 700;
```

**themes/windows old/theme.css**

```text
39: .app-title { color: @nord6; font-weight: 700; text-shadow: 1px 1px @nord4; }
65: .folder-section-title { color: @nord4; font-weight: 700; font-size: 14px; text-shadow: 1px 1px @nord5; }
66: .folder-section-count { color: @nord4; font-size: 12px; }
112: .offline-badge { min-width: 20px; min-height: 20px; padding: 0; border-radius: 9999px; color: @nord0; background: @nord13; font-weight: 700; }
113: .sidebar-offline-badge { min-width: 16px; min-height: 16px; padding: 0; border-radius: 9999px; color: @nord0; background: @nord13; font-weight: 700; font-size: 10px; }
119: .section-heading { color: @nord4; font-size: 15px; text-shadow: 1px 1px @nord5; }
139: .sidebar-count { min-width: 38px; font-variant-numeric: tabular-nums; }
140: .section-count { font-size: 13px; }
146: font-weight: 700;
156: .navigation-sidebar .sidebar-section-heading-title { color: @nord4; font-size: inherit; font-weight: 700; }
```

### Icon sizing declarations and calls (53 matching lines)

**src/albums/card.rs**

```text
572: placeholder.set_pixel_size(48);
611: placeholder.set_pixel_size(24);
```

**src/collage/editor.rs**

```text
1121: image.set_pixel_size(12);
```

**src/css/components/metadata_filter_layout.css**

```text
131: -gtk-icon-size: 11px;
```

**src/css/components/quick_filters.css**

```text
72: .metadata-filter-toolbar menubutton.quick-filter-add > button image { -gtk-icon-size: 12px; }
```

**src/css/components/sidebar_action_consistency.css**

```text
17: -gtk-icon-size: 14px;
```

**src/css/components/toolbar_action_consistency.css**

```text
42: -gtk-icon-size: 16px;
```

**src/edit/draw_ui.rs**

```text
30: icon.set_pixel_size(22);
213: custom_icon.set_pixel_size(16);
```

**src/edit/editor.rs**

```text
317: warning_icon.set_pixel_size(18);
```

**src/edit/overlays_ui.rs**

```text
1288: warning.set_pixel_size(16);
```

**src/edit/text_ui.rs**

```text
158: custom_icon.set_pixel_size(16);
```

**src/grid/tile.rs**

```text
1372: badge.set_pixel_size(18);
1410: badge.set_pixel_size(18);
1449: badge.set_pixel_size(18);
```

**src/grid/view.rs**

```text
331: placeholder.set_pixel_size(32);
337: checkmark.set_pixel_size(18);
346: favorite_badge.set_pixel_size(18);
356: edited_badge.set_pixel_size(18);
617: folder_icon.set_pixel_size(16);
```

**src/grid/virtualization.rs**

```text
199: placeholder.set_pixel_size(32);
```

**src/infobar.rs**

```text
177: preview.set_pixel_size(40);
1014: thumbnail.set_pixel_size(38);
```

**src/library_home/scrapbook.rs**

```text
240: folder.set_pixel_size(22);
765: icon.set_pixel_size(21);
```

**src/library_home.rs**

```text
189: image.set_pixel_size(20);
783: placeholder.set_pixel_size(32);
```

**src/sd_import.rs**

```text
534: device_icon.set_pixel_size(64);
```

**src/sidebar.rs**

```text
178: const SIDEBAR_ICON_SIZE: i32 = 18;
180: pub(crate) const SHARED_ICON_SIZE: i32 = 16;
181: const SIDEBAR_RECENT_ICON_SIZE: i32 = 16;
184: const SIDEBAR_ACTION_ICON_SIZE: i32 = 14;
2810: disclosure.set_pixel_size(SIDEBAR_DISCLOSURE_SLOT - 2);
2834: icon_overlay.set_size_request(SIDEBAR_ICON_SIZE, SIDEBAR_ICON_SIZE);
2844: icon.set_pixel_size(if is_shared_root {
2845: SHARED_ICON_SIZE
2847: SIDEBAR_ICON_SIZE
2859: watch_badge.set_pixel_size(10);
3331: sidebar_icon_slot_with_size(icon_name, SIDEBAR_ICON_SIZE)
3337: slot.set_size_request(SIDEBAR_ICON_SIZE, SIDEBAR_ICON_SIZE);
3342: icon.set_pixel_size(icon_size);
3350: slot.set_size_request(SIDEBAR_DISCLOSURE_SLOT, SIDEBAR_ICON_SIZE);
3413: SIDEBAR_RECENT_ICON_SIZE
3415: SIDEBAR_ICON_SIZE
3613: image.set_pixel_size(12);
3636: image.set_pixel_size(SIDEBAR_ACTION_ICON_SIZE);
3652: image.set_pixel_size(SIDEBAR_ACTION_ICON_SIZE);
```

**src/window/layout.rs**

```text
927: image.set_pixel_size(18);
953: folder_icon.set_pixel_size(18);
969: shared_icon.set_pixel_size(sidebar::SHARED_ICON_SIZE);
```

**src/window/photo_actions.rs**

```text
1841: missing.set_pixel_size(56);
1849: close_icon.set_pixel_size(12);
2258: icon.set_pixel_size(20);
```

### Pango and markup font sizing (8 matching lines)

**src/edit/text_render.rs**

```text
27: let mut description = pango::FontDescription::new();
29: description.set_absolute_size(size);
36: layout.set_font_description(Some(&description));
```

**src/screensaver/matrix.rs**

```text
32: let font = gtk4::pango::FontDescription::from_string("Sans 10");
33: layout.set_font_description(Some(&font));
136: let font = gtk4::pango::FontDescription::from_string(&format!(
140: layout.set_font_description(Some(&font));
```

**src/screensaver.rs**

```text
712: layout.set_font_description(Some(&gtk::pango::FontDescription::from_string(
```
