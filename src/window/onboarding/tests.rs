use super::*;
use crate::onboarding::view::ImportPresentation;
use crate::scanner::ScanEvent;

#[test]
fn wizard_ignores_other_scan_generations_and_discovery_does_not_enable_browsing() {
    let mut tracker = ImportTracker::default();
    tracker.start(ImportTicket {
        generation: 42,
        root: "/photos".into(),
    });
    assert!(!tracker.event(41, &ScanEvent::DiscoveryProgress { found: 999 }));
    assert_eq!(tracker.progress.discovered, None);
    assert!(tracker.event(42, &ScanEvent::DiscoveryProgress { found: 327 }));
    assert_eq!(tracker.progress.discovered, Some(327));
    assert_eq!(tracker.progress.indexed, 0);
    assert!(tracker.progress.running);
    assert!(!tracker.event(
        42,
        &ScanEvent::Started {
            root: "/other-root".into()
        }
    ));
    assert!(!tracker.event(42, &ScanEvent::DiscoveryProgress { found: 800 }));
    assert_eq!(tracker.progress.discovered, Some(327));
}
#[test]
fn empty_and_cancelled_imports_allow_recovery_without_claiming_running() {
    let mut tracker = ImportTracker::default();
    tracker.start(ImportTicket {
        generation: 1,
        root: "/empty".into(),
    });
    tracker.event(
        1,
        &ScanEvent::Finished {
            imported: 0,
            failed: 0,
        },
    );
    assert!(!tracker.progress.running);
    assert_eq!(tracker.mode, ImportPresentation::Empty);
    assert!(!tracker.successful);
    tracker.start(ImportTicket {
        generation: 2,
        root: "/empty".into(),
    });
    tracker.event(2, &ScanEvent::Cancelled { imported: 0 });
    assert!(!tracker.progress.running);
    assert_eq!(tracker.mode, ImportPresentation::Recovery);
}
#[test]
fn partial_import_keeps_usable_photos_but_retains_failure_for_recovery() {
    let mut tracker = ImportTracker::default();
    tracker.start(ImportTicket {
        generation: 7,
        root: "/photos".into(),
    });
    tracker.progress.indexed = 1;
    tracker.event(
        7,
        &ScanEvent::Failed {
            path: "/photos/bad.jpg".into(),
            error: "Unreadable".into(),
        },
    );
    assert_eq!(tracker.progress.indexed, 1);
    assert!(tracker.progress.running);
    assert!(tracker
        .progress
        .warning
        .as_ref()
        .unwrap()
        .contains("Unreadable"));
    tracker.event(
        7,
        &ScanEvent::Finished {
            imported: 1,
            failed: 1,
        },
    );
    assert!(!tracker.successful);
    assert_eq!(tracker.mode, ImportPresentation::Error);
    tracker.start(ImportTicket {
        generation: 8,
        root: "/photos".into(),
    });
    tracker.progress.indexed = 1;
    tracker.event(
        8,
        &ScanEvent::Finished {
            imported: 0,
            failed: 0,
        },
    );
    assert!(tracker.successful);
}

fn test_parent() -> libadwaita::ApplicationWindow {
    use adw::prelude::*;
    libadwaita::init().unwrap();
    let app = libadwaita::Application::builder()
        .application_id("io.pic.CoordinatorTests")
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .build();
    app.register(None::<&gio::Cancellable>).unwrap();
    let parent = libadwaita::ApplicationWindow::new(&app);
    parent.set_content(Some(&gtk4::Box::new(gtk4::Orientation::Vertical, 0)));
    parent.present();
    parent
}
fn test_connection() -> Rc<RefCell<Connection>> {
    let connection = Connection::open_in_memory().unwrap();
    connection.execute_batch(crate::db::SCHEMA).unwrap();
    Rc::new(RefCell::new(connection))
}
fn click(parent: &libadwaita::ApplicationWindow, text: &str) {
    use adw::prelude::*;
    fn find(widget: &gtk4::Widget, text: &str) -> Option<gtk4::Button> {
        if let Some(button) = widget.downcast_ref::<gtk4::Button>() {
            if button.label().as_deref() == Some(text) {
                return Some(button.clone());
            }
        }
        let mut child = widget.first_child();
        while let Some(current) = child {
            if let Some(result) = find(&current, text) {
                return Some(result);
            }
            child = current.next_sibling();
        }
        None
    }
    let dialog = parent.visible_dialog().unwrap();
    find(dialog.upcast_ref(), text).unwrap().emit_clicked();
}
#[test]
#[ignore = "requires GTK; run individually"]
fn wizard_interrupted_import_does_not_enqueue_automatically_and_continue_queues_once() {
    let parent = test_parent();
    let connection = test_connection();
    save_stage(
        &connection.borrow(),
        OnboardingStage::Importing,
        Some("/photos"),
    )
    .unwrap();
    let queued = Rc::new(Cell::new(0));
    let count = queued.clone();
    let coordinator = OnboardingCoordinator::new(
        &parent,
        connection.clone(),
        Rc::new(|| {}),
        Rc::new(move |_| count.set(count.get() + 1)),
        Rc::new(|| {}),
    );
    coordinator.present_on_startup();
    assert_eq!(queued.get(), 0);
    click(&parent, "Continue adding photos");
    coordinator.import_started(ImportTicket {
        generation: 4,
        root: "/photos".into(),
    });
    click(&parent, "Continue adding photos");
    assert_eq!(queued.get(), 1);
    assert_eq!(
        load_preferences(&connection.borrow()).unwrap().stage,
        Some(OnboardingStage::Importing)
    );
    coordinator.close();
    parent.close();
}
#[test]
#[ignore = "requires GTK; run individually"]
fn wizard_help_reopens_when_suppressed_without_resetting_preferences() {
    let parent = test_parent();
    let connection = test_connection();
    save_never_show(&connection.borrow(), true).unwrap();
    finish_onboarding(&connection.borrow()).unwrap();
    let coordinator = OnboardingCoordinator::new(
        &parent,
        connection.clone(),
        Rc::new(|| {}),
        Rc::new(|_| {}),
        Rc::new(|| {}),
    );
    coordinator.present_on_startup();
    assert!(!coordinator.is_visible());
    coordinator.present_manually();
    assert!(coordinator.is_visible());
    assert!(coordinator
        .wizard
        .borrow()
        .as_ref()
        .unwrap()
        .never_show
        .is_active());
    assert_eq!(
        load_preferences(&connection.borrow()).unwrap().stage,
        Some(OnboardingStage::Complete)
    );
    coordinator.close();
    parent.close();
}
#[test]
#[ignore = "requires GTK; run individually"]
fn wizard_late_picker_completion_after_close_does_not_import() {
    let parent = test_parent();
    let connection = test_connection();
    let picked = Rc::new(Cell::new(0));
    let count = picked.clone();
    let coordinator = OnboardingCoordinator::new(
        &parent,
        connection,
        Rc::new(move || count.set(count.get() + 1)),
        Rc::new(|_| {}),
        Rc::new(|| {}),
    );
    coordinator.present_manually();
    click(&parent, "Choose Photos Folder");
    assert_eq!(picked.get(), 1);
    let token = coordinator.picker_token().unwrap();
    coordinator.close();
    assert!(!coordinator.picker_result_is_current(token));
    coordinator.present_manually();
    assert!(!coordinator.picker_result_is_current(token));
    coordinator.close();
    parent.close();
}

#[test]
#[ignore = "requires GTK; run individually"]
fn wizard_tips_are_inside_library_and_dismissal_preserves_active_recovery() {
    let parent = test_parent();
    let connection = test_connection();
    connection
        .borrow()
        .execute(
            "INSERT INTO photos(path, width, height) VALUES('/photos/a.jpg', 100, 100)",
            [],
        )
        .unwrap();
    let owner = OnboardingCoordinator::new(
        &parent,
        connection.clone(),
        Rc::new(|| {}),
        Rc::new(|_| {}),
        Rc::new(|| {}),
    );
    let host = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    parent.set_content(Some(&host));
    owner.install_tips_host(&host);
    owner.present_manually();
    owner.folder_selected("/photos").unwrap();
    owner.import_started(ImportTicket {
        generation: 3,
        root: "/photos".into(),
    });
    owner.enter_library();
    assert!(owner.tips.borrow().as_ref().unwrap().widget().is_visible());
    assert!(parent.visible_dialog().is_none() || !owner.is_visible());
    owner.dismiss_library_tips();
    let prefs = load_preferences(&connection.borrow()).unwrap();
    assert!(prefs.tips_dismissed);
    assert_eq!(prefs.stage, Some(OnboardingStage::Importing));
    assert_eq!(prefs.root.as_deref(), Some("/photos"));
    owner.scan_event(
        3,
        &ScanEvent::Finished {
            imported: 1,
            failed: 0,
        },
    );
    assert_eq!(
        load_preferences(&connection.borrow()).unwrap().stage,
        Some(OnboardingStage::Complete)
    );
    parent.close();
}
#[test]
#[ignore = "requires GTK; run individually"]
fn wizard_tips_are_optional_and_never_show_suppresses_them() {
    let parent = test_parent();
    let connection = test_connection();
    connection
        .borrow()
        .execute(
            "INSERT INTO photos(path, width, height) VALUES('/photos/a.jpg', 100, 100)",
            [],
        )
        .unwrap();
    save_never_show(&connection.borrow(), true).unwrap();
    let owner = OnboardingCoordinator::new(
        &parent,
        connection.clone(),
        Rc::new(|| {}),
        Rc::new(|_| {}),
        Rc::new(|| {}),
    );
    let host = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    parent.set_content(Some(&host));
    owner.install_tips_host(&host);
    owner.present_manually();
    owner.folder_selected("/photos").unwrap();
    owner.import_started(ImportTicket {
        generation: 3,
        root: "/photos".into(),
    });
    owner.enter_library();
    assert!(owner.tips.borrow().is_none());
    assert!(owner.is_running());
    owner.scan_event(
        3,
        &ScanEvent::Finished {
            imported: 1,
            failed: 0,
        },
    );
    assert_eq!(
        load_preferences(&connection.borrow()).unwrap().stage,
        Some(OnboardingStage::Complete)
    );
    parent.close();
}

#[test]
#[ignore = "requires GTK; run individually"]
fn wizard_dialog_close_invalidates_picker_and_reopening_can_choose_again() {
    let parent = test_parent();
    let count = Rc::new(Cell::new(0));
    let picked = count.clone();
    let owner = OnboardingCoordinator::new(
        &parent,
        test_connection(),
        Rc::new(move || picked.set(picked.get() + 1)),
        Rc::new(|_| {}),
        Rc::new(|| {}),
    );
    owner.present_manually();
    click(&parent, "Choose Photos Folder");
    let token = owner.picker_token().unwrap();
    owner.wizard.borrow().as_ref().unwrap().close();
    assert!(owner.picker_token().is_none());
    owner.present_manually();
    click(&parent, "Choose Photos Folder");
    assert_eq!(count.get(), 2);
    assert!(!owner.picker_result_is_current(token));
    owner.close();
    parent.close();
}
#[test]
#[ignore = "requires GTK; run individually"]
fn ordinary_browsing_without_tips_does_not_dismiss_future_onboarding() {
    let parent = test_parent();
    let connection = test_connection();
    let owner = OnboardingCoordinator::new(
        &parent,
        connection.clone(),
        Rc::new(|| {}),
        Rc::new(|_| {}),
        Rc::new(|| {}),
    );
    owner.dismiss_library_tips();
    assert!(
        !load_preferences(&connection.borrow())
            .unwrap()
            .tips_dismissed
    );
    parent.close();
}
#[test]
fn wizard_superseded_import_retains_recovery_and_accepts_no_more_old_events() {
    let mut tracker = ImportTracker::default();
    tracker.start(ImportTicket {
        generation: 3,
        root: "/photos".into(),
    });
    tracker.superseded(4);
    assert!(!tracker.progress.running);
    assert_eq!(tracker.mode, ImportPresentation::Recovery);
    assert!(!tracker.event(3, &ScanEvent::DiscoveryProgress { found: 30 }));
}

#[test]
fn ordinary_folder_import_registers_and_authorizes_one_scan() {
    let connection = test_connection();
    let job = Rc::new(RefCell::new(super::super::ScanJobState::default()));
    assert!(queue_local_import(&connection, &job, "/photos", None).unwrap());
    assert_eq!(job.borrow().kind, Some(super::super::ScanJobKind::Import));
    assert_eq!(job.borrow().generation, 1);
    assert_eq!(
        job.borrow().pending.front().map(String::as_str),
        Some("/photos")
    );
    assert_eq!(
        crate::db::imported_root_paths(&connection.borrow()).unwrap(),
        vec!["/photos"]
    );
}
#[test]
#[ignore = "requires GTK; run individually"]
fn wizard_import_reuses_one_authorized_job_and_open_library_leaves_it_active() {
    let parent = test_parent();
    let connection = test_connection();
    let job = Rc::new(RefCell::new(super::super::ScanJobState::default()));
    let owner = OnboardingCoordinator::new(
        &parent,
        connection.clone(),
        Rc::new(|| {}),
        Rc::new(|_| {}),
        Rc::new(|| {}),
    );
    owner.present_manually();
    assert!(queue_local_import(&connection, &job, "/photos", Some(&owner)).unwrap());
    let control = crate::scanner::ScanControl::default();
    job.borrow_mut().active = Some(control.clone());
    assert!(!queue_local_import(&connection, &job, "/photos", Some(&owner)).unwrap());
    assert_eq!(job.borrow().generation, 1);
    assert_eq!(job.borrow().pending.len(), 1);
    assert_eq!(
        crate::db::imported_root_paths(&connection.borrow()).unwrap(),
        vec!["/photos"]
    );
    owner.enter_library();
    assert!(!control.is_cancelled());
    assert!(owner.is_running());
    assert!(!owner.is_visible());
    assert_eq!(
        load_preferences(&connection.borrow()).unwrap().stage,
        Some(OnboardingStage::Importing)
    );
    parent.close();
}

#[test]
#[ignore = "requires GTK; run individually"]
fn wizard_partial_import_can_open_library_before_thumbnail_completion() {
    let parent = test_parent();
    let connection = test_connection();
    let owner = OnboardingCoordinator::new(
        &parent,
        connection.clone(),
        Rc::new(|| {}),
        Rc::new(|_| {}),
        Rc::new(|| {}),
    );
    owner.present_manually();
    owner.folder_selected("/photos").unwrap();
    owner.import_started(ImportTicket {
        generation: 3,
        root: "/photos".into(),
    });
    owner.scan_event(3, &ScanEvent::DiscoveryProgress { found: 300 });
    assert!(!owner
        .wizard
        .borrow()
        .as_ref()
        .unwrap()
        .open_library
        .is_sensitive());
    connection
        .borrow()
        .execute(
            "INSERT INTO photos(path,width,height) VALUES('/photos/a.jpg',100,100)",
            [],
        )
        .unwrap();
    let photo = crate::db::photos(&connection.borrow(), None, false, None)
        .unwrap()
        .remove(0);
    owner.scan_event(
        3,
        &ScanEvent::PhotosIndexed {
            photos: vec![crate::scanner::IndexedPhoto {
                photo,
                newly_discovered: true,
            }],
            counts: crate::db::SidebarCounts::default(),
        },
    );
    assert!(owner
        .wizard
        .borrow()
        .as_ref()
        .unwrap()
        .open_library
        .is_sensitive());
    assert!(owner.is_running());
    owner.scan_event(
        3,
        &ScanEvent::Failed {
            path: "/photos/bad.jpg".into(),
            error: "unreadable".into(),
        },
    );
    assert!(owner
        .wizard
        .borrow()
        .as_ref()
        .unwrap()
        .open_library
        .is_sensitive());
    assert!(owner.tracker.borrow().progress.warning.is_some());
    owner.close();
    parent.close();
}

#[test]
#[ignore = "requires GTK; run individually"]
fn wizard_tips_dismissal_write_failure_remains_retryable() {
    let parent = test_parent();
    let connection = test_connection();
    connection
        .borrow()
        .execute(
            "INSERT INTO photos(path,width,height) VALUES('/photos/a.jpg',100,100)",
            [],
        )
        .unwrap();
    save_stage(&connection.borrow(), OnboardingStage::Tips, Some("/photos")).unwrap();
    let owner = OnboardingCoordinator::new(
        &parent,
        connection.clone(),
        Rc::new(|| {}),
        Rc::new(|_| {}),
        Rc::new(|| {}),
    );
    let host = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    parent.set_content(Some(&host));
    owner.install_tips_host(&host);
    owner.present_on_startup();
    owner.enter_library();
    connection
        .borrow()
        .execute_batch("PRAGMA query_only=ON")
        .unwrap();
    owner.dismiss_library_tips();
    assert!(owner.tips.borrow().as_ref().unwrap().widget().is_visible());
    assert!(
        !load_preferences(&connection.borrow())
            .unwrap()
            .tips_dismissed
    );
    connection
        .borrow()
        .execute_batch("PRAGMA query_only=OFF")
        .unwrap();
    owner.dismiss_library_tips();
    assert!(!owner.tips.borrow().as_ref().unwrap().widget().is_visible());
    assert_eq!(
        load_preferences(&connection.borrow()).unwrap().stage,
        Some(OnboardingStage::Complete)
    );
    parent.close();
}

#[test]
#[ignore = "requires GTK; run individually"]
fn wizard_actual_window_recovery_import_and_library_card() {
    use adw::prelude::*;
    let base = std::env::temp_dir().join(format!(
        "pic-wizard-window-{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap()
    ));
    for (key, directory) in [
        ("XDG_DATA_HOME", "data"),
        ("XDG_CONFIG_HOME", "config"),
        ("XDG_CACHE_HOME", "cache"),
    ] {
        std::fs::create_dir_all(base.join(directory)).unwrap();
        std::env::set_var(key, base.join(directory));
    }
    libadwaita::init().unwrap();
    crate::register_bundled_icons();
    let library = crate::db::initialize_library_manager().unwrap();
    let connection = crate::db::open(&library.path).unwrap();
    let photos = base.join("photos");
    std::fs::create_dir_all(&photos).unwrap();
    image::RgbImage::from_pixel(32, 32, image::Rgb([80, 150, 220]))
        .save(photos.join("first.png"))
        .unwrap();
    save_stage(&connection, OnboardingStage::Importing, photos.to_str()).unwrap();
    let app = libadwaita::Application::builder()
        .application_id("io.pic.FullOnboardingTests")
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .build();
    app.register(None::<&gio::Cancellable>).unwrap();
    let window = crate::window::build(&app, connection);
    window.present();
    let check = crate::db::open_existing(&library.path).unwrap();
    fn settle_for(duration: std::time::Duration) {
        let context = glib::MainContext::default();
        let until = std::time::Instant::now() + duration;
        while std::time::Instant::now() < until {
            while context.pending() {
                context.iteration(false);
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }
    settle_for(std::time::Duration::from_millis(350));
    assert!(window.visible_dialog().is_some());
    assert_eq!(
        check
            .query_row("SELECT COUNT(*) FROM photos", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0,
        "recovery must not start automatically"
    );
    gtk4::prelude::WidgetExt::activate_action(&window, "win.getting-started", None).unwrap();
    assert_eq!(window.dialogs().n_items(), 1);
    click(&window, "Continue adding photos");
    let until = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while load_preferences(&check).unwrap().stage != Some(OnboardingStage::Tips)
        && std::time::Instant::now() < until
    {
        settle_for(std::time::Duration::from_millis(20));
    }
    assert_eq!(
        load_preferences(&check).unwrap().stage,
        Some(OnboardingStage::Tips)
    );
    assert_eq!(
        check
            .query_row("SELECT COUNT(*) FROM photos", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(crate::db::imported_root_paths(&check).unwrap().len(), 1);
    click(&window, "Open Library");
    settle_for(std::time::Duration::from_millis(300));
    fn find(widget: &gtk4::Widget, text: &str) -> Option<gtk4::Button> {
        if let Some(button) = widget.downcast_ref::<gtk4::Button>() {
            if button.label().as_deref() == Some(text) {
                return Some(button.clone());
            }
        }
        let mut child = widget.first_child();
        while let Some(current) = child {
            if let Some(button) = find(&current, text) {
                return Some(button);
            }
            child = current.next_sibling();
        }
        None
    }
    let got_it = find(window.upcast_ref(), "×").expect("dismiss control in discovery row");
    assert!(got_it.is_mapped());
    got_it.emit_clicked();
    assert_eq!(
        load_preferences(&check).unwrap().stage,
        Some(OnboardingStage::Complete)
    );
    assert!(window.visible_dialog().is_none());
    window.close();
    settle_for(std::time::Duration::from_millis(50));
    std::fs::remove_dir_all(base).unwrap();
}

#[test]
#[ignore = "requires GTK; run individually"]
fn wizard_picker_cancel_restores_welcome_or_saved_recovery() {
    let parent = test_parent();
    let connection = test_connection();
    let owner = OnboardingCoordinator::new(
        &parent,
        connection.clone(),
        Rc::new(|| {}),
        Rc::new(|_| {}),
        Rc::new(|| {}),
    );
    owner.present_manually();
    click(&parent, "Choose Photos Folder");
    owner.picker_cancelled();
    assert!(owner
        .wizard
        .borrow()
        .as_ref()
        .unwrap()
        .preference_row
        .is_visible());
    assert!(load_preferences(&connection.borrow())
        .unwrap()
        .root
        .is_none());
    owner.close();
    save_stage(
        &connection.borrow(),
        OnboardingStage::Importing,
        Some("/missing/photos"),
    )
    .unwrap();
    owner.present_manually();
    click(&parent, "Choose another folder");
    owner.picker_cancelled();
    assert!(!owner
        .wizard
        .borrow()
        .as_ref()
        .unwrap()
        .preference_row
        .is_visible());
    assert!(owner
        .wizard
        .borrow()
        .as_ref()
        .unwrap()
        .open_library
        .is_sensitive());
    assert_eq!(
        load_preferences(&connection.borrow())
            .unwrap()
            .root
            .as_deref(),
        Some("/missing/photos")
    );
    assert!(!owner.is_running());
    owner.close();
    parent.close();
}

#[test]
#[ignore = "requires GTK; run individually"]
fn wizard_state_write_failure_allows_retry_without_reopening() {
    let parent = test_parent();
    let connection = test_connection();
    let job = Rc::new(RefCell::new(super::super::ScanJobState::default()));
    let picked = Rc::new(Cell::new(0));
    let count = picked.clone();
    let owner_slot = Rc::new(RefCell::new(None::<Rc<OnboardingCoordinator>>));
    let retry_owner = owner_slot.clone();
    let retry_conn = connection.clone();
    let retry_job = job.clone();
    let owner = OnboardingCoordinator::new(
        &parent,
        connection.clone(),
        Rc::new(move || count.set(count.get() + 1)),
        Rc::new(move |root| {
            queue_local_import(
                &retry_conn,
                &retry_job,
                &root,
                retry_owner.borrow().as_ref(),
            )
            .unwrap();
        }),
        Rc::new(|| {}),
    );
    owner_slot.replace(Some(owner.clone()));
    owner.present_manually();
    click(&parent, "Choose Photos Folder");
    connection
        .borrow()
        .execute_batch("PRAGMA query_only=ON")
        .unwrap();
    let error = queue_local_import(&connection, &job, "/photos/new", Some(&owner)).unwrap_err();
    owner.import_error(&error);
    assert!(
        owner.picker_token().is_none(),
        "completed picker must not stay pending after a failed save"
    );
    assert_eq!(owner.tracker.borrow().progress.root, "/photos/new");
    assert!(crate::db::imported_root_paths(&connection.borrow())
        .unwrap()
        .is_empty());
    connection
        .borrow()
        .execute_batch("PRAGMA query_only=OFF")
        .unwrap();
    click(&parent, "Try again");
    assert_eq!(job.borrow().generation, 1);
    assert_eq!(
        job.borrow().pending.front().map(String::as_str),
        Some("/photos/new")
    );
    owner.close();
    owner_slot.replace(None);
    parent.close();
}

#[test]
#[ignore = "requires GTK; run individually"]
fn wizard_empty_root_does_not_count_photos_elsewhere_and_partial_retry_counts_its_root() {
    let parent = test_parent();
    let connection = test_connection();
    connection.borrow().execute("INSERT INTO photos(path,width,height) VALUES('/elsewhere/a.jpg',100,100),('/photos/retry/a.jpg',100,100),('/photos/retry-sibling/b.jpg',100,100)", []).unwrap();
    let owner = OnboardingCoordinator::new(
        &parent,
        connection.clone(),
        Rc::new(|| {}),
        Rc::new(|_| {}),
        Rc::new(|| {}),
    );
    owner.present_manually();
    owner.folder_selected("/photos/empty").unwrap();
    owner.import_started(ImportTicket {
        generation: 1,
        root: "/photos/empty".into(),
    });
    assert_eq!(
        owner.tracker.borrow().progress.indexed,
        0,
        "the selected root must not count existing photos elsewhere"
    );
    assert!(
        owner
            .wizard
            .borrow()
            .as_ref()
            .unwrap()
            .open_library
            .is_sensitive(),
        "existing library photos still permit browsing"
    );
    owner.scan_event(
        1,
        &ScanEvent::Finished {
            imported: 0,
            failed: 0,
        },
    );
    assert_eq!(owner.tracker.borrow().mode, ImportPresentation::Empty);
    assert_eq!(
        load_preferences(&connection.borrow()).unwrap().stage,
        Some(OnboardingStage::Importing)
    );
    owner.folder_selected("/photos/retry").unwrap();
    owner.import_started(ImportTicket {
        generation: 2,
        root: "/photos/retry".into(),
    });
    assert_eq!(owner.tracker.borrow().progress.indexed, 1);
    owner.scan_event(
        2,
        &ScanEvent::Finished {
            imported: 0,
            failed: 0,
        },
    );
    assert_eq!(
        load_preferences(&connection.borrow()).unwrap().stage,
        Some(OnboardingStage::Tips)
    );
    owner.close();
    parent.close();
}

#[test]
#[ignore = "requires GTK; run individually"]
fn wizard_recovery_reuses_ordinary_active_import_without_cancelling_or_requeueing() {
    let parent = test_parent();
    let connection = test_connection();
    save_stage(
        &connection.borrow(),
        OnboardingStage::Importing,
        Some("/photos"),
    )
    .unwrap();
    let job = Rc::new(RefCell::new(super::super::ScanJobState::default()));
    let owner = OnboardingCoordinator::new(
        &parent,
        connection.clone(),
        Rc::new(|| {}),
        Rc::new(|_| {}),
        Rc::new(|| {}),
    );
    owner.bind_scan_job(&job);
    owner.present_on_startup();
    owner.enter_library();
    assert!(queue_local_import(&connection, &job, "/photos", None).unwrap());
    let control = crate::scanner::ScanControl::default();
    {
        let mut state = job.borrow_mut();
        state.pending.pop_front();
        state.active = Some(control.clone());
        state.active_root = Some("/photos".into());
    }
    let generation = job.borrow().generation;
    owner.present_manually();
    assert!(
        owner.is_running(),
        "recovery must reflect the shared scan already adding this root"
    );
    assert!(!queue_local_import(&connection, &job, "/photos", Some(&owner)).unwrap());
    assert_eq!(job.borrow().generation, generation);
    assert!(!control.is_cancelled());
    assert!(job.borrow().pending.is_empty());
    owner.close();
    parent.close();
}

#[test]
#[ignore = "requires GTK; run individually"]
fn wizard_completed_library_reopens_at_startup_until_checkbox_is_saved() {
    let parent = test_parent();
    let connection = test_connection();
    connection
        .borrow()
        .execute("INSERT INTO photos(path) VALUES('/existing/photo.jpg')", [])
        .unwrap();
    crate::db::mark_import_root(&connection.borrow(), "/existing").unwrap();
    finish_onboarding(&connection.borrow()).unwrap();
    let make_owner = || {
        OnboardingCoordinator::new(
            &parent,
            connection.clone(),
            Rc::new(|| {}),
            Rc::new(|_| {}),
            Rc::new(|| {}),
        )
    };
    let owner = make_owner();
    owner.present_on_startup();
    assert!(owner.is_visible());
    assert_eq!(
        load_preferences(&connection.borrow()).unwrap().stage,
        Some(OnboardingStage::Complete)
    );
    click(&parent, "Skip for now");
    assert!(!owner.is_visible());
    drop(owner);
    let owner = make_owner();
    owner.present_on_startup();
    assert!(owner.is_visible());
    owner
        .wizard
        .borrow()
        .as_ref()
        .unwrap()
        .never_show
        .set_active(true);
    assert!(load_preferences(&connection.borrow()).unwrap().never_show);
    owner.close();
    drop(owner);
    let owner = make_owner();
    owner.present_on_startup();
    assert!(!owner.is_visible());
    save_never_show(&connection.borrow(), false).unwrap();
    owner.present_on_startup();
    assert!(owner.is_visible());
    owner.close();
    parent.close();
}

#[test]
#[ignore = "requires GTK; run individually"]
fn wizard_hides_gallery_and_discovery_row_until_closed() {
    let parent = test_parent();
    let connection = test_connection();
    connection
        .borrow()
        .execute("INSERT INTO photos(path) VALUES('/photos/a.jpg')", [])
        .unwrap();
    let host = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let gallery = gtk::Box::new(gtk::Orientation::Vertical, 0);
    host.append(&gallery);
    parent.set_content(Some(&host));
    let owner = OnboardingCoordinator::new(
        &parent,
        connection,
        Rc::new(|| {}),
        Rc::new(|_| {}),
        Rc::new(|| {}),
    );
    owner.install_tips_host(&host);
    owner.install_library_surface(&gallery);
    owner.present_on_startup();
    assert!(!gallery.is_visible());
    assert!(owner.tips.borrow().is_none());
    click(&parent, "Skip for now");
    assert!(gallery.is_visible());
    assert!(owner.tips.borrow().as_ref().unwrap().widget().is_visible());
    owner.present_manually();
    let until = std::time::Instant::now() + std::time::Duration::from_millis(500);
    while std::time::Instant::now() < until {
        while glib::MainContext::default().pending() {
            glib::MainContext::default().iteration(false);
        }
        std::thread::sleep(std::time::Duration::from_millis(4));
    }
    assert!(owner.is_visible());
    assert!(!gallery.is_visible());
    assert!(!owner.tips.borrow().as_ref().unwrap().widget().is_visible());
    owner.close();
    assert!(gallery.is_visible());
    assert!(owner.tips.borrow().as_ref().unwrap().widget().is_visible());
    owner.dismiss_library_tips();
    owner.present_manually();
    owner.close();
    assert!(!owner.tips.borrow().as_ref().unwrap().widget().is_visible());
    parent.close();
}
