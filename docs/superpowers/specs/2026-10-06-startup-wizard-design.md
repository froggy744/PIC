# Startup wizard design

## Intent and approved scope

Help a new PIC user add their first folder, see photos promptly, and discover browsing, organising and editing. The user approved the welcome → folder selection → progressive photos → optional tips proposal and requested a visible toggle to never show the wizard again. This document follows the user-revised implementation plan.

## Experience

Two main screens: Welcome → Adding photos → Library. Recovery, empty and error results reuse the import layout. No Next/Back/Finish steps or mandatory tips page.

Welcome shows the PIC logo and a compact arrangement of photo prints. Copy: “Welcome to PIC”; “All your photos. One place. On your computer.” Actions: a centred Choose Photos Folder button and a smaller Skip for now link. A quiet preference checkbox sits at the bottom.

Adding photos shows folder name and a smaller wrapped path, Finding photos…, live discovered counts and unknown-total progress without invented percentages. Open Library becomes available after a usable indexed photo, never discovery alone. Progressive gallery updates continue behind the dialog. Opening the library closes the dialog without stopping scanning; the existing notification area explains that photos are still being added.

Recovery: Continue adding photos, saved path, explicit Continue adding photos, Choose another folder and Open Library. Empty: No photos found, supported-photos explanation, Choose another folder and Open Library. Errors: concise inline warning, Try again when stopped, Choose another folder and Open Library. Open Library always permits exiting recovery/empty/error, including an empty library. Nonfatal failures never disable browsing usable photos. A matching active scan prevents duplicate retries. Cancelled pickers restore the previous screen/root. Closing/Escape never cancels scanning.

Optional library card (never a wizard page): Welcome to PIC; “Double-click a photo to open it.”; “Use the sidebar to browse folders and albums.”; “Right-click photos for more actions.”; Got it action. Browsing does not require dismissal. Preserve offline-preview help outside this compact card. Choosing another folder never silently removes an existing root.

## Never show again and persistence

A checkbox labelled **Don’t show this automatically again** is visible only on Welcome, initially off for a new library. Its saved state is shown when reopened manually. Changing it persists immediately; turning it on does not close the current wizard. It suppresses subsequent automatic wizard launches and automatic onboarding tips, including after Skip, closing the app or a power outage. Turning it off permits automatic display only if onboarding is incomplete and the library remains eligible. Completion still prevents automatic display. Manual **Help → Getting started** always works and does not clear preferences or restart imports.

Use the existing library database settings; preferences are per library, matching existing application settings. No schema migration or new dependencies. Keys:
- `onboarding-never-show`: `true` or `false`, default `false`.
- `onboarding-stage`: `welcome`, `importing`, `tips`, `complete`; absent means not started.
- `onboarding-root`: selected canonical source reference, absent before selection.
- `onboarding-tips-dismissed`: `true` or `false`, default `false`.

Skip/window dismissal without the checkbox leaves onboarding incomplete; an eligible empty library can show Welcome on a later launch. Tips dismissal saves only `onboarding-tips-dismissed` during an active import so recovery stage/root survive outages. After successful import with usable photos, advance to Tips when an eligible undismissed card remains, otherwise Complete. Cancelled/failed scans retain recovery state. Persistence failures keep the checkbox at its last saved value, display an error and leave the library usable. Unreadable settings/counts suppress automatic display rather than assume the library is fresh.

## Startup and recovery rules

A library with no onboarding stage, no photo records (including trashed photos), and no registered folder records is fresh. Existing libraries, including empty registered or disconnected folders, do not receive an unsolicited wizard. Incomplete recorded onboarding is eligible for recovery unless suppressed. A completed stage always suppresses automatic display.

After interrupted `importing`, use persisted root, current database contents and current scan state to reconstruct the page. Existing photos permit Open library. An idle interrupted import offers an explicit **Continue adding photos** action through the existing scan queue; never automatically enqueue a duplicate import merely because a wizard reopened. Do not claim an inactive scan is running. If the selected source is unavailable, show retry/choose-another actions. An interrupted tips stage resumes tips only when photos remain available in the library and onboarding is not suppressed; otherwise show the recovery page.

Only one wizard exists per application window. Present after the main window maps and callbacks are installed. Repeated activation focuses the existing wizard. Manual invocation during an unrelated scan presents introductory/help content without claiming ownership of that job.

## Architecture

A new focused `src/onboarding.rs` module owns the pure state model and persistence operations. `src/onboarding/view.rs` owns a scrollable libadwaita dialog and tip UI. `src/window/onboarding.rs` coordinates the view with existing import callbacks and accepted scan events. Keep integration changes to the large window builder small; do not restructure unrelated startup or scanner code.

Reuse the folder-registration callback, `PhotoScanRequestReason::ImportFolder`, existing scan queue, `ScanUiEvent.generation` rejection, and progressive `PhotosIndexed` handling. An onboarding import ticket includes the generation and selected root; update wizard progress only for its matching job, never background maintenance or a superseding job. No new scanning engine, original copying, or cache regeneration logic.

Add a compact Help menu with Getting started in the header; preserve the existing sidebar pin button and responsive toolbar. Tip cards are dismissed before opening lightbox/edit flows. All callbacks use weak UI ownership or explicit disconnection on close.

## Layout and accessibility

Use a compact scrollable dialog with approximately 600×480 logical pixels for Welcome and 600×400 for a directly presented import screen as its preferred size, clamped to available parent space. No new main-window minimum width. At 1920×1080 and 1366×768, and a 360-pixel-wide app window, text wraps and all primary/skip/toggle controls remain reachable. Respect theme, keyboard traversal, Escape/close semantics and accessible labels. Bundle the Welcome photo arrangement and reuse the existing PIC logo. No new dependencies.

## Success criteria

A fresh user adds a folder and reaches real photos without waiting for all thumbnails. Existing users retain normal startup. Never-show survives restart and suppresses only automatic onboarding. Cancellation, missing sources, partial scans and interrupted setup remain recoverable without duplicate jobs. Narrow-window resizing and automatic sidebar hiding remain functional.
