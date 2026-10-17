# V1 documentation media checklist

Scope: documentation and media only. Application source, runtime assets and packaging metadata are excluded.

- [x] Inventory the supplied V1 captures and walkthrough.
- [x] Convert all supplied PNG screenshots to quality-92 JPG, preserving dimensions.
- [x] Move earlier screenshots and the old editor video to `screenshots/rc`.
- [x] Repair historical RC release links to use archived captures.
- [x] Replace screenshot placeholders in the main README and documentation index.
- [x] Illustrate getting started, navigation, folders, library, editing and cropping guides.
- [x] Add current metadata-filter and appearance-theme screenshots.
- [x] Refresh the creator kit and V1.0.0 release draft with supplied media.
- [x] Publish a local media index covering every supplied screenshot.
- [x] Finish normal-speed and 2× walkthrough MP4 encoding at source 1920×1080.
- [x] Check local media links, output formats, dimensions and video durations.
- [x] Confirm the final change set contains documentation and media only.

No replacement screenshot was supplied specifically for the screensaver, camera import dialog or standalone photo viewer; those workflows remain described in text. The full media gallery includes all supplied captures without presenting unrelated pictures as those workflows.

AppStream URLs are recorded in the Linux guide as a future packaging task; packaging resources are outside this documentation-only change.

Validation: 38 JPG screenshots, zero remaining PNGs, and 130 local documentation media links resolving successfully. MP4 probes confirm H.264/AAC at 1920×1080, with durations 128.93 seconds and 64.53 seconds. `git diff --check` passed. Changed tracked paths are limited to Markdown documentation and screenshot media; no application tests or builds were needed.

An additional PNG with no filename extension (`albums bookshelf theme`) was identified during commit review, converted to JPG and added to the library guide, release draft and gallery.
