# Progressive network import: proposal and checklist

Date: 2026-10-06
Branch: `feature/progressive-network-import`
Status: implemented on the feature branch; automated checks pass. Live NAS validation remains open.

## Intended experience

The user adds a network folder once and leaves PIC to work, as in Picasa.
Photos appear as scanning proceeds. Browsing, opening photos, and editing
remain usable throughout a large import, including a share containing
175,540 photos. Originals stay on the share; adding means cataloging them.

The user suggested splitting discovery and adding across workers. The
recommendation is separate background threads, scheduled normally by the OS,
rather than dedicating particular CPU cores or introducing a helper process.
Discovery largely waits for network I/O; decoding uses CPU. More cores alone
do not remove the current requirement to finish discovery before adding.

## Findings in the current code

- `src/scanner.rs::scan_with_control` calls `collect_files` for the full
  recursive tree before preparing or committing the first photo batch.
- `collect_files` retains a vector of every file and its GIO objects. For
  direct SMB/NFS it also calls `network_shares::info` for each supported file.
- `native/private_nfs.c::pic_nfs_list` opens and destroys a session for each
  directory. It passes only name and kind to Rust. The installed libnfs
  `nfsdirent` also exposes size and modification time from READDIRPLUS.
- NFS stat already reuses a thread-local session. The problem is repeated
  requests, not a fresh connection for every stat.
- `network_exif` requests up to 512 KiB per photo. For 175,540 photos this
  could request roughly 86 GiB of headers. This is an upper-bound illustration,
  not a measurement of the reported import; shorter files return fewer bytes.
- Thumbnail generation starts after indexing. The existing thumbnail system
  already prioritizes visible tiles and makes bulk work yield to them.
- Database writes are batched and source reads happen outside transactions,
  which should be preserved. `commit_prepared` currently calculates full
  sidebar counts for every changed batch; this needs measurement at scale.
- `db::upsert_photo` replaces metadata fields on conflict and uses zero
  sentinels for examined-but-absent metadata. It cannot safely serve as a
  placeholder insert/update for the proposed first pass.
- The UI and onboarding currently consume sequential scan phases. Concurrent
  discovery, cataloging, and enrichment need independent counters and states.
- Existing scans deliberately start from authorized actions. Resuming a saved
  import must not turn sidebar selection or startup into a general rescan.

These observations establish a code-level bottleneck. They do not establish
the exact duration or current state of the user's live NFS import.

## Approaches considered

| Approach | Benefit | Limitation |
| --- | --- | --- |
| Discovery worker plus catalog writer, followed by asynchronous enrichment | Photos become available promptly; slow originals do not block catalog writes; controlled memory | Requires durable job state and changes to progress events |
| Stream one directory at a time through the current metadata-first indexer | Smaller initial refactor; existing metadata behavior largely preserved | A slow image still delays cataloging; directory size affects time to first result |
| Separate importer process or fixed CPU-core assignments | Process isolation; possible independent resource controls | Adds IPC, lifecycle and packaging work; does not itself fix network requests or the full-tree barrier |

Recommendation: the first approach. Start with in-process workers. Introduce
process isolation only if measurements later show a need for it.

## Proposed pipeline

### 1. Discovery worker

Walk the explicitly added root and send directory/file records to a bounded
channel. Records include stable URI, owning folder, and available source
fingerprint (size and modification time). Preserve current supported-format
and cache-directory exclusions and URI normalization.

Reuse one NFS discovery session per worker/export, with cleanup and
invalidation on failure. Carry directory-entry attributes through the C/Rust
boundary instead of issuing redundant stats. If attributes are unavailable,
record that explicitly and obtain them in bounded fallback work. Missing data
must never masquerade as a zero-sized, unchanged file. SMB uses the same
discovery interface; optimize its listing attributes only where the supported
Samba API provides reliable data. Do not assume NFS and SMB APIs are equivalent.

Avoid building a vector of the entire share. Stream or page directory entries
where the transport permits it. libnfs may internally materialize a directory,
so distinguish library buffering from PIC's queue and measure very large
single-directory cases. No global sorting is required for import discovery.

### 2. Catalog writer

One background writer owns import-related SQLite writes. It consumes discovery
records and enrichment results fairly, prioritizing newly discovered files.
Other existing UI writes remain supported through short SQLite transactions.
No network access or image decoding occurs inside a write transaction.

Commit on a size OR time limit: start with at most 128 records or 200 ms,
whichever comes first, and tune from measurements. Flush a partial batch even
when discovery stalls on a slow directory. Use a bounded discovery queue,
initially 1,024 records, with cancellation-aware backpressure.

For a new photo, insert identity, folder, fingerprint when known, and an
mtime-based date fallback. Mark metadata as pending. For an existing photo,
retain ID, edits, rating, favorites, albums, and useful metadata. Discovery must
not overwrite existing metadata with empty placeholder values. A changed
fingerprint invalidates pending/completed enrichment for that source version.

Publish photo additions after commit, without waiting for the tree to finish.
Publish metadata updates separately so they do not count as additional photos.
Update RAW/JPEG pairing from cataloged directory records without rescanning
the network. Query existing fingerprints in batches rather than loading the
entire library into a new HashMap for every root.

### 3. Background enrichment

Use a small bounded scheduler for EXIF, geometry, and thumbnail work. Begin
with one bulk enrichment worker for a network share and retain the existing
foreground thumbnail workers. This is a starting limit, not a speed claim.
Foreground requests take precedence; bulk work must not hold a shared network
read lock across an unbounded sequence of reads. Measure lock wait times and
session contention before increasing concurrency or adding session pools.

Reuse the existing visible-thumbnail queue rather than adding a competing
queue. Deduplicate jobs by source URI and fingerprint, including foreground
requests. Where practical, share fetched headers/embedded previews between
metadata and thumbnail work. Prefer bounded reads; expensive full-original
fallbacks must not gate discovery or cataloging.

Return metadata results to the catalog writer. Apply a result only if its
job generation and source fingerprint still match. A late result must not
recreate a deleted row or update a newer version of a photo. Pending metadata
and examined-but-absent metadata are distinct persistent states; failed reads
are not recorded as successfully examined. Unsupported formats and failures
must settle into explicit states rather than retrying forever.

## Job lifecycle and recovery

- Persist the root, job identity/generation, directory work states, completion
  status, and pending enrichment work. Store seen paths/folders in SQLite or
  equivalent durable scan tables instead of an unbounded in-memory manifest.
- Catalog rows and their seen markers must commit together. Persist a directory
  completion checkpoint only after all its emitted records are acknowledged
  as committed; a crash cannot falsely mark uncommitted files as completed.
- A bounded in-memory queue is an accelerator, not the only record of work.
  Restart an interrupted directory from its beginning if no reliable listing
  cursor is available. Stable paths and idempotent writes prevent duplicates.
- Pause/Stop ends scheduling and cancels queued work promptly; already committed
  rows stay visible. Running network calls require finite timeouts. Do not
  promise immediate interruption of a blocking call before verifying the API.
- Closing the app preserves work. The user selected automatic continuation of
  a previously authorized unfinished import when PIC reopens. An explicit
  Pause/Stop remains paused across restart until the user resumes it. Completed
  roots do not receive a fresh recursive scan merely because PIC starts.
- A lost share pauses that job with an actionable status. It must not repeatedly
  monopolize the worker or prevent unrelated roots from proceeding.
- An explicit Refresh starts a fresh traversal/generation. Recovered discovery
  checkpoints from an older interrupted traversal are insufficient evidence
  for deleting files; a complete uninterrupted verification traversal is needed
  before removal reconciliation after such a recovery.
- Reconcile deletions only after successful complete discovery and availability
  verification. Cancellation, unreadable directories, or disconnection preserve
  existing catalog rows. Retain current mount/device protections for local roots.
- Queue additional explicitly added roots rather than canceling the import already
  in progress. Preserve generation isolation when roots or libraries change.

## Progress and gallery behavior

Show combined progress, for example: "Found 12,430 · Added 12,288 · Previews
ready 840". These counters can advance concurrently. Until discovery finishes,
do not show a percentage based on an unknown total. Count unique additions
separately from unchanged rows, refreshed metadata, failures, and retries.

Keep new photos and folders appearing during the scan. Pending geometry uses
the gallery's fallback aspect ratio; later geometry updates refresh affected
objects and tiles without rebuilding the entire gallery each batch. Coalesce
UI work, sidebar counts, and sorting updates so enrichment does not repeatedly
move the viewport or steal selection. Date fallback and final EXIF dates must
have a defined update policy that preserves browsing position.

Track "catalog complete" separately from "background previews/metadata still
pending". Update onboarding and terminal-event handling so discovery finishing
does not falsely declare enrichment complete or reset concurrent counters.

## Checklist for proceeding

### Design decisions and baseline

- [x] Create the branch.
- [x] Trace the full-tree discovery barrier, redundant NFS stats, and UI events.
- [x] Compare worker-based, directory-based, and process-based approaches.
- [x] Draft the recommended design and this checklist.
- [x] Resolve restart behavior: automatically continue unfinished imports;
  preserve an explicit Pause/Stop until the user resumes.
- [x] Review the proposal, especially placeholder rows and background enrichment.
- [x] Finalize the approved design spec and create the implementation plan.
- [x] Run the existing scanner, database, thumbnail, and onboarding checks before
  implementation; record any baseline failures.
- [ ] Measure time to first committed photo and visible preview, discovery/catalog
  rates, network requests/bytes, queue sizes, SQLite lock time, and UI event delay
  on a representative share. Avoid launching a second scan over the live import.

### Implement in reviewable stages

- [x] Add a discovery record API and tests that emit the first records before
  finishing traversal. Cover cancellation and slow/unreadable directories.
- [x] Carry NFS listing attributes through FFI; reuse discovery sessions and test
  missing attributes, errors, Unicode/escaped paths, and export changes.
- [x] Add durable job/seen/pending state with idempotent schema migration and
  explicit metadata readiness; preserve existing photo and folder ownership.
- [x] Implement the catalog writer and bounded channels with size/time flushing,
  cancellation-aware sends, fair result consumption, and short transactions.
- [x] Emit additions during discovery. Test that a deliberately blocked later
  directory does not prevent earlier photos becoming queryable and visible.
- [ ] Connect bounded enrichment to existing thumbnail priorities. Test dedup,
  source-version validation, failure states, and foreground responsiveness.
- [x] Replace full-tree vectors, global fingerprint snapshots, and per-batch
  full-library counts where measurements show they undermine scaling.
- [x] Add safe reconciliation, interruption/restart recovery, queued roots,
  library-switch isolation, and the agreed Resume policy.
- [x] Adapt gallery, sidebar, progress and onboarding to concurrent phases and
  partial geometry. Test filters, counts, sorting, selection and scroll stability.

### Validate the experience

- [x] Verify committed photos appear before full discovery completes.
- [ ] Verify a cached visible photo and normal editing remain responsive during
  discovery and bulk enrichment; measure uncached foreground preview latency.
- [x] Verify unchanged Refresh avoids image reads and duplicate rows.
- [x] Verify crash/restart, Stop/Resume and NAS disconnection retain committed
  photos, never delete unseen records, and never apply stale results.
- [ ] Test nested folders, one very large directory, JPEG/RAW pairs, missing EXIF,
  corrupt files, unsupported remote previews and partial directory failures.
- [x] Run relevant scanner, DB, thumbnail, onboarding, and UI event tests plus
  Rust build checks. Verify local-folder and SD-import behavior remains sound.
- [ ] Test NFS and SMB with representative large shares, including the reported
  175,540-photo workload when available. Compare baseline measurements and
  confirm PIC queue sizes stay within configured limits.
- [ ] Review the completed change and merge only after validation.

## Initial scope

First deliver the progressive experience for explicitly added SMB/NFS folders.
Reuse common scanner components where appropriate, but retain existing local
and SD-card behavior until verified. Exclude privileged helpers, CPU affinity,
NAS-side indexing services, and unrestricted parallel full-image decoding.
The numeric batch, queue and worker values above are proposed initial settings
to validate, not fixed performance guarantees.


## Implementation results

- Branch: `feature/progressive-network-import`; no merge or deployment.
- Separate discovery and enrichment workers feed a single catalog writer.
  The discovery queue is bounded at 1,024 records, with commits after 128
  records or 200 ms. Photos and subfolders are published after each commit. Network sidebar
  additions update its cached tree and coalesce visible rebuilding.
- NFS listing size/mtime avoids a stat for each photo; worker-owned reusable
  NFS/SMB discovery contexts are separate from viewer access.
- Durable directory checkpoints and pending metadata/preview work continue on
  reopening PIC. Explicit Stop stays paused; adding the folder again resumes it.
- Share loss reports that the job is waiting and suspends it without draining failed reads across the entire
  catalog. Recovered imports preserve unseen catalog records; only a fresh,
  complete and available traversal reconciles deletions.
- Existing edits and metadata survive discovery; version-checked enrichment
  updates existing rows. Gallery metadata lookup is indexed and updates arriving
  before a progressive gallery batch are retained for that batch.
- Baseline: 540 passed, 105 ignored. Final automated suite: 562 passed,
  0 failed, 106 ignored. Regression tests cover blocked discovery, timed commits,
  checkpoint recovery, stale results, cancellation with a full queue, share loss,
  Stop during an unavailable probe, retried failed work and early subfolder events.
- Independent review identified five material issues; fixes and regression
  coverage are included. No live NAS throughput claim is made.

Remaining manual checks: representative NFS/SMB imports (including the reported
175,540 photos), time to first visible preview, foreground browsing/editing
latency, bytes transferred and native cancellation latency. A GTK regression
for metadata arriving before later gallery batches is present but requires a
display and was not executed in this environment. Other pre-existing ignored
GTK/network/hardware tests retain their requirements. CPU affinity and a helper
process are unnecessary for this implementation; worker scheduling stays with
the operating system.


## Stop responsiveness follow-up

Stop now signals cancellation before saving paused state. Worker cancellation
is linked to the user control, so discovery and enrichment see it immediately
without waiting for the catalog writer's final commit. Cancelled reads do not
start another availability probe or thumbnail. Expected worker-channel closure
on Stop follows the normal cancellation/checkpoint path.

The import's NFS read scope checks cancellation while waiting for the shared
read lock and between 64 KiB read chunks. Cancellation discards partial image
bytes, closes opened handles with a short cleanup timeout and invalidates the
read context when needed. Viewer reads on other threads retain their own
behavior. Queued progress events cannot overwrite “Stopping scan…”.

Validation: 565 automated tests pass, 106 remain ignored. A native contention
probe cancelled a worker in 10 ms while the read lock remained held. This
measures lock-wait cancellation, not a live NAS RPC. An already-issued native
network call or decoder operation must still return before the worker exits;
cleanup timeout is not an exact wall-clock deadline because libnfs polls.

## Startup responsiveness follow-up

The 2026-10-06 trace showed 174,540 catalog photos loaded, then automatic
thumbnail recovery probed direct NFS/SMB files one by one before preparing
recovery work. Startup recovery now skips direct network files; visible preview
requests and resumable imports supply network work. Recovery checks cancellation
between local-file probes.

Startup gallery creation now uses the existing 2,000-photo progressive replace
instead of repeatedly appending 500 photos and rebuilding cached grouping for
the entire prefix. It restores the saved view after the progressive model is
ready, and stops adding batches when navigation changes the active generation.
Photo Wall aspect ratios read existing backing cells rather than repeated
GObject property lookups; the ratios and rotation behavior are unchanged.

The trace captured the GTK thread inside `photo_wall_aspect_ratio()` while it
read properties for catalog-wide Photo Wall geometry. It also showed recovery
issuing a stream of cached NFS export lookups. The 684 ms cold-start line only
measures catalog query startup and excludes subsequent GTK model construction.
A production `cargo build` passes after these changes. The two interactive GTK
regression tests require display-driven execution; the large live NFS catalog
was not benchmarked again after the changes.
