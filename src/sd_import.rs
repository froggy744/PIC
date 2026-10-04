use gtk::prelude::*;
use gtk4 as gtk;
use libadwaita as adw;

/// Present the dedicated camera / SD-card import surface.
///
/// Phase 1 deliberately keeps this separate from PIC's existing "Import Folder"
/// action: removable media is an import source, not a permanent library folder.
/// Copying/scanning is wired in the next phase.
pub fn present(parent: &adw::ApplicationWindow) {
    let window = gtk::Window::builder()
        .title("Import Photos")
        .transient_for(parent)
        .modal(false)
        .default_width(920)
        .default_height(640)
        .build();

    let root = gtk::Box::new(gtk::Orientation::Vertical, 14);
    root.set_margin_top(18);
    root.set_margin_bottom(18);
    root.set_margin_start(18);
    root.set_margin_end(18);

    let source_row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    let source_label = gtk::Label::new(Some("Import from:"));
    source_label.set_xalign(0.0);
    let source_value = gtk::Label::new(Some("No device selected"));
    source_value.set_xalign(0.0);
    source_value.set_hexpand(true);
    source_value.set_ellipsize(gtk::pango::EllipsizeMode::Middle);

    let choose_source = gtk::Button::with_label("Choose SD card…");
    source_row.append(&source_label);
    source_row.append(&source_value);
    source_row.append(&choose_source);
    root.append(&source_row);

    let exclude_duplicates = gtk::CheckButton::with_label("Exclude duplicates");
    exclude_duplicates.set_active(true);
    root.append(&exclude_duplicates);

    let preview = gtk::Box::new(gtk::Orientation::Vertical, 10);
    preview.set_hexpand(true);
    preview.set_vexpand(true);
    preview.set_halign(gtk::Align::Fill);
    preview.set_valign(gtk::Align::Fill);

    let device_icon = gtk::Image::from_icon_name("media-removable-symbolic");
    device_icon.set_pixel_size(64);
    device_icon.set_vexpand(true);
    device_icon.set_valign(gtk::Align::End);

    let empty = gtk::Label::new(Some(
        "Choose a camera or SD card to preview photos for import",
    ));
    empty.add_css_class("dim-label");
    empty.set_vexpand(true);
    empty.set_valign(gtk::Align::Start);

    preview.append(&device_icon);
    preview.append(&empty);

    let scrolled = gtk::ScrolledWindow::builder()
        .hexpand(true)
        .vexpand(true)
        .child(&preview)
        .build();
    root.append(&scrolled);

    let separator = gtk::Separator::new(gtk::Orientation::Horizontal);
    root.append(&separator);

    let destination_row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    let destination_label = gtk::Label::new(Some("Import to:"));
    let destination_value = gtk::Label::new(Some("Choose destination folder"));
    destination_value.set_xalign(0.0);
    destination_value.set_hexpand(true);
    destination_value.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    let choose_destination = gtk::Button::with_label("Destination…");

    destination_row.append(&destination_label);
    destination_row.append(&destination_value);
    destination_row.append(&choose_destination);
    root.append(&destination_row);

    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    actions.set_halign(gtk::Align::End);
    let cancel = gtk::Button::with_label("Cancel");
    let import_selected = gtk::Button::with_label("Import Selected");
    let import_all = gtk::Button::with_label("Import All");
    import_selected.set_sensitive(false);
    import_all.set_sensitive(false);
    actions.append(&cancel);
    actions.append(&import_selected);
    actions.append(&import_all);
    root.append(&actions);

    let source_value_for_dialog = source_value.clone();
    let parent_for_source = window.clone();
    choose_source.connect_clicked(move |_| {
        let dialog = gtk::FileDialog::builder()
            .title("Choose SD Card or Camera")
            .accept_label("Choose")
            .modal(true)
            .build();
        let source_value = source_value_for_dialog.clone();
        dialog.select_folder(
            Some(&parent_for_source),
            None::<&gio::Cancellable>,
            move |result| {
                if let Ok(folder) = result {
                    let text = folder
                        .path()
                        .map(|path| path.display().to_string())
                        .unwrap_or_else(|| folder.uri().to_string());
                    source_value.set_text(&text);
                }
            },
        );
    });

    let destination_value_for_dialog = destination_value.clone();
    let parent_for_destination = window.clone();
    choose_destination.connect_clicked(move |_| {
        let dialog = gtk::FileDialog::builder()
            .title("Choose Import Destination")
            .accept_label("Choose")
            .modal(true)
            .build();
        let destination_value = destination_value_for_dialog.clone();
        dialog.select_folder(
            Some(&parent_for_destination),
            None::<&gio::Cancellable>,
            move |result| {
                if let Ok(folder) = result {
                    let text = folder
                        .path()
                        .map(|path| path.display().to_string())
                        .unwrap_or_else(|| folder.uri().to_string());
                    destination_value.set_text(&text);
                }
            },
        );
    });

    let window_for_cancel = window.clone();
    cancel.connect_clicked(move |_| window_for_cancel.close());

    window.set_child(Some(&root));
    window.present();
}
