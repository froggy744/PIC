use super::OnboardingPreferences;
use adw::prelude::*;
use gtk4 as gtk;
use libadwaita as adw;
use std::cell::Cell;
use std::rc::Rc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WizardPage {
    Welcome,
    Importing,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
    pub library_has_photos: bool,
    pub discovered: Option<usize>,
    pub running: bool,
    pub warning: Option<String>,
}
pub(crate) struct WizardCallbacks {
    pub choose_folder: Rc<dyn Fn()>,
    pub continue_import: Rc<dyn Fn()>,
    pub tutorial: Rc<dyn Fn()>,
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
    library_has_photos: Cell<bool>,
    header: adw::HeaderBar,
    content: gtk::Box,
    branding: gtk::Box,
    logo: gtk::Image,
    photo_visual: adw::Clamp,
    welcome_spacer: gtk::Box,
    heading: gtk::Label,
    description: gtk::Label,
    folder: gtk::Label,
    path: gtk::Label,
    pub(crate) progress_text: gtk::Label,
    progress: gtk::ProgressBar,
    warning: gtk::Label,
    choose: gtk::Button,
    retry: gtk::Button,
    tutorial: gtk::Button,
    pub(crate) open_library: gtk::Button,
    skip: gtk::Button,
    pub(crate) preference_row: gtk::Box,
    pub(crate) never_show: gtk::CheckButton,
    pub(crate) preference_error: gtk::Label,
}
fn label(text: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.set_wrap(true);
    label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
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
        dialog.set_title("Welcome to PIC");
        dialog.set_content_width(600);
        dialog.set_content_height(520);
        let shell = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let header = adw::HeaderBar::new();
        header.add_css_class("flat");
        shell.append(&header);
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
        let branding = gtk::Box::new(gtk::Orientation::Vertical, 6);
        let logo_texture = gtk::gdk::Texture::from_bytes(&glib::Bytes::from_static(
            include_bytes!("../../icon/pic-128.png"),
        ))
        .expect("bundled PIC logo is valid");
        let logo = gtk::Image::from_paintable(Some(&logo_texture));
        // Welcome camera size
        //  in logical pixels; increase to enlarge it.
        logo.set_pixel_size(155);
        logo.set_halign(gtk::Align::Center);
        branding.append(&heading);
        logo.set_valign(gtk::Align::End);
        // Increase this bottom margin to move the camera upward.
        logo.set_margin_bottom(20);
        logo.add_css_class("welcome-camera");
        let camera_style = gtk::CssProvider::new();
        camera_style
            .load_from_data(".welcome-camera { -gtk-icon-shadow: 0 2px 4px rgba(0, 0, 0, 0.24); }");
        logo.style_context()
            .add_provider(&camera_style, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
        let photos_texture = gtk::gdk::Texture::from_bytes(&glib::Bytes::from_static(
            include_bytes!("../../images/onboarding/welcome-photos.png"),
        ))
        .expect("bundled welcome photos are valid");
        let photos = gtk::Picture::for_paintable(&photos_texture);
        photos.set_can_shrink(true);
        photos.set_content_fit(gtk::ContentFit::Contain);
        photos.set_size_request(-1, 176);
        photos.set_alternative_text(Some(
            "Photo prints of wildflowers, a mountain lake and a beach.",
        ));
        let photo_visual = adw::Clamp::new();
        photo_visual.set_maximum_size(400);
        photo_visual.set_tightening_threshold(400);
        // Reserve room below the prints for the foreground camera, centred
        // across the middle print's lower border.
        let composition = gtk::Overlay::new();
        let photo_canvas = gtk::Box::new(gtk::Orientation::Vertical, 0);
        photo_canvas.append(&photos);
        let lower_space = gtk::Box::new(gtk::Orientation::Vertical, 0);
        lower_space.set_size_request(-1, 32);
        photo_canvas.append(&lower_space);
        composition.set_child(Some(&photo_canvas));
        composition.add_overlay(&logo);
        photo_visual.set_child(Some(&composition));
        photo_visual.set_margin_top(4);
        photo_visual.set_margin_bottom(4);
        content.append(&branding);
        content.append(&photo_visual);
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
        for child in [&description, &folder, &path, &progress_text] {
            content.append(child);
        }
        content.append(&progress);
        content.append(&warning);
        let choose = button("Choose Photos Folder");
        choose.add_css_class("suggested-action");
        choose.add_css_class("pill");
        let retry = button("Continue adding photos");
        let tutorial = gtk::Button::with_label("Tutorial");
        tutorial.add_css_class("flat");
        tutorial.set_halign(gtk::Align::Center);
        let open_library = button("Open Library");
        let skip = button("Skip for now");
        skip.add_css_class("flat");
        skip.add_css_class("caption");
        skip.add_css_class("dim-label");
        for child in [&choose, &tutorial, &retry, &open_library] {
            content.append(child);
        }
        let welcome_spacer = gtk::Box::new(gtk::Orientation::Vertical, 0);
        welcome_spacer.set_vexpand(true);
        content.append(&welcome_spacer);
        let preference_row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        preference_row.set_halign(gtk::Align::Center);
        preference_row.set_margin_top(8);
        let preference_label = label("Don’t show this automatically again");
        preference_label.add_css_class("caption");
        preference_label.add_css_class("dim-label");
        let never_show = gtk::CheckButton::new();
        never_show.set_child(Some(&preference_label));
        never_show.set_valign(gtk::Align::Center);
        never_show.set_active(prefs.never_show);
        never_show.update_property(&[gtk::accessible::Property::Label(
            "Don’t show this automatically again",
        )]);
        preference_row.append(&skip);
        preference_row.append(&never_show);
        content.append(&preference_row);
        let preference_error = label("");
        preference_error.add_css_class("error");
        preference_error.add_css_class("caption");
        preference_error.set_visible(false);
        content.append(&preference_error);
        let scroll = gtk::ScrolledWindow::new();
        scroll.set_hscrollbar_policy(gtk::PolicyType::Never);
        scroll.set_vexpand(true);
        let viewport = gtk::Viewport::new(None::<&gtk::Adjustment>, None::<&gtk::Adjustment>);
        viewport.set_scroll_to_focus(true);
        viewport.set_child(Some(&content));
        scroll.set_child(Some(&viewport));
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
            library_has_photos: Cell::new(false),
            header,
            content,
            branding,
            logo,
            photo_visual,
            welcome_spacer,
            heading,
            description,
            folder,
            path,
            progress_text,
            progress,
            warning,
            choose,
            retry,
            tutorial,
            open_library,
            skip,
            preference_row,
            never_show,
            preference_error,
        });
        let choose = callbacks.choose_folder;
        wizard.choose.connect_clicked(move |_| choose());
        let tutorial = callbacks.tutorial;
        wizard.tutorial.connect_clicked(move |_| tutorial());
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
            let width = if parent.width() > 0 {
                parent.width()
            } else {
                parent.default_width()
            };
            let height = if parent.height() > 0 {
                parent.height()
            } else {
                parent.default_height()
            };
            self.dialog.set_content_width(width.clamp(1, 600));
            let preferred_height = if self.page.get() == WizardPage::Welcome {
                520
            } else {
                400
            };
            self.dialog
                .set_content_height(height.clamp(1, preferred_height));
            self.visible.set(true);
            self.dialog.present(Some(&parent));
        }
    }
    pub(crate) fn connect_closed(&self, callback: impl Fn() + 'static) {
        self.dialog.connect_closed(move |_| callback());
    }
    pub(crate) fn close(&self) {
        self.visible.set(false);
        self.dialog.close();
    }
    pub(crate) fn is_visible(&self) -> bool {
        self.visible.get()
    }
    pub(crate) fn is_welcome(&self) -> bool {
        self.page.get() == WizardPage::Welcome
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
        self.library_has_photos.set(data.library_has_photos);
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
        self.dialog.set_title(if welcome {
            "Welcome to PIC"
        } else {
            "Adding photos"
        });
        self.header.set_show_title(!welcome);
        self.logo.set_visible(welcome);
        self.photo_visual.set_visible(welcome);
        self.welcome_spacer.set_visible(welcome);
        self.content.set_spacing(if welcome { 8 } else { 12 });
        self.content.set_margin_top(if welcome { 4 } else { 24 });
        self.content
            .set_margin_bottom(if welcome { 16 } else { 24 });
        self.branding.set_halign(if welcome {
            gtk::Align::Center
        } else {
            gtk::Align::Fill
        });
        self.heading.set_xalign(if welcome { 0.5 } else { 0.0 });
        self.description.set_xalign(if welcome { 0.5 } else { 0.0 });
        self.description.set_justify(if welcome {
            gtk::Justification::Center
        } else {
            gtk::Justification::Left
        });
        self.choose.set_halign(if welcome {
            gtk::Align::Center
        } else {
            gtk::Align::Fill
        });
        self.choose.set_margin_top(if welcome { 8 } else { 0 });
        self.skip.set_halign(gtk::Align::Center);
        if welcome {
            self.choose.add_css_class("pill");
        } else {
            self.choose.remove_css_class("pill");
        }
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
        self.description.set_text(if welcome {
            "All your photos. One place. On your computer."
        } else {
            match mode {
                ImportPresentation::Recovery => "PIC was adding photos from:",
                ImportPresentation::Empty => "PIC couldn’t find supported photos in this folder.",
                _ => "You can start browsing as soon as photos are added.",
            }
        });
        self.preference_row.set_visible(welcome);
        self.preference_error
            .set_visible(welcome && !self.preference_error.text().is_empty());
        self.choose.set_label(if welcome {
            "Choose Photos Folder"
        } else {
            "Choose another folder"
        });
        self.choose.set_sensitive(!self.running.get());
        self.choose
            .set_visible(welcome || (!self.running.get() && mode != ImportPresentation::Adding));
        self.skip.set_visible(welcome);
        self.tutorial.set_visible(welcome);
        self.open_library.set_visible(!welcome);
        self.open_library.set_sensitive(
            self.indexed.get() > 0
                || self.library_has_photos.get()
                || mode != ImportPresentation::Adding,
        );
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

pub(crate) struct GettingStartedTips {
    root: gtk::Box,
    error: gtk::Label,
}
impl GettingStartedTips {
    pub(crate) fn new(on_finish: Rc<dyn Fn()>, on_tour: Rc<dyn Fn()>) -> Self {
        let root = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        root.add_css_class("card");
        root.set_margin_start(12);
        root.set_margin_end(12);
        root.set_margin_top(8);
        root.set_margin_bottom(8);
        let copy = gtk::Box::new(gtk::Orientation::Vertical, 4);
        copy.set_hexpand(true);
        copy.set_valign(gtk::Align::Center);
        copy.set_margin_start(16);
        copy.set_margin_top(10);
        copy.set_margin_bottom(10);
        let description =
            label("Discover PIC · Browse, create albums, edit photos and make collages.");
        description.add_css_class("caption");
        copy.append(&description);
        let error = label("");
        error.add_css_class("error");
        error.add_css_class("caption");
        error.set_visible(false);
        copy.append(&error);
        root.append(&copy);
        let tour = gtk::Button::with_label("Take a tour");
        tour.add_css_class("flat");
        tour.set_valign(gtk::Align::Center);
        tour.connect_clicked(move |_| on_tour());
        root.append(&tour);
        let done = button("×");
        done.add_css_class("flat");
        done.set_valign(gtk::Align::Center);
        done.set_margin_end(8);
        done.set_tooltip_text(Some("Dismiss Discover PIC"));
        done.update_property(&[gtk::accessible::Property::Label("Dismiss Discover PIC")]);
        done.connect_clicked(move |_| on_finish());
        root.append(&done);
        Self { root, error }
    }
    pub(crate) fn widget(&self) -> gtk::Widget {
        self.root.clone().upcast()
    }
    pub(crate) fn dismiss(&self) {
        self.root.set_visible(false);
    }
    pub(crate) fn show_error(&self, error: &str) {
        self.error
            .set_text(&format!("Could not save dismissal: {error}"));
        self.error.set_visible(true);
    }
}
