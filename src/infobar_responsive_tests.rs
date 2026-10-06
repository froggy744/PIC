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
    let album_popover = gtk::Popover::new();
    album_popover.set_child(Some(&gtk::Label::new(Some("Test album"))));
    bar.add_to_album.set_popover(Some(&album_popover));
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
    assert!(!overflow.is_visible());
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
        for width in [960, 600, 1400, 960, 600] {
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
            let compact = width < 900;
            assert_eq!(overflow.is_visible(), compact);
            assert_eq!(bar.export.is_sensitive(), selected);
            assert_eq!(bar.add_to_album.is_sensitive(), selected);
            assert!(bar.import_photos.is_sensitive());
            for control in &moved {
                assert_eq!(
                    control.parent().as_ref() == Some(&original_parent),
                    !compact
                );
            }
            if compact {
                overflow.popup();
                settle();
                for control in &moved {
                    assert!(
                        control.is_mapped(),
                        "{} hidden in drawer",
                        control.type_().name()
                    );
                }
                if selected {
                    bar.add_to_album.popup();
                    settle();
                    assert!(album_popover.is_mapped(), "nested album menu did not open");
                    bar.add_to_album.popdown();
                }
                bar.import_photos.emit_clicked();
                settle();
                assert!(!overflow.popover().unwrap().is_visible());
            } else {
                bar.import_photos.emit_clicked();
            }
        }
    }
    assert_eq!(
        clicks.get(),
        10,
        "action handler must survive every move exactly once"
    );
    window.close();
}
