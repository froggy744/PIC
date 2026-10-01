mod albums_view;
mod app_paths;
mod collage;
mod css;
mod db;
mod diagnostics;
mod edit;
mod grid;
mod image_format;
mod infobar;
mod lightbox;
mod library_home;
#[cfg(target_os="linux")]
mod private_smb;
#[cfg(target_os="linux")]
mod private_nfs;
#[cfg(target_os="linux")]
mod network_shares;
#[cfg(target_os="linux")]
mod network_picker;
mod photo_object;
mod photo_texture;
mod platform;
mod scanner;
mod settings;
mod sidebar;
mod smooth_scroll;
mod source;
mod thumbnail;
mod thumbnail_display;
mod window;

/// Register the bundled hicolor icon subset (resources/icons.gresource).
/// Linux picks icons from the system Adwaita theme, but Windows/macOS
/// bundles ship no icon theme at all, leaving every symbolic icon blank.
/// The bundled subset guarantees all referenced names resolve everywhere.
fn register_bundled_icons() {
    const ICONS_GRESOURCE: &[u8] = include_bytes!("../resources/icons.gresource");
    let resource = gio::Resource::from_data(&glib::Bytes::from_static(ICONS_GRESOURCE))
        .expect("bundled icon resource is valid");
    gio::resources_register(&resource);
}

fn main() {
    use gio::prelude::*;
    use gtk::prelude::*;
    use gtk4 as gtk;
    use libadwaita as adw;

    std::panic::set_hook(Box::new(|panic| {
        eprintln!("PICASA PANIC: {panic}");
        eprintln!(
            "PICASA PANIC BACKTRACE:\n{}",
            std::backtrace::Backtrace::force_capture()
        );
    }));

    adw::init().expect("libadwaita initialization failed");
    register_bundled_icons();
    let application = adw::Application::new(
        Some("io.github.you.PicRs"),
        gio::ApplicationFlags::default(),
    );
    application.connect_activate(|application| {
        // Make the bundled hicolor subset resolvable. Windows/macOS bundles
        // ship no system icon theme, so every symbolic icon would be blank
        // without this (Linux keeps using the system Adwaita theme).
        if let Some(display) = gtk::gdk::Display::default() {
            gtk::IconTheme::for_display(&display).add_resource_path("/picrs/icons");
        }
        // Startup loads indexed rows and recovers missing cached previews.
        // Folder discovery runs only through explicit import/refresh actions.
        let selected = db::initialize_library_manager();
        match selected.and_then(|library| {
            let existed = library.path.is_file();
            let connection = db::open(&library.path)?;
            if existed {
                std::thread::spawn(move || {
                    if let Err(error) = db::automatic_backup_if_due(&library) {
                        eprintln!("Could not create automatic library backup: {error:#}");
                    }
                });
            }
            Ok(connection)
        }) {
            Ok(connection) => window::build(application, connection).present(),
            Err(error) => {
                eprintln!("Could not open photo library: {error:#}");
                let dialog = gtk::MessageDialog::builder()
                    .message_type(gtk::MessageType::Error)
                    .buttons(gtk::ButtonsType::None)
                    .text("Could not open the photo library")
                    .secondary_text(format!("{error:#}\n\nChoose another PIC database or close the app."))
                    .build();
                dialog.add_button("Close", gtk::ResponseType::Close);
                dialog.add_button("Choose Database…", gtk::ResponseType::Accept);
                let application = application.clone();
                dialog.connect_response(move |dialog, response| {
                    dialog.close();
                    if response != gtk::ResponseType::Accept {
                        return;
                    }
                    let chooser = gtk::FileChooserNative::new(
                        Some("Open PIC Database"),
                        None::<&gtk::Window>,
                        gtk::FileChooserAction::Open,
                        Some("Open"),
                        Some("Cancel"),
                    );
                    let filter = gtk::FileFilter::new();
                    filter.set_name(Some("SQLite databases"));
                    filter.add_pattern("*.db");
                    chooser.add_filter(&filter);
                    let application = application.clone();
                    chooser.connect_response(move |chooser, response| {
                        if response == gtk::ResponseType::Accept {
                            if let Some(path) = chooser.file().and_then(|file| file.path()) {
                                let known = db::known_libraries()
                                    .ok()
                                    .and_then(|libraries| {
                                        libraries.into_iter().find(|library| library.path == path)
                                    });
                                let selected = known.map(Ok).unwrap_or_else(|| {
                                    let name = path
                                        .file_stem()
                                        .and_then(|stem| stem.to_str())
                                        .unwrap_or("PIC Library");
                                    db::add_existing_library(&path, name, "")
                                });
                                match selected.and_then(|library| db::select_library(&library.id)) {
                                    Ok((_library, connection)) => {
                                        window::build(&application, connection).present();
                                    }
                                    Err(error) => eprintln!("Could not open selected library: {error:#}"),
                                }
                            }
                        }
                        chooser.destroy();
                    });
                    chooser.show();
                });
                dialog.present();
            }
        }
    });
    application.run();
}
