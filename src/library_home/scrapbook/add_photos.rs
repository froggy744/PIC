//! Native multi-photo picker for an inline Scrapbook Library album.
//! Searches the entire indexed library, paginates thumbnails and preserves
//! checked selections across search/pages. No importing, copying or moving.
use crate::{db, thumbnail, thumbnail_display};
use gtk::prelude::*;
use gtk4 as gtk;
use libadwaita as adw;
use libadwaita::prelude::*;
use std::{
    cell::{Cell, RefCell},
    collections::HashSet,
    path::PathBuf,
    rc::Rc,
};

const PAGE_SIZE: usize = 72;

#[derive(Clone)]
struct Picker {
    database: PathBuf,
    in_album: Rc<HashSet<i64>>,
    selected: Rc<RefCell<HashSet<i64>>>,
    search: Rc<RefCell<String>>,
    page: Rc<Cell<usize>>,
    generation: Rc<Cell<u64>>,
    results: gtk::FlowBox,
    count: gtk::Label,
    page_info: gtk::Label,
    previous: gtk::Button,
    next: gtk::Button,
    dialog: adw::AlertDialog,
}

impl Picker {
    fn update_count(&self) {
        let count = self.selected.borrow().len();
        self.count.set_label(&format!("{count} selected"));
        self.dialog.set_response_enabled("add", count > 0);
    }

    fn refresh(&self) {
        while let Some(child) = self.results.first_child() {
            self.results.remove(&child);
        }
        let connection = match db::open_existing(&self.database) {
            Ok(connection) => connection,
            Err(error) => {
                self.page_info.set_label(&format!("Could not load library: {error}"));
                return;
            }
        };
        let query = self.search.borrow();
        let all = match db::photos_limited(
            &connection,
            None,
            false,
            if query.trim().is_empty() { None } else { Some(query.trim()) },
            -1,
        ) {
            Ok(photos) => photos,
            Err(error) => {
                self.page_info.set_label(&format!("Could not search photos: {error}"));
                return;
            }
        };
        let matches: Vec<_> = all.into_iter()
            .filter(|photo| !self.in_album.contains(&photo.id)).collect();
        let page_count = matches.len().div_ceil(PAGE_SIZE).max(1);
        let page = self.page.get().min(page_count - 1);
        self.page.set(page);
        self.previous.set_sensitive(page > 0);
        self.next.set_sensitive(page + 1 < page_count);
        self.page_info.set_label(&format!(
            "{} available photos  •  Page {} of {}",
            matches.len(), page + 1, page_count,
        ));
        for photo in matches.iter().skip(page * PAGE_SIZE).take(PAGE_SIZE) {
            let button = gtk::ToggleButton::new();
            button.add_css_class("scrapbook-photo-picker-tile");
            button.set_size_request(124, 124);
            button.set_active(self.selected.borrow().contains(&photo.id));
            button.set_tooltip_text(Some(&photo.path));
            let tile = gtk::Box::new(gtk::Orientation::Vertical, 3);
            let preview = gtk::Picture::new();
            preview.set_content_fit(gtk::ContentFit::Cover);
            preview.set_can_shrink(true);
            preview.set_size_request(116, 82);
            preview.add_css_class("scrapbook-picker-thumb");
            if let Ok(cache_path) = thumbnail::cache_path(
                &photo.path, photo.mtime, photo.size_bytes,
            ) {
                let request = thumbnail_display::request_for(
                    cache_path.to_string_lossy().into_owned(),
                    photo.path.clone(),
                    photo.mtime.unwrap_or_default(),
                    photo.size_bytes.unwrap_or_default(),
                    photo.rotation,
                    photo.edit_recipe.clone(),
                    photo.width.unwrap_or_default(),
                    photo.height.unwrap_or_default(),
                    false,
                );
                if let thumbnail_display::DisplayOutcome::Loaded {
                    width, height, pixels,
                } = thumbnail_display::load_cached_display_thumbnail(&request) {
                    let texture = gtk::gdk::MemoryTexture::new(
                        width, height,
                        gtk::gdk::MemoryFormat::R8g8b8a8,
                        &glib::Bytes::from_owned(pixels),
                        width as usize * 4,
                    );
                    preview.set_paintable(Some(&texture));
                }
            }
            tile.append(&preview);
            let name = gtk::Label::new(Some(&crate::source::filename(&photo.path)));
            name.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
            name.set_max_width_chars(17);
            name.add_css_class("scrapbook-picker-name");
            tile.append(&name);
            button.set_child(Some(&tile));
            let photo_id = photo.id;
            let state = self.clone();
            button.connect_toggled(move |button| {
                if button.is_active() {
                    state.selected.borrow_mut().insert(photo_id);
                } else {
                    state.selected.borrow_mut().remove(&photo_id);
                }
                state.update_count();
            });
            self.results.insert(&button, -1);
        }
        if matches.is_empty() {
            let empty = gtk::Label::new(Some("No matching photos outside this album."));
            empty.add_css_class("dim-label");
            empty.set_margin_top(28);
            empty.set_margin_bottom(28);
            self.results.insert(&empty, -1);
        }
        self.update_count();
    }
}

pub(super) fn open(
    anchor: &gtk::Widget,
    database: PathBuf,
    album_id: i64,
    after_add: Rc<dyn Fn(i64)>,
) {
    let Some(parent) = anchor.root().and_then(|root| root.downcast::<gtk::Window>().ok())
    else {
        return;
    };
    let connection = match db::open_existing(&database) {
        Ok(connection) => connection,
        Err(error) => {
            eprintln!("PIC Scrapbook: cannot open photo picker: {error:#}");
            return;
        }
    };
    let in_album: HashSet<i64> = match db::photos_in_album(&connection, album_id, None) {
        Ok(photos) => photos.into_iter().map(|photo| photo.id).collect(),
        Err(error) => {
            eprintln!("PIC Scrapbook: cannot load album: {error:#}");
            return;
        }
    };
    let dialog = adw::AlertDialog::new(
        Some("Add Photos to Album"),
        Some("Select photos from PIC's library. Originals are never moved or copied."),
    );
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("add", "Add selected photos");
    dialog.set_response_appearance("add", adw::ResponseAppearance::Suggested);
    dialog.set_response_enabled("add", false);
    let body = gtk::Box::new(gtk::Orientation::Vertical, 9);
    body.set_margin_start(12);
    body.set_margin_end(12);
    body.set_margin_top(10);
    body.set_margin_bottom(10);

    let search = gtk::SearchEntry::new();
    search.set_placeholder_text(Some("Search all library filenames…"));
    body.append(&search);
    let header = gtk::Box::new(gtk::Orientation::Horizontal, 14);
    let count = gtk::Label::new(Some("0 selected"));
    count.add_css_class("scrapbook-picker-count");
    count.set_hexpand(true);
    count.set_xalign(0.0);
    let page_info = gtk::Label::new(None);
    page_info.add_css_class("dim-label");
    header.append(&count);
    header.append(&page_info);
    body.append(&header);
    let results = gtk::FlowBox::new();
    results.add_css_class("scrapbook-picker-grid");
    results.set_selection_mode(gtk::SelectionMode::None);
    results.set_min_children_per_line(3);
    results.set_max_children_per_line(4);
    results.set_row_spacing(8);
    results.set_column_spacing(8);
    let scroller = gtk::ScrolledWindow::new();
    scroller.set_size_request(520, 330);
    scroller.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    scroller.set_child(Some(&results));
    body.append(&scroller);

    let footer = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    footer.set_halign(gtk::Align::Center);
    let previous = gtk::Button::with_label("← Previous");
    let next = gtk::Button::with_label("Next →");
    footer.append(&previous);
    footer.append(&next);
    body.append(&footer);
    let picker = Picker {
        database: database.clone(), in_album: Rc::new(in_album),
        selected: Rc::new(RefCell::new(HashSet::new())),
        search: Rc::new(RefCell::new(String::new())),
        page: Rc::new(Cell::new(0)),
        generation: Rc::new(Cell::new(0)),
        results, count, page_info, previous: previous.clone(),
        next: next.clone(), dialog: dialog.clone(),
    };
    {
        let picker = picker.clone();
        search.connect_search_changed(move |entry| {
            picker.search.replace(entry.text().to_string());
            picker.page.set(0);
            let generation = picker.generation.get().wrapping_add(1);
            picker.generation.set(generation);
            let picker = picker.clone();
            glib::timeout_add_local_once(std::time::Duration::from_millis(160), move || {
                if picker.generation.get() == generation {
                    picker.refresh();
                }
            });
        });
    }
    {
        let picker = picker.clone();
        previous.connect_clicked(move |_| {
            picker.page.set(picker.page.get().saturating_sub(1));
            picker.refresh();
        });
    }
    {
        let picker = picker.clone();
        next.connect_clicked(move |_| {
            picker.page.set(picker.page.get().saturating_add(1));
            picker.refresh();
        });
    }
    picker.refresh();
    dialog.set_extra_child(Some(&body));
    dialog.connect_response(Some("add"), move |_, _| {
        let ids: Vec<i64> = picker.selected.borrow().iter().copied().collect();
        if ids.is_empty() { return; }
        match db::open_existing(&database)
            .and_then(|connection| db::add_photos_to_album(&connection, album_id, &ids))
        {
            Ok(()) => after_add(album_id),
            Err(error) => eprintln!("PIC Scrapbook: could not add photos: {error:#}"),
        }
    });
    dialog.present(Some(&parent));
}
