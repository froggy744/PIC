use super::*;
use adw::prelude::*;
use gtk4 as gtk;
use libadwaita as adw;
use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};
use view::*;

fn settle() {
    let context = glib::MainContext::default();
    let until = Instant::now() + Duration::from_millis(120);
    while Instant::now() < until {
        while context.pending() {
            context.iteration(false);
        }
        std::thread::sleep(Duration::from_millis(4));
    }
}
fn parent() -> adw::ApplicationWindow {
    parent_at(800, 600)
}
fn parent_at(width: i32, height: i32) -> adw::ApplicationWindow {
    adw::init().unwrap();
    static NEXT_APP: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let app = adw::Application::builder()
        .application_id(format!(
            "io.pic.OnboardingTests.Run{}",
            NEXT_APP.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ))
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .build();
    app.register(None::<&gio::Cancellable>).unwrap();
    let window = adw::ApplicationWindow::new(&app);
    window.set_resizable(false);
    window.set_default_size(width, height);
    window.set_content(Some(&gtk::Box::new(gtk::Orientation::Vertical, 0)));
    window.present();
    settle();
    window
}
fn callbacks(save: Rc<dyn Fn(bool) -> Result<(), String>>) -> WizardCallbacks {
    WizardCallbacks {
        choose_folder: Rc::new(|| {}),
        continue_import: Rc::new(|| {}),
        open_library: Rc::new(|| {}),
        skip: Rc::new(|| {}),
        set_never_show: save,
    }
}

#[test]
#[ignore = "requires GTK; run individually"]
fn wizard_toggle_persists_without_closing() {
    let parent = parent();
    let saved = Rc::new(Cell::new(false));
    let value = saved.clone();
    let wizard = StartupWizard::new(
        &parent,
        &OnboardingPreferences::default(),
        callbacks(Rc::new(move |v| {
            value.set(v);
            Ok(())
        })),
    );
    wizard.present();
    settle();
    assert_eq!(
        wizard.never_show.accessible_role(),
        gtk::AccessibleRole::Checkbox
    );
    assert!(!wizard.never_show.is_active());
    wizard.never_show.set_active(true);
    settle();
    assert!(saved.get());
    assert!(wizard.is_visible());
    wizard.close();
    parent.close();
}
#[test]
#[ignore = "requires GTK; run individually"]
fn wizard_toggle_write_failure_restores_saved_state() {
    let parent = parent();
    let wizard = StartupWizard::new(
        &parent,
        &OnboardingPreferences::default(),
        callbacks(Rc::new(|_| Err("Disk is read-only".into()))),
    );
    wizard.present();
    wizard.never_show.set_active(true);
    settle();
    assert!(!wizard.never_show.is_active());
    assert!(wizard.preference_error.is_visible());
    assert!(wizard.preference_error.text().contains("Disk is read-only"));
    wizard.close();
    parent.close();
}
#[test]
#[ignore = "requires GTK; run individually"]
fn wizard_preference_is_welcome_only_and_import_variations_allow_exit() {
    let parent = parent();
    let wizard = StartupWizard::new(
        &parent,
        &OnboardingPreferences::default(),
        callbacks(Rc::new(|_| Ok(()))),
    );
    wizard.present();
    assert!(wizard.preference_row.is_visible());
    wizard.set_page(WizardPage::Importing);
    assert!(!wizard.preference_row.is_visible());
    wizard.set_progress(&WizardProgress {
        root: "/photos/folder".into(),
        indexed: 0,
        discovered: Some(327),
        running: true,
        warning: None,
        library_has_photos: false,
    });
    assert!(!wizard.open_library.is_sensitive());
    assert!(wizard.progress_text.text().contains("327"));
    for mode in [
        ImportPresentation::Recovery,
        ImportPresentation::Empty,
        ImportPresentation::Error,
    ] {
        wizard.set_import_presentation(mode);
        assert!(wizard.open_library.is_sensitive());
    }
    wizard.set_import_presentation(ImportPresentation::Adding);
    wizard.set_progress(&WizardProgress {
        root: "/photos/folder".into(),
        indexed: 1,
        discovered: None,
        running: true,
        warning: None,
        library_has_photos: false,
    });
    assert!(wizard.open_library.is_sensitive());
    assert!(!wizard.progress_text.text().contains('%'));
    wizard.close();
    parent.close();
}
#[test]
#[ignore = "requires GTK; run individually"]
fn wizard_repeated_present_uses_one_dialog() {
    let parent = parent();
    let wizard = StartupWizard::new(
        &parent,
        &OnboardingPreferences::default(),
        callbacks(Rc::new(|_| Ok(()))),
    );
    wizard.present();
    settle();
    let root = wizard.open_library.root().unwrap();
    let toplevels = gtk::Window::list_toplevels().len();
    wizard.present();
    settle();
    assert_eq!(wizard.open_library.root().unwrap(), root);
    assert_eq!(gtk::Window::list_toplevels().len(), toplevels);
    assert!(parent.dialogs().n_items() <= 1);
    assert!(wizard.is_visible());
    wizard.close();
    settle();
    assert!(!wizard.is_visible());
    parent.close();
}

#[test]
#[ignore = "requires GTK; run individually"]
fn wizard_narrow_layout_keeps_controls_reachable() {
    for (width, height, dark) in [
        (1920, 1080, false),
        (1366, 768, false),
        (360, 640, false),
        (1920, 1080, true),
        (1366, 768, true),
        (360, 640, true),
    ] {
        adw::init().unwrap();
        adw::StyleManager::default().set_color_scheme(if dark {
            adw::ColorScheme::ForceDark
        } else {
            adw::ColorScheme::ForceLight
        });
        let parent = parent_at(width, height);
        let wizard = StartupWizard::new(
            &parent,
            &OnboardingPreferences::default(),
            callbacks(Rc::new(|_| Ok(()))),
        );
        wizard.present();
        settle();
        assert!(
            wizard.preference_row.width() <= parent.width(),
            "preference width {} exceeds parent {} at target {width}",
            wizard.preference_row.width(),
            parent.width()
        );
        assert!(wizard.never_show.grab_focus());
        settle();
        if let Ok(directory) = std::env::var("PIC_WIZARD_SCREENSHOTS") {
            use gtk::gdk::prelude::PaintableExt;
            let root = wizard
                .never_show
                .root()
                .unwrap()
                .downcast::<gtk::Window>()
                .unwrap();
            let snapshot = gtk::Snapshot::new();
            gtk::WidgetPaintable::new(Some(&root)).snapshot(
                &snapshot,
                root.width() as f64,
                root.height() as f64,
            );
            root.renderer()
                .unwrap()
                .render_texture(&snapshot.to_node().unwrap(), None)
                .save_to_png(format!(
                    "{directory}/welcome-{width}-{}.png",
                    if dark { "dark" } else { "light" }
                ))
                .unwrap();
        }

        wizard.set_page(WizardPage::Importing);
        wizard.set_progress(&WizardProgress {
            root: format!("/photos/{}/holiday", "long_directory_name".repeat(20)),
            indexed: 1,
            running: true,
            ..Default::default()
        });
        settle();
        assert!(wizard.open_library.is_sensitive());
        assert!(wizard.open_library.grab_focus());
        assert!(
            wizard.open_library.width() <= parent.width(),
            "action exceeds parent width"
        );
        settle();
        // Dialog content may be reparented to a native window by libadwaita.
        let dialog = wizard
            .open_library
            .root()
            .unwrap()
            .downcast::<gtk::Window>()
            .unwrap()
            .upcast::<gtk::Widget>();
        let bounds = wizard.open_library.compute_bounds(&dialog).unwrap();
        assert!(bounds.x() >= 0.0 && bounds.y() >= 0.0);
        // The focus outline can extend a few pixels beyond the viewport;
        // the action's label and click target must remain reachable.
        let label_bounds = wizard
            .open_library
            .child()
            .unwrap()
            .compute_bounds(&dialog)
            .unwrap();
        assert!(
            label_bounds.y() >= 0.0
                && label_bounds.y() + label_bounds.height() <= dialog.height() as f32,
            "focused action label is not reachable at {width}px"
        );
        assert!(bounds.y() + bounds.height() / 2.0 < dialog.height() as f32);
        if let Ok(directory) = std::env::var("PIC_WIZARD_SCREENSHOTS") {
            use gtk::gdk::prelude::PaintableExt;
            let snapshot = gtk::Snapshot::new();
            gtk::WidgetPaintable::new(Some(&dialog)).snapshot(
                &snapshot,
                dialog.width() as f64,
                dialog.height() as f64,
            );
            let node = snapshot.to_node().unwrap();
            parent
                .renderer()
                .unwrap()
                .render_texture(&node, None)
                .save_to_png(format!(
                    "{directory}/wizard-{width}-{}.png",
                    if dark { "dark" } else { "light" }
                ))
                .unwrap();
        }
        wizard.close();
        settle();
        parent.close();
        settle();
    }
}

#[test]
#[ignore = "requires GTK; run individually"]
fn discovery_row_offers_tour_and_accessible_dismissal() {
    libadwaita::init().unwrap();
    let dismissed = Rc::new(Cell::new(false));
    let toured = Rc::new(Cell::new(false));
    let saved = dismissed.clone();
    let tour = toured.clone();
    let tips = GettingStartedTips::new(
        Rc::new(move || saved.set(true)),
        Rc::new(move || tour.set(true)),
    );
    let root = tips.widget().downcast::<gtk::Box>().unwrap();
    let tour = find_button(root.upcast_ref(), "Take a tour").unwrap();
    tour.emit_clicked();
    assert!(toured.get());
    assert!(!dismissed.get());
    let dismiss = find_button(root.upcast_ref(), "×").unwrap();
    assert_eq!(
        dismiss.tooltip_text().as_deref(),
        Some("Dismiss Discover PIC")
    );
    dismiss.emit_clicked();
    assert!(dismissed.get());
    for width in [1366, 360] {
        let parent = parent_at(width, 600);
        let host = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let row = GettingStartedTips::new(Rc::new(|| {}), Rc::new(|| {}));
        host.append(&row.widget());
        let space = gtk::Box::new(gtk::Orientation::Vertical, 0);
        space.set_vexpand(true);
        host.append(&space);
        parent.set_content(Some(&host));
        parent.present();
        settle();
        assert!(row.widget().width() <= parent.width());
        assert!(row.widget().height() <= if width == 360 { 130 } else { 64 });
        if let Ok(directory) = std::env::var("PIC_WIZARD_SCREENSHOTS") {
            use gtk::gdk::prelude::PaintableExt;
            let snapshot = gtk::Snapshot::new();
            gtk::WidgetPaintable::new(Some(&parent)).snapshot(
                &snapshot,
                parent.width() as f64,
                parent.height() as f64,
            );
            parent
                .renderer()
                .unwrap()
                .render_texture(&snapshot.to_node().unwrap(), None)
                .save_to_png(format!("{directory}/discovery-row-{width}.png"))
                .unwrap();
        }
        parent.close();
        settle();
    }
}

fn find_button(widget: &gtk::Widget, text: &str) -> Option<gtk::Button> {
    if let Some(button) = widget.downcast_ref::<gtk::Button>() {
        if button.label().as_deref() == Some(text) {
            return Some(button.clone());
        }
    }
    let mut child = widget.first_child();
    while let Some(current) = child {
        if let Some(button) = find_button(&current, text) {
            return Some(button);
        }
        child = current.next_sibling();
    }
    None
}

#[test]
#[ignore = "requires GTK; run individually"]
fn discovery_tour_navigates_browse_albums_edit_collage() {
    let parent = parent_at(900, 650);
    let dialog = super::tour::build();
    let pictures = tour_pictures(&dialog.child().unwrap());
    assert_eq!(pictures.len(), 4);
    assert!(pictures.iter().all(|picture| picture.paintable().is_some()));
    dialog.present(Some(&parent));
    settle();
    let root = dialog.upcast_ref::<gtk::Widget>();
    let back = find_button(root, "Back").unwrap();
    let next = find_button(root, "Next").unwrap();
    assert!(!back.is_sensitive());
    next.emit_clicked();
    assert!(back.is_sensitive());
    back.emit_clicked();
    assert!(!back.is_sensitive());
    for _ in 0..3 {
        next.emit_clicked();
    }
    assert_eq!(next.label().as_deref(), Some("Start browsing"));
    back.emit_clicked();
    assert_eq!(next.label().as_deref(), Some("Next"));
    next.emit_clicked();
    next.emit_clicked();
    settle();
    assert!(parent.visible_dialog().is_none());
    parent.close();
    for width in [900, 360] {
        let parent = parent_at(width, 650);
        let dialog = super::tour::build();
        dialog.set_content_width(width);
        dialog.set_content_height(600);
        dialog.present(Some(&parent));
        settle();
        let next = find_button(dialog.upcast_ref(), "Next").unwrap();
        for title in ["Browse", "Albums", "Edit", "Collage"] {
            let pictures = tour_pictures(&dialog.child().unwrap());
            let picture = pictures
                .iter()
                .find(|picture| {
                    picture.alternative_text().as_deref() == Some(&format!("PIC {title} screen"))
                })
                .unwrap();
            let deadline = Instant::now() + Duration::from_secs(2);
            while !picture.is_mapped() && Instant::now() < deadline {
                settle();
            }
            assert!(picture.is_mapped(), "{title} page did not map at {width}px");
            let root = picture.root().unwrap().downcast::<gtk::Window>().unwrap();
            assert!(picture.width() <= root.width());
            assert!(picture.height() > 0);
            assert!(next.grab_focus());
            settle();
            if let Ok(directory) = std::env::var("PIC_WIZARD_SCREENSHOTS") {
                use gtk::gdk::prelude::PaintableExt;
                let snapshot = gtk::Snapshot::new();
                gtk::WidgetPaintable::new(Some(&root)).snapshot(
                    &snapshot,
                    root.width() as f64,
                    root.height() as f64,
                );
                root.renderer()
                    .unwrap()
                    .render_texture(&snapshot.to_node().unwrap(), None)
                    .save_to_png(format!("{directory}/tour-{width}-{title}.png"))
                    .unwrap();
            }
            next.emit_clicked();
            settle();
        }
        let skipped = super::tour::build();
        skipped.present(Some(&parent));
        settle();
        find_button(skipped.upcast_ref(), "Next")
            .unwrap()
            .emit_clicked();
        find_button(skipped.upcast_ref(), "Skip tour")
            .unwrap()
            .emit_clicked();
        settle();
        assert!(parent.visible_dialog().is_none());
        parent.close();
        settle();
    }
}

fn tour_pictures(widget: &gtk::Widget) -> Vec<gtk::Picture> {
    let mut pictures = Vec::new();
    if let Some(picture) = widget.downcast_ref::<gtk::Picture>() {
        pictures.push(picture.clone());
    }
    let mut child = widget.first_child();
    while let Some(current) = child {
        pictures.extend(tour_pictures(&current));
        child = current.next_sibling();
    }
    pictures
}

#[test]
#[ignore = "requires GTK; run individually"]
fn discovery_tour_reload_uses_replacement_screenshot_without_rebuild() {
    libadwaita::init().unwrap();
    let previous_dir = std::env::current_dir().unwrap();
    let base = std::env::temp_dir().join(format!("pic-tour-swap-{}", std::process::id()));
    std::fs::create_dir_all(base.join("screenshots")).unwrap();
    std::env::set_current_dir(&base).unwrap();
    let path = base.join("screenshots/wiz-browse.jpg");
    image::RgbImage::from_pixel(31, 17, image::Rgb([20, 90, 130]))
        .save(&path)
        .unwrap();
    let dialog = super::tour::build();
    let pictures = tour_pictures(&dialog.child().unwrap());
    assert_eq!(pictures.len(), 4);
    assert_eq!(pictures[0].paintable().unwrap().intrinsic_width(), 31);
    image::RgbImage::from_pixel(43, 29, image::Rgb([120, 10, 100]))
        .save(&path)
        .unwrap();
    let reopened = super::tour::build();
    let pictures = tour_pictures(&reopened.child().unwrap());
    assert_eq!(pictures[0].paintable().unwrap().intrinsic_width(), 43);
    std::env::set_current_dir(previous_dir).unwrap();
    std::fs::remove_dir_all(base).unwrap();
}
