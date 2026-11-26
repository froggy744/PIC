//! GTK adapter of the scan view (candidate 19): the window's 50 ms scan
//! timer. It drains the refresh-prepare events (job startup) and the
//! scanner's events, runs each through [`ScanView`], and applies the
//! [`ScanEffect`]s and the per-tick [`TickWork`] to the status bar, sidebar,
//! gallery and onboarding. Diagnostic lines are printed here with their
//! exact text.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use gtk4 as gtk;
use gtk4::prelude::*;

use super::onboarding::OnboardingCoordinator;
use super::scan_view::{ScanEffect, ScanTrace, ScanView, ScanViewFacts};
use super::{
    browse_scope, photo_changed, refresh_grid, refresh_grid_preserving_folder_viewport,
    refresh_smart_album_counts, restore_section_layout, run_ui_guarded, PhotoActionContext,
    PhotoChange, RefreshPrepareEvent, ScanJobState, ScanUiEvent,
};
use crate::{db, grid, sidebar};

/// Drained per tick at most, so input, redraws, scrolling and Stop stay
/// responsive.
const MAX_EVENTS_PER_TICK: usize = 64;
const MAX_EVENT_TIME_PER_TICK: Duration = Duration::from_millis(8);

/// Everything the scan timer touches. `build()` fills it and calls
/// [`install`].
pub(super) struct ScanTimerParts {
    /// Connection, filter, search, sort, InfoBar, operation progress, and
    /// the photo-change and album-count fan-out.
    pub context: PhotoActionContext,
    pub gallery: Rc<grid::Gallery>,
    pub sidebar: gtk::ScrolledWindow,
    pub onboarding: Rc<OnboardingCoordinator>,
    /// Kept alive with the timer.
    pub onboarding_slot: Rc<RefCell<Option<Rc<OnboardingCoordinator>>>>,
    pub scan_job: Rc<RefCell<ScanJobState>>,
    pub scan_receiver: Receiver<ScanUiEvent>,
    pub refresh_prepare_receiver: Receiver<RefreshPrepareEvent>,
    pub start_next_scan: Rc<dyn Fn()>,
    pub start_thumbnail_recovery: Rc<dyn Fn()>,
    pub availability_refresh: Rc<dyn Fn()>,
    pub rebuild_folder_watches: Rc<dyn Fn()>,
    pub thumbnail_recovery_deferred: Rc<Cell<bool>>,
    /// `PIC_WATCH_TRACE`.
    pub watch_trace: bool,
}

/// Start the window's scan timer.
pub(super) fn install(parts: ScanTimerParts) {
    let mut timer = ScanTimer {
        view: ScanView::new(Instant::now()),
        parts,
    };
    glib::timeout_add_local(Duration::from_millis(50), move || {
        timer.tick();
        glib::ControlFlow::Continue
    });
}

struct ScanTimer {
    view: ScanView,
    parts: ScanTimerParts,
}

/// The window as the reducer sees it, read when asked.
struct WindowFacts<'a>(&'a ScanTimerParts);

impl ScanViewFacts for WindowFacts<'_> {
    fn filter(&self) -> sidebar::SidebarFilter {
        self.0.context.filter.get()
    }
    fn search_active(&self) -> bool {
        !self.0.context.search.borrow().is_empty()
    }
    fn scope(&self) -> db::BrowseScope {
        let context = &self.0.context;
        browse_scope(
            &context.connection.borrow(),
            context.filter.get(),
            &context.search.borrow(),
        )
    }
    fn format_enabled(&self, path: &str) -> bool {
        crate::image_format::path_is_enabled(&self.0.context.connection.borrow(), path)
    }
    fn folder_exists(&self, id: i64) -> bool {
        db::folder_exists(&self.0.context.connection.borrow(), id).unwrap_or(true)
    }
    fn recovery_deferred(&self) -> bool {
        self.0.thumbnail_recovery_deferred.get()
    }
    fn pair_mode(&self) -> crate::image_format::RawJpegPairMode {
        crate::image_format::raw_jpeg_pair_mode(&self.0.context.connection.borrow())
    }
    fn has_pair_twin(&self, photo: &db::Photo) -> bool {
        crate::image_format::has_raw_jpeg_twin(&self.0.context.connection.borrow(), photo)
    }
}

impl ScanTimer {
    fn tick(&mut self) {
        let parts = &self.parts;
        // Keep the callback slot alive for this window's event processing.
        let _onboarding_lifetime = &parts.onboarding_slot;
        parts
            .onboarding
            .scan_generation_changed(parts.scan_job.borrow().generation);
        let ui_tick_started = Instant::now();
        // Drain event-triggered recovery requests once the current scan ends.
        // With no request, this checks only a flag and performs no disk probes.
        (parts.start_thumbnail_recovery)();
        let mut priority_thumbnail_paths = Vec::new();
        for path in crate::thumbnail::take_priority_completions() {
            crate::thumbnail_trace::event(
                &path.to_string_lossy(),
                "generation_ui_handoff",
                "delivered_to_event_loop",
            );
            priority_thumbnail_paths.push(path);
        }
        let priority_pending = crate::thumbnail::priority_pending_count();

        self.drain_prepare_events();

        let begin = self.view.begin_tick(
            &self.parts.scan_job.borrow(),
            priority_pending,
            priority_thumbnail_paths.len(),
        );
        self.apply(begin, Instant::now());

        let mut handled_events = 0;
        let mut max_event_queue_wait = Duration::ZERO;
        while handled_events < MAX_EVENTS_PER_TICK
            && ui_tick_started.elapsed() < MAX_EVENT_TIME_PER_TICK
        {
            let Ok(ui_event) = self.parts.scan_receiver.try_recv() else {
                break;
            };
            handled_events += 1;
            max_event_queue_wait = max_event_queue_wait.max(ui_event.queued_at.elapsed());
            let admission = self.view.admit(
                &self.parts.scan_job.borrow(),
                ui_event.generation,
                &ui_event.event,
            );
            self.apply(admission.effects, ui_event.queued_at);
            if !admission.admitted {
                continue;
            }
            self.parts
                .onboarding
                .scan_event(ui_event.generation, &ui_event.event);
            let effects = {
                let mut job = self.parts.scan_job.borrow_mut();
                self.view.on_event(
                    &mut job,
                    &ui_event.event,
                    &WindowFacts(&self.parts),
                    Instant::now(),
                )
            };
            self.apply(effects, ui_event.queued_at);
        }

        let job_generation = self.parts.scan_job.borrow().generation;
        let search_active = !self.parts.context.search.borrow().is_empty();
        let work = self.view.end_tick(
            job_generation,
            priority_thumbnail_paths,
            search_active,
            Instant::now(),
        );
        self.apply_tick_work(work);

        if std::env::var_os("PICASA_TRACE").is_some()
            && (ui_tick_started.elapsed() >= Duration::from_millis(100)
                || max_event_queue_wait >= Duration::from_millis(500))
        {
            eprintln!(
                "PIC_SCAN_UI tick elapsed_ms={} events={} max_queue_ms={} pending_photos={} thumbnail_progress={}/{} priority_pending={} filter={:?}",
                ui_tick_started.elapsed().as_millis(),
                handled_events,
                max_event_queue_wait.as_millis(),
                self.view.pending_photo_count(),
                self.view.scan_count(),
                self.view.thumbnail_total(),
                priority_pending,
                self.parts.context.filter.get()
            );
        }
    }

    fn apply_tick_work(&self, work: super::scan_view::TickWork) {
        let parts = &self.parts;
        if let Some(counts) = work.sidebar_counts {
            sidebar::update_library_counts(&parts.sidebar, counts);
        }
        if !work.append.is_empty() {
            let append_started = Instant::now();
            let appended = work.append.len();
            run_ui_guarded("photo batch append", || {
                parts.gallery.append_photos(&work.append)
            });
            if parts.watch_trace {
                eprintln!(
                    "PIC_WATCH grid append count={appended} remaining={} filter={:?}",
                    self.view.pending_photo_count(),
                    parts.context.filter.get()
                );
            }
            if std::env::var_os("PICASA_TRACE").is_some()
                && append_started.elapsed() >= Duration::from_millis(20)
            {
                eprintln!(
                    "PIC_SCAN_UI photo_batch_append count={appended} elapsed_ms={}",
                    append_started.elapsed().as_millis()
                );
            }
        }
        if !work.thumbnails.is_empty() {
            let paths: Vec<PathBuf> = work.thumbnails;
            run_ui_guarded("targeted thumbnail refresh", || {
                if std::env::var_os("PICASA_TRACE").is_some() {
                    eprintln!("PIC_THUMBNAIL tile_refresh count={}", paths.len());
                }
                parts.gallery.refresh_thumbnails_for_paths(&paths)
            });
        }
    }

    /// Apply one event's effects in order. `queued_at` is when the event was
    /// sent (for trace lines).
    fn apply(&self, effects: Vec<ScanEffect>, queued_at: Instant) {
        let parts = &self.parts;
        let context = &parts.context;
        let progress = &context.operation_progress;
        for effect in effects {
            match effect {
                ScanEffect::ShowProgress => progress.show_if_not_dismissed(),
                ScanEffect::Status(text) => progress.label().set_text(&text),
                ScanEffect::StatusVisible(visible) => progress.root().set_visible(visible),
                ScanEffect::Spinning(spinning) => progress.spinner().set_spinning(spinning),
                ScanEffect::StopButton(visible) => progress.stop().set_visible(visible),
                ScanEffect::HideProgressAfter(delay) => progress.hide_after(delay),
                ScanEffect::AppendSidebarFolder(folder) => {
                    run_ui_guarded("sidebar folder append", || {
                        sidebar::append_folder(
                            &parts.sidebar,
                            &folder,
                            parts.availability_refresh.clone(),
                        )
                    });
                }
                ScanEffect::PhotosChanged(photos) => {
                    photo_changed(context, PhotoChange::Indexed(&photos));
                }
                ScanEffect::PhotosRemoved(ids) => {
                    photo_changed(context, PhotoChange::Removed(&ids))
                }
                ScanEffect::RecountSmartAlbums => refresh_smart_album_counts(context),
                ScanEffect::RefreshAvailability => (parts.availability_refresh)(),
                ScanEffect::ShowAllPhotos => {
                    context.filter.set(sidebar::SidebarFilter::All);
                    restore_section_layout(
                        &context.connection.borrow(),
                        &parts.gallery,
                        &context.info,
                        sidebar::SidebarFilter::All,
                    );
                }
                ScanEffect::RefreshGrid => refresh_grid(
                    &context.connection,
                    context.filter.get(),
                    &context.search.borrow(),
                    context.sort.get(),
                    &parts.gallery,
                ),
                ScanEffect::RebuildFolderStream => refresh_grid_preserving_folder_viewport(
                    &context.connection,
                    context.filter.get(),
                    &context.search.borrow(),
                    context.sort.get(),
                    &parts.gallery,
                ),
                ScanEffect::RecoveryDeferred(deferred) => {
                    parts.thumbnail_recovery_deferred.set(deferred)
                }
                ScanEffect::StartNextScan => (parts.start_next_scan)(),
                ScanEffect::RefreshAvailabilityStats => {
                    crate::settings::refresh_library_availability_stats(context.connection.clone())
                }
                ScanEffect::RedrawFolderRows => {
                    if let Ok(folders) = db::folders(&context.connection.borrow()) {
                        sidebar::refresh_folder_rows(
                            &parts.sidebar,
                            &folders,
                            &parts.availability_refresh,
                        );
                    }
                }
                ScanEffect::RebuildFolderWatches => (parts.rebuild_folder_watches)(),
                ScanEffect::Trace(trace) => self.trace(trace, queued_at),
            }
        }
    }

    fn trace(&self, trace: ScanTrace, queued_at: Instant) {
        let watch_trace = self.parts.watch_trace;
        let picasa_trace = || std::env::var_os("PICASA_TRACE").is_some();
        match trace {
            ScanTrace::WatchIndexed {
                kind,
                filter,
                photos,
                new,
                queued_for_append,
                search_active,
                filters_active,
            } => {
                if watch_trace {
                    eprintln!(
                        "PIC_WATCH grid indexed kind={kind:?} filter={filter:?} photos={photos} new={new} queued_for_append={queued_for_append} search_active={search_active} filters_active={filters_active}",
                    );
                }
            }
            ScanTrace::IndexingFinished { imported, filter } => {
                if picasa_trace() {
                    eprintln!(
                        "PIC_SCAN_UI indexing_finished queued_ms={} imported={} filter={:?}",
                        queued_at.elapsed().as_millis(),
                        imported,
                        filter
                    );
                }
            }
            ScanTrace::ThumbnailsStarted { total } => {
                if picasa_trace() {
                    eprintln!(
                        "PIC_SCAN_UI thumbnails_started queued_ms={} total={total}",
                        queued_at.elapsed().as_millis()
                    );
                }
            }
            ScanTrace::ThumbnailReceived(path) => crate::thumbnail_trace::event(
                &path.to_string_lossy(),
                "generation_ui_handoff",
                "thumbnail_created_event_received",
            ),
            ScanTrace::Failed { path, error } => {
                eprintln!("SCAN FAILED: {}: {}", path.display(), error);
            }
            ScanTrace::Complete { imported, failed } => eprintln!(
                "===== SCAN COMPLETE: imported={} failed={} | progressive gallery updates complete =====",
                imported, failed
            ),
            ScanTrace::WatchFinished {
                kind,
                imported,
                filter,
                refresh_grid,
                folder_stream_rebuild,
                folder_rows_redraw,
            } => {
                if watch_trace {
                    eprintln!(
                        "PIC_WATCH grid finished kind={kind:?} imported={imported} filter={filter:?} refresh_grid={refresh_grid} folder_stream_rebuild={folder_stream_rebuild} folder_rows_redraw={folder_rows_redraw}",
                    );
                }
            }
        }
    }

    /// Job startup: a refresh's folder list or a folder refresh's check
    /// arrives from a worker; start the job or report why not. Unchanged
    /// from `build()`; not part of the reducer.
    fn drain_prepare_events(&self) {
        let parts = &self.parts;
        let scan_job = &parts.scan_job;
        let progress = &parts.context.operation_progress;
        let label = progress.label();
        let spinner = progress.spinner();
        let status_box = progress.root();
        let stop_scan = progress.stop();
        while let Ok(prepare_event) = parts.refresh_prepare_receiver.try_recv() {
            match prepare_event {
                RefreshPrepareEvent::LibraryReady { generation, roots } => {
                    if generation != scan_job.borrow().generation {
                        continue;
                    }
                    let roots = match roots {
                        Ok(roots) => roots.into_iter().collect::<VecDeque<_>>(),
                        Err(error) => {
                            eprintln!("Could not refresh library folders: {error}");
                            let mut job = scan_job.borrow_mut();
                            job.kind = None;
                            job.active = None;
                            job.active_root = None;
                            job.pending.clear();
                            spinner.set_spinning(false);
                            stop_scan.set_visible(false);
                            label.set_text("Could not read library folders");
                            status_box.set_visible(true);
                            progress.hide_after(Duration::from_millis(3000));
                            continue;
                        }
                    };
                    if roots.is_empty() {
                        let mut job = scan_job.borrow_mut();
                        job.kind = None;
                        job.active = None;
                        job.active_root = None;
                        job.pending.clear();
                        spinner.set_spinning(false);
                        stop_scan.set_visible(false);
                        label.set_text("No library folders to refresh");
                        status_box.set_visible(true);
                        progress.hide_after(Duration::from_millis(2500));
                        continue;
                    }
                    let should_start = {
                        let mut job = scan_job.borrow_mut();
                        if job.stop_requested {
                            job.kind = None;
                            job.active = None;
                            job.active_root = None;
                            job.pending.clear();
                            false
                        } else {
                            job.pending = roots;
                            true
                        }
                    };
                    if should_start {
                        (parts.start_next_scan)();
                    } else {
                        spinner.set_spinning(false);
                        stop_scan.set_visible(false);
                        label.set_text("Library refresh stopped");
                        status_box.set_visible(true);
                        progress.hide_after(Duration::from_millis(2500));
                    }
                }
                RefreshPrepareEvent::FolderReady {
                    generation,
                    path,
                    imported_root,
                } => {
                    if generation != scan_job.borrow().generation {
                        continue;
                    }
                    let ok = match imported_root {
                        Ok(ok) => ok,
                        Err(error) => {
                            eprintln!("Could not validate refresh folder: {error}");
                            false
                        }
                    };
                    let should_start = {
                        let mut job = scan_job.borrow_mut();
                        if !ok || job.stop_requested {
                            job.kind = None;
                            job.active = None;
                            job.active_root = None;
                            job.pending.clear();
                            false
                        } else {
                            job.pending.clear();
                            job.pending.push_back(path);
                            true
                        }
                    };
                    if should_start {
                        (parts.start_next_scan)();
                    } else {
                        spinner.set_spinning(false);
                        stop_scan.set_visible(false);
                        label.set_text("Folder refresh stopped");
                        status_box.set_visible(true);
                        progress.hide_after(Duration::from_millis(2500));
                    }
                }
            }
        }
    }
}
