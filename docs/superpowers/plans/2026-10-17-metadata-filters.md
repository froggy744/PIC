# Metadata filters implementation plan

Goal: implement the user's Metadata Smart Filters specification on feature/metadata-filters.

Architecture: extend existing SQLite photo queries, existing EXIF extraction and asynchronous gallery refresh. A one-to-one photo_metadata table stores additional originals, normalized search keys and indexing status without changing edit recipes or thumbnail fingerprints. The toolbar owns a native collapsible panel and session filter state. SQL supplies choices/counts; gallery model replacement remains the existing generation-checked path.

Constraints: no Smart Albums, sidebar redesign, hardcoded colors or new dependencies. AND between categories, OR between choices; inclusive numeric/date ranges. Search retains existing library-wide scope. Folder mode retains its continuous stream. Unknown and not-yet-indexed metadata stay distinguishable. Closing the panel preserves filters.

- [ ] Persistence: additive schema, indexes, normalization, import/remote extraction integration and migration tests.
- [ ] Queries: shared typed filter/predicate, scope-aware facets, existing photo/album query integration, combined/search/rating/empty-result tests and large-catalog timing.
- [ ] UI: Filter beside Sort, responsive revealer, chips, Clear All, native category choices/ranges and expandable configurable four-column explorer.
- [ ] Backfill: cancellable worker, bounded batches, stale source checks, progress, retryable unavailable sources, no thumbnail/edit mutations.
- [ ] Integration: refresh all photo layouts, album bookshelf counts, panel state and library transitions, conditional PIC_METADATA_TRACE.
- [ ] Validation: formatting, compilation, Clippy, database/extraction tests, GTK panel lifecycle and existing GUI harness checks. Record exact results and limitations.

Review focus: malformed/nonfinite EXIF; Nikon make/model duplication; offline or changed originals while indexing; stale worker responses on collection/library changes; filters remaining active with a closed panel and empty results.

Audit: db/photos.rs owns photos/photos_limited/photos_in_album; window/library.rs runs queries on a worker and replaces the gallery with generation checks; window/toolbar.rs owns Sort/rating; scanner.rs uses kamadak-exif and rawler geometry fallback; scanner/progressive.rs handles remote imports; SQLite open creates additive schema before column migrations. Native GTK tests use serialized test execution and isolated GUI scripts create temporary databases.
