//! Scan view: what the window shows while a scan runs (candidate 19).
//!
//! The scanner sends [`ScanEvent`]s; the window drains them on a 50 ms timer.
//! [`ScanView`] is the reducer behind that timer: per event it updates the
//! per-generation state (progress counters, the queue of new photos waiting
//! to be appended, pending sidebar counts, the dirty-thumbnail queue) and the
//! job's bookkeeping, and returns the [`ScanEffect`]s to apply in order. It
//! decides generation isolation, which events pass after Stop, which new
//! photo joins the view, the rebuild rules at the end of indexing and of a
//! job, the completion messages, and the per-tick batches and throttles.
//!
//! It sees the view only through [`ScanViewFacts`], asked lazily so the same
//! queries run at the same moments as before the reducer existed. The GTK
//! adapter is `window::scan_view_widgets`.

use std::collections::{HashSet, VecDeque};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use super::{format_count, scan_may_append_new_photos, ScanJobKind, ScanJobState};
use crate::db;
use crate::image_format::RawJpegPairMode;
use crate::scanner::ScanEvent;
use crate::sidebar::SidebarFilter;

/// New photos appended to the gallery per tick.
pub(super) const PHOTO_APPEND_BATCH: usize = 192;
/// Thumbnail tiles refreshed per tick.
pub(super) const THUMBNAIL_REFRESH_BATCH: usize = 128;
/// Indexed photos after which pending sidebar counts are shown at once.
const SIDEBAR_COUNT_PHOTOS: usize = 50;
/// Otherwise pending sidebar counts wait at most this long.
const SIDEBAR_COUNT_INTERVAL: Duration = Duration::from_millis(250);
/// Progress text ("Indexed n photos", "Creating thumbnails n / m") at most this often.
const PROGRESS_INTERVAL: Duration = Duration::from_millis(150);
/// How long a finished or stopped job's message stays.
const MESSAGE_HOLD: Duration = Duration::from_millis(2500);

/// What the reducer needs to know about the window, answered by the adapter
/// when asked (a database read for some).
pub(super) trait ScanViewFacts {
    /// The sidebar destination shown now.
    fn filter(&self) -> SidebarFilter;
    /// Whether a search is active.
    fn search_active(&self) -> bool;
    /// The browse scope shown now (asked once per indexed batch, and at the
    /// end of a refresh job in a Folder view).
    fn scope(&self) -> db::BrowseScope;
    /// Whether the photo's format is shown (asked per newly found photo that
    /// may join the view).
    fn format_enabled(&self, path: &str) -> bool;
    /// Whether a folder is still catalogued (asked after folders are removed).
    fn folder_exists(&self, id: i64) -> bool;
    /// Whether the thumbnail pass deferred offline sources.
    fn recovery_deferred(&self) -> bool;
    /// The RAW+JPEG pair mode (asked once per indexed batch that may append).
    fn pair_mode(&self) -> RawJpegPairMode;
    /// Whether a new RAW or JPEG photo's twin (same folder and stem, the other
    /// kind) is already catalogued (asked only outside Both mode).
    fn has_pair_twin(&self, photo: &db::Photo) -> bool;
}

/// One step for the adapter, applied in the order given.
#[derive(Debug, Clone)]
pub(super) enum ScanEffect {
    /// Show the operation progress, unless the user dismissed it.
    ShowProgress,
    /// Set the status label.
    Status(String),
    /// Show (`true`) or hide the status box.
    StatusVisible(bool),
    Spinning(bool),
    /// Show or hide the Stop button.
    StopButton(bool),
    /// Hide the operation progress after a delay.
    HideProgressAfter(Duration),
    /// An import found a folder: add its sidebar row.
    AppendSidebarFolder(db::Folder),
    /// Photos already shown were re-read or updated (`PhotoChange::Indexed`).
    PhotosChanged(Vec<db::Photo>),
    /// Photos left the catalogue (`PhotoChange::Removed`).
    PhotosRemoved(Vec<i64>),
    /// Smart Album counts may have changed.
    RecountSmartAlbums,
    /// Refresh folder availability (the owner of folder-row counts).
    RefreshAvailability,
    /// The open folder is gone: show All Photos (filter and section layout).
    ShowAllPhotos,
    /// Rebuild the gallery from the database.
    RefreshGrid,
    /// Rebuild the Folder stream in place, keeping the visible position.
    RebuildFolderStream,
    /// Record whether the thumbnail pass deferred offline sources.
    RecoveryDeferred(bool),
    /// Start the next queued folder of the job.
    StartNextScan,
    /// Refresh the library availability statistics.
    RefreshAvailabilityStats,
    /// Redraw the folder rows from the catalogue.
    RedrawFolderRows,
    /// Give newly catalogued subfolders watches.
    RebuildFolderWatches,
    /// A diagnostic line; the adapter prints it with its exact text.
    Trace(ScanTrace),
}

/// The numbers behind each diagnostic line (texts live in the adapter).
#[derive(Debug, Clone)]
pub(super) enum ScanTrace {
    /// `PIC_WATCH grid indexed` (only with `PIC_WATCH_TRACE`).
    WatchIndexed {
        kind: Option<ScanJobKind>,
        filter: SidebarFilter,
        photos: usize,
        new: usize,
        queued_for_append: usize,
        search_active: bool,
        filters_active: bool,
    },
    /// `PIC_SCAN_UI indexing_finished` (only with `PICASA_TRACE`).
    IndexingFinished {
        imported: usize,
        filter: SidebarFilter,
    },
    /// `PIC_SCAN_UI thumbnails_started` (only with `PICASA_TRACE`).
    ThumbnailsStarted { total: usize },
    /// The thumbnail trace's `thumbnail_created_event_received`.
    ThumbnailReceived(PathBuf),
    /// `SCAN FAILED` (always).
    Failed { path: PathBuf, error: String },
    /// `SCAN COMPLETE` (always).
    Complete { imported: usize, failed: usize },
    /// `PIC_WATCH grid finished` (only with `PIC_WATCH_TRACE`).
    WatchFinished {
        kind: Option<ScanJobKind>,
        imported: usize,
        filter: SidebarFilter,
        refresh_grid: bool,
        folder_stream_rebuild: bool,
        folder_rows_redraw: bool,
    },
}

/// The batched work at the end of a tick, in the order to apply it.
#[derive(Debug, Default)]
pub(super) struct TickWork {
    pub sidebar_counts: Option<db::SidebarCounts>,
    pub append: Vec<db::Photo>,
    pub thumbnails: Vec<PathBuf>,
}

/// Whether an event is shown, and what showing its generation needs first.
#[derive(Debug, Default)]
pub(super) struct Admission {
    /// Apply these even when the event is not shown.
    pub effects: Vec<ScanEffect>,
    /// Pass the event to onboarding, then to [`ScanView::on_event`].
    pub admitted: bool,
}

/// The scan as shown: per-generation state between ticks.
pub(super) struct ScanView {
    displayed_generation: Option<u64>,
    scan_count: usize,
    thumbnail_total: usize,
    network_scan: bool,
    failure_message_shown: bool,
    last_progress_update: Instant,
    pending_photos: VecDeque<db::Photo>,
    pending_sidebar_counts: Option<db::SidebarCounts>,
    photos_since_sidebar_update: usize,
    last_sidebar_count_update: Instant,
    thumbnail_dirty_paths: VecDeque<PathBuf>,
    thumbnail_dirty_seen: HashSet<PathBuf>,
    /// The shown list differs from the catalogue as listed: photos it lacks
    /// that no live append adds (new photos in a Folder view, a withheld
    /// preferred pair half), or photos appended live at the end whatever the
    /// sort. A rebuild clears it; the job's end or Stop rebuilds while set.
    view_stale: bool,
}

impl ScanView {
    pub(super) fn new(now: Instant) -> Self {
        Self {
            displayed_generation: None,
            scan_count: 0,
            thumbnail_total: 0,
            network_scan: false,
            failure_message_shown: false,
            last_progress_update: now,
            pending_photos: VecDeque::new(),
            pending_sidebar_counts: None,
            photos_since_sidebar_update: 0,
            last_sidebar_count_update: now,
            thumbnail_dirty_paths: VecDeque::new(),
            thumbnail_dirty_seen: HashSet::new(),
            view_stale: false,
        }
    }

    /// New photos waiting to be appended.
    pub(super) fn pending_photo_count(&self) -> usize {
        self.pending_photos.len()
    }

    /// Progress count of the current phase (indexed photos or thumbnails).
    pub(super) fn scan_count(&self) -> usize {
        self.scan_count
    }

    pub(super) fn thumbnail_total(&self) -> usize {
        self.thumbnail_total
    }

    /// The status before this tick's events: visible thumbnails being made
    /// for the gallery outside a thumbnail pass.
    pub(super) fn begin_tick(
        &self,
        job: &ScanJobState,
        priority_pending: usize,
        priority_completions: usize,
    ) -> Vec<ScanEffect> {
        let quiet = !self.network_scan && !job.stop_requested && self.thumbnail_total == 0;
        if quiet && priority_pending > 0 {
            vec![
                ScanEffect::Status(format!(
                    "Creating visible thumbnails ({priority_pending} queued)"
                )),
                ScanEffect::StatusVisible(true),
            ]
        } else if quiet && priority_completions > 0 {
            vec![ScanEffect::StatusVisible(false)]
        } else {
            Vec::new()
        }
    }

    /// Whether an event of generation `generation` is shown: not from an
    /// older generation, and after Stop only the job's end and catalogue
    /// changes. The first event of a generation resets the view.
    pub(super) fn admit(
        &mut self,
        job: &ScanJobState,
        generation: u64,
        event: &ScanEvent,
    ) -> Admission {
        let mut admission = Admission::default();
        // Events from a cancelled/superseded scan are still allowed to
        // finish in their worker threads, but they must never overwrite
        // the progress UI or completion state of the newer job.
        if generation != job.generation {
            return admission;
        }
        if self.displayed_generation != Some(generation) {
            self.displayed_generation = Some(generation);
            admission.effects.push(ScanEffect::ShowProgress);
            self.scan_count = 0;
            self.thumbnail_total = 0;
            self.failure_message_shown = false;
            self.pending_sidebar_counts = None;
            self.photos_since_sidebar_update = 0;
        }
        admission.admitted = !job.stop_requested || passes_after_stop(event);
        admission
    }

    /// An admitted event: update the view and the job, and say what to show.
    pub(super) fn on_event(
        &mut self,
        job: &mut ScanJobState,
        event: &ScanEvent,
        facts: &dyn ScanViewFacts,
        now: Instant,
    ) -> Vec<ScanEffect> {
        let mut effects = Vec::new();
        match event {
            ScanEvent::Started { root } => {
                self.scan_count = 0;
                self.thumbnail_total = 0;
                self.network_scan = root
                    .to_str()
                    .is_some_and(|path| path.starts_with("nfs://") || path.starts_with("smb://"));
                let is_user_job = !matches!(job.kind, Some(ScanJobKind::Maintenance) | None);
                let folder_name = root
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or_else(|| root.to_str().unwrap_or("folder"));
                effects.extend([
                    ScanEffect::StopButton(is_user_job),
                    ScanEffect::Status(format!("Scanning {folder_name}…")),
                    ScanEffect::Spinning(true),
                    ScanEffect::StatusVisible(true),
                ]);
            }
            ScanEvent::FolderStarted { folder } => {
                // Only imports append rows immediately. Refreshing an existing
                // library/folder appended duplicate sidebar rows; right-clicking
                // those stale rows could start another refresh against
                // duplicated UI state.
                if job.kind == Some(ScanJobKind::Import) {
                    effects.push(ScanEffect::AppendSidebarFolder(folder.clone()));
                }
            }
            ScanEvent::DiscoveryProgress { found } => {
                effects.extend([
                    ScanEffect::Status(format!("Scanning… found {found} photos")),
                    ScanEffect::StatusVisible(true),
                ]);
            }
            ScanEvent::NetworkProgress {
                found,
                added,
                previews_ready,
                catalog_complete,
                metadata_ready: _metadata_ready,
            } => {
                let phase = if *catalog_complete {
                    "Finishing previews"
                } else {
                    "Adding photos"
                };
                let done = if *catalog_complete {
                    *previews_ready
                } else {
                    *added
                };
                effects.extend([
                    ScanEffect::Status(format!(
                        "{phase} · {} / {}",
                        format_count(done),
                        format_count(*found)
                    )),
                    ScanEffect::StatusVisible(true),
                    ScanEffect::Spinning(true),
                ]);
            }
            ScanEvent::PhotosUpdated { photos } => {
                for photo in photos {
                    if let Some(queued) = self
                        .pending_photos
                        .iter_mut()
                        .find(|queued| queued.id == photo.id)
                    {
                        *queued = photo.clone();
                    }
                }
                effects.push(ScanEffect::PhotosChanged(photos.clone()));
            }
            ScanEvent::PhotosIndexed { photos, counts } => {
                self.scan_count += photos.len();
                self.photos_since_sidebar_update += photos.len();
                self.pending_sidebar_counts = Some(*counts);
                self.photos_indexed(job, photos, facts, &mut effects);
                if !self.network_scan
                    && now.duration_since(self.last_progress_update) >= PROGRESS_INTERVAL
                {
                    effects.extend([
                        ScanEffect::Status(format!("Indexed {} photos", self.scan_count)),
                        ScanEffect::StatusVisible(true),
                    ]);
                    self.last_progress_update = now;
                }
            }
            ScanEvent::PhotosRemoved { ids } => {
                self.pending_photos.retain(|photo| !ids.contains(&photo.id));
                effects.push(ScanEffect::PhotosRemoved(ids.clone()));
            }
            ScanEvent::FoldersRemoved => {
                effects.extend([
                    ScanEffect::RecountSmartAlbums,
                    ScanEffect::RefreshAvailability,
                ]);
                if let SidebarFilter::Folder(id) = facts.filter() {
                    if !facts.folder_exists(id) {
                        effects.push(ScanEffect::ShowAllPhotos);
                    }
                    effects.push(ScanEffect::RefreshGrid);
                }
            }
            ScanEvent::LibraryCountsChanged { counts } => {
                self.pending_sidebar_counts = Some(*counts);
                // Reconciliation and final snapshots should be visible on the
                // next tick even when no photos needed reindexing.
                self.photos_since_sidebar_update =
                    self.photos_since_sidebar_update.max(SIDEBAR_COUNT_PHOTOS);
            }
            ScanEvent::IndexingFinished { imported } => {
                // Imported and refreshed photos can join Smart Albums.
                effects.push(ScanEffect::RecountSmartAlbums);
                effects.push(ScanEffect::Trace(ScanTrace::IndexingFinished {
                    imported: *imported,
                    filter: facts.filter(),
                }));
                // Progressive imports append quickly; refresh jobs do one
                // final rebuild after the whole serialized multi-folder job.
                // A Folder view had no live appends: rebuild its stream in
                // place (selected folder and scroll kept), and only when new
                // photos are missing from it.
                if !is_refresh(job.kind) && shows_folder_stream(facts) {
                    if self.view_stale {
                        effects.push(ScanEffect::RebuildFolderStream);
                    }
                } else if !is_refresh(job.kind) {
                    effects.push(ScanEffect::RefreshGrid);
                }
                if !self.network_scan {
                    effects.push(ScanEffect::Status(format!("Indexed {imported} photos")));
                }
                effects.push(ScanEffect::StatusVisible(true));
            }
            ScanEvent::ThumbnailsDeferred { total } => {
                effects.push(ScanEffect::RecoveryDeferred(*total > 0));
            }
            ScanEvent::ThumbnailsStarted { total } => {
                effects.push(ScanEffect::Trace(ScanTrace::ThumbnailsStarted {
                    total: *total,
                }));
                self.scan_count = 0;
                self.thumbnail_total = *total;
                effects.extend([
                    ScanEffect::Status(format!("Creating thumbnails 0 / {total}")),
                    ScanEffect::Spinning(true),
                    ScanEffect::StatusVisible(true),
                    ScanEffect::StopButton(true),
                ]);
            }
            ScanEvent::ThumbnailCreated { path } => {
                self.scan_count += 1;
                effects.push(ScanEffect::Trace(ScanTrace::ThumbnailReceived(
                    path.clone(),
                )));
                // Coalesce source paths and refresh only matching realized
                // tiles. Non-realized rows discover the new cache entry through
                // the background presentation loader when they bind later.
                self.mark_thumbnail_dirty(path.clone());
                if !self.network_scan
                    && now.duration_since(self.last_progress_update) >= PROGRESS_INTERVAL
                {
                    effects.extend([
                        ScanEffect::Status(format!(
                            "Creating thumbnails {} / {}",
                            self.scan_count, self.thumbnail_total
                        )),
                        ScanEffect::StatusVisible(true),
                    ]);
                    self.last_progress_update = now;
                }
            }
            ScanEvent::Failed { path, error } => {
                effects.push(ScanEffect::Trace(ScanTrace::Failed {
                    path: path.clone(),
                    error: error.clone(),
                }));
                let text = if self.failure_message_shown {
                    "Scanning… some files failed".to_owned()
                } else {
                    self.failure_message_shown = true;
                    format!("Some files could not be added: {error}")
                };
                effects.extend([ScanEffect::Status(text), ScanEffect::StatusVisible(true)]);
            }
            ScanEvent::Finished { imported, failed } => {
                self.finished(job, *imported, *failed, facts, &mut effects);
            }
            ScanEvent::Cancelled { imported } => {
                job.active = None;
                job.active_root = None;
                job.pending.clear();
                job.imported_total += *imported;
                let kind = job.kind.take();
                let message = match kind {
                    Some(ScanJobKind::Refresh) => {
                        format!("Library refresh stopped · {imported} photos updated")
                    }
                    Some(ScanJobKind::FolderRefresh) => {
                        format!("Folder refresh stopped · {imported} photos updated")
                    }
                    _ => format!("Import stopped · {imported} photos added"),
                };
                // Photos catalogued before Stop that the view lacks: rebuild
                // it once, as the job's end would have.
                if self.view_stale {
                    effects.push(self.rebuild_effect(facts));
                }
                // Folders found before Stop were added as flat rows counted
                // 0; rebuild them as a finished job does (the availability
                // pass, or for a folder refresh the row redraw).
                effects.push(if kind == Some(ScanJobKind::FolderRefresh) {
                    ScanEffect::RedrawFolderRows
                } else {
                    ScanEffect::RefreshAvailability
                });
                effects.extend([
                    ScanEffect::RebuildFolderWatches,
                    ScanEffect::StopButton(false),
                    ScanEffect::Spinning(false),
                    ScanEffect::Status(message),
                    ScanEffect::StatusVisible(true),
                    ScanEffect::HideProgressAfter(MESSAGE_HOLD),
                ]);
            }
        }
        // Any rebuild shows the catalogue as it is: nothing left to reconcile,
        // and appends still queued would show those photos twice.
        if effects.iter().any(|effect| {
            matches!(
                effect,
                ScanEffect::RefreshGrid | ScanEffect::RebuildFolderStream
            )
        }) {
            self.view_stale = false;
            self.pending_photos.clear();
        }
        effects
    }

    /// Which indexed photos join the view now (appends) and which shown
    /// photos update in place. In-place updates are skipped only when a
    /// search or metadata filter could drop an updated photo; appending new
    /// ones also needs no toolbar rating, the photo's format shown, and the
    /// view to be All or the photo's own folder. The final refresh covers
    /// the rest.
    fn photos_indexed(
        &mut self,
        job: &ScanJobState,
        photos: &[crate::scanner::IndexedPhoto],
        facts: &dyn ScanViewFacts,
        effects: &mut Vec<ScanEffect>,
    ) {
        let search_active = facts.search_active();
        // One scope snapshot per indexed batch.
        let scope = facts.scope();
        let filter = facts.filter();
        let filters_active = !scope.filters.is_empty();
        let appends_new = scan_may_append_new_photos(&scope);
        // Only All Photos takes live appends. A Folder view shows the whole
        // library as the Folder stream, and an append cannot land in its
        // folder's section; the stream is rebuilt instead.
        let appends_here = filter == SidebarFilter::All;
        let new = photos
            .iter()
            .filter(|indexed| indexed.newly_discovered)
            .count();
        let matching = photos
            .iter()
            .filter(|indexed| indexed.newly_discovered && appends_here)
            .count();
        effects.push(ScanEffect::Trace(ScanTrace::WatchIndexed {
            kind: job.kind,
            filter,
            photos: photos.len(),
            new,
            queued_for_append: if search_active || !appends_new {
                0
            } else {
                matching
            },
            search_active,
            filters_active,
        }));
        if new > 0 && shows_folder_stream(facts) {
            self.view_stale = true;
        }
        if search_active || filters_active {
            return;
        }
        let mut reread = Vec::new();
        // Asked once per batch, and only when a photo may be appended.
        let mut pair_mode = None;
        for indexed in photos {
            let photo = &indexed.photo;
            if indexed.newly_discovered
                && appends_here
                && appends_new
                && facts.format_enabled(&photo.path)
            {
                let mode = *pair_mode.get_or_insert_with(|| facts.pair_mode());
                if let Some(preferred) = pair_half(photo, mode) {
                    if facts.has_pair_twin(photo) {
                        // One half of a pair the full list collapses: never
                        // show both. A withheld preferred half means the view
                        // shows the other one, so it is rebuilt at the end
                        // (or on Stop).
                        if preferred {
                            self.view_stale = true;
                        }
                        continue;
                    }
                }
                self.pending_photos.push_back(photo.clone());
                // Appended at the end, out of order until a rebuild.
                self.view_stale = true;
            } else if !indexed.newly_discovered {
                // Folder refreshes deliberately avoid a full gallery
                // rebuild: update the shown photos in place.
                reread.push(photo.clone());
            }
        }
        if !reread.is_empty() {
            effects.push(ScanEffect::PhotosChanged(reread));
        }
    }

    /// One folder of the job finished: start the next, or end the job.
    fn finished(
        &mut self,
        job: &mut ScanJobState,
        imported: usize,
        failed: usize,
        facts: &dyn ScanViewFacts,
        effects: &mut Vec<ScanEffect>,
    ) {
        effects.push(ScanEffect::Trace(ScanTrace::Complete { imported, failed }));
        job.active = None;
        job.active_root = None;
        job.imported_total += imported;
        job.failed_total += failed;
        let (kind, total_imported, total_failed) = (job.kind, job.imported_total, job.failed_total);

        if !job.pending.is_empty() && !job.stop_requested {
            // Start exactly one next folder. Finished arrives after that
            // folder's thumbnail pass, so refresh progress stays serialized
            // end-to-end.
            effects.push(ScanEffect::StartNextScan);
            return;
        }

        // An entirely offline (or already cached) pass did no work. Report
        // that check briefly, then leave it quiet until the next reconnect.
        if kind == Some(ScanJobKind::Maintenance) && self.thumbnail_total == 0 {
            job.kind = None;
            effects.extend([
                ScanEffect::Status("No previews need work".to_owned()),
                ScanEffect::Spinning(false),
                ScanEffect::StopButton(false),
                ScanEffect::HideProgressAfter(MESSAGE_HOLD),
            ]);
            return;
        }

        let in_folder = matches!(facts.filter(), SidebarFilter::Folder(_));
        let refresh_grid = is_refresh(kind) && (!in_folder || facts.scope().is_narrowed());
        // Progressive appends land at the end of the Folder stream, so new
        // photos would form a second, misplaced section for their folder.
        // Rebuild the stream in place once the refresh added photos,
        // keeping the visible position.
        let mut rebuild_stream =
            !refresh_grid && is_refresh(kind) && total_imported > 0 && in_folder;
        let mut refresh_grid = refresh_grid;
        // New photos the view still lacks (a Folder view has no live appends).
        if self.view_stale && !refresh_grid && !rebuild_stream {
            match self.rebuild_effect(facts) {
                ScanEffect::RebuildFolderStream => rebuild_stream = true,
                _ => refresh_grid = true,
            }
        }
        let folder_refresh = kind == Some(ScanJobKind::FolderRefresh);
        effects.push(ScanEffect::Trace(ScanTrace::WatchFinished {
            kind,
            imported: total_imported,
            filter: facts.filter(),
            refresh_grid,
            folder_stream_rebuild: rebuild_stream,
            folder_rows_redraw: folder_refresh,
        }));
        if rebuild_stream {
            effects.push(ScanEffect::RebuildFolderStream);
        }
        if refresh_grid {
            effects.push(ScanEffect::RefreshGrid);
        }
        if !folder_refresh {
            effects.push(ScanEffect::RefreshAvailabilityStats);
        }
        effects.extend([ScanEffect::StopButton(false), ScanEffect::Spinning(false)]);
        let message = completion_message(
            kind,
            self.thumbnail_total,
            total_imported,
            total_failed,
            facts.recovery_deferred(),
        );
        job.kind = None;
        // A folder refresh skips the availability pass, which is the normal
        // owner of folder-row counts, so redraw the rows from the catalogue
        // or they keep pre-scan counts.
        effects.push(if folder_refresh {
            ScanEffect::RedrawFolderRows
        } else {
            ScanEffect::RefreshAvailability
        });
        // A scan may have catalogued new subfolders of a watched folder;
        // give them monitors too.
        effects.extend([
            ScanEffect::RebuildFolderWatches,
            ScanEffect::Status(message),
            ScanEffect::StatusVisible(true),
            ScanEffect::HideProgressAfter(MESSAGE_HOLD),
        ]);
    }

    /// The rebuild that shows the catalogue in the current view: the Folder
    /// stream in place, otherwise a grid refresh.
    fn rebuild_effect(&self, facts: &dyn ScanViewFacts) -> ScanEffect {
        if shows_folder_stream(facts) {
            ScanEffect::RebuildFolderStream
        } else {
            ScanEffect::RefreshGrid
        }
    }

    fn mark_thumbnail_dirty(&mut self, path: PathBuf) {
        if self.thumbnail_dirty_seen.insert(path.clone()) {
            self.thumbnail_dirty_paths.push_back(path);
        }
    }

    /// The batched work after this tick's events: throttled sidebar counts,
    /// one batch of new photos, one batch of thumbnail tiles (this tick's
    /// visible-thumbnail completions joining the queue first).
    pub(super) fn end_tick(
        &mut self,
        job_generation: u64,
        priority_paths: Vec<PathBuf>,
        search_active: bool,
        now: Instant,
    ) -> TickWork {
        let mut work = TickWork::default();
        if self.displayed_generation.is_some() && self.displayed_generation != Some(job_generation)
        {
            // A library switch or scan preemption invalidates already-drained
            // photo events as well as events still waiting in the channel.
            self.pending_photos.clear();
            self.pending_sidebar_counts = None;
            self.photos_since_sidebar_update = 0;
            self.displayed_generation = None;
            self.view_stale = false;
        }
        if self.pending_sidebar_counts.is_some()
            && (self.photos_since_sidebar_update >= SIDEBAR_COUNT_PHOTOS
                || now.duration_since(self.last_sidebar_count_update) >= SIDEBAR_COUNT_INTERVAL)
        {
            work.sidebar_counts = self.pending_sidebar_counts.take();
            self.photos_since_sidebar_update = 0;
            self.last_sidebar_count_update = now;
        }
        if search_active {
            // Search results supersede progressive scan appends. The next
            // debounced refresh replaces the model from the database.
            self.pending_photos.clear();
        }
        let appends = PHOTO_APPEND_BATCH.min(self.pending_photos.len());
        work.append = self.pending_photos.drain(..appends).collect();
        for path in priority_paths {
            self.mark_thumbnail_dirty(path);
        }
        let refreshes = THUMBNAIL_REFRESH_BATCH.min(self.thumbnail_dirty_paths.len());
        work.thumbnails = self.thumbnail_dirty_paths.drain(..refreshes).collect();
        for path in &work.thumbnails {
            self.thumbnail_dirty_seen.remove(path);
        }
        work
    }
}

/// Events still shown after Stop: the job's end and catalogue changes the
/// gallery and sidebar must not miss.
fn passes_after_stop(event: &ScanEvent) -> bool {
    matches!(
        event,
        ScanEvent::Cancelled { .. }
            | ScanEvent::Finished { .. }
            | ScanEvent::PhotosIndexed { .. }
            | ScanEvent::PhotosUpdated { .. }
            | ScanEvent::FolderStarted { .. }
            | ScanEvent::PhotosRemoved { .. }
            | ScanEvent::FoldersRemoved
            | ScanEvent::LibraryCountsChanged { .. }
            | ScanEvent::ThumbnailCreated { .. }
    )
}

/// For a RAW or JPEG photo while pairs collapse (not Both mode): whether it
/// is the half the mode shows. `None` when pairing does not apply to it.
fn pair_half(photo: &db::Photo, mode: RawJpegPairMode) -> Option<bool> {
    if mode == RawJpegPairMode::Both {
        return None;
    }
    let format = crate::image_format::for_path(&photo.path)?;
    let raw = format.decoder == crate::image_format::DecoderKind::Raw;
    if !raw && format.id != "jpeg" {
        return None;
    }
    Some(match mode {
        RawJpegPairMode::PreferRaw => raw,
        _ => !raw,
    })
}

/// Whether the view is the Folder stream (a Folder view without a search).
fn shows_folder_stream(facts: &dyn ScanViewFacts) -> bool {
    matches!(facts.filter(), SidebarFilter::Folder(_)) && !facts.search_active()
}

fn is_refresh(kind: Option<ScanJobKind>) -> bool {
    matches!(
        kind,
        Some(ScanJobKind::Refresh | ScanJobKind::FolderRefresh)
    )
}

/// The status a finished job leaves.
fn completion_message(
    kind: Option<ScanJobKind>,
    thumbnail_total: usize,
    imported: usize,
    failed: usize,
    recovery_deferred: bool,
) -> String {
    match kind {
        Some(ScanJobKind::Import) if thumbnail_total > 0 && failed == 0 => {
            let completed = if imported > 0 {
                imported
            } else {
                thumbnail_total
            };
            format!("✓ Previews complete · {} photos", format_count(completed))
        }
        Some(ScanJobKind::Refresh) if failed == 0 => {
            format!("Library refresh complete · {imported} photos updated")
        }
        Some(ScanJobKind::Refresh) => {
            format!("Library refresh complete · {imported} updated · {failed} failed")
        }
        Some(ScanJobKind::FolderRefresh) if failed == 0 => {
            format!("Folder refresh complete · {imported} photos updated")
        }
        Some(ScanJobKind::FolderRefresh) => {
            format!("Folder refresh complete · {imported} updated · {failed} failed")
        }
        Some(ScanJobKind::Maintenance) if failed == 0 && recovery_deferred => {
            "Available thumbnails recovered · waiting for offline sources".to_owned()
        }
        Some(ScanJobKind::Maintenance) if failed == 0 => "Thumbnail recovery complete".to_owned(),
        Some(ScanJobKind::Maintenance) => format!("Thumbnail recovery complete · {failed} failed"),
        _ if failed == 0 => format!("Added {imported} photos"),
        _ => format!("Added {imported} photos · {failed} failed"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    use crate::db::filters::{Field, Selection};
    use crate::db::{BrowseScope, Destination, RatingFilter};
    use crate::scanner::IndexedPhoto;
    use std::cell::RefCell;

    struct Facts {
        filter: SidebarFilter,
        search: bool,
        scope: BrowseScope,
        disabled: Vec<&'static str>,
        folders: Vec<i64>,
        deferred: bool,
        scope_asked: Cell<usize>,
        formats_asked: Cell<usize>,
        pair_mode: RawJpegPairMode,
        /// What the scanner has committed so far: (folder, path).
        catalogue: RefCell<Vec<(Option<i64>, String)>>,
        twin_lookups: Cell<usize>,
    }

    impl Facts {
        /// The scanner commits a batch before it sends the event.
        fn commit(&self, event: &ScanEvent) {
            if let ScanEvent::PhotosIndexed { photos, .. } = event {
                self.catalogue.borrow_mut().extend(
                    photos
                        .iter()
                        .map(|indexed| (indexed.photo.folder_id, indexed.photo.path.clone())),
                );
            }
        }
    }

    impl Default for Facts {
        fn default() -> Self {
            Self {
                filter: SidebarFilter::All,
                search: false,
                scope: BrowseScope::default(),
                disabled: Vec::new(),
                folders: vec![1, 2],
                deferred: false,
                scope_asked: Cell::new(0),
                formats_asked: Cell::new(0),
                pair_mode: RawJpegPairMode::Both,
                catalogue: RefCell::default(),
                twin_lookups: Cell::new(0),
            }
        }
    }

    impl ScanViewFacts for Facts {
        fn filter(&self) -> SidebarFilter {
            self.filter
        }
        fn search_active(&self) -> bool {
            self.search
        }
        fn scope(&self) -> BrowseScope {
            self.scope_asked.set(self.scope_asked.get() + 1);
            self.scope.clone()
        }
        fn format_enabled(&self, path: &str) -> bool {
            self.formats_asked.set(self.formats_asked.get() + 1);
            !self.disabled.iter().any(|ext| path.ends_with(ext))
        }
        fn folder_exists(&self, id: i64) -> bool {
            self.folders.contains(&id)
        }
        fn recovery_deferred(&self) -> bool {
            self.deferred
        }
        fn pair_mode(&self) -> RawJpegPairMode {
            self.pair_mode
        }
        /// The same stem in the same folder, the other kind (RAW / JPEG).
        fn has_pair_twin(&self, photo: &db::Photo) -> bool {
            self.twin_lookups.set(self.twin_lookups.get() + 1);
            let split = |path: &str| {
                let (stem, extension) = path.rsplit_once('.').unwrap();
                (stem.to_owned(), extension.eq_ignore_ascii_case("jpg"))
            };
            let (stem, jpeg) = split(&photo.path);
            self.catalogue.borrow().iter().any(|(folder, path)| {
                let (other_stem, other_jpeg) = split(path);
                *folder == photo.folder_id && other_stem == stem && other_jpeg != jpeg
            })
        }
    }

    fn job(kind: ScanJobKind) -> ScanJobState {
        ScanJobState {
            generation: 7,
            kind: Some(kind),
            ..ScanJobState::default()
        }
    }

    fn photo(id: i64, folder: i64, path: &str) -> db::Photo {
        db::Photo {
            id,
            path: path.to_owned(),
            folder_id: Some(folder),
            ..db::Photo::default()
        }
    }

    fn indexed(photos: &[(i64, i64, &str, bool)]) -> ScanEvent {
        ScanEvent::PhotosIndexed {
            photos: photos
                .iter()
                .map(|(id, folder, path, new)| IndexedPhoto {
                    photo: photo(*id, *folder, path),
                    newly_discovered: *new,
                })
                .collect(),
            counts: db::SidebarCounts::default(),
        }
    }

    /// One event as the adapter runs it: admission, then the body. Returns
    /// nothing for an event that is not admitted beyond its reset.
    fn step(
        view: &mut ScanView,
        job: &mut ScanJobState,
        generation: u64,
        event: &ScanEvent,
        facts: &Facts,
        now: Instant,
    ) -> Vec<ScanEffect> {
        let mut admission = view.admit(job, generation, event);
        if admission.admitted {
            admission
                .effects
                .extend(view.on_event(job, event, facts, now));
        }
        admission.effects
    }

    fn statuses(effects: &[ScanEffect]) -> Vec<String> {
        effects
            .iter()
            .filter_map(|effect| match effect {
                ScanEffect::Status(text) => Some(text.clone()),
                _ => None,
            })
            .collect()
    }

    fn has(effects: &[ScanEffect], wanted: fn(&ScanEffect) -> bool) -> bool {
        effects.iter().any(wanted)
    }

    /// The ids appended over as many ticks as it takes.
    fn drain_appends(view: &mut ScanView, generation: u64, now: Instant) -> Vec<i64> {
        let mut ids = Vec::new();
        loop {
            let work = view.end_tick(generation, Vec::new(), false, now);
            if work.append.is_empty() {
                return ids;
            }
            ids.extend(work.append.iter().map(|photo| photo.id));
        }
    }

    #[test]
    fn older_generations_are_ignored_and_a_new_one_resets_the_view() {
        let now = Instant::now();
        let mut view = ScanView::new(now);
        let mut job = job(ScanJobKind::Import);
        let facts = Facts::default();

        let stale = step(
            &mut view,
            &mut job,
            6,
            &ScanEvent::DiscoveryProgress { found: 3 },
            &facts,
            now,
        );
        assert!(stale.is_empty());

        let first = step(
            &mut view,
            &mut job,
            7,
            &ScanEvent::DiscoveryProgress { found: 3 },
            &facts,
            now,
        );
        assert!(matches!(first[0], ScanEffect::ShowProgress));
        assert_eq!(statuses(&first), ["Scanning… found 3 photos"]);

        let second = step(
            &mut view,
            &mut job,
            7,
            &ScanEvent::DiscoveryProgress { found: 4 },
            &facts,
            now,
        );
        assert!(!has(&second, |e| matches!(e, ScanEffect::ShowProgress)));

        // A newer generation clears the failure note and the counters.
        let failed = ScanEvent::Failed {
            path: "/a.jpg".into(),
            error: "bad".into(),
        };
        step(&mut view, &mut job, 7, &failed, &facts, now);
        assert_eq!(
            statuses(&step(&mut view, &mut job, 7, &failed, &facts, now)),
            ["Scanning… some files failed"]
        );
        job.generation = 8;
        let again = step(&mut view, &mut job, 8, &failed, &facts, now);
        assert!(matches!(again[0], ScanEffect::ShowProgress));
        assert_eq!(statuses(&again), ["Some files could not be added: bad"]);
    }

    #[test]
    fn after_stop_only_completion_and_catalogue_events_pass() {
        let now = Instant::now();
        for (event, passes) in [
            (ScanEvent::DiscoveryProgress { found: 1 }, false),
            (ScanEvent::Started { root: "/p".into() }, false),
            (ScanEvent::ThumbnailsStarted { total: 2 }, false),
            (
                ScanEvent::Failed {
                    path: "/a".into(),
                    error: "x".into(),
                },
                false,
            ),
            (ScanEvent::IndexingFinished { imported: 1 }, false),
            (indexed(&[(1, 1, "/a.jpg", true)]), true),
            (ScanEvent::PhotosUpdated { photos: vec![] }, true),
            (ScanEvent::PhotosRemoved { ids: vec![1] }, true),
            (ScanEvent::FoldersRemoved, true),
            (
                ScanEvent::LibraryCountsChanged {
                    counts: db::SidebarCounts::default(),
                },
                true,
            ),
            (
                ScanEvent::ThumbnailCreated {
                    path: "/a.jpg".into(),
                },
                true,
            ),
            (ScanEvent::Cancelled { imported: 0 }, true),
        ] {
            let mut view = ScanView::new(now);
            let mut job = job(ScanJobKind::Import);
            job.stop_requested = true;
            let admission = view.admit(&job, 7, &event);
            assert_eq!(admission.admitted, passes, "{event:?}");
            assert!(
                matches!(admission.effects[..], [ScanEffect::ShowProgress]),
                "{event:?}"
            );
        }
    }

    #[test]
    fn which_new_photo_joins_the_view() {
        let now = Instant::now();
        let narrowed_by_filters = {
            let mut scope = BrowseScope::default();
            scope.filters.categories.insert(
                Field::Camera,
                Selection {
                    values: vec!["X".into()],
                    ..Selection::default()
                },
            );
            scope
        };
        let narrowed_by_rating = BrowseScope {
            rating: RatingFilter::Three,
            ..BrowseScope::default()
        };
        // Ratings ignore the toolbar rating, but the gate checks the view.
        let rating_destination = BrowseScope {
            destination: Destination::Ratings,
            rating: RatingFilter::Three,
            ..BrowseScope::default()
        };
        // (filter, search, scope, disabled jpg, photo folder, new) -> appended, reread
        for (case, filter, search, scope, disabled, folder, new, appended, reread) in [
            (
                "all",
                SidebarFilter::All,
                false,
                BrowseScope::default(),
                false,
                1,
                true,
                true,
                false,
            ),
            // A Folder view is the whole library as the Folder stream: a live
            // append cannot land in its section, so nothing joins it live.
            (
                "own folder",
                SidebarFilter::Folder(1),
                false,
                BrowseScope::default(),
                false,
                1,
                true,
                false,
                false,
            ),
            (
                "other folder",
                SidebarFilter::Folder(2),
                false,
                BrowseScope::default(),
                false,
                1,
                true,
                false,
                false,
            ),
            (
                "favourites",
                SidebarFilter::Favorites,
                false,
                BrowseScope::default(),
                false,
                1,
                true,
                false,
                false,
            ),
            (
                "search",
                SidebarFilter::All,
                true,
                BrowseScope::default(),
                false,
                1,
                true,
                false,
                false,
            ),
            (
                "metadata filter",
                SidebarFilter::All,
                false,
                narrowed_by_filters.clone(),
                false,
                1,
                true,
                false,
                false,
            ),
            (
                "toolbar rating",
                SidebarFilter::All,
                false,
                narrowed_by_rating.clone(),
                false,
                1,
                true,
                false,
                false,
            ),
            (
                "rating page",
                SidebarFilter::Ratings,
                false,
                rating_destination,
                false,
                1,
                true,
                false,
                false,
            ),
            (
                "disabled format",
                SidebarFilter::All,
                false,
                BrowseScope::default(),
                true,
                1,
                true,
                false,
                false,
            ),
            (
                "known photo",
                SidebarFilter::All,
                false,
                BrowseScope::default(),
                false,
                1,
                false,
                false,
                true,
            ),
            // A rating narrows appends only; known photos still update in place.
            (
                "known, rated view",
                SidebarFilter::All,
                false,
                narrowed_by_rating,
                false,
                1,
                false,
                false,
                true,
            ),
            (
                "known, filtered",
                SidebarFilter::All,
                false,
                narrowed_by_filters,
                false,
                1,
                false,
                false,
                false,
            ),
            (
                "known, search",
                SidebarFilter::All,
                true,
                BrowseScope::default(),
                false,
                1,
                false,
                false,
                false,
            ),
        ] {
            let facts = Facts {
                filter,
                search,
                scope,
                disabled: if disabled { vec![".jpg"] } else { vec![] },
                ..Facts::default()
            };
            let mut view = ScanView::new(now);
            let mut job = job(ScanJobKind::Import);
            let effects = step(
                &mut view,
                &mut job,
                7,
                &indexed(&[(5, folder, "/a.jpg", new)]),
                &facts,
                now,
            );
            assert_eq!(
                drain_appends(&mut view, 7, now) == [5],
                appended,
                "{case}: appended"
            );
            let changed = has(
                &effects,
                |e| matches!(e, ScanEffect::PhotosChanged(p) if p.len() == 1),
            );
            assert_eq!(changed, reread, "{case}: reread");
            assert_eq!(facts.scope_asked.get(), 1, "{case}: one scope per batch");
        }
    }

    #[test]
    fn the_format_is_asked_only_for_new_photos_that_may_join() {
        let now = Instant::now();
        let batch = indexed(&[
            (1, 1, "/a.jpg", true),
            (2, 1, "/b.jpg", false),
            (3, 2, "/c.jpg", true),
        ]);
        for (filter, asked, appended) in [
            (SidebarFilter::All, 2, vec![1, 3]),
            // Nothing joins a Folder view live: no format is asked.
            (SidebarFilter::Folder(1), 0, vec![]),
        ] {
            let facts = Facts {
                filter,
                ..Facts::default()
            };
            let mut view = ScanView::new(now);
            let mut job = job(ScanJobKind::Import);
            step(&mut view, &mut job, 7, &batch, &facts, now);
            assert_eq!(facts.formats_asked.get(), asked, "{filter:?}");
            assert_eq!(drain_appends(&mut view, 7, now), appended, "{filter:?}");
        }
    }

    #[test]
    fn appends_go_out_in_batches_and_updates_reach_queued_photos() {
        let now = Instant::now();
        let facts = Facts::default();
        let mut view = ScanView::new(now);
        let mut job = job(ScanJobKind::Import);
        let rows: Vec<_> = (1..=400).map(|id| (id, 1, "/a.jpg", true)).collect();
        step(&mut view, &mut job, 7, &indexed(&rows), &facts, now);

        let mut updated = photo(3, 1, "/a.jpg");
        updated.camera = Some("Updated".into());
        let effects = step(
            &mut view,
            &mut job,
            7,
            &ScanEvent::PhotosUpdated {
                photos: vec![updated],
            },
            &facts,
            now,
        );
        assert!(has(
            &effects,
            |e| matches!(e, ScanEffect::PhotosChanged(p) if p.len() == 1)
        ));
        step(
            &mut view,
            &mut job,
            7,
            &ScanEvent::PhotosRemoved { ids: vec![2] },
            &facts,
            now,
        );

        let first = view.end_tick(7, Vec::new(), false, now);
        assert_eq!(first.append.len(), PHOTO_APPEND_BATCH);
        assert_eq!(first.append[0].id, 1);
        assert_eq!(
            first.append[1].id, 3,
            "removed photo 2 is dropped from the queue"
        );
        assert_eq!(first.append[1].camera.as_deref(), Some("Updated"));
        assert_eq!(view.pending_photo_count(), 399 - PHOTO_APPEND_BATCH);
        assert_eq!(
            view.end_tick(7, Vec::new(), false, now).append.len(),
            PHOTO_APPEND_BATCH
        );
        assert_eq!(
            view.end_tick(7, Vec::new(), false, now).append.len(),
            399 - 2 * PHOTO_APPEND_BATCH
        );
        assert!(view.end_tick(7, Vec::new(), false, now).append.is_empty());
    }

    #[test]
    fn a_search_or_a_new_generation_drops_queued_appends() {
        let now = Instant::now();
        let facts = Facts::default();
        let mut job = job(ScanJobKind::Import);

        let mut view = ScanView::new(now);
        step(
            &mut view,
            &mut job,
            7,
            &indexed(&[(1, 1, "/a.jpg", true)]),
            &facts,
            now,
        );
        assert!(view.end_tick(7, Vec::new(), true, now).append.is_empty());
        assert_eq!(view.pending_photo_count(), 0);

        let mut view = ScanView::new(now);
        step(
            &mut view,
            &mut job,
            7,
            &indexed(&[(1, 1, "/a.jpg", true)]),
            &facts,
            now,
        );
        // A library switch or preemption moved the job on.
        let work = view.end_tick(8, Vec::new(), false, now);
        assert!(work.append.is_empty());
        assert!(work.sidebar_counts.is_none());
        // The next event of the new generation shows the progress again.
        job.generation = 8;
        let effects = step(
            &mut view,
            &mut job,
            8,
            &ScanEvent::DiscoveryProgress { found: 1 },
            &facts,
            now,
        );
        assert!(matches!(effects[0], ScanEffect::ShowProgress));
    }

    #[test]
    fn sidebar_counts_wait_for_fifty_photos_or_a_quarter_second() {
        let start = Instant::now();
        let facts = Facts {
            filter: SidebarFilter::Favorites,
            ..Facts::default()
        };
        let mut job = job(ScanJobKind::Import);
        let mut view = ScanView::new(start);
        let later = |ms| start + Duration::from_millis(ms);

        step(
            &mut view,
            &mut job,
            7,
            &indexed(&[(1, 1, "/a.jpg", true)]),
            &facts,
            later(10),
        );
        assert!(view
            .end_tick(7, Vec::new(), false, later(10))
            .sidebar_counts
            .is_none());
        assert!(view
            .end_tick(7, Vec::new(), false, later(260))
            .sidebar_counts
            .is_some());
        assert!(
            view.end_tick(7, Vec::new(), false, later(270))
                .sidebar_counts
                .is_none(),
            "taken once"
        );

        let rows: Vec<_> = (1..=50).map(|id| (id, 1, "/a.jpg", true)).collect();
        step(&mut view, &mut job, 7, &indexed(&rows), &facts, later(280));
        assert!(view
            .end_tick(7, Vec::new(), false, later(280))
            .sidebar_counts
            .is_some());

        // Reconciliation counts show on the next tick without photos.
        step(
            &mut view,
            &mut job,
            7,
            &ScanEvent::LibraryCountsChanged {
                counts: db::SidebarCounts::default(),
            },
            &facts,
            later(290),
        );
        assert!(view
            .end_tick(7, Vec::new(), false, later(290))
            .sidebar_counts
            .is_some());
    }

    #[test]
    fn progress_text_is_throttled_and_thumbnails_are_batched_once_each() {
        let start = Instant::now();
        let later = |ms| start + Duration::from_millis(ms);
        let facts = Facts::default();
        let mut job = job(ScanJobKind::Import);
        let mut view = ScanView::new(start);

        let first = step(
            &mut view,
            &mut job,
            7,
            &indexed(&[(1, 1, "/a.jpg", true)]),
            &facts,
            later(200),
        );
        assert_eq!(statuses(&first), ["Indexed 1 photos"]);
        let second = step(
            &mut view,
            &mut job,
            7,
            &indexed(&[(2, 1, "/a.jpg", true)]),
            &facts,
            later(250),
        );
        assert!(statuses(&second).is_empty());

        let started = step(
            &mut view,
            &mut job,
            7,
            &ScanEvent::ThumbnailsStarted { total: 300 },
            &facts,
            later(260),
        );
        assert_eq!(statuses(&started), ["Creating thumbnails 0 / 300"]);
        assert!(has(&started, |e| matches!(e, ScanEffect::StopButton(true))));
        for index in 0..300 {
            let path = PathBuf::from(format!("/t{index}.jpg"));
            step(
                &mut view,
                &mut job,
                7,
                &ScanEvent::ThumbnailCreated { path },
                &facts,
                later(270),
            );
        }
        // The same path twice is refreshed once; a priority path joins the queue.
        step(
            &mut view,
            &mut job,
            7,
            &ScanEvent::ThumbnailCreated {
                path: "/t0.jpg".into(),
            },
            &facts,
            later(270),
        );
        let created = step(
            &mut view,
            &mut job,
            7,
            &ScanEvent::ThumbnailCreated {
                path: "/t1.jpg".into(),
            },
            &facts,
            later(500),
        );
        assert_eq!(statuses(&created), ["Creating thumbnails 302 / 300"]);

        let mut refreshed = Vec::new();
        let mut priority = vec![PathBuf::from("/visible.jpg"), PathBuf::from("/t5.jpg")];
        loop {
            let work = view.end_tick(7, std::mem::take(&mut priority), false, later(600));
            if work.thumbnails.is_empty() {
                break;
            }
            assert!(work.thumbnails.len() <= THUMBNAIL_REFRESH_BATCH);
            refreshed.extend(work.thumbnails);
        }
        assert_eq!(refreshed.len(), 301);
        assert_eq!(refreshed.last(), Some(&PathBuf::from("/visible.jpg")));
    }

    #[test]
    fn network_scans_keep_their_own_progress_text() {
        let now = Instant::now() + Duration::from_secs(1);
        let facts = Facts::default();
        let mut job = job(ScanJobKind::Import);
        let mut view = ScanView::new(Instant::now());
        let started = step(
            &mut view,
            &mut job,
            7,
            &ScanEvent::Started {
                root: "smb://nas/photos".into(),
            },
            &facts,
            now,
        );
        assert_eq!(statuses(&started), ["Scanning photos…"]);
        assert!(has(&started, |e| matches!(e, ScanEffect::StopButton(true))));
        let indexed = step(
            &mut view,
            &mut job,
            7,
            &indexed(&[(1, 1, "/a.jpg", true)]),
            &facts,
            now,
        );
        assert!(statuses(&indexed).is_empty());
        let progress = ScanEvent::NetworkProgress {
            found: 2000,
            added: 1500,
            metadata_ready: 0,
            previews_ready: 10,
            catalog_complete: false,
        };
        assert_eq!(
            statuses(&step(&mut view, &mut job, 7, &progress, &facts, now)),
            ["Adding photos · 1,500 / 2,000"]
        );
        let finished = step(
            &mut view,
            &mut job,
            7,
            &ScanEvent::IndexingFinished { imported: 4 },
            &facts,
            now,
        );
        assert!(statuses(&finished).is_empty());
    }

    #[test]
    fn maintenance_hides_the_stop_button_when_it_starts() {
        let now = Instant::now();
        let facts = Facts::default();
        let mut job = job(ScanJobKind::Maintenance);
        let mut view = ScanView::new(now);
        let started = step(
            &mut view,
            &mut job,
            7,
            &ScanEvent::Started {
                root: "/p/Trips".into(),
            },
            &facts,
            now,
        );
        assert_eq!(statuses(&started), ["Scanning Trips…"]);
        assert!(has(&started, |e| matches!(
            e,
            ScanEffect::StopButton(false)
        )));
    }

    #[test]
    fn imports_rebuild_when_indexing_ends_and_refreshes_wait_for_the_job() {
        let now = Instant::now();
        let facts = Facts::default();
        for (kind, rebuilds) in [
            (ScanJobKind::Import, true),
            (ScanJobKind::Maintenance, true),
            (ScanJobKind::Refresh, false),
            (ScanJobKind::FolderRefresh, false),
        ] {
            let mut view = ScanView::new(now);
            let mut job = job(kind);
            let effects = step(
                &mut view,
                &mut job,
                7,
                &ScanEvent::IndexingFinished { imported: 3 },
                &facts,
                now,
            );
            assert!(has(&effects, |e| matches!(
                e,
                ScanEffect::RecountSmartAlbums
            )));
            assert_eq!(
                has(&effects, |e| matches!(e, ScanEffect::RefreshGrid)),
                rebuilds,
                "{kind:?}"
            );
            assert_eq!(statuses(&effects), ["Indexed 3 photos"]);
        }
    }

    #[test]
    fn a_finished_folder_starts_the_next_one_and_keeps_the_totals() {
        let now = Instant::now();
        let facts = Facts::default();
        let mut view = ScanView::new(now);
        let mut job = job(ScanJobKind::Refresh);
        job.pending.push_back("/p/b".into());
        job.active_root = Some("/p/a".into());

        let effects = step(
            &mut view,
            &mut job,
            7,
            &ScanEvent::Finished {
                imported: 3,
                failed: 1,
            },
            &facts,
            now,
        );
        assert!(has(&effects, |e| matches!(e, ScanEffect::StartNextScan)));
        assert!(statuses(&effects).is_empty());
        assert!(!has(&effects, |e| matches!(e, ScanEffect::RefreshGrid)));
        assert_eq!((job.imported_total, job.failed_total), (3, 1));
        assert_eq!(job.active_root, None);
        assert_eq!(job.kind, Some(ScanJobKind::Refresh));

        job.pending.clear();
        let last = step(
            &mut view,
            &mut job,
            7,
            &ScanEvent::Finished {
                imported: 2,
                failed: 0,
            },
            &facts,
            now,
        );
        assert_eq!(
            statuses(&last),
            ["Library refresh complete · 5 updated · 1 failed"]
        );
        assert_eq!(job.kind, None);
        assert!(has(&last, |e| matches!(e, ScanEffect::RefreshGrid)));
        assert!(has(&last, |e| matches!(e, ScanEffect::RefreshAvailability)));
        assert!(has(&last, |e| matches!(
            e,
            ScanEffect::RefreshAvailabilityStats
        )));
        assert!(has(&last, |e| matches!(
            e,
            ScanEffect::RebuildFolderWatches
        )));

        // Stop wins over the queue: no next folder.
        let mut job = job_with_queue(ScanJobKind::Refresh);
        job.stop_requested = true;
        let stopped = step(
            &mut view,
            &mut job,
            7,
            &ScanEvent::Finished {
                imported: 0,
                failed: 0,
            },
            &facts,
            now,
        );
        assert!(!has(&stopped, |e| matches!(e, ScanEffect::StartNextScan)));
    }

    fn job_with_queue(kind: ScanJobKind) -> ScanJobState {
        let mut job = job(kind);
        job.pending.push_back("/p/next".into());
        job
    }

    #[test]
    fn how_a_refresh_job_ends_in_each_view() {
        let now = Instant::now();
        let narrowed = BrowseScope {
            rating: RatingFilter::Two,
            ..BrowseScope::default()
        };
        // (kind, filter, scope, imported) -> refresh grid, rebuild stream, redraw folder rows
        for (case, kind, filter, scope, imported, grid, stream, rows) in [
            (
                "refresh, all",
                ScanJobKind::Refresh,
                SidebarFilter::All,
                BrowseScope::default(),
                0,
                true,
                false,
                false,
            ),
            (
                "refresh, folder, new photos",
                ScanJobKind::Refresh,
                SidebarFilter::Folder(1),
                BrowseScope::default(),
                4,
                false,
                true,
                false,
            ),
            (
                "refresh, folder, nothing new",
                ScanJobKind::Refresh,
                SidebarFilter::Folder(1),
                BrowseScope::default(),
                0,
                false,
                false,
                false,
            ),
            (
                "refresh, folder, narrowed",
                ScanJobKind::Refresh,
                SidebarFilter::Folder(1),
                narrowed,
                4,
                true,
                false,
                false,
            ),
            (
                "folder refresh, folder",
                ScanJobKind::FolderRefresh,
                SidebarFilter::Folder(1),
                BrowseScope::default(),
                4,
                false,
                true,
                true,
            ),
            (
                "folder refresh, all",
                ScanJobKind::FolderRefresh,
                SidebarFilter::All,
                BrowseScope::default(),
                4,
                true,
                false,
                true,
            ),
            (
                "import, all",
                ScanJobKind::Import,
                SidebarFilter::All,
                BrowseScope::default(),
                4,
                false,
                false,
                false,
            ),
            (
                "import, folder",
                ScanJobKind::Import,
                SidebarFilter::Folder(1),
                BrowseScope::default(),
                4,
                false,
                false,
                false,
            ),
        ] {
            let facts = Facts {
                filter,
                scope,
                ..Facts::default()
            };
            let mut view = ScanView::new(now);
            let mut job = job(kind);
            let effects = step(
                &mut view,
                &mut job,
                7,
                &ScanEvent::Finished {
                    imported,
                    failed: 0,
                },
                &facts,
                now,
            );
            assert_eq!(
                has(&effects, |e| matches!(e, ScanEffect::RefreshGrid)),
                grid,
                "{case}: grid"
            );
            assert_eq!(
                has(&effects, |e| matches!(e, ScanEffect::RebuildFolderStream)),
                stream,
                "{case}: stream"
            );
            assert_eq!(
                has(&effects, |e| matches!(e, ScanEffect::RedrawFolderRows)),
                rows,
                "{case}: rows"
            );
            assert_eq!(
                has(&effects, |e| matches!(e, ScanEffect::RefreshAvailability)),
                !rows,
                "{case}: availability"
            );
            assert_eq!(
                has(&effects, |e| matches!(
                    e,
                    ScanEffect::RefreshAvailabilityStats
                )),
                !rows,
                "{case}: stats"
            );
            let asked = usize::from(
                matches!(kind, ScanJobKind::Refresh | ScanJobKind::FolderRefresh)
                    && matches!(filter, SidebarFilter::Folder(_)),
            );
            assert_eq!(facts.scope_asked.get(), asked, "{case}: scope asked");
        }
    }

    #[test]
    fn completion_messages_per_job_kind() {
        let now = Instant::now();
        let finish = |kind, thumbnails: usize, imported, failed, deferred| {
            let facts = Facts {
                deferred,
                ..Facts::default()
            };
            let mut view = ScanView::new(now);
            let mut job = job(kind);
            if thumbnails > 0 {
                step(
                    &mut view,
                    &mut job,
                    7,
                    &ScanEvent::ThumbnailsStarted { total: thumbnails },
                    &facts,
                    now,
                );
            }
            let effects = step(
                &mut view,
                &mut job,
                7,
                &ScanEvent::Finished { imported, failed },
                &facts,
                now,
            );
            assert!(has(
                &effects,
                |e| matches!(e, ScanEffect::HideProgressAfter(d) if *d == MESSAGE_HOLD)
            ));
            assert_eq!(job.kind, None);
            statuses(&effects).pop().unwrap()
        };
        use ScanJobKind::*;
        assert_eq!(
            finish(Import, 1200, 1500, 0, false),
            "✓ Previews complete · 1,500 photos"
        );
        assert_eq!(
            finish(Import, 1200, 0, 0, false),
            "✓ Previews complete · 1,200 photos"
        );
        assert_eq!(
            finish(Import, 1200, 5, 2, false),
            "Added 5 photos · 2 failed"
        );
        assert_eq!(finish(Import, 0, 5, 0, false), "Added 5 photos");
        assert_eq!(
            finish(Refresh, 0, 5, 0, false),
            "Library refresh complete · 5 photos updated"
        );
        assert_eq!(
            finish(FolderRefresh, 0, 5, 0, false),
            "Folder refresh complete · 5 photos updated"
        );
        assert_eq!(
            finish(FolderRefresh, 0, 5, 3, false),
            "Folder refresh complete · 5 updated · 3 failed"
        );
        assert_eq!(finish(Maintenance, 0, 0, 0, false), "No previews need work");
        assert_eq!(
            finish(Maintenance, 10, 0, 0, false),
            "Thumbnail recovery complete"
        );
        assert_eq!(
            finish(Maintenance, 10, 0, 0, true),
            "Available thumbnails recovered · waiting for offline sources"
        );
        assert_eq!(
            finish(Maintenance, 10, 0, 4, false),
            "Thumbnail recovery complete · 4 failed"
        );
    }

    #[test]
    fn maintenance_with_no_work_ends_quietly() {
        let now = Instant::now();
        let facts = Facts::default();
        let mut view = ScanView::new(now);
        let mut job = job(ScanJobKind::Maintenance);
        job.active_root = Some("/p".into());
        let effects = step(
            &mut view,
            &mut job,
            7,
            &ScanEvent::Finished {
                imported: 0,
                failed: 0,
            },
            &facts,
            now,
        );
        assert_eq!(statuses(&effects), ["No previews need work"]);
        assert!(!has(&effects, |e| matches!(
            e,
            ScanEffect::RebuildFolderWatches | ScanEffect::RefreshGrid
        )));
        assert_eq!((job.kind, job.active_root.as_deref()), (None, None));
    }

    #[test]
    fn cancelling_clears_the_job_and_says_what_was_done() {
        let now = Instant::now();
        let facts = Facts::default();
        for (kind, message) in [
            (
                ScanJobKind::Refresh,
                "Library refresh stopped · 4 photos updated",
            ),
            (
                ScanJobKind::FolderRefresh,
                "Folder refresh stopped · 4 photos updated",
            ),
            (ScanJobKind::Import, "Import stopped · 4 photos added"),
        ] {
            let mut view = ScanView::new(now);
            let mut job = job_with_queue(kind);
            job.stop_requested = true;
            job.imported_total = 10;
            let effects = step(
                &mut view,
                &mut job,
                7,
                &ScanEvent::Cancelled { imported: 4 },
                &facts,
                now,
            );
            assert_eq!(statuses(&effects), [message]);
            assert!(has(&effects, |e| matches!(
                e,
                ScanEffect::RebuildFolderWatches
            )));
            assert!(has(&effects, |e| matches!(
                e,
                ScanEffect::StopButton(false)
            )));
            assert!(job.pending.is_empty());
            assert_eq!((job.kind, job.imported_total), (None, 14));
        }
    }

    /// A stopped job leaves the sidebar as a finished one does: imported
    /// folders were added as flat rows counted 0, and only the availability
    /// pass (or, for a folder refresh, the row redraw) rebuilds the tree and
    /// recounts them.
    #[test]
    fn a_stopped_job_rebuilds_the_folder_rows_like_a_finished_one() {
        let now = Instant::now();
        let facts = Facts::default();
        for (kind, rows) in [
            (ScanJobKind::Import, false),
            (ScanJobKind::Refresh, false),
            (ScanJobKind::FolderRefresh, true),
        ] {
            let mut view = ScanView::new(now);
            let mut job = job(kind);
            job.stop_requested = true;
            let cancelled = ScanEvent::Cancelled { imported: 2 };
            let effects = step(&mut view, &mut job, 7, &cancelled, &facts, now);
            let redraw = has(&effects, |e| matches!(e, ScanEffect::RedrawFolderRows));
            let availability = has(&effects, |e| matches!(e, ScanEffect::RefreshAvailability));
            assert_eq!((redraw, availability), (rows, !rows), "{kind:?}");
        }
    }

    #[test]
    fn removed_folders_leave_a_folder_view_that_is_gone() {
        let now = Instant::now();
        for (filter, folders, leaves, refreshes) in [
            (SidebarFilter::All, vec![1], false, false),
            (SidebarFilter::Folder(1), vec![1], false, true),
            (SidebarFilter::Folder(3), vec![1], true, true),
        ] {
            let facts = Facts {
                filter,
                folders,
                ..Facts::default()
            };
            let mut view = ScanView::new(now);
            let mut job = job(ScanJobKind::Refresh);
            let effects = step(
                &mut view,
                &mut job,
                7,
                &ScanEvent::FoldersRemoved,
                &facts,
                now,
            );
            assert!(matches!(effects[1], ScanEffect::RecountSmartAlbums));
            assert!(matches!(effects[2], ScanEffect::RefreshAvailability));
            assert_eq!(
                has(&effects, |e| matches!(e, ScanEffect::ShowAllPhotos)),
                leaves,
                "{filter:?}"
            );
            assert_eq!(
                has(&effects, |e| matches!(e, ScanEffect::RefreshGrid)),
                refreshes,
                "{filter:?}"
            );
        }
    }

    #[test]
    fn only_imports_add_sidebar_folder_rows() {
        let now = Instant::now();
        let facts = Facts::default();
        let folder = db::Folder {
            id: 9,
            path: "/p/new".into(),
            name: "new".into(),
            parent_id: None,
            imported_root: true,
            watched: false,
            photo_count: 0,
            subfolder_count: 0,
            available: true,
        };
        for (kind, appends) in [(ScanJobKind::Import, true), (ScanJobKind::Refresh, false)] {
            let mut view = ScanView::new(now);
            let mut job = job(kind);
            let effects = step(
                &mut view,
                &mut job,
                7,
                &ScanEvent::FolderStarted {
                    folder: folder.clone(),
                },
                &facts,
                now,
            );
            assert_eq!(
                has(&effects, |e| matches!(
                    e,
                    ScanEffect::AppendSidebarFolder(_)
                )),
                appends
            );
        }
    }

    fn grid_rebuilds(effects: &[ScanEffect]) -> (usize, usize) {
        let count = |wanted: fn(&ScanEffect) -> bool| effects.iter().filter(|e| wanted(e)).count();
        (
            count(|e| matches!(e, ScanEffect::RefreshGrid)),
            count(|e| matches!(e, ScanEffect::RebuildFolderStream)),
        )
    }

    /// Runs `events` and returns every effect.
    fn run_events(
        view: &mut ScanView,
        job: &mut ScanJobState,
        facts: &Facts,
        events: &[ScanEvent],
    ) -> Vec<ScanEffect> {
        let now = Instant::now();
        let mut effects = Vec::new();
        for event in events {
            facts.commit(event);
            effects.extend(step(view, job, 7, event, facts, now));
        }
        effects
    }

    fn one(id: i64, folder: i64, path: &str) -> ScanEvent {
        indexed(&[(id, folder, path, true)])
    }

    #[test]
    fn a_new_pair_never_shows_both_halves_and_stop_shows_the_preferred_one() {
        let now = Instant::now();
        let (raw, jpeg) = (one(1, 1, "/p/a.NEF"), one(2, 1, "/p/a.jpg"));
        // (mode, first, second) -> the live half.
        for (mode, first, second, live) in [
            (RawJpegPairMode::PreferRaw, &raw, &jpeg, 1),
            (RawJpegPairMode::PreferRaw, &jpeg, &raw, 2),
            (RawJpegPairMode::PreferJpeg, &jpeg, &raw, 2),
            (RawJpegPairMode::PreferJpeg, &raw, &jpeg, 1),
        ] {
            let case = format!("{mode:?} first={first:?}");
            let facts = Facts {
                pair_mode: mode,
                ..Facts::default()
            };
            let mut view = ScanView::new(now);
            let mut job = job_with_queue(ScanJobKind::Refresh);
            run_events(
                &mut view,
                &mut job,
                &facts,
                &[first.clone(), second.clone()],
            );
            assert_eq!(drain_appends(&mut view, 7, now), [live], "{case}");
            // Stop right after the second half is catalogued.
            job.stop_requested = true;
            let stopped = run_events(
                &mut view,
                &mut job,
                &facts,
                &[ScanEvent::Cancelled { imported: 2 }],
            );
            // Stop rebuilds once either way: the first half was appended
            // live (out of order), and when it is the non-preferred half
            // the rebuild also shows the preferred one.
            assert_eq!(grid_rebuilds(&stopped), (1, 0), "{case}");

            // The same order finishing normally: an import's end of indexing
            // rebuilds the gallery from the catalogue (the full list pairs).
            let facts = Facts {
                pair_mode: mode,
                ..Facts::default()
            };
            let mut view = ScanView::new(now);
            let mut import = self::job(ScanJobKind::Import);
            run_events(
                &mut view,
                &mut import,
                &facts,
                &[first.clone(), second.clone()],
            );
            assert_eq!(drain_appends(&mut view, 7, now), [live], "{case}");
            let end = run_events(
                &mut view,
                &mut import,
                &facts,
                &[ScanEvent::IndexingFinished { imported: 2 }],
            );
            assert_eq!(grid_rebuilds(&end), (1, 0), "{case}");
            let finished = run_events(
                &mut view,
                &mut import,
                &facts,
                &[ScanEvent::Finished {
                    imported: 2,
                    failed: 0,
                }],
            );
            assert_eq!(grid_rebuilds(&finished), (0, 0), "{case}: reconciled once");
        }
    }

    #[test]
    fn both_halves_in_one_batch_wait_for_the_rebuild() {
        let now = Instant::now();
        let facts = Facts {
            pair_mode: RawJpegPairMode::PreferRaw,
            ..Facts::default()
        };
        let mut view = ScanView::new(now);
        let mut job = job_with_queue(ScanJobKind::Refresh);
        run_events(
            &mut view,
            &mut job,
            &facts,
            &[indexed(&[
                (1, 1, "/p/a.NEF", true),
                (2, 1, "/p/a.jpg", true),
            ])],
        );
        assert!(drain_appends(&mut view, 7, now).is_empty());
        job.stop_requested = true;
        let stopped = run_events(
            &mut view,
            &mut job,
            &facts,
            &[ScanEvent::Cancelled { imported: 2 }],
        );
        assert_eq!(grid_rebuilds(&stopped), (1, 0));
    }

    #[test]
    fn unpaired_files_and_both_mode_append_as_before() {
        let now = Instant::now();
        let batches = [
            one(1, 1, "/p/solo.NEF"),
            one(2, 1, "/p/other.jpg"),
            // The same name in another folder is not a twin.
            one(3, 1, "/p/a.NEF"),
            one(4, 2, "/q/a.jpg"),
            // PNG never pairs: no lookup.
            one(5, 1, "/p/a.png"),
        ];
        for (mode, lookups) in [(RawJpegPairMode::PreferRaw, 4), (RawJpegPairMode::Both, 0)] {
            let facts = Facts {
                pair_mode: mode,
                ..Facts::default()
            };
            let mut view = ScanView::new(now);
            let mut job = job_with_queue(ScanJobKind::Refresh);
            run_events(&mut view, &mut job, &facts, &batches);
            assert_eq!(
                drain_appends(&mut view, 7, now),
                [1, 2, 3, 4, 5],
                "{mode:?}"
            );
            assert_eq!(facts.twin_lookups.get(), lookups, "{mode:?}");
            job.stop_requested = true;
            let stopped = run_events(
                &mut view,
                &mut job,
                &facts,
                &[ScanEvent::Cancelled { imported: 5 }],
            );
            // Live appends are out of order until a rebuild: Stop rebuilds.
            assert_eq!(grid_rebuilds(&stopped), (1, 0), "{mode:?}");
        }
        // Both mode shows both halves of a pair live.
        let facts = Facts::default();
        let mut view = ScanView::new(now);
        let mut job = job(ScanJobKind::Import);
        run_events(
            &mut view,
            &mut job,
            &facts,
            &[one(1, 1, "/p/a.NEF"), one(2, 1, "/p/a.jpg")],
        );
        assert_eq!(drain_appends(&mut view, 7, now), [1, 2]);
        assert_eq!(facts.twin_lookups.get(), 0);
    }

    #[test]
    fn a_folder_view_shows_new_photos_at_the_rebuild_not_live() {
        let now = Instant::now();
        // Own folder, a subfolder, an unrelated folder.
        let batch = indexed(&[
            (1, 1, "/p/a.jpg", true),
            (2, 3, "/p/sub/b.jpg", true),
            (3, 9, "/q/c.jpg", true),
        ]);
        for filter in [
            SidebarFilter::Folder(1),
            SidebarFilter::Folder(3),
            SidebarFilter::Folder(9),
        ] {
            let facts = Facts {
                filter,
                ..Facts::default()
            };
            let mut view = ScanView::new(now);
            let mut job = job(ScanJobKind::Import);
            let effects = run_events(
                &mut view,
                &mut job,
                &facts,
                &[batch.clone(), ScanEvent::IndexingFinished { imported: 3 }],
            );
            assert!(drain_appends(&mut view, 7, now).is_empty(), "{filter:?}");
            // One in-place rebuild of the stream (keeps the selected folder
            // and the scroll position), not a full refresh.
            assert_eq!(grid_rebuilds(&effects), (0, 1), "{filter:?}");
            // Nothing left to reconcile when the job ends or is stopped.
            let end = run_events(
                &mut view,
                &mut job,
                &facts,
                &[ScanEvent::Finished {
                    imported: 3,
                    failed: 0,
                }],
            );
            assert_eq!(grid_rebuilds(&end), (0, 0), "{filter:?}");
        }
    }

    #[test]
    fn a_folder_view_is_not_rebuilt_when_no_photo_is_new() {
        let facts = Facts {
            filter: SidebarFilter::Folder(1),
            ..Facts::default()
        };
        let mut view = ScanView::new(Instant::now());
        let mut job = job(ScanJobKind::Import);
        let effects = run_events(
            &mut view,
            &mut job,
            &facts,
            &[
                indexed(&[(1, 1, "/p/a.jpg", false)]),
                ScanEvent::IndexingFinished { imported: 1 },
            ],
        );
        assert_eq!(grid_rebuilds(&effects), (0, 0));
        // The known photo still updates in place.
        assert!(has(&effects, |e| matches!(e, ScanEffect::PhotosChanged(_))));
    }

    #[test]
    fn all_photos_keeps_its_live_appends_and_its_refresh() {
        let now = Instant::now();
        let facts = Facts::default();
        let mut view = ScanView::new(now);
        let mut job = job(ScanJobKind::Import);
        run_events(
            &mut view,
            &mut job,
            &facts,
            &[indexed(&[
                (1, 1, "/p/a.jpg", true),
                (2, 3, "/p/sub/b.jpg", true),
            ])],
        );
        assert_eq!(drain_appends(&mut view, 7, now), [1, 2]);
        let effects = run_events(
            &mut view,
            &mut job,
            &facts,
            &[ScanEvent::IndexingFinished { imported: 2 }],
        );
        assert_eq!(grid_rebuilds(&effects), (1, 0));
        // Stop after live appends: rebuilt once, in order.
        let mut view = ScanView::new(now);
        let mut job = job_with_queue(ScanJobKind::Refresh);
        job.stop_requested = true;
        let stopped = run_events(
            &mut view,
            &mut job,
            &facts,
            &[
                indexed(&[(1, 1, "/p/a.jpg", true)]),
                ScanEvent::Cancelled { imported: 1 },
            ],
        );
        assert_eq!(grid_rebuilds(&stopped), (1, 0));
    }

    #[test]
    fn stop_in_a_folder_view_rebuilds_the_stream_only_when_photos_are_missing() {
        let facts = Facts {
            filter: SidebarFilter::Folder(1),
            ..Facts::default()
        };
        for (kind, events, rebuilds) in [
            // New photos, then Stop before the end of indexing.
            (
                ScanJobKind::Import,
                vec![indexed(&[(1, 9, "/q/a.jpg", true)])],
                (0, 1),
            ),
            (
                ScanJobKind::Refresh,
                vec![indexed(&[(1, 3, "/p/sub/a.jpg", true)])],
                (0, 1),
            ),
            (
                ScanJobKind::FolderRefresh,
                vec![indexed(&[(1, 1, "/p/a.jpg", true)])],
                (0, 1),
            ),
            // Nothing new: nothing to rebuild.
            (
                ScanJobKind::Refresh,
                vec![indexed(&[(1, 1, "/p/a.jpg", false)])],
                (0, 0),
            ),
            (ScanJobKind::Import, vec![], (0, 0)),
            // Already rebuilt when indexing finished (Stop during thumbnails).
            (
                ScanJobKind::Import,
                vec![
                    indexed(&[(1, 9, "/q/a.jpg", true)]),
                    ScanEvent::IndexingFinished { imported: 1 },
                ],
                (0, 1),
            ),
        ] {
            let mut view = ScanView::new(Instant::now());
            let mut job = job_with_queue(kind);
            let mut all = run_events(&mut view, &mut job, &facts, &events);
            job.stop_requested = true;
            all.extend(run_events(
                &mut view,
                &mut job,
                &facts,
                &[ScanEvent::Cancelled { imported: 1 }],
            ));
            assert_eq!(grid_rebuilds(&all), rebuilds, "{kind:?} {events:?}");
        }
    }

    #[test]
    fn a_multi_folder_refresh_in_a_folder_view_rebuilds_once_at_the_end() {
        let facts = Facts {
            filter: SidebarFilter::Folder(1),
            ..Facts::default()
        };
        let mut view = ScanView::new(Instant::now());
        let mut job = job_with_queue(ScanJobKind::Refresh);
        let first = run_events(
            &mut view,
            &mut job,
            &facts,
            &[
                indexed(&[(1, 3, "/p/sub/a.jpg", true)]),
                ScanEvent::IndexingFinished { imported: 1 },
                ScanEvent::Finished {
                    imported: 1,
                    failed: 0,
                },
            ],
        );
        assert_eq!(
            grid_rebuilds(&first),
            (0, 0),
            "the next folder starts first"
        );
        job.pending.clear();
        let last = run_events(
            &mut view,
            &mut job,
            &facts,
            &[ScanEvent::Finished {
                imported: 0,
                failed: 0,
            }],
        );
        assert_eq!(grid_rebuilds(&last), (0, 1));
    }

    #[test]
    fn stop_after_live_appends_rebuilds_all_photos_in_order() {
        // Live appends land at the end whatever the sort: Stop must rebuild
        // once, as the end of indexing or of the job would.
        for kind in [
            ScanJobKind::Import,
            ScanJobKind::Refresh,
            ScanJobKind::FolderRefresh,
        ] {
            let facts = Facts::default();
            let mut view = ScanView::new(Instant::now());
            let mut job = job_with_queue(kind);
            run_events(&mut view, &mut job, &facts, &[one(1, 1, "/p/a.jpg")]);
            job.stop_requested = true;
            let stopped = run_events(
                &mut view,
                &mut job,
                &facts,
                &[ScanEvent::Cancelled { imported: 1 }],
            );
            assert_eq!(grid_rebuilds(&stopped), (1, 0), "{kind:?}");
        }
        // Stop after the end of indexing already rebuilt: nothing more.
        let facts = Facts::default();
        let mut view = ScanView::new(Instant::now());
        let mut job = job(ScanJobKind::Import);
        run_events(
            &mut view,
            &mut job,
            &facts,
            &[
                one(1, 1, "/p/a.jpg"),
                ScanEvent::IndexingFinished { imported: 1 },
            ],
        );
        job.stop_requested = true;
        let stopped = run_events(
            &mut view,
            &mut job,
            &facts,
            &[ScanEvent::Cancelled { imported: 1 }],
        );
        assert_eq!(grid_rebuilds(&stopped), (0, 0));
    }

    #[test]
    fn a_rebuild_drops_appends_still_queued() {
        // The rebuild lists every catalogued photo; appending the queued
        // ones after it would show them twice.
        let now = Instant::now();
        let facts = Facts::default();
        let mut view = ScanView::new(now);
        let mut job = job(ScanJobKind::Import);
        let rows: Vec<_> = (1..=400).map(|id| (id, 1, "/p/a.jpg", true)).collect();
        run_events(&mut view, &mut job, &facts, &[indexed(&rows)]);
        assert_eq!(
            view.end_tick(7, Vec::new(), false, now).append.len(),
            PHOTO_APPEND_BATCH
        );
        let end = run_events(
            &mut view,
            &mut job,
            &facts,
            &[ScanEvent::IndexingFinished { imported: 400 }],
        );
        assert_eq!(grid_rebuilds(&end), (1, 0));
        assert_eq!(view.pending_photo_count(), 0);
        assert!(view.end_tick(7, Vec::new(), false, now).append.is_empty());
    }

    #[test]
    fn visible_thumbnails_show_outside_a_thumbnail_pass() {
        let now = Instant::now();
        let view = ScanView::new(now);
        let mut job = job(ScanJobKind::Import);
        assert_eq!(
            statuses(&view.begin_tick(&job, 3, 0)),
            ["Creating visible thumbnails (3 queued)"]
        );
        assert!(matches!(
            view.begin_tick(&job, 0, 2)[..],
            [ScanEffect::StatusVisible(false)]
        ));
        assert!(view.begin_tick(&job, 0, 0).is_empty());
        job.stop_requested = true;
        assert!(view.begin_tick(&job, 3, 0).is_empty());
    }
}
