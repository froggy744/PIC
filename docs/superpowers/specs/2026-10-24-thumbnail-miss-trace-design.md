# Thumbnail Miss Lifecycle Trace

## Intent

Diagnose blank NFS thumbnails by following selected photos from display queue submission through thumbnail generation, completion delivery, tile refresh, and editor open/save/close. The trace is opt-in and filtered by a URI substring so normal tracing remains quiet. When tracing demonstrates a repeatable lifecycle bug, fix that bug and add a focused regression test.

## Scope and constraints

- Add an environment-controlled per-URI trace for editor transitions and the thumbnail presentation/generation lifecycle.
- Include events for queue admission or rejection, generation start/outcome, completion delivery, and tile refresh/application.
- Keep existing trace switches and unrelated rendering behavior unchanged.
- Add focused tests for URI filter matching and any trace-proven thumbnail lifecycle fix.
- Wall-quality generation follows separate queues and is outside this ordinary grid-thumbnail trace.

## Trace finding and fix

The “thumbnails” album trace showed that a missing tile was marked settled, then excluded from visible-grid candidate scans. Its queued remote generation was discarded as the viewport list changed, and later retries were suppressed as stale network prefetch. Some tiles recovered only when background prefetch reached them.

Both visible-grid scans now keep missing, non-corrupt placeholders in the current visible request set. This keeps the generation wanted-set aligned with the displayed viewport until the thumbnail is generated and applied.

## Approaches considered

- Add a URI-filtered lifecycle trace and filter test. Chosen because it connects the specific photo across worker and GTK boundaries without flooding logs.
- Add a test-only retry regression. This can test state transitions but cannot reveal which live stage stopped progressing.
- Add unfiltered lifecycle logging. Rejected because a large NFS library produces too much output to inspect usefully.

## Implementation calls

Use a new `PIC_THUMBNAIL_TRACE_URI` substring filter, independent of `PICASA_TRACE`. Trace events include the source URI and a concise stage/outcome. Keep the helper pure so its matching behavior can be tested without GTK or network access.

## Verification

Run the focused URI-filter and missing-placeholder candidate tests, then `cargo check --locked`. Verify the album trace reaches `cache_generation ready` and `paintable_applied` for a previously missing visible tile.
