//! Native, data-backed scrapbook Library dashboard.
//! Kept separate from the normal Library layout so switching back to any
//! existing theme restores the original home without changing preferences.
use super::{Navigate, OpenPhoto, PreviewImage, Snapshot};
use crate::{photo_object::PhotoObject, sidebar::SidebarFilter};
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
    pub(super) fn new(navigate: &Navigate) -> Self {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 18);
        root.add_css_class("scrapbook-only");
        root.add_css_class("scrapbook-dashboard");
        root.set_visible(false);
        root.set_hexpand(true);
        root.set_margin_start(22);
        root.set_margin_end(22);
        root.set_margin_top(18);
        root.set_margin_bottom(30);

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
        root.append(&hero);

        let title_row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        title_row.set_valign(gtk::Align::Center);
        title_row.append(&add_sheet("Albums", "scrapbook-sheet-heading"));
        title_row.append(&section_button("View all albums", SidebarFilter::Albums, navigate));
        root.append(&title_row);

        let albums = gtk::FlowBox::new();
        albums.add_css_class("scrapbook-albums-row");
        albums.set_selection_mode(gtk::SelectionMode::None);
        albums.set_min_children_per_line(1);
        albums.set_max_children_per_line(5);
        albums.set_homogeneous(false);
        albums.set_row_spacing(18);
        albums.set_column_spacing(18);
        albums.set_valign(gtk::Align::Start);
        root.append(&albums);

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
        root.append(&gallery);

        Self { root, albums, photos, title, count, cover, actions }
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
            tile.connect_clicked(move |_| {
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
        }
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
