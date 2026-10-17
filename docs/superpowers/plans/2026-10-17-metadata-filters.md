# Metadata filters implementation plan

Goal: implement the user's Metadata Smart Filters specification on feature/metadata-filters.

Architecture: extend existing SQLite photo queries, existing EXIF extraction and asynchronous gallery refresh. A one-to-one photo_metadata table stores additional originals, normalized search keys and indexing status without changing edit recipes or thumbnail fingerprints. The toolbar owns a native collapsible panel and session filter state. SQL supplies choices/counts; gallery model replacement remains the existing generation-checked path.

Constraints: no Smart Albums, sidebar redesign, hardcoded colors or new dependencies. AND between categories, OR between choices; inclusive numeric/date ranges. Search retains existing library-wide scope. Folder mode retains its continuous stream. Unknown and not-yet-indexed metadata stay distinguishable. Closing the panel preserves filters.

- [x] Persistence: additive schema, indexes, normalization, import/remote extraction integration and migration tests.
- [x] Queries: shared typed filter/predicate, scope-aware facets, existing photo/album query integration, combined/search/rating/empty-result tests and large-catalog timing.
- [x] UI: Filter beside Sort, responsive revealer, chips, Clear All, native category choices/ranges and expandable configurable four-column explorer.
- [x] Backfill: cancellable worker, bounded batches, stale source checks, progress, retryable unavailable sources, no thumbnail/edit mutations.
- [x] Integration: refresh all photo layouts, album bookshelf counts, panel state and library transitions, conditional PIC_METADATA_TRACE.
- [x] Validation: formatting, compilation, Clippy, database/extraction tests, GTK panel lifecycle and existing GUI harness checks. Record exact results and limitations.

Review focus: malformed/nonfinite EXIF; Nikon make/model duplication; offline or changed originals while indexing; stale worker responses on collection/library changes; filters remaining active with a closed panel and empty results.

Audit: db/photos.rs owns photos/photos_limited/photos_in_album; window/library.rs runs queries on a worker and replaces the gallery with generation checks; window/toolbar.rs owns Sort/rating; scanner.rs uses kamadak-exif and rawler geometry fallback; scanner/progressive.rs handles remote imports; SQLite open creates additive schema before column migrations. Native GTK tests use serialized test execution and isolated GUI scripts create temporary databases.

## Completed integration audit

- `window/toolbar.rs`: Sort and legacy rating popover. Preserve `RatingFilter` and `photo-rating-filter`; the new panel references that existing rating selection rather than creating a second rating state.
- `window/library.rs`: existing worker, generation guards, format/pair filtering, recently-added limit, ratings, sorting and model replacement. Extend its SQL query inputs; retain the rest of this pipeline.
- `window/layout.rs`: search is global; ordinary folder clicks reuse a cached continuous stream. Filter mutations invalidate membership through the existing refresh path; navigation keeps reuse.
- `window/build.rs`: startup, next/previous collection, edits/ratings/favorites, library switching and progressive import are additional refresh paths. Prevent direct import appends from admitting nonmatching photos; refresh membership after relevant edits/backfill.
- `db/photos.rs`: shared photo and album SQL is also used outside browsing. Keep unfiltered APIs for folder actions, thumbnail maintenance and screensaver source selection. Add filter arguments to the same underlying query implementations, with explicit browsing wrappers.
- `db/albums.rs`, `albums/card.rs`: bookshelf is an album-card presentation, not another photo grid. Filter-aware browsing counts/covers must use the same SQL predicate; retain all albums with matching counts, including zero.
- `db/library_home.rs`: bounded home preview queries are distinct from All Photos. Keep home overview semantics; activating metadata filters from Library opens All Photos through existing navigation.
- `scanner.rs`, `scanner/progressive.rs`, `db/network_import.rs`: reuse kamadak-exif, rawler dimension fallback and bounded remote header reads. Additional fields remain nullable; an explicit indexing version distinguishes pending from examined/missing.
- `scripts/README-gui-tests.md`: isolated Xvfb GUI harness and native Wayland harness. Reuse existing fixtures and accessibility conventions. Unit tests use in-memory catalogs; GTK tests must run serially.

Implementation decision: explicit filtered variants of existing SQL queries, not implicit global filtering of every database caller and not a separate in-memory filtering model. Panel close affects visibility only. Queries/choices run on workers, stale results are discarded, and original files are read only by imports or explicit indexing.

## Implementation and validation report

See [Metadata filters](../../metadata-filters.md) for usage, changed files, exact executed checks, performance measurements and remaining validation limits. The existing accessibility GUI suite was partially blocked by AT-SPI activation; it is not reported as passing.
