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
    adw::init().unwrap();
    let app = adw::Application::builder()
        .application_id("io.pic.OnboardingTests")
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .build();
    app.register(None::<&gio::Cancellable>).unwrap();
    let window = adw::ApplicationWindow::new(&app);
    window.set_default_size(800, 600);
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
    wizard.present();
    settle();
    assert_eq!(parent.dialogs().n_items(), 1);
    assert!(wizard.is_visible());
    wizard.close();
    settle();
    assert!(!wizard.is_visible());
    parent.close();
}
