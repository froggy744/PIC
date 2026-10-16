# Lightbox offline preview implementation plan

> **For agentic workers:** Use superpowers:executing-plans for inline implementation and test each task before claiming completion.

**Goal:** Show a cached photo preview with the existing offline badge instead of an empty lightbox when the original is unavailable.

**Architecture:** Keep online viewer requests unchanged. A lightbox-owned presentation object supplies the badge, preview notice and availability-change tracking. Cache-only thumbnail reads run off the GTK thread using the gallery's existing presentation loader, which handles rotation, edits and legacy caches. All asynchronous results retain the lightbox generation guard. Reconnection reloads the current original.

**Tech stack:** Existing Rust/GTK4/libadwaita, thumbnail display loader and lightbox generation machinery; no new dependencies.

**Spec:** The approved in-chat plan: best cached preview, offline badge/tooltip and unavailable dialog, explicit missing-preview message, preserved navigation/metadata, lower-resolution notice, original reload on reconnection, no stale images during fast navigation.

## Tasks

- [x] Add GTK regressions for offline cache hit, absent/corrupt cache, online-to-offline navigation, badge action, stale result rejection, and source reconnection; run a failing case before product changes.
- [x] Add `src/lightbox/offline.rs` with badge/notice ownership, availability tracking and cache-only preview loading. Reuse existing thumbnail presentation and avoid original reads for known offline photos.
- [x] Wire open, arrow/wheel navigation, refresh and close to this presentation; preserve generation cancellation and navigation completion. Classify unexpected decode failures without labelling readable corrupt originals as offline.
- [x] Connect the badge to the existing unavailable dialog and Retry availability refresh; prevent badge clicks being treated as backdrop clicks.
- [x] Verify fit, rotation/edits, unavailable-to-available transitions, online loading, rapid navigation and close/reopen. Run relevant existing viewer checks, default suite/build, scoped formatting and whitespace checks.

## Constraints

- Offline cache loading must never generate thumbnails or fetch originals.
- A missing cache must never leave another photo's image visible.
- Cached thumbnails must not enter the full-quality viewer cache or count as native resolution.
- A cached preview is explicitly labelled; source dimensions remain unchanged.
- Source availability changes must only reload the currently displayed photo.

## Verification results

- Ten GTK regressions passed individually: cached preview, missing-cache navigation and badge action, quality-cache fallback and rotation/crop, stale completion, reconnect, explicit Retry, disconnect during decode versus readable corrupt originals, online/offline navigation, and both reconnect-publication races.
- Reconnect-publication regressions were observed failing before their fixes and passing afterward.
- Regular suite: 530 passed, 80 ignored, one excluded existing history date test (`history_group_labels_bucket_by_edit_age_not_import_date`) whose 400-day calculation expects a fixed August 2025 label.
- Existing native-centering GTK check could not meet its overflow precondition: the tiled desktop allocated 935×467, while the fixture requires less than 400×300. Its result is a failure, not a pass.
- New Rust files pass scoped formatting; `git diff --check` passes. `cargo build` passes.
