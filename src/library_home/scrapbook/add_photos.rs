//! Scrapbook Library: add existing catalog photos to the selected album.
//! This is the same non-destructive catalog operation used by PIC's Albums UI.
//! The chooser searches indexed library photos; no filesystem re-import occurs.
use crate::db;
use gtk::prelude::*;
use gtk4 as gtk;
use libadwaita as adw;
use libadwaita::prelude::*;
use std::{cell::RefCell, collections::HashSet, path::PathBuf, rc::Rc};

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
            eprintln!("PIC Scrapbook: cannot open photo chooser: {error:#}");
            return;
        }
    };
    let photos = match db::photos_limited(&connection, None, false, None, 500) {
        Ok(photos) => photos,
        Err(error) => {
            eprintln!("PIC Scrapbook: cannot load library photo choices: {error:#}");
            return;
        }
    };
    let present: HashSet<i64> = match db::photos_in_album(&connection, album_id, None) {
        Ok(photos) => photos.into_iter().map(|photo| photo.id).collect(),
        Err(error) => {
            eprintln!("PIC Scrapbook: cannot load current album: {error:#}");
            return;
        }
    };

    let dialog = adw::AlertDialog::new(
        Some("Add Photos to Album"),
        Some("Select photos from your existing PIC library. Originals are not copied or moved."),
    );
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("add", "Add selected photos");
    dialog.set_response_appearance("add", adw::ResponseAppearance::Suggested);
    dialog.set_response_enabled("add", false);
    let content = gtk::Box::new(gtk::Orientation::Vertical, 9);
    content.set_margin_start(12);
    content.set_margin_end(12);
    content.set_margin_top(8);
    content.set_margin_bottom(8);
    let search = gtk::SearchEntry::new();
    search.set_placeholder_text(Some("Search library filenames…"));
    content.append(&search);
    let counter = gtk::Label::new(Some("0 photos selected"));
    counter.set_xalign(0.0);
    counter.add_css_class("dim-label");
    content.append(&counter);

    let list = gtk::ListBox::new();
    list.set_selection_mode(gtk::SelectionMode::None);
    list.add_css_class("boxed-list");
    let choices: Rc<RefCell<Vec<(i64, String, gtk::CheckButton, gtk::ListBoxRow)>>> =
        Rc::new(RefCell::new(Vec::new()));
    for photo in photos {
        if present.contains(&photo.id) {
            continue;
        }
        let row = gtk::ListBoxRow::new();
        let check = gtk::CheckButton::new();
        check.set_label(Some(&crate::source::filename(&photo.path)));
        check.set_tooltip_text(Some(&photo.path));
        check.set_margin_start(10);
        check.set_margin_end(10);
        check.set_margin_top(5);
        check.set_margin_bottom(5);
        row.set_child(Some(&check));
        list.append(&row);
        choices.borrow_mut().push((
            photo.id, crate::source::filename(&photo.path).to_lowercase(),
            check, row,
        ));
    }
    let scroll = gtk::ScrolledWindow::new();
    scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    scroll.set_min_content_height(330);
    scroll.set_max_content_height(390);
    scroll.set_propagate_natural_height(true);
    scroll.set_child(Some(&list));
    content.append(&scroll);
    if choices.borrow().is_empty() {
        let empty = gtk::Label::new(Some("No additional photos available in the first 500 library results."));
        empty.set_wrap(true);
        empty.add_css_class("dim-label");
        content.append(&empty);
    }
    let update_count: Rc<dyn Fn()> = Rc::new({
        let choices = choices.clone();
        let counter = counter.clone();
        let dialog = dialog.clone();
        move || {
            let count = choices.borrow().iter().filter(|(_,_,check,_)| check.is_active()).count();
            counter.set_label(&format!("{count} photos selected"));
            dialog.set_response_enabled("add", count > 0);
        }
    });
    for (_, _, check, _) in choices.borrow().iter() {
        let update_count = update_count.clone();
        check.connect_toggled(move |_| update_count());
    }
    {
        let choices = choices.clone();
        search.connect_search_changed(move |entry| {
            let text = entry.text().trim().to_lowercase();
            for (_, name, _, row) in choices.borrow().iter() {
                row.set_visible(text.is_empty() || name.contains(&text));
            }
        });
    }
    dialog.set_extra_child(Some(&content));
    dialog.connect_response(Some("add"), move |_, _| {
        let ids: Vec<i64> = choices.borrow().iter()
            .filter(|(_,_,check,_)| check.is_active())
            .map(|(id,_,_,_)| *id)
            .collect();
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
