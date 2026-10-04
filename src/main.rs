mod albums_view;
mod app_paths;
mod collage;
mod css;
mod db;
mod diagnostics;
mod edit;
mod file_picker;
mod grid;
mod image_format;
mod infobar;
mod library_home;
mod lightbox;
#[cfg(target_os = "linux")]
mod network_picker;
#[cfg(target_os = "linux")]
mod network_shares;
mod photo_object;
mod photo_texture;
mod platform;
#[cfg(target_os = "linux")]
mod private_nfs;
#[cfg(target_os = "linux")]
mod private_smb;
mod scanner;
mod sd_import;
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
    use adw::prelude::*;
    use gio::prelude::*;
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
        Some("io.github.froggy744.PIC"),
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
                // Startup recovery needs an application-owned parent even though the
                // normal main window cannot be built until a database is open. Keeping
                // this tiny window alive also gives native choosers a valid transient
                // parent instead of triggering GtkDialog's unparented-dialog warning.
                let recovery_parent = adw::ApplicationWindow::builder()
                    .application(application)
                    .title("PIC")
                    .default_width(1)
                    .default_height(1)
                    .build();
                recovery_parent.present();

                let dialog = adw::AlertDialog::builder()
                    .heading("Could not open the photo library")
                    .body(format!("{error:#}\n\nChoose another PIC database, create a new library, or close the app."))
                    .close_response("close")
                    .build();
                dialog.add_response("close", "Close");
                dialog.add_response("create", "Create New Library");
                dialog.add_response("choose", "Choose Database…");
                let parent = recovery_parent.clone();
                let application = application.clone();
                dialog.connect_response(None, move |_, response| {
                    if response == "create" {
                        let result = db::suggested_library_path("Default Library")
                            .and_then(|path| db::create_library(&path, "Default Library", ""))
                            .and_then(|library| db::select_library(&library.id));
                        match result {
                            Ok((_library, connection)) => {
                                window::build(&application, connection).present();
                                recovery_parent.close();
                            }
                            Err(error) => {
                                eprintln!("Could not create a new photo library: {error:#}");
                                recovery_parent.close();
                            }
                        }
                        return;
                    }

                    if response != "choose" {
                        recovery_parent.close();
                        return;
                    }
                    let filter = gtk::FileFilter::new();
                    filter.set_name(Some("SQLite databases"));
                    filter.add_pattern("*.db");
                    let application = application.clone();
                    let recovery_parent = recovery_parent.clone();
                    let parent = recovery_parent.clone();
                    crate::file_picker::choose(
                        &parent, "Open PIC Database", "Open",
                        crate::file_picker::FileChoice::Open, None, Some(&filter),
                        move |path| {
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
                                        recovery_parent.close();
                                    }
                                    Err(error) => eprintln!("Could not open selected library: {error:#}"),
                                }
                        },
                    );
                });
                dialog.present(Some(&parent));
            }
        }
    });
    application.run();
}
