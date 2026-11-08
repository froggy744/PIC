# UI sizing roles

These are logical GTK pixels and design targets, not fixed physical pixels or
guarantees about widget allocation. GTK, desktop text scaling, theme rules,
content and accessibility can affect the result. Use semantic component
classes and keep the desktop font on Linux; do not apply a global font family
or size. Decorative theme headings and context-specific hierarchies may vary.

| Text role | Target size / weight |
| --- | --- |
| Application title | 15px / 700 |
| Main page heading | 24px / 700 |
| Section heading | 14px / 600 |
| Album name | 15px / 600 |
| Sidebar navigation and standard controls | 13px / 400–500 |
| Properties values | 13px / 400 |
| Metadata labels | 12px / 500 |
| Secondary text and filenames | 12px / 400 |
| Counts and small badges | 11px / 500–600 |

| Icon role | Target size |
| --- | --- |
| Main toolbar, InfoBar actions and quick filters | 16px |
| Sidebar navigation, including compact mode | 18px |
| Sidebar header and small inline actions | 14px |
| Standard menu | 16px, or native GTK size |
| Disclosure | 12px |
| Grid status badge | 18px |

Toolbar and InfoBar actions target 34×34px, quick-filter actions 32–34px,
sidebar header actions 28×28px and compact inline actions 28–30px. A 6px
radius and 4px adjacent toolbar gap are the general targets; compact toolbar
radii may range from 4–6px. CSS minimums, borders, padding and widget size
requests all contribute to allocation, so measure the rendered controls.
Keep quick filters flat and single-row, and retain the distinct metadata
explorer. Placeholders, preview images, photo content and screensaver artwork
retain their own context-specific sizing.

These targets guide incremental consolidation; existing controls and theme
overrides can differ. Verify normal and compact layouts, narrow windows,
resizing and enlarged desktop text before treating a role as conforming. See
[theme layering](../THEMES.md#layering) for provider precedence.
