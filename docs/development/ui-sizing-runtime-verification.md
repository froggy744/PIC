# UI sizing runtime verification

Date: 2026-11-08. Status: **COMPLETE WITH LIMITATIONS**. The scoped source
changes have no confirmed blocking regression, but the missing measurements
and failed GUI interactions below prevent a full visual acceptance claim.

## Setup and evidence

PIC was run from the current worktree's debug binary on private Xvfb displays
with the GTK X11/Cairo renderer and Openbox. The GUI harness generated 15
sample photos and used isolated configuration and database directories. A
second private run used a copy of that generated database; no real library was
opened. Captures below are under gitignored `reports/gui-tests/` and are local
artifacts, not versioned files. Images were inspected directly; automated
checkpoint results are identified separately.

The harness reports its fallback result as BLOCKED with “No application
interaction was performed”, but its log and screenshots show that the first
four checkpoints did execute. After those checkpoints, its AT-SPI client
aborted: D-Bus could not execute `org.a11y.Bus` (permission denied). Therefore
the viewer and later checkpoints were not reached. In PIC-Default, PIC-Dark
and Terminal the folder checkpoint also reported zero visible tiles; captured
folder images include a transition, so the cause is not established.

## Theme screenshots

All six runs reached a populated Library and captured the sidebar and InfoBar.
Scrapbook's handwriting and Terminal's monospace appearance are visible.
These snapshots do not verify the other screens or theme switching.

| Theme | Capture | Automated folder checkpoint |
| --- | --- | --- |
| Standard | `reports/gui-tests/20261108T115621Z-568366/screenshots/02b-folder-view.png` | Passed |
| iDark | `reports/gui-tests/20261108T115648Z-568822/screenshots/02b-folder-view.png` | Passed |
| PIC-Default | `reports/gui-tests/20261108T115658Z-569192/screenshots/02b-folder-view.png` | Failed: zero tiles detected |
| PIC-Dark | `reports/gui-tests/20261108T115706Z-569556/screenshots/02b-folder-view.png` | Failed: zero tiles detected |
| Scrapbook | `reports/gui-tests/20261108T115715Z-569913/screenshots/02b-folder-view.png` | Passed |
| Terminal | `reports/gui-tests/20261108T115723Z-570371/screenshots/02b-folder-view.png` | Failed: zero tiles detected |

Each harness run also recorded successful startup, a 15-photo database and
rating persistence before the accessibility failure. These checks do not
measure the typography or prove that controls meet the sizing contract.

## Observations and measurements

| Subject | Target | Actual runtime evidence |
| --- | --- | --- |
| Normal application window | Representative wide layout | Requested 1440 × 900; screenshot `reports/gui-tests/ui-runtime-manual/standard-wide.png` |
| Narrow application window | Check reduced width | Requested 800 × 700; screenshot `reports/gui-tests/ui-runtime-manual/standard-narrow.png` |
| Header/InfoBar action | 34 × 34, icon 16px | Actual allocation, padding, border and rendered icon size not measured |
| Sidebar navigation icon | 18px | Actual rendered glyph size not measured |
| Sidebar header action | 28 × 28, icon 14px | Actual allocation and rendered glyph size not measured |
| Font roles | See `ui-sizing.md` | Effective family, size and weight not measured in the application |

The window dimensions above are requested X11 sizes, not measurements of
individual widgets. CSS minimums are not equivalent to allocations. GTK
Inspector's computed styles, selectors, provider precedence, icon bounds and
padding/border measurements were not collected; source declarations cannot
substitute for those measurements.

The standard sidebar is visible in the wide capture. Compact mode was started
from an isolated database setting and captured at
`reports/gui-tests/ui-runtime-compact/standard-wide.png`. Mode switching,
saved widths, resizing, minimum widths and hover auto-hide remain unverified.

Properties was opened by a click in the wide Standard run:
`reports/gui-tests/ui-runtime-manual/standard-properties-attempt.png`. The
preview, metadata, Tags and Notes are visible. At the requested 800 × 700 size
the sidebar is hidden and the Properties panel remains open; the lower part of
the panel is below the viewport. Scrolling, full control reachability, clipping
at other widths and enlarged text were not tested. The InfoBar is visible in
these captures, but its sizes and full action behavior were not measured.

Albums, Viewer and filter-panel screenshots, enlarged desktop text, normal vs
high display scaling, and the remaining theme/screen combinations were not
captured. No further product code changes were made in this verification pass.
No visually confirmed defect is claimed fixed by this pass.

## Automated checks and outstanding failures

- `cargo check --locked` and `git diff --check`: passed.
- `cargo fmt --all -- --check`: fails on pre-existing formatting differences
  across 17 Rust files; unrelated files were not reformatted.
- Theme discovery: 18 passed, 2 display-dependent tests ignored.
- The GTK Properties resize, sidebar progressive-count and compact InfoBar
  toolbar tests passed under Xvfb.
- Previously observed: the ignored Albums view test expects obsolete brown
  `#3a210f`, while the committed Bookshelf CSS computes `#fff8e7`; the ignored
  sidebar Ratings test expects a box where the committed widget tree starts
  with an overlay. Neither failure was changed or re-run in this pass.

Complete the missing Inspector measurements, screenshots, interaction and
scaling checks before accepting the overall cleanup.

## Follow-up: accessibility and folder checkpoints

The isolated Xvfb harness originally forced `GTK_A11Y=none` even though the
rating-filter steps use AT-SPI. Independently, D-Bus activation of the system
`org.a11y.Bus` executable failed with permission denied in this environment.
Starting the launcher directly inside the private session worked, but Fedora's
default accessibility broker then attempted to use an unavailable user
systemd. The harness now enables GTK AT-SPI, starts the launcher with its
`dbus-daemon` backend and starts the registry directly, terminating both with
the private session. Launcher and registry paths can be overridden with
`PIC_GUI_ATSPI_LAUNCHER` and `PIC_GUI_ATSPI_REGISTRY`.

The registry and the rating-filter toggle were discovered in a PIC-Default
run. Selecting a rating still timed out: the fixed screen coordinates missed
the popup, and an experimental AT-SPI query returned `(0, 0, 101, 24)` for
the visibly positioned three-star option. The experimental click was removed;
no complete GUI suite is claimed. The remaining rating interaction must be
made reliable before later viewer and filter checkpoints can be accepted.

The folder checkpoint had captured immediately after the click. It now waits
for the folder gallery-replacement trace and for 12 fixture tiles before
recording the screenshot. The readiness-based checkpoint recorded 12 tiles
in PIC-Default (`reports/gui-tests/20261108T120314Z-573478/screenshots/02b-folder-view.png`),
PIC-Dark (`reports/gui-tests/20261108T120518Z-574883/screenshots/02b-folder-view.png`)
and Terminal (`reports/gui-tests/20261108T120536Z-575332/screenshots/02b-folder-view.png`).
These results support a transition-timing explanation for the earlier zero
counts; their application logs also contain the folder-replacement trace.
They do not constitute exhaustive rendering verification. The gallery trace
wait was added after those three runs and must be validated in the current
script on each affected theme. In a subsequent Standard run the new trace
wait timed out: the coordinate click did not navigate to the folder. The old
pixel count could therefore report a false pass on the previous grid. The
Standard folder click and the later rating interaction remain unresolved.

### Sizing acceptance table

All measurements below refer to actual application widgets. A source CSS value
or a whole-window screenshot is not a measurement of a widget. No GTK
Inspector access or equivalent per-widget measurement was obtained in this
follow-up, so no sizing role is marked as passing.

| Role | Target | Measured result / theme | Status and evidence | Correction |
| --- | --- | --- | --- | --- |
| Application title | 15px / 700 | Not measured | Unverified | None |
| Main heading | 24px / 700 | Not measured | Unverified | None |
| Section heading | 14px / 600 | Not measured | Unverified | None |
| Album name | 15px / 600 | Not measured | Unverified | None |
| Navigation and controls | 13px / 400–500 | Not measured | Unverified | None |
| Properties values | 13px / 400 | Not measured | Unverified | None |
| Metadata labels | 12px / 500 | Not measured | Unverified | None |
| Secondary text and filenames | 12px / 400 | Not measured | Unverified | None |
| Counts and badges | 11px / 500–600 | Not measured | Unverified | None |
| Toolbar and InfoBar actions | 34 × 34, 16px icon | Not measured | Unverified | None |
| Quick filters | 32–34px, 16px icon | Not measured | Unverified | None |
| Sidebar navigation | 18px icon | Not measured | Unverified | None |
| Sidebar header action | 28 × 28, 14px icon | Not measured | Unverified | None |
| Inline action | 28–30px, 14px icon | Not measured | Unverified | None |
| Disclosure | 12px icon | Not measured | Unverified | None |
| Grid status badge | 18px icon | Not measured | Unverified | None |
| Standard menu icon | 16px or GTK native | Not measured | Unverified | None |

Provider/selector provenance, font sizes and weights, icon bounds, widget
allocations, padding and borders remain unverified. No UI corrections were
made during this follow-up. The broader mode-switching, saved-width,
auto-hide, scrolling, enlarged-text, Albums and Viewer checks above remain
outstanding. Release readiness for the sizing cleanup cannot be established.

## Final pass

The cleanup consolidates album title/name declarations, adjusts the
quick-filter add icon and toolbar radii, aligns Shared and Recent sidebar
icons, and documents sizing roles and provider priorities. The isolated
accessibility bus now waits for both the bus and registry before launching
PIC. The Xvfb startup suite passed (one checkpoint) in
`reports/gui-tests/20261108T121111Z-578205/`.

The longer Library suite remains unreliable: fixed-position folder navigation
can miss, and the rating popover cannot reliably be selected with the existing
coordinates. A keyboard-focus attempt failed with an AT-SPI error and was
removed. These are harness limitations, not evidence that PIC navigation is
broken; they leave the later GUI checkpoints unverified. The older Albums
color and sidebar Ratings test failures described above remain outstanding.

Final checks: `cargo check --locked`, `git diff --check`, Python compilation,
and shell syntax passed; Albums tests passed 41 with 11 ignored. The GTK
Properties resize, compact InfoBar and sidebar progressive-count tests and
theme discovery tests passed in the preceding runtime pass. The repository
format check continues to report pre-existing differences in 17 Rust files.
The theme screenshots, narrow window and compact sidebar observations above
are the extent of visual verification. No enlarged-text or actual widget
allocation measurements were obtained, and no visual defect was confirmed
fixed during the final pass.
