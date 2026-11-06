//! Native, data-backed scrapbook Library dashboard.
//! Kept separate from the normal Library layout so switching back to any
//! existing theme restores the original home without changing preferences.
use super::{Navigate, OpenPhoto, PreviewImage, Snapshot};
use crate::{db, photo_object::PhotoObject, sidebar::SidebarFilter};
use std::cell::Cell;
use std::path::PathBuf;
use gtk::prelude::*;
use gtk4 as gtk;
use std::rc::Rc;
use std::sync::Arc;

#[derive(Clone)]
pub(super) struct ScrapbookDashboard {
    pub root: gtk::Box,
    albums: gtk::FlowBox,
    photos: gtk::FlowBox,
    title: gtk::Label,
    count: gtk::Label,
    cover: gtk::Picture,
    actions: gtk::Box,
    selected_cover: gtk::Picture,
    selected_name: gtk::Label,
    selected_metadata: gtk::Label,
    selected_tags: gtk::FlowBox,
    notes: gtk::TextBuffer,
    selected_id: Rc<Cell<Option<i64>>>,
    note_status: gtk::Label,
    inspector_actions: gtk::Box,
    database: PathBuf,
    open: OpenPhoto,
    navigate: Navigate,
}

fn deco(text: &str, class: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.add_css_class(class);
    label.set_can_target(false);
    label
}

fn add_sheet(label: &str, class: &str) -> gtk::Label {
    let tag = gtk::Label::new(Some(label));
    tag.add_css_class(class);
    tag.set_halign(gtk::Align::Start);
    tag.set_valign(gtk::Align::Center);
    tag
}

fn section_button(label: &str, destination: SidebarFilter, navigate: &Navigate) -> gtk::Button {
    let button = gtk::Button::with_label(label);
    button.add_css_class("scrapbook-section-action");
    let navigate = navigate.clone();
    button.connect_clicked(move |_| navigate(destination));
    button
}

fn photo_texture(image: &Arc<PreviewImage>) -> gtk::gdk::MemoryTexture {
    gtk::gdk::MemoryTexture::new(
        image.width,
        image.height,
        gtk::gdk::MemoryFormat::R8g8b8a8,
        &glib::Bytes::from_owned(image.pixels.clone()),
        image.width as usize * 4,
    )
}

fn set_photo(picture: &gtk::Picture, image: Option<&Arc<PreviewImage>>) {
    picture.set_paintable(image.map(|img| {
        photo_texture(img).upcast::<gtk::gdk::Paintable>()
    }).as_ref());
}

fn find_create_album(root: &gtk::Widget) -> Option<gtk::Button> {
    if root.has_css_class("albums-create") {
        return root.clone().downcast::<gtk::Button>().ok();
    }
    let mut child = root.first_child();
    while let Some(current) = child {
        child = current.next_sibling();
        if let Some(found) = find_create_album(&current) {
            return Some(found);
        }
    }
    None
}

impl ScrapbookDashboard {
    pub(super) fn new(navigate: &Navigate, open: &OpenPhoto, database: PathBuf) -> Self {
        let root = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        root.add_css_class("scrapbook-only");
        root.add_css_class("scrapbook-dashboard");
        root.set_visible(false);
        root.set_hexpand(true);
        let canvas = gtk::Box::new(gtk::Orientation::Vertical, 18);
        canvas.add_css_class("scrapbook-dashboard-canvas");
        canvas.set_hexpand(true);
        canvas.set_margin_start(22);
        canvas.set_margin_end(22);
        canvas.set_margin_top(18);
        canvas.set_margin_bottom(30);

        let hero = gtk::Box::new(gtk::Orientation::Horizontal, 20);
        hero.add_css_class("scrapbook-masthead");
        hero.set_hexpand(true);
        hero.set_valign(gtk::Align::Center);
        let identity = gtk::Box::new(gtk::Orientation::Vertical, 2);
        identity.add_css_class("scrapbook-masthead-identity");
        identity.set_hexpand(true);
        identity.set_valign(gtk::Align::Center);
        let heading = deco("Library", "scrapbook-masthead-title");
        heading.set_halign(gtk::Align::Start);
        identity.append(&heading);
        identity.append(&add_sheet("Your creative photo workspace", "scrapbook-masthead-subtitle"));
        hero.append(&identity);

        let note = gtk::Box::new(gtk::Orientation::Vertical, 3);
        note.add_css_class("scrapbook-checklist");
        note.set_valign(gtk::Align::Center);
        note.append(&deco("●", "scrapbook-checklist-pin"));
        for text in ["☑  Organise projects", "☑  Tag and annotate", "☐  Curate and share  ♡"] {
            note.append(&add_sheet(text, "scrapbook-checklist-line"));
        }
        hero.append(&note);

        let polaroid = gtk::Overlay::new();
        polaroid.add_css_class("scrapbook-masthead-photo");
        polaroid.set_valign(gtk::Align::Center);
        polaroid.set_halign(gtk::Align::End);
        polaroid.set_size_request(166, 132);
        let cover = gtk::Picture::new();
        cover.set_content_fit(gtk::ContentFit::Cover);
        cover.set_can_shrink(true);
        cover.set_size_request(1, 1);
        cover.set_hexpand(true);
        cover.set_vexpand(true);
        polaroid.set_child(Some(&cover));
        let paper_clip = deco("✦", "scrapbook-hero-sparkle");
        paper_clip.set_halign(gtk::Align::End);
        paper_clip.set_valign(gtk::Align::Start);
        polaroid.add_overlay(&paper_clip);
        hero.append(&polaroid);
        canvas.append(&hero);

        let title_row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        title_row.set_valign(gtk::Align::Center);
        title_row.append(&add_sheet("Albums", "scrapbook-sheet-heading"));
        title_row.append(&section_button("View all albums", SidebarFilter::Albums, navigate));
        canvas.append(&title_row);

        let albums = gtk::FlowBox::new();
        albums.add_css_class("scrapbook-albums-row");
        albums.set_selection_mode(gtk::SelectionMode::None);
        albums.set_min_children_per_line(1);
        albums.set_max_children_per_line(5);
        albums.set_homogeneous(false);
        albums.set_row_spacing(18);
        albums.set_column_spacing(18);
        albums.set_valign(gtk::Align::Start);
        canvas.append(&albums);

        let gallery = gtk::Box::new(gtk::Orientation::Vertical, 14);
        gallery.add_css_class("scrapbook-gallery-sheet");
        let gallery_head = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        gallery_head.add_css_class("scrapbook-gallery-header");
        gallery_head.set_valign(gtk::Align::Center);
        let folder = gtk::Image::from_icon_name("folder-pictures-symbolic");
        folder.add_css_class("scrapbook-gallery-folder");
        folder.set_pixel_size(22);
        gallery_head.append(&folder);
        let title = gtk::Label::new(Some("Recently Added"));
        title.add_css_class("scrapbook-gallery-title");
        title.set_xalign(0.0);
        title.set_hexpand(true);
        title.set_ellipsize(gtk::pango::EllipsizeMode::End);
        gallery_head.append(&title);
        let count = gtk::Label::new(Some("0 photos"));
        count.add_css_class("scrapbook-gallery-count");
        gallery_head.append(&count);
        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        actions.set_halign(gtk::Align::End);
        actions.add_css_class("scrapbook-gallery-actions");
        gallery_head.append(&actions);
        gallery.append(&gallery_head);

        let photos = gtk::FlowBox::new();
        photos.add_css_class("scrapbook-photo-grid");
        photos.set_selection_mode(gtk::SelectionMode::None);
        photos.set_min_children_per_line(1);
        photos.set_max_children_per_line(8);
        photos.set_homogeneous(false);
        photos.set_row_spacing(9);
        photos.set_column_spacing(9);
        photos.set_valign(gtk::Align::Start);
        gallery.append(&photos);
        canvas.append(&gallery);
        root.append(&canvas);

        // The right-hand inspector is part of the Library scrapbook, so the
        // reference's paper/Polaroid/notes column exists even on Library Home.
        let inspector = gtk::Box::new(gtk::Orientation::Vertical, 12);
        inspector.add_css_class("scrapbook-home-inspector");
        inspector.set_size_request(258, -1);
        inspector.set_valign(gtk::Align::Fill);
        let selected_header = add_sheet("Selected Photo", "scrapbook-home-inspector-title");
        inspector.append(&selected_header);
        let selected_cover = gtk::Picture::new();
        selected_cover.add_css_class("scrapbook-home-inspector-cover");
        selected_cover.set_content_fit(gtk::ContentFit::Cover);
        selected_cover.set_can_shrink(true);
        selected_cover.set_size_request(228, 154);
        inspector.append(&selected_cover);
        let selected_name = gtk::Label::new(Some("Select a photo"));
        selected_name.add_css_class("scrapbook-home-inspector-name");
        selected_name.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        selected_name.set_max_width_chars(25);
        selected_name.set_xalign(0.0);
        inspector.append(&selected_name);
        let selected_metadata = gtk::Label::new(Some("Your photo details appear here"));
        selected_metadata.add_css_class("scrapbook-home-inspector-metadata");
        selected_metadata.set_xalign(0.0);
        selected_metadata.set_wrap(true);
        inspector.append(&selected_metadata);

        inspector.append(&add_sheet("Tags", "scrapbook-home-paper-label"));
        let selected_tags = gtk::FlowBox::new();
        selected_tags.add_css_class("scrapbook-home-tags");
        selected_tags.set_selection_mode(gtk::SelectionMode::None);
        selected_tags.set_min_children_per_line(1);
        selected_tags.set_max_children_per_line(3);
        selected_tags.set_column_spacing(4);
        selected_tags.set_row_spacing(4);
        inspector.append(&selected_tags);

        inspector.append(&add_sheet("Notes", "scrapbook-home-paper-label"));
        let note_frame = gtk::Box::new(gtk::Orientation::Vertical, 5);
        note_frame.add_css_class("scrapbook-home-note");
        let note_editor = gtk::TextView::new();
        note_editor.set_wrap_mode(gtk::WrapMode::WordChar);
        note_editor.set_size_request(224, 70);
        note_editor.add_css_class("scrapbook-home-note-editor");
        let notes = note_editor.buffer();
        note_frame.append(&note_editor);
        let note_status = gtk::Label::new(None);
        note_status.add_css_class("scrapbook-note-status");
        note_status.set_halign(gtk::Align::Start);
        note_frame.append(&note_status);
        let save = gtk::Button::with_label("Save note");
        save.add_css_class("scrapbook-home-note-save");
        save.set_halign(gtk::Align::End);
        note_frame.append(&save);
        inspector.append(&note_frame);

        inspector.append(&add_sheet("Actions", "scrapbook-home-paper-label"));
        let inspector_actions = gtk::Box::new(gtk::Orientation::Horizontal, 7);
        inspector_actions.add_css_class("scrapbook-home-actions");
        inspector.append(&inspector_actions);
        root.append(&inspector);

        let dashboard = Self {
            root, albums, photos, title, count, cover, actions,
            selected_cover, selected_name, selected_metadata, selected_tags,
            notes, selected_id: Rc::new(Cell::new(None)), note_status,
            inspector_actions, database, open: open.clone(), navigate: navigate.clone(),
        };
        let dashboard_for_save = dashboard.clone();
        save.connect_clicked(move |_| {
            let Some(id) = dashboard_for_save.selected_id.get() else { return; };
            let (start, end) = dashboard_for_save.notes.bounds();
            let value = dashboard_for_save.notes.text(&start, &end, true);
            let result = db::open_existing(&dashboard_for_save.database)
                .and_then(|connection| db::scrapbook_set_note(&connection, id, value.as_str()));
            match result {
                Ok(()) => dashboard_for_save.note_status.set_label("Saved ✓"),
                Err(error) => dashboard_for_save.note_status.set_label(&format!("{error}")),
            }
        });
        dashboard
    }

    pub(super) fn populate(&self, snapshot: &Snapshot, navigate: &Navigate, open: &OpenPhoto) {
        while let Some(child) = self.albums.first_child() {
            self.albums.remove(&child);
        }
        while let Some(child) = self.photos.first_child() {
            self.photos.remove(&child);
        }
        while let Some(child) = self.actions.first_child() {
            self.actions.remove(&child);
        }
        // The physical album covers use the library's real cover image and
        // count; presentation-only pins and badges never change album data.
        for (index, (album, photo)) in snapshot.data.albums.iter().take(4).enumerate() {
            let item = album_tile(
                &album.name,
                album.photo_count,
                photo.as_ref().and_then(|item| snapshot.images.get(&item.id)),
                index,
            );
            let navigate = navigate.clone();
            let id = album.id;
            item.connect_clicked(move |_| navigate(SidebarFilter::Album(id)));
            self.albums.insert(&item, -1);
        }

        let add_album = gtk::Button::new();
        add_album.add_css_class("scrapbook-create-album");
        add_album.set_tooltip_text(Some("Create a new album"));
        let add_body = gtk::Box::new(gtk::Orientation::Vertical, 8);
        add_body.set_halign(gtk::Align::Center);
        add_body.set_valign(gtk::Align::Center);
        add_body.append(&deco("+", "scrapbook-create-plus"));
        add_body.append(&deco("Create Album", "scrapbook-create-caption"));
        add_album.set_child(Some(&add_body));
        let navigate_to_albums = navigate.clone();
        add_album.connect_clicked(move |button| {
            navigate_to_albums(SidebarFilter::Albums);
            // Reuse the existing album-creation dialog rather than writing
            // to the catalog from a decorative Library widget.
            let root = button.root();
            glib::idle_add_local_once(move || {
                if let Some(root) = root.as_ref() {
                    if let Some(create) = find_create_album(root.upcast_ref()) {
                        create.emit_clicked();
                    }
                }
            });
        });
        self.albums.insert(&add_album, -1);

        self.title.set_label(&snapshot.data.scrapbook_title);
        self.count.set_label(&format!("{} photos", snapshot.data.scrapbook_total));
        let id = snapshot.data.albums.iter()
            .find(|(album, _)| album.name == snapshot.data.scrapbook_title)
            .map(|(album, _)| album.id);
        self.actions.append(&section_button(
            "View photos",
            id.map(SidebarFilter::Album).unwrap_or(SidebarFilter::All),
            navigate,
        ));

        let feature_photo = snapshot.data.albums.iter()
            .find(|(album, _)| Some(album.id) == id)
            .and_then(|(_, photo)| photo.as_ref())
            .or_else(|| snapshot.data.scrapbook_photos.first());
        set_photo(&self.cover, feature_photo.and_then(|item| snapshot.images.get(&item.id)));

        let objects: Vec<_> = snapshot.data.scrapbook_photos.iter()
            .map(PhotoObject::from_photo).collect();
        for (index, photo) in snapshot.data.scrapbook_photos.iter().enumerate() {
            let image = snapshot.images.get(&photo.id);
            let tile = photo_tile(
                &crate::source::filename(&photo.path),
                image,
                photo.favorite,
                photo.rating,
                photo.width.unwrap_or_default() < photo.height.unwrap_or_default(),
            );
            tile.set_tooltip_text(Some(&photo.path));
            let photo_grid = self.photos.clone();
            let selected = tile.clone();
            let inspector = self.clone();
            let actual_photo = photo.clone();
            let actual_image = image.cloned();
            tile.connect_clicked(move |_| {
                inspector.select_photo(&actual_photo, actual_image.as_ref());
                let mut child = photo_grid.first_child();
                while let Some(widget) = child {
                    widget.remove_css_class("scrapbook-photo-selected");
                    if let Some(button) = widget.first_child() {
                        button.remove_css_class("scrapbook-photo-selected");
                    }
                    child = widget.next_sibling();
                }
                selected.add_css_class("scrapbook-photo-selected");
            });
            let gesture = gtk::GestureClick::new();
            gesture.set_button(1);
            gesture.set_propagation_phase(gtk::PropagationPhase::Capture);
            let open = open.clone();
            let objects = objects.clone();
            gesture.connect_pressed(move |gesture, presses, _, _| {
                if presses == 2 {
                    gesture.set_state(gtk::EventSequenceState::Claimed);
                    open(objects.clone(), index, false);
                }
            });
            tile.add_controller(gesture);
            self.photos.insert(&tile, -1);
        }
        if snapshot.data.scrapbook_photos.is_empty() {
            self.photos.insert(&add_sheet("No photos yet. Import some to start your scrapbook!", "scrapbook-empty"), -1);
        } else {
            let selected = snapshot.data.scrapbook_photos.iter()
                .find(|photo| Some(photo.id) == self.selected_id.get())
                .unwrap_or(&snapshot.data.scrapbook_photos[0]);
            self.select_photo(selected, snapshot.images.get(&selected.id));
        }
    }

    fn refresh_tags(&self) {
        while let Some(child) = self.selected_tags.first_child() {
            self.selected_tags.remove(&child);
        }
        let Some(photo_id) = self.selected_id.get() else { return; };
        let details = db::open_existing(&self.database)
            .and_then(|connection| db::scrapbook_details(&connection, photo_id));
        let (tags, _) = details.unwrap_or_default();
        for tag in tags {
            let chip = gtk::Button::with_label(&format!("{tag} ×"));
            chip.add_css_class("scrapbook-tag-chip");
            chip.set_tooltip_text(Some("Remove this tag"));
            let dashboard = self.clone();
            chip.connect_clicked(move |_| {
                let result = db::open_existing(&dashboard.database).and_then(|connection|
                    db::scrapbook_remove_tag(&connection, photo_id, &tag)
                );
                if let Err(error) = result { eprintln!("PIC scrapbook tag: {error:#}"); }
                dashboard.refresh_tags();
            });
            self.selected_tags.insert(&chip, -1);
        }
        let add = gtk::Button::with_label("+ Add tag");
        add.add_css_class("scrapbook-tag-add");
        let dashboard = self.clone();
        add.connect_clicked(move |button| {
            let dialog = libadwaita::AlertDialog::new(Some("New scrapbook tag"), Some("Choose a short label for this photo."));
            let entry = gtk::Entry::new();
            entry.set_placeholder_text(Some("Tag name"));
            entry.set_max_length(40);
            dialog.set_extra_child(Some(&entry));
            dialog.add_response("cancel", "Cancel");
            dialog.add_response("add", "Add tag");
            let dashboard_for_response = dashboard.clone();
            dialog.connect_response(None, move |_, response| {
                if response == "add" {
                    let result = db::open_existing(&dashboard_for_response.database).and_then(|connection|
                        db::scrapbook_add_tag(&connection, photo_id, entry.text().as_str())
                    );
                    if let Err(error) = result { eprintln!("PIC scrapbook tag: {error:#}"); }
                    dashboard_for_response.refresh_tags();
                }
            });
            let parent = button.root().and_then(|root| root.downcast::<gtk::Window>().ok());
            dialog.present(parent.as_ref());
        });
        self.selected_tags.insert(&add, -1);
    }

    fn select_photo(&self, photo: &db::Photo, image: Option<&Arc<PreviewImage>>) {
        let previous_id = self.selected_id.replace(Some(photo.id));
        set_photo(&self.selected_cover, image);
        self.selected_name.set_label(&crate::source::filename(&photo.path));
        let date = photo.taken_at.as_deref().unwrap_or("Date unknown");
        let dims = match (photo.width, photo.height) {
            (Some(w), Some(h)) if w > 0 && h > 0 => format!("{w} × {h}"),
            _ => "Dimensions unknown".to_string(),
        };
        let aperture = photo.aperture.map(|value| format!("f/{value:.1}"))
            .unwrap_or_else(|| "—".to_string());
        self.selected_metadata.set_label(&format!(
            "▣  {date}\n▧  {dims}\n◉  {aperture}   •   ISO {}\n⌖  {}",
            photo.iso.map(|value| value.to_string()).unwrap_or_else(|| "—".to_string()),
            photo.folder_path.as_deref().unwrap_or("Local library")
        ));
        if previous_id != Some(photo.id) {
            self.note_status.set_label("");
            let (_, note) = db::open_existing(&self.database)
                .and_then(|connection| db::scrapbook_details(&connection, photo.id))
                .unwrap_or_default();
            self.notes.set_text(&note);
        }
        self.refresh_tags();
        while let Some(child) = self.inspector_actions.first_child() {
            self.inspector_actions.remove(&child);
        }
        let open = gtk::Button::with_label("Open");
        open.add_css_class("scrapbook-home-action");
        let open_action = self.open.clone();
        let object = PhotoObject::from_photo(photo);
        open.connect_clicked(move |_| open_action(vec![object.clone()], 0, false));
        self.inspector_actions.append(&open);
        let albums = gtk::Button::with_label("Albums");
        albums.add_css_class("scrapbook-home-action");
        let navigate = self.navigate.clone();
        albums.connect_clicked(move |_| navigate(SidebarFilter::Albums));
        self.inspector_actions.append(&albums);
        let favorite = gtk::Button::with_label(if photo.favorite { "♥ Favourite" } else { "♡ Favourite" });
        favorite.add_css_class("scrapbook-home-action");
        let dashboard = self.clone();
        let toggle = !photo.favorite;
        let photo_for_update = photo.clone();
        let image_for_update = image.cloned();
        favorite.connect_clicked(move |_| {
            let result = db::open_existing(&dashboard.database)
                .and_then(|connection| db::set_favorite(&connection, photo_for_update.id, toggle));
            if let Err(error) = result { eprintln!("PIC scrapbook favourite: {error:#}"); }
            dashboard.select_photo(&db::Photo { favorite: toggle, ..photo_for_update.clone() }, image_for_update.as_ref());
        });
        self.inspector_actions.append(&favorite);
    }
}

fn album_tile(
    name: &str,
    count: i64,
    image: Option<&Arc<PreviewImage>>,
    index: usize,
) -> gtk::Button {
    let button = gtk::Button::new();
    button.add_css_class("scrapbook-album-tile");
    button.add_css_class(match index % 3 {
        0 => "scrapbook-lavender",
        1 => "scrapbook-peach",
        _ => "scrapbook-sage",
    });
    button.set_halign(gtk::Align::Start);
    let overlay = gtk::Overlay::new();
    let body = gtk::Box::new(gtk::Orientation::Vertical, 5);
    body.set_size_request(200, -1);
    body.add_css_class("scrapbook-album-body");
    let photo = gtk::Picture::new();
    photo.set_size_request(172, 132);
    photo.set_can_shrink(true);
    photo.set_content_fit(gtk::ContentFit::Cover);
    photo.add_css_class("scrapbook-polaroid");
    set_photo(&photo, image);
    body.append(&photo);
    let title = gtk::Label::new(Some(name));
    title.add_css_class("scrapbook-album-title");
    title.set_ellipsize(gtk::pango::EllipsizeMode::End);
    title.set_max_width_chars(22);
    title.set_xalign(0.0);
    body.append(&title);
    body.append(&add_sheet("Photo collection", "scrapbook-album-subtitle"));
    overlay.set_child(Some(&body));
    let sticker = deco(&count.to_string(), "scrapbook-count-sticker");
    sticker.set_halign(gtk::Align::End);
    sticker.set_valign(gtk::Align::End);
    sticker.set_margin_end(0);
    sticker.set_margin_bottom(46);
    overlay.add_overlay(&sticker);
    let pin = deco("●", "scrapbook-album-pin");
    pin.set_halign(gtk::Align::Center);
    pin.set_valign(gtk::Align::Start);
    overlay.add_overlay(&pin);
    button.set_child(Some(&overlay));
    button
}

fn photo_tile(name: &str, image: Option<&Arc<PreviewImage>>, favorite: bool, rating: i32, tall: bool) -> gtk::Button {
    let button = gtk::Button::new();
    button.add_css_class("scrapbook-photo-tile");
    button.set_halign(gtk::Align::Start);
    let body = gtk::Box::new(gtk::Orientation::Vertical, 4);
    body.set_size_request(154, -1);
    let frame = gtk::Overlay::new();
    frame.add_css_class("scrapbook-photo-frame");
    let photo = gtk::Picture::new();
    photo.set_size_request(154, if tall { 175 } else { 116 });
    photo.set_content_fit(gtk::ContentFit::Cover);
    photo.set_can_shrink(true);
    set_photo(&photo, image);
    frame.set_child(Some(&photo));
    if favorite {
        let heart = deco("♥", "scrapbook-photo-heart");
        heart.set_halign(gtk::Align::End);
        heart.set_valign(gtk::Align::End);
        frame.add_overlay(&heart);
    }
    if rating > 0 {
        let stars = deco(&format!("★ {rating}"), "scrapbook-photo-rating");
        stars.set_halign(gtk::Align::Start);
        stars.set_valign(gtk::Align::End);
        frame.add_overlay(&stars);
    }
    body.append(&frame);
    let caption = gtk::Label::new(Some(name));
    caption.add_css_class("scrapbook-photo-caption");
    caption.set_ellipsize(gtk::pango::EllipsizeMode::End);
    caption.set_max_width_chars(20);
    caption.set_xalign(0.0);
    body.append(&caption);
    button.set_child(Some(&body));
    button
}
