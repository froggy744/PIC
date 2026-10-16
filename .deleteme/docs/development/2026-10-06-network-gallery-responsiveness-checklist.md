# Network gallery responsiveness checklist

## Goal and evidence

Keep PIC responsive while browsing the 174,540-photo NFS library. Preserve
progressive, resumable imports and existing preview completion updates.
The latest `/tmp/pic-scan-trace.log` contains 817,860 Photo Wall row traces,
gallery replacement calls lasting 1,093 ms and 528 ms, and about 19,136
prefetch generation requests with 952 cache writes. These counts include
browsing activity and do not establish that all requests ran or failed.

## Approved approach

Use one layout summary per rebuild, with detailed rows behind a separate
trace flag. Trace replacement reasons and remove synchronous library-wide
object updates from the UI callback. Keep visible network preview generation
first, limit speculative generation around the viewport, and discard queued
remote work from old viewports. Leave in-flight reads to complete safely.

## Work

- [x] Inspect logs and identify layout logging and replacement hot paths.
- [x] Add compact layout trace summaries and opt-in detailed row traces.
- [x] Trace gallery replacement reasons and whether work is skipped or batched.
- [x] Make large photo comparisons/reorders cooperate with the GTK main loop.
- [x] Preserve grouping, selection, metadata, and navigation cancellation.
- [x] Limit network prefetch and discard stale queued remote generations.
- [x] Keep visible requests ahead of speculative work and preserve deduplication.
- [x] Add meaningful regression checks for cooperative replacement and queues.
- [x] Run relevant checks, production build, and independent review.
- [x] Recheck startup/scrolling with the live library and compare a fresh trace.

## Validation limits

Record automated results and measured model timings below. Live NAS scrolling
and end-to-end startup latency require a fresh application run; do not infer
those timings from the catalog query's `cold_start_ms` alone.

## Implementation and checks

- Layout tracing uses `PIC_WALL_LAYOUT` summaries. Set
  `PICASA_TRACE_WALL_ROWS=1` together with `PICASA_TRACE=1` to include every row.
- Large query comparisons and object reorders use 2,000-photo idle batches;
  unchanged results skip publication. Owned DB results avoid a full catalog
  clone in GTK, and stale navigation cancels comparisons and future batches.
- Group ranges append only the new suffix. Metadata changes to already emitted
  objects mark ranges dirty so subsequent batches regroup accurately.
- Hidden wall model changes keep a pending rebind; the mapped tick recycles the
  bounded live tile set before displaying the changed model.
- Only eight nearby direct NFS/SMB sources qualify for speculative generation.
  A viewport change discards queued stale remote jobs and releases only queued
  ownership. Reads already generating retain their ownership and complete.
- The standard suite passed: 572 tests, 0 failures, 112 ignored.
- GTK regressions passed for cooperative reorder/cancellation/object reuse,
  grouping after late metadata, hidden wall remapping, progressive Folder/Wall
  range consistency, and metadata arriving ahead of model batches.
- Independent review found the hidden rebind and late grouping risks; regression
  checks reproduced them and the fixes passed follow-up review.

## Final verification

- Eight separate GTK checks passed, including startup restoration/cancellation
  and an opt-in smoke check reading the real 4TBP catalog through a read-only
  SQLite connection. The standard suite's other ignored checks were not run.
- With 175,540 synthetic photos, owned result handoff took under 1 ms, reorder
  completed in 94 ms while yielding, and an unchanged result completed its
  cooperative comparison in 42 ms without publishing the store. The earlier
  scaled run's longest creation/reorder batch was 17 ms.
- The rebuilt app opened the real 174,540-photo catalog. Its query startup
  reported 602 ms, cooperative model build 742 ms, maximum model batch 38 ms,
  and no scan failures or read failures in `/tmp/pic-gallery-live.log`.
- Photo Wall traversed 0%, 25%, 75%, then 0% of the real catalog. Each step
  verified tile identity against the current model and a network prefetch queue
  at or below eight. Only 36–54 tiles were realized. This run used cached
  visible previews; it does not benchmark uncached NAS reads or sustained
  interactive scrolling. Its trace contains three layout summaries and zero
  per-row dumps (`/tmp/pic-gallery-live-wall.log`).
- `cargo build` succeeded; `git diff --check` is clean. The rebuilt PIC instance
  was left open for interactive inspection.
