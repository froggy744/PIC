//! Honeycomb-only separator preference; independent of photo layout choices.
use rusqlite::Connection;
pub(super) const WIDTHS: [u32; 12] = [0, 2, 3, 4, 5, 6, 7, 8, 9, 10, 12, 16];
pub(crate) fn saved_width(connection: &Connection) -> u32 {
    if crate::db::setting(connection, "honeycomb.gap")
        .ok()
        .flatten()
        .as_deref()
        == Some("borderless")
    {
        return 0;
    }
    crate::db::setting(connection, "honeycomb.border-width")
        .ok()
        .flatten()
        .and_then(|value| value.parse::<u32>().ok())
        .map(|width| if width == 11 { 16 } else { width })
        .filter(|width| WIDTHS.contains(width) && *width != 0)
        .unwrap_or(5)
}
pub(super) fn save_width(connection: &Connection, width: u32) -> anyhow::Result<()> {
    if !WIDTHS.contains(&width) {
        anyhow::bail!("Unsupported Honeycomb border width");
    }
    crate::db::set_setting(connection, "honeycomb.border-width", &width.to_string())?;
    crate::db::set_setting(
        connection,
        "honeycomb.gap",
        if width == 0 { "borderless" } else { "standard" },
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn border_width_defaults_persists_and_handles_invalid_values() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE settings(key TEXT PRIMARY KEY,value TEXT NOT NULL)")
            .unwrap();
        assert_eq!(saved_width(&db), 5);
        save_width(&db, 8).unwrap();
        assert_eq!(saved_width(&db), 8);
        save_width(&db, 0).unwrap();
        assert_eq!(saved_width(&db), 0);
        save_width(&db, 5).unwrap();
        assert_eq!(saved_width(&db), 5);
        crate::db::set_setting(&db, "honeycomb.border-width", "invalid").unwrap();
        assert_eq!(saved_width(&db), 5);
        crate::db::set_setting(&db, "honeycomb.border-width", "11").unwrap();
        assert_eq!(saved_width(&db), 16);
        save_width(&db, 16).unwrap();
        assert_eq!(saved_width(&db), 16);
        assert!(save_width(&db, 1).is_err());
    }
}

#[cfg(test)]
mod gtk_tests {
    use super::*;
    use gtk4::{self as gtk, prelude::*};
    use std::{
        cell::{Cell, RefCell},
        rc::Rc,
    };
    fn border_dropdown(widget: &gtk::Widget) -> Option<gtk::DropDown> {
        if let Some(dropdown) = widget.downcast_ref::<gtk::DropDown>() {
            if dropdown
                .model()
                .and_downcast::<gtk::StringList>()
                .is_some_and(|list| {
                    list.n_items() == WIDTHS.len() as u32
                        && list.string(0).as_deref() == Some("None")
                })
            {
                return Some(dropdown.clone());
            }
        }
        let mut child = widget.first_child();
        while let Some(current) = child {
            if let Some(found) = border_dropdown(&current) {
                return Some(found);
            }
            child = current.next_sibling();
        }
        None
    }
    #[test]
    #[ignore = "requires a GTK display; verifies the real Settings control"]
    fn honeycomb_border_setting_saves_and_notifies_live() {
        gtk::init().unwrap();
        let connection = Rc::new(RefCell::new(Connection::open_in_memory().unwrap()));
        connection
            .borrow()
            .execute_batch("CREATE TABLE settings(key TEXT PRIMARY KEY,value TEXT NOT NULL)")
            .unwrap();
        let changes = Rc::new(Cell::new(0));
        let notified = changes.clone();
        let page = super::super::interface_page(
            connection.clone(),
            Rc::new(|| {}),
            Rc::new(move || notified.set(notified.get() + 1)),
            Rc::new(|| {}),
            Rc::new(|| {}),
        );
        let dropdown =
            border_dropdown(page.upcast_ref()).expect("Honeycomb border setting missing");
        assert_eq!(WIDTHS[dropdown.selected() as usize], 5);
        assert_eq!(changes.get(), 0);
        dropdown.set_selected(7);
        assert_eq!(saved_width(&connection.borrow()), 8);
        assert_eq!(changes.get(), 1);
        dropdown.set_selected(0);
        assert_eq!(saved_width(&connection.borrow()), 0);
        assert_eq!(changes.get(), 2);
        assert!(crate::db::setting(&connection.borrow(), "photo_layout.all")
            .unwrap()
            .is_none());
    }
}
