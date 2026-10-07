use super::*;
use std::time::{Duration, Instant};

fn settle() {
    let context = glib::MainContext::default();
    let until = Instant::now() + Duration::from_millis(400);
    while Instant::now() < until {
        while context.pending() {
            context.iteration(false);
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
#[ignore = "requires a GTK display; run individually"]
fn compact_toolbar_keeps_actions_usable_and_restores_wide_layout() {
    libadwaita::init().unwrap();
    crate::css::install_foundation(&gtk::gdk::Display::default().unwrap());
    let bar = InfoBar::new();
    bar.set_photo(None);
    let original_parent = bar.import_photos.parent().unwrap();
    let clicks = Rc::new(Cell::new(0));
    let clicks_for_import = clicks.clone();
    bar.import_photos
        .connect_clicked(move |_| clicks_for_import.set(clicks_for_import.get() + 1));
    let window = gtk::Window::builder()
        .default_width(1400)
        .default_height(100)
        .child(&bar.root)
        .build();
    window.present();
    settle();
    let overflow = original_parent
        .last_child()
        .and_downcast::<gtk::MenuButton>()
        .unwrap();
    assert!(overflow.is_visible());
    let moved: Vec<gtk::Widget> = vec![
        bar.collage.clone().upcast(),
        bar.add_to_album.clone().upcast(),
        bar.one_to_one.clone().upcast(),
        bar.rotate.clone().upcast(),
        bar.export.clone().upcast(),
        bar.import_photos.clone().upcast(),
        bar.more.clone().upcast(),
        bar.print.clone().upcast(),
    ];
    for selected in [false, true] {
        if selected {
            let photo: PhotoObject = glib::Object::new();
            photo.set_filename("A long selected photo filename.jpg");
            photo.set_taken_at("2026-10-06");
            photo.set_camera("Camera with a long descriptive name");
            photo.set_width(6000);
            photo.set_height(4000);
            photo.set_aperture(5.6);
            bar.set_photo(Some(&photo));
        }
        for width in [960, 700, 600, 1400, 960, 700, 600] {
            // Continue native resize requests while responsive fields release
            // their previous minimum. A permanent size floor cannot pass.
            for attempt in 0..6 {
                window.set_default_size(width - (attempt % 2), 100);
                settle();
                if window.width() <= width + 2 {
                    break;
                }
            }
            eprintln!("TOOLBAR_RESIZE selected={selected} requested={width} actual={} minimum={} content={} actions={}", window.width(), bar.root.measure(gtk::Orientation::Horizontal, -1).0, bar.root.width(), original_parent.measure(gtk::Orientation::Horizontal, -1).0);
            assert!(
                window.width() <= width + 2,
                "selected={selected} requested={width} toolbar trapped at {}, minimum={} content={} actions={}",
                window.width(), bar.root.measure(gtk::Orientation::Horizontal, -1).0, bar.root.width(), original_parent.measure(gtk::Orientation::Horizontal, -1).0
            );
            let visible_count = match width {
                0..=559 => 0,
                560..=639 => 2,
                640..=759 => 4,
                760..=1199 => 6,
                _ => moved.len(),
            };
            assert!(overflow.is_visible());
            assert_eq!(bar.export.is_sensitive(), selected);
            assert_eq!(bar.add_to_album.is_sensitive(), selected);
            assert!(bar.import_photos.is_sensitive());
            if selected && width == 700 {
                assert!(bar.filename.is_visible(), "filename hidden at 700px");
            }
            for (index, control) in moved.iter().enumerate() {
                assert_eq!(
                    control.parent().as_ref() == Some(&original_parent),
                    index < visible_count
                );
            }
            if visible_count < moved.len() {
                overflow.popup();
                settle();
                for control in &moved {
                    assert!(
                        control.is_mapped(),
                        "{} hidden in drawer",
                        control.type_().name()
                    );
                }
                if selected && width == 600 {
                    bar.rotate.emit_clicked();
                    settle();
                    assert!(
                        overflow.popover().unwrap().is_visible(),
                        "Rotate should leave the action menu open"
                    );
                }
                bar.import_photos.emit_clicked();
                settle();
                assert!(!overflow.popover().unwrap().is_visible());
            } else {
                bar.import_photos.emit_clicked();
            }
        }
    }
    let menu = overflow.popover().unwrap();
    let menu_box = menu.child().unwrap().downcast::<gtk::Box>().unwrap();
    let hide_buttons = menu_box
        .first_child()
        .unwrap()
        .downcast::<gtk::CheckButton>()
        .unwrap();
    hide_buttons.set_active(true);
    settle();
    assert!(overflow.is_visible());
    assert_ne!(bar.grid_zoom.parent().as_ref(), Some(&original_parent));
    assert_ne!(bar.favorite.parent().as_ref(), Some(&original_parent));
    assert_ne!(bar.rating.parent().as_ref(), Some(&original_parent));
    assert_ne!(bar.edit.parent().as_ref(), Some(&original_parent));
    assert!(
        bar.details.is_visible(),
        "metadata should use the released toolbar width"
    );
    hide_buttons.set_active(false);
    settle();
    assert_eq!(bar.grid_zoom.parent().as_ref(), Some(&original_parent));
    assert_eq!(bar.favorite.parent().as_ref(), Some(&original_parent));
    assert_eq!(bar.rating.parent().as_ref(), Some(&original_parent));
    assert_eq!(bar.edit.parent().as_ref(), Some(&original_parent));
    assert_eq!(
        clicks.get(),
        14,
        "action handler must survive every move exactly once"
    );
    window.close();
}
