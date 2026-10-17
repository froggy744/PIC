# Metadata filters

## Using filters

The filter icon immediately after **Sort** opens a collapsible panel above the photo area. Closing it retains the active filters; the button shows an active indicator. Remove individual value/range chips, or use **Clear All** (which also clears the existing rating filter).

Categories combine with AND. Multiple values in a category combine with OR. Numeric and date bounds are inclusive; dates use `YYYY-MM-DD`, shutter ranges use seconds. **More** exposes manufacturer, exposure, white balance, dimensions, GPS, favourite, edited and EXIF-presence properties. Rating uses PIC's original rating state.

**Metadata Explorer** expands into four configurable columns. Choices and disjunctive counts come from catalog metadata in the current browsing scope. Lists display 100 choices at a time; **Show more** makes every remaining value accessible without creating thousands of GTK controls at once.

**Index missing metadata** scans previously imported originals on a worker, with progress and cancellation. Originals are never modified. Unavailable/changed files remain eligible for retry. Refresh a changed folder before retrying. Existing camera/lens metadata and file types are available immediately after migration, before expanded EXIF indexing.

Filters are session browsing state: panel close/reopen and layout changes preserve them; application startup and library switching reset them. PIC's global search and continuous Folder stream retain their existing semantics. Activating filters on the Library overview opens All Photos. Bookshelf keeps album cards and updates their matching counts/covers, including zero-result albums.

## Architecture and changed files

- `src/db.rs`, `src/db/{core,metadata}.rs`: additive `photo_metadata` table, indexes, original strings plus normalized keys, database-only seeding, guarded metadata updates.
- `src/db/{filters,photos,albums,history}.rs`: one parameterized SQL predicate shared by browsing queries and facets. Existing unfiltered administrative APIs remain available.
- `src/scanner.rs`, `src/db/network_import.rs`: existing EXIF/import pipeline supplies searchable fields, including TIFF/NEF and JPEG EXIF. No new dependencies.
- `src/metadata_index.rs`, `src/main.rs`: cancellable background indexing with source fingerprint checks and bounded batches; preserves edit recipes, history and thumbnail fingerprints.
- `src/window/{metadata_panel,toolbar,build,library,layout,photo_actions}.rs`, `src/window.rs`, `src/albums/card.rs`: panel, async facets, stale-response guards, original gallery refresh pipeline, collection/layout integration and Bookshelf covers.
- `src/css/base.css`, `resources/custom-icons/metadata-filter-symbolic.svg`, `resources/icons.gresource{,.xml}`: theme-derived styling and bundled symbolic icon.

Enable `PIC_METADATA_TRACE=1` for facet-query durations and indexing totals. Existing navigation traces continue to report gallery/model refresh timing.

## Validation (2026-10-17)

Executed locally on `feature/metadata-filters`:

- `cargo test -- --test-threads=1`: **693 passed, 0 failed, 148 ignored**. Display-dependent tests are intentionally ignored in this run.
- `cargo test metadata_ -- --test-threads=1 --nocapture`: **27 passed, 0 failed, 6 ignored**. Covers combined camera/lens/ISO filters, OR choices, empty/cleared sets, missing versus pending EXIF, normalization/originals, migrations, folders/albums/search/ratings, synthetic NEF/JPEG extraction, cancellation/unavailable sources and successful PNG backfill preserving edits/fingerprints.
- Isolated Xvfb native GTK test `metadata_filters_actual_window_folder_album_and_layouts`: **passed**. Checks toolbar adjacency, real camera/ISO controls, persisted membership, close/reopen, Clear All, album counts and the existing gallery refresh path across Grid, Photo Wall and Masonry.
- Isolated Xvfb native GTK test `metadata_panel_theme_layout_and_close_reopen`: **passed** across **26 themes**, at 900px and 420px widths. This was an earlier run; generated screenshots were inspected for Monochrome Studio and Terminal. Following the user’s updated instruction, the final test now checks only Standard GTK4 and the toolbar button is icon-only.
- Final Standard GTK4-only panel test: **passed** at both widths with the icon-only button; final live-window/layout test also **passed**. No further all-theme checks were run after the user narrowed validation.
- `cargo build`: passed.
- `cargo clippy --all-targets`: completed successfully, with existing project warnings; this is not a warning-free Clippy run.
- `rustfmt` applied to new Rust modules and added implementation blocks; `git diff --check` passed. Repository-wide `cargo fmt --check` reports pre-existing formatting differences, starting in unchanged `src/albums_view.rs`; unrelated files were not reformatted.
- Existing `scripts/test-gui.sh` library suite: startup/sample gallery, persisted ratings and folder membership passed. The run then stopped when AT-SPI bus activation was denied (`org.a11y.Bus: Permission denied`). **The full accessibility-driven suite did not pass.**

Reproduce the dedicated display tests separately (GTK tests must be serialized):

```sh
xvfb-run -a dbus-run-session -- env GTK_A11Y=none GSK_RENDERER=cairo cargo test metadata_filters_actual_window_folder_album_and_layouts -- --ignored --test-threads=1 --nocapture
xvfb-run -a dbus-run-session -- env GTK_A11Y=none GSK_RENDERER=cairo cargo test metadata_panel_theme_layout_and_close_reopen -- --ignored --test-threads=1 --nocapture
```

### Performance and practical limits

The in-memory catalog benchmark contained **100,004 photos**. A combined camera/ISO query materialized **25,001 matching records in 56 ms** on this machine. All source paths were offline; filtering did not open originals. This measures SQL/result construction, not cold-disk performance or end-to-end GUI latency.

Facet counts describe catalog matches before existing format visibility, RAW/JPEG pairing and Recently Added presentation limits; those existing presentation rules still govern displayed photos. The legacy Corrupt filter continues using its existing inspection path and is not incorporated into SQL facet counts.

Cancellation takes effect after the current source read; an already-blocked network read cannot be interrupted by the indexer. The existing bounded remote EXIF reader can leave fields unavailable when metadata lies outside its header budget. Proprietary MakerNote-only lens identifiers are not invented or decoded by a new dependency. RAW coverage here uses synthetic standards-based NEF/TIFF containers; no new real-camera RAW corpus or manual Wayland smoke run was available.

Smart Albums are intentionally outside this feature. No branch was merged into main or force-pushed.
