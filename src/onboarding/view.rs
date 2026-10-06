use super::OnboardingPreferences;
use adw::prelude::*;
use gtk4 as gtk;
use libadwaita as adw;
use std::cell::Cell;
use std::rc::Rc;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum WizardPage {
    Welcome,
    Importing,
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ImportPresentation {
    Adding,
    Recovery,
    Empty,
    Error,
}
#[derive(Clone, Default)]
pub(crate) struct WizardProgress {
    pub root: String,
    pub indexed: usize,
    pub discovered: Option<usize>,
    pub running: bool,
    pub warning: Option<String>,
}
pub(crate) struct WizardCallbacks {
    pub choose_folder: Rc<dyn Fn()>,
    pub continue_import: Rc<dyn Fn()>,
    pub open_library: Rc<dyn Fn()>,
    pub skip: Rc<dyn Fn()>,
    pub set_never_show: Rc<dyn Fn(bool) -> Result<(), String>>,
}
pub(crate) struct StartupWizard {
    dialog: adw::Dialog,
    parent: glib::WeakRef<adw::ApplicationWindow>,
    visible: Rc<Cell<bool>>,
    page: Cell<WizardPage>,
    mode: Cell<ImportPresentation>,
    running: Cell<bool>,
    indexed: Cell<usize>,
    heading: gtk::Label,
    description: gtk::Label,
    folder: gtk::Label,
    path: gtk::Label,
    pub(crate) progress_text: gtk::Label,
    progress: gtk::ProgressBar,
    warning: gtk::Label,
    choose: gtk::Button,
    retry: gtk::Button,
    pub(crate) open_library: gtk::Button,
    skip: gtk::Button,
    pub(crate) preference_row: gtk::Box,
    pub(crate) never_show: gtk::Switch,
    pub(crate) preference_error: gtk::Label,
}
fn label(text: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.set_wrap(true);
    label.set_xalign(0.0);
    label.set_hexpand(true);
    label
}
fn button(text: &str) -> gtk::Button {
    let button = gtk::Button::with_label(text);
    if let Some(label) = button.child().and_downcast::<gtk::Label>() {
        label.set_wrap(true);
    }
    button
}
impl StartupWizard {
    pub(crate) fn new(
        parent: &adw::ApplicationWindow,
        prefs: &OnboardingPreferences,
        callbacks: WizardCallbacks,
    ) -> Rc<Self> {
        let dialog = adw::Dialog::new();
        dialog.set_title("Getting started");
        dialog.set_content_width(600);
        dialog.set_content_height(400);
        let shell = gtk::Box::new(gtk::Orientation::Vertical, 0);
        shell.append(&adw::HeaderBar::new());
        let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
        for set in [
            gtk::prelude::WidgetExt::set_margin_start,
            gtk::prelude::WidgetExt::set_margin_end,
            gtk::prelude::WidgetExt::set_margin_top,
            gtk::prelude::WidgetExt::set_margin_bottom,
        ] {
            set(&content, 24);
        }
        let heading = label("Welcome to PIC");
        heading.add_css_class("title-1");
        let description = label("");
        let folder = label("");
        folder.add_css_class("heading");
        let path = label("");
        path.add_css_class("dim-label");
        path.add_css_class("caption");
        path.set_selectable(true);
        let progress_text = label("");
        let progress = gtk::ProgressBar::new();
        let warning = label("");
        warning.add_css_class("error");
        for child in [&heading, &description, &folder, &path, &progress_text] {
            content.append(child);
        }
        content.append(&progress);
        content.append(&warning);
        let choose = button("Choose Photos Folder");
        choose.add_css_class("suggested-action");
        let retry = button("Continue adding photos");
        let open_library = button("Open Library");
        let skip = button("Skip for now");
        skip.add_css_class("flat");
        for child in [&choose, &retry, &open_library, &skip] {
            content.append(child);
        }
        let preference_row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        let preference_label = label("Don’t show this automatically again");
        let never_show = gtk::Switch::new();
        never_show.set_valign(gtk::Align::Center);
        never_show.set_active(prefs.never_show);
        never_show.update_property(&[gtk::accessible::Property::Label(
            "Don’t show this automatically again",
        )]);
        preference_row.append(&preference_label);
        preference_row.append(&never_show);
        content.append(&preference_row);
        let preference_error = label("");
        preference_error.add_css_class("error");
        preference_error.set_visible(false);
        content.append(&preference_error);
        let scroll = gtk::ScrolledWindow::new();
        scroll.set_hscrollbar_policy(gtk::PolicyType::Never);
        scroll.set_vexpand(true);
        scroll.set_child(Some(&content));
        shell.append(&scroll);
        dialog.set_child(Some(&shell));
        let visible = Rc::new(Cell::new(false));
        let visibility = visible.clone();
        dialog.connect_closed(move |_| visibility.set(false));
        let wizard = Rc::new(Self {
            dialog,
            parent: parent.downgrade(),
            visible,
            page: Cell::new(WizardPage::Welcome),
            mode: Cell::new(ImportPresentation::Adding),
            running: Cell::new(false),
            indexed: Cell::new(0),
            heading,
            description,
            folder,
            path,
            progress_text,
            progress,
            warning,
            choose,
            retry,
            open_library,
            skip,
            preference_row,
            never_show,
            preference_error,
        });
        let choose = callbacks.choose_folder;
        wizard.choose.connect_clicked(move |_| choose());
        let retry = callbacks.continue_import;
        wizard.retry.connect_clicked(move |_| retry());
        let open = callbacks.open_library;
        wizard.open_library.connect_clicked(move |_| open());
        let skip = callbacks.skip;
        wizard.skip.connect_clicked(move |_| skip());
        let weak = Rc::downgrade(&wizard);
        let saved = Cell::new(prefs.never_show);
        let reverting = Cell::new(false);
        wizard.never_show.connect_active_notify(move |toggle| {
            if reverting.get() {
                return;
            }
            let value = toggle.is_active();
            let Some(wizard) = weak.upgrade() else {
                return;
            };
            match (callbacks.set_never_show)(value) {
                Ok(()) => {
                    saved.set(value);
                    wizard.preference_error.set_visible(false);
                }
                Err(error) => {
                    reverting.set(true);
                    toggle.set_active(saved.get());
                    reverting.set(false);
                    wizard
                        .preference_error
                        .set_text(&format!("Could not save this preference: {error}"));
                    wizard.preference_error.set_visible(true);
                }
            }
        });
        wizard.refresh();
        wizard
    }
    pub(crate) fn present(&self) {
        if let Some(parent) = self.parent.upgrade() {
            self.visible.set(true);
            self.dialog.present(Some(&parent));
        }
    }
    pub(crate) fn close(&self) {
        self.visible.set(false);
        self.dialog.close();
    }
    pub(crate) fn is_visible(&self) -> bool {
        self.visible.get()
    }
    pub(crate) fn set_page(&self, page: WizardPage) {
        self.page.set(page);
        self.refresh();
    }
    pub(crate) fn set_import_presentation(&self, mode: ImportPresentation) {
        self.mode.set(mode);
        self.refresh();
    }
    pub(crate) fn set_progress(&self, data: &WizardProgress) {
        self.running.set(data.running);
        self.indexed.set(data.indexed);
        let name = crate::source::filename(&data.root);
        self.folder.set_text(&name);
        self.path.set_text(&data.root);
        self.progress_text.set_text(&match data.discovered {
            Some(count) => format!("{count} photos found · {} added", data.indexed),
            None if data.running => "Finding photos…".into(),
            None => format!("{} photos added", data.indexed),
        });
        if data.running {
            self.progress.pulse();
        }
        self.warning.set_text(data.warning.as_deref().unwrap_or(""));
        self.refresh();
    }
    fn refresh(&self) {
        let welcome = self.page.get() == WizardPage::Welcome;
        let mode = self.mode.get();
        self.heading.set_text(if welcome {
            "Welcome to PIC"
        } else {
            match mode {
                ImportPresentation::Adding => "Adding your photos",
                ImportPresentation::Recovery => "Continue adding photos",
                ImportPresentation::Empty => "No photos found",
                ImportPresentation::Error => "Adding your photos",
            }
        });
        self.description.set_text(if welcome { "Your photos stay on your computer. Choose a folder and PIC will build your photo library.\n\nYou can add more folders later." } else { match mode {
            ImportPresentation::Recovery => "PIC was adding photos from:",
            ImportPresentation::Empty => "PIC couldn’t find supported photos in this folder.",
            _ => "You can start browsing as soon as photos are added.",
        }});
        self.preference_row.set_visible(welcome);
        self.preference_error
            .set_visible(welcome && !self.preference_error.text().is_empty());
        self.choose.set_label(if welcome {
            "Choose Photos Folder"
        } else {
            "Choose another folder"
        });
        self.choose
            .set_visible(welcome || (!self.running.get() && mode != ImportPresentation::Adding));
        self.skip.set_visible(welcome);
        self.open_library.set_visible(!welcome);
        self.open_library
            .set_sensitive(self.indexed.get() > 0 || mode != ImportPresentation::Adding);
        self.retry.set_visible(
            !welcome
                && matches!(
                    mode,
                    ImportPresentation::Recovery | ImportPresentation::Error
                ),
        );
        self.retry.set_sensitive(!self.running.get());
        self.retry.set_label(if mode == ImportPresentation::Error {
            "Try again"
        } else {
            "Continue adding photos"
        });
        for child in [&self.folder, &self.path, &self.progress_text] {
            child.set_visible(!welcome);
        }
        self.progress.set_visible(!welcome && self.running.get());
        self.warning
            .set_visible(!welcome && !self.warning.text().is_empty());
    }
}
