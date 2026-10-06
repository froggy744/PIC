use crate::onboarding::view::*;
use crate::onboarding::*;
use crate::scanner::ScanEvent;
use adw::prelude::*;
use gtk4 as gtk;
use libadwaita as adw;
use rusqlite::Connection;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ImportTicket {
    pub generation: u64,
    pub root: String,
}
struct ImportTracker {
    ticket: Option<ImportTicket>,
    progress: WizardProgress,
    mode: ImportPresentation,
    successful: bool,
    matching_root: bool,
}
impl Default for ImportTracker {
    fn default() -> Self {
        Self {
            ticket: None,
            progress: WizardProgress::default(),
            mode: ImportPresentation::Adding,
            successful: false,
            matching_root: true,
        }
    }
}
impl ImportTracker {
    fn superseded(&mut self, generation: u64) -> bool {
        if self.progress.running
            && self
                .ticket
                .as_ref()
                .is_some_and(|ticket| ticket.generation != generation)
        {
            self.ticket = None;
            self.progress.running = false;
            self.mode = ImportPresentation::Recovery;
            self.progress.warning =
                Some("Adding photos was stopped. You can continue later.".into());
            return true;
        }
        false
    }
    fn start(&mut self, ticket: ImportTicket) {
        self.progress = WizardProgress {
            root: ticket.root.clone(),
            running: true,
            ..Default::default()
        };
        self.ticket = Some(ticket);
        self.mode = ImportPresentation::Adding;
        self.successful = false;
        self.matching_root = true;
    }
    fn finish(&mut self, failed: usize) {
        self.progress.running = false;
        self.successful = failed == 0 && self.progress.indexed > 0;
        self.mode = if failed > 0 {
            ImportPresentation::Error
        } else if self.progress.indexed == 0 {
            ImportPresentation::Empty
        } else {
            ImportPresentation::Adding
        };
        if failed > 0 && self.progress.warning.is_none() {
            self.progress.warning = Some("Could not read some files".into());
        }
    }
    fn event(&mut self, generation: u64, event: &ScanEvent) -> bool {
        let Some(ticket) = &self.ticket else {
            return false;
        };
        if generation != ticket.generation {
            return false;
        }
        if let ScanEvent::Started { root } = event {
            self.matching_root = root.to_string_lossy() == ticket.root;
        }
        if !self.matching_root {
            return false;
        }
        match event {
            ScanEvent::DiscoveryProgress { found } => self.progress.discovered = Some(*found),
            ScanEvent::PhotosIndexed { photos, .. } => {
                self.progress.indexed += photos
                    .iter()
                    .filter(|item| {
                        !item.photo.trashed
                            && item.photo.width.unwrap_or(0) > 0
                            && item.photo.height.unwrap_or(0) > 0
                    })
                    .count()
            }
            ScanEvent::Failed { error, .. } => {
                self.progress.warning = Some(format!("Could not read some files: {error}"))
            }
            ScanEvent::Finished { failed, .. } => self.finish(*failed),
            ScanEvent::Cancelled { .. } => {
                self.progress.running = false;
                self.mode = ImportPresentation::Recovery;
                self.progress.warning =
                    Some("Adding photos was stopped. You can continue later.".into());
            }
            _ => {}
        }
        true
    }
}

pub(super) struct OnboardingCoordinator {
    window: glib::WeakRef<adw::ApplicationWindow>,
    connection: Rc<RefCell<Connection>>,
    wizard: RefCell<Option<Rc<StartupWizard>>>,
    tracker: RefCell<ImportTracker>,
    choose_folder: Rc<dyn Fn()>,
    continue_import: Rc<dyn Fn(String)>,
    browse: Rc<dyn Fn()>,
    picker_epoch: Cell<u64>,
    pending_picker: Cell<Option<u64>>,
    entered_library: Cell<bool>,
    tips_host: glib::WeakRef<gtk::Box>,
    library_surfaces: RefCell<Vec<(glib::WeakRef<gtk::Widget>, bool)>>,
    library_hidden: Cell<bool>,
    tips: RefCell<Option<GettingStartedTips>>,
    self_weak: RefCell<std::rc::Weak<Self>>,
    scan_job: RefCell<std::rc::Weak<RefCell<super::ScanJobState>>>,
}
impl OnboardingCoordinator {
    pub(super) fn new(
        window: &adw::ApplicationWindow,
        connection: Rc<RefCell<Connection>>,
        choose_folder: Rc<dyn Fn()>,
        continue_import: Rc<dyn Fn(String)>,
        open_library: Rc<dyn Fn()>,
    ) -> Rc<Self> {
        let owner = Rc::new(Self {
            window: window.downgrade(),
            connection,
            wizard: RefCell::new(None),
            tracker: RefCell::new(ImportTracker::default()),
            choose_folder,
            continue_import,
            browse: open_library,
            picker_epoch: Cell::new(0),
            pending_picker: Cell::new(None),
            entered_library: Cell::new(false),
            tips_host: glib::WeakRef::new(),
            library_surfaces: RefCell::new(Vec::new()),
            library_hidden: Cell::new(false),
            tips: RefCell::new(None),
            self_weak: RefCell::new(std::rc::Weak::new()),
            scan_job: RefCell::new(std::rc::Weak::new()),
        });
        owner.self_weak.replace(Rc::downgrade(&owner));
        owner
    }
    pub(super) fn bind_scan_job(&self, job: &Rc<RefCell<super::ScanJobState>>) {
        self.scan_job.replace(Rc::downgrade(job));
    }
    fn ensure_wizard(self: &Rc<Self>) -> Option<Rc<StartupWizard>> {
        if let Some(wizard) = self.wizard.borrow().as_ref() {
            return Some(wizard.clone());
        }
        let window = self.window.upgrade()?;
        let prefs = load_preferences(&self.connection.borrow()).ok()?;
        let weak = Rc::downgrade(self);
        let choose = weak.clone();
        let retry = weak.clone();
        let open = weak.clone();
        let skip = weak.clone();
        let wizard = StartupWizard::new(
            &window,
            &prefs,
            WizardCallbacks {
                choose_folder: Rc::new(move || {
                    if let Some(owner) = choose.upgrade() {
                        if owner.pending_picker.get().is_some() || owner.is_running() {
                            return;
                        }
                        let token = owner.picker_epoch.get().wrapping_add(1);
                        owner.picker_epoch.set(token);
                        owner.pending_picker.set(Some(token));
                        (owner.choose_folder)();
                    }
                }),
                continue_import: Rc::new(move || {
                    if let Some(owner) = retry.upgrade() {
                        owner.resume();
                    }
                }),
                open_library: Rc::new(move || {
                    if let Some(owner) = open.upgrade() {
                        owner.enter_library();
                    }
                }),
                skip: Rc::new(move || {
                    if let Some(owner) = skip.upgrade() {
                        owner.close();
                    }
                }),
                set_never_show: Rc::new(move |value| {
                    let owner = weak
                        .upgrade()
                        .ok_or_else(|| "The window was closed".to_string())?;
                    let result = save_never_show(&owner.connection.borrow(), value)
                        .map_err(|error| error.to_string());
                    result
                }),
            },
        );
        let weak = Rc::downgrade(self);
        wizard.connect_closed(move || {
            if let Some(owner) = weak.upgrade() {
                owner.library_dialog_closed();
            }
        });
        self.wizard.replace(Some(wizard.clone()));
        Some(wizard)
    }
    pub(super) fn is_visible(&self) -> bool {
        self.wizard
            .borrow()
            .as_ref()
            .is_some_and(|view| view.is_visible())
    }
    pub(super) fn is_running(&self) -> bool {
        self.tracker.borrow().progress.running
    }
    pub(super) fn picker_token(&self) -> Option<u64> {
        self.pending_picker.get().filter(|_| self.is_visible())
    }
    pub(super) fn picker_result_is_current(&self, token: u64) -> bool {
        self.pending_picker.get() == Some(token)
            && self.is_visible()
            && self
                .window
                .upgrade()
                .is_some_and(|window| window.is_visible())
    }
    pub(super) fn picker_cancelled(&self) {
        self.pending_picker.set(None);
    }
    pub(super) fn close(&self) {
        self.pending_picker.set(None);
        if let Some(wizard) = self.wizard.borrow().as_ref() {
            wizard.close();
        }
        self.library_dialog_closed();
    }
    pub(super) fn install_library_surface(&self, surface: &impl IsA<gtk::Widget>) {
        self.library_surfaces
            .borrow_mut()
            .push((surface.as_ref().downgrade(), surface.as_ref().is_visible()));
    }
    fn present_wizard(&self, wizard: &StartupWizard) {
        if !self.library_hidden.replace(true) {
            for (surface, was_visible) in self.library_surfaces.borrow_mut().iter_mut() {
                if let Some(surface) = surface.upgrade() {
                    *was_visible = surface.is_visible();
                    surface.set_visible(false);
                }
            }
        }
        if let Some(tips) = self.tips.borrow().as_ref() {
            tips.widget().set_visible(false);
        }
        wizard.present();
    }
    fn library_dialog_closed(&self) {
        if self.library_hidden.replace(false) {
            for (surface, was_visible) in self.library_surfaces.borrow().iter() {
                if let Some(surface) = surface.upgrade() {
                    surface.set_visible(*was_visible);
                }
            }
        }
        self.entered_library.set(true);
        self.show_tips_if_eligible();
    }
    pub(super) fn folder_selected(&self, root: &str) -> Result<(), String> {
        // The chooser has completed even if saving this selection fails.
        // Keep its runtime path so Try again can retry the same folder.
        self.pending_picker.set(None);
        self.tracker.replace(ImportTracker {
            progress: WizardProgress {
                root: root.into(),
                ..Default::default()
            },
            ..Default::default()
        });
        save_stage(
            &self.connection.borrow(),
            OnboardingStage::Importing,
            Some(root),
        )
        .map_err(|error| error.to_string())?;
        self.entered_library.set(false);
        (self.browse)();
        self.render();
        Ok(())
    }
    pub(super) fn import_started(&self, ticket: ImportTicket) {
        self.tracker.borrow_mut().start(ticket);
        self.refresh_indexed_counts();
        if self.is_visible() {
            self.render();
        }
    }
    fn attach_active_root(&self, root: &str) -> bool {
        let Some(job) = self.scan_job.borrow().upgrade() else {
            return false;
        };
        let ticket = {
            let job = job.borrow();
            if job.kind != Some(super::ScanJobKind::Import)
                || job.active.is_none()
                || job.active_root.as_deref() != Some(root)
            {
                return false;
            }
            ImportTicket {
                generation: job.generation,
                root: root.into(),
            }
        };
        if self.tracker.borrow().ticket.as_ref() != Some(&ticket) {
            self.import_started(ticket);
        }
        true
    }
    fn reuse_matching_active_import(&self) -> bool {
        let Some(job) = self.scan_job.borrow().upgrade() else {
            return false;
        };
        let generation = job.borrow().generation;
        self.scan_generation_changed(generation);
        let Ok(prefs) = load_preferences(&self.connection.borrow()) else {
            return false;
        };
        if prefs.stage != Some(OnboardingStage::Importing) {
            return false;
        }
        prefs
            .root
            .is_some_and(|root| self.attach_active_root(&root))
    }
    pub(super) fn import_error(&self, message: &str) {
        let mut tracker = self.tracker.borrow_mut();
        tracker.mode = ImportPresentation::Error;
        tracker.progress.running = false;
        tracker.progress.warning = Some(message.into());
        drop(tracker);
        self.render();
    }
    fn usable_photos(&self) -> usize {
        self.connection
            .borrow()
            .query_row(
                "SELECT COUNT(*) FROM photos WHERE trashed=0 AND width>0 AND height>0",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap_or(0) as usize
    }
    fn usable_photos_in_root(&self, root: &str) -> usize {
        if root.is_empty() {
            return 0;
        }
        let separator = if root.contains("://") {
            '/'
        } else {
            std::path::MAIN_SEPARATOR
        };
        let prefix = if root.ends_with(separator) {
            root.to_string()
        } else {
            format!("{root}{separator}")
        };
        let upper = format!("{prefix}\u{10ffff}");
        self.connection.borrow().query_row(
            "SELECT COUNT(*) FROM photos WHERE trashed=0 AND width>0 AND height>0 AND (path=?1 OR (path>=?2 AND path<?3))",
            rusqlite::params![root, prefix, upper], |row| row.get::<_, i64>(0),
        ).unwrap_or(0) as usize
    }
    fn refresh_indexed_counts(&self) {
        let root = self.tracker.borrow().progress.root.clone();
        let indexed = self.usable_photos_in_root(&root);
        let library_has_photos = self.usable_photos() > 0;
        let mut tracker = self.tracker.borrow_mut();
        tracker.progress.indexed = indexed;
        tracker.progress.library_has_photos = library_has_photos;
    }
    fn render(&self) {
        if let Some(wizard) = self.wizard.borrow().as_ref() {
            let tracker = self.tracker.borrow();
            wizard.set_page(WizardPage::Importing);
            wizard.set_import_presentation(tracker.mode);
            wizard.set_progress(&tracker.progress);
        }
    }
    fn resume(&self) {
        self.reuse_matching_active_import();
        if self.is_running() {
            self.render();
            return;
        }
        let selected_root = self.tracker.borrow().progress.root.clone();
        let root = if selected_root.is_empty() {
            load_preferences(&self.connection.borrow())
                .ok()
                .and_then(|prefs| prefs.root)
        } else {
            Some(selected_root)
        };
        if let Some(root) = root {
            (self.continue_import)(root);
        } else if self.pending_picker.get().is_none() {
            let token = self.picker_epoch.get().wrapping_add(1);
            self.picker_epoch.set(token);
            self.pending_picker.set(Some(token));
            (self.choose_folder)();
        }
    }
    fn enter_library(&self) {
        self.entered_library.set(true);
        self.close();
        (self.browse)();
        self.show_tips_if_eligible();
    }
    pub(super) fn present_on_startup(self: &Rc<Self>) {
        let decision = {
            let conn = self.connection.borrow();
            load_preferences(&conn).map(|prefs| startup_decision(&prefs))
        };
        match decision {
            Ok(StartupDecision::Welcome) => {
                if let Some(wizard) = self.ensure_wizard() {
                    let can_resume = load_preferences(&self.connection.borrow())
                        .ok()
                        .is_some_and(|prefs| {
                            prefs.stage == Some(OnboardingStage::Importing) && prefs.root.is_some()
                        });
                    wizard.set_resume_available(can_resume);
                    wizard.set_page(WizardPage::Welcome);
                    self.present_wizard(&wizard);
                }
            }
            _ => {}
        }
    }
    pub(super) fn present_manually(self: &Rc<Self>) {
        if self.is_visible() {
            if let Some(wizard) = self.wizard.borrow().as_ref() {
                self.present_wizard(&wizard);
            }
            return;
        }
        self.pending_picker.set(None);
        self.reuse_matching_active_import();
        if self.is_running() {
            if let Some(wizard) = self.ensure_wizard() {
                self.render();
                self.present_wizard(&wizard);
            }
            return;
        }
        let prefs = load_preferences(&self.connection.borrow()).ok();
        if prefs.is_some_and(|prefs| prefs.stage == Some(OnboardingStage::Importing)) {
            self.present_recovery();
        } else if let Some(wizard) = self.ensure_wizard() {
            self.pending_picker.set(None);
            wizard.set_resume_available(false);
            wizard.set_page(WizardPage::Welcome);
            self.present_wizard(&wizard);
        }
    }
    fn present_recovery(self: &Rc<Self>) {
        self.reuse_matching_active_import();
        let Some(wizard) = self.ensure_wizard() else {
            return;
        };
        if !self.is_running() {
            let root = load_preferences(&self.connection.borrow())
                .ok()
                .and_then(|prefs| prefs.root)
                .unwrap_or_default();
            self.tracker.replace(ImportTracker {
                progress: WizardProgress {
                    indexed: self.usable_photos_in_root(&root),
                    library_has_photos: self.usable_photos() > 0,
                    root,
                    ..Default::default()
                },
                mode: ImportPresentation::Recovery,
                ..Default::default()
            });
        }
        self.render();
        self.present_wizard(&wizard);
    }
    pub(super) fn scan_generation_changed(&self, generation: u64) {
        let changed = self.tracker.borrow_mut().superseded(generation);
        if changed && self.is_visible() {
            self.render();
        }
    }
    pub(super) fn scan_event(&self, generation: u64, event: &ScanEvent) {
        if matches!(event, ScanEvent::Started { .. }) {
            self.reuse_matching_active_import();
        }
        if !self.tracker.borrow_mut().event(generation, event) {
            return;
        }
        // DB batches are committed before events. Include pre-existing usable
        // photos on retry without counting updates as newly usable photos.
        if matches!(
            event,
            ScanEvent::PhotosIndexed { .. }
                | ScanEvent::PhotosRemoved { .. }
                | ScanEvent::Finished { .. }
                | ScanEvent::Cancelled { .. }
        ) {
            self.refresh_indexed_counts();
            if let ScanEvent::Finished { failed, .. } = event {
                self.tracker.borrow_mut().finish(*failed);
            }
        }
        if self.is_visible() {
            self.render();
        }
        if self.tracker.borrow().successful {
            self.persist_completion();
        }
        if self.entered_library.get() {
            self.show_tips_if_eligible();
        }
    }
    pub(super) fn install_tips_host(&self, host: &gtk::Box) {
        self.tips_host.set(Some(host));
    }
    pub(super) fn show_tips_if_eligible(&self) {
        if !self.entered_library.get() || self.is_visible() {
            return;
        }
        let Ok(prefs) = load_preferences(&self.connection.borrow()) else {
            return;
        };
        if prefs.never_show || prefs.tips_dismissed {
            return;
        }
        if let Some(tips) = self.tips.borrow().as_ref() {
            tips.widget().set_visible(true);
            return;
        }
        let Some(host) = self.tips_host.upgrade() else {
            return;
        };
        let weak = self.self_weak.borrow().clone();
        let tour_owner = self.self_weak.borrow().clone();
        let tips = GettingStartedTips::new(
            Rc::new(move || {
                if let Some(owner) = weak.upgrade() {
                    owner.dismiss_library_tips();
                }
            }),
            Rc::new(move || {
                if let Some(owner) = tour_owner.upgrade() {
                    if let Some(window) = owner.window.upgrade() {
                        crate::onboarding::tour::present(&window);
                    }
                }
            }),
        );
        host.append(&tips.widget());
        self.tips.replace(Some(tips));
    }
    pub(super) fn dismiss_library_tips(&self) {
        if !self
            .tips
            .borrow()
            .as_ref()
            .is_some_and(|tips| tips.widget().is_visible())
        {
            return;
        }
        let result = dismiss_tips(&self.connection.borrow());
        if let Err(error) = result {
            if let Some(tips) = self.tips.borrow().as_ref() {
                tips.show_error(&error.to_string());
            }
            return;
        }
        if let Some(tips) = self.tips.borrow().as_ref() {
            tips.dismiss();
        }
        if self.tracker.borrow().successful
            || load_preferences(&self.connection.borrow())
                .ok()
                .is_some_and(|prefs| prefs.stage == Some(OnboardingStage::Tips))
        {
            self.persist_completion();
        }
    }
    fn persist_completion(&self) {
        let result = (|| {
            let conn = self.connection.borrow();
            let prefs = load_preferences(&conn)?;
            if prefs.stage == Some(OnboardingStage::Complete) {
                return Ok(());
            }
            if prefs.never_show || prefs.tips_dismissed {
                finish_onboarding(&conn)
            } else {
                save_stage(&conn, OnboardingStage::Tips, prefs.root.as_deref())
            }
        })();
        if let Err(error) = result {
            eprintln!("Could not save onboarding completion: {error}");
        }
    }
    pub(super) fn library_changed(&self) {
        self.close();
        self.wizard.replace(None);
        self.tracker.replace(ImportTracker::default());
        self.entered_library.set(false);
        if let Some(tips) = self.tips.borrow_mut().take() {
            if let Some(host) = self.tips_host.upgrade() {
                host.remove(&tips.widget());
            }
        }
    }
}
#[cfg(test)]
mod tests;

pub(super) fn queue_local_import(
    connection: &Rc<RefCell<Connection>>,
    job: &Rc<RefCell<super::ScanJobState>>,
    root: &str,
    owner: Option<&Rc<OnboardingCoordinator>>,
) -> Result<bool, String> {
    if let Some(owner) = owner {
        owner.bind_scan_job(job);
        owner.reuse_matching_active_import();
        if owner.is_running() {
            return Ok(false);
        }
        owner.folder_selected(root)?;
        if owner.attach_active_root(root) {
            return Ok(false);
        }
    }
    crate::db::mark_import_root(&connection.borrow(), root).map_err(|error| error.to_string())?;
    let generation = {
        let mut job = job.borrow_mut();
        let generation = job
            .authorize_photo_scan(super::PhotoScanRequestReason::ImportFolder)
            .expect("local import is an authorized scan reason");
        job.pending.push_back(root.to_string());
        generation
    };
    if let Some(owner) = owner {
        owner.import_started(ImportTicket {
            generation,
            root: root.into(),
        });
    }
    Ok(true)
}
