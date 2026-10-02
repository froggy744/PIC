use rusqlite::Connection;

use crate::{db, grid::PhotoLayout, sidebar::SidebarFilter};

// Folder and album IDs deliberately share their section's preference.
fn section_key(filter: SidebarFilter) -> &'static str {
    match filter {
        SidebarFilter::All => "photo_layout.all",
        SidebarFilter::Favorites => "photo_layout.favorites",
        SidebarFilter::RecentlyAdded => "photo_layout.recently_added",
        SidebarFilter::History => "photo_layout.history",
        SidebarFilter::Folder(_) => "photo_layout.folders",
        SidebarFilter::Album(_) | SidebarFilter::Albums => "photo_layout.albums",
        SidebarFilter::Library => "photo_layout.library",
    }
}

pub(super) fn saved_section_layout(connection: &Connection, filter: SidebarFilter) -> PhotoLayout {
    // Leave the legacy value untouched, so customizing one section cannot
    // change the initial layout of sections that have not been customized.
    let saved = db::setting(connection, section_key(filter))
        .ok()
        .flatten()
        .or_else(|| db::setting(connection, "photo_layout").ok().flatten());
    match saved.as_deref() {
        Some("photo_wall") => PhotoLayout::PhotoWall,
        Some("masonry") => PhotoLayout::Masonry,
        _ => PhotoLayout::Grid,
    }
}

pub(super) fn save_section_layout(
    connection: &Connection,
    filter: SidebarFilter,
    layout: PhotoLayout,
) -> anyhow::Result<()> {
    let value = match layout {
        PhotoLayout::Grid => "grid",
        PhotoLayout::PhotoWall => "photo_wall",
        PhotoLayout::Masonry => "masonry",
    };
    db::set_setting(connection, section_key(filter), value)
}

pub(super) fn restore_section_layout(
    connection: &Connection,
    gallery: &std::rc::Rc<crate::grid::Gallery>,
    info: &crate::infobar::InfoBar,
    filter: SidebarFilter,
) {
    let layout = saved_section_layout(connection, filter);
    info.set_photo_layout(layout);
    gallery.set_layout(layout);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn connection() -> Connection {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch("CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL)")
            .unwrap();
        connection
    }

    #[test]
    fn sections_remember_independent_layouts() {
        let connection = connection();
        let choices = [
            (SidebarFilter::All, PhotoLayout::PhotoWall),
            (SidebarFilter::Favorites, PhotoLayout::Masonry),
            (SidebarFilter::RecentlyAdded, PhotoLayout::Grid),
            (SidebarFilter::History, PhotoLayout::PhotoWall),
            (SidebarFilter::Folder(1), PhotoLayout::Grid),
            (SidebarFilter::Album(1), PhotoLayout::Masonry),
        ];
        for (section, layout) in choices {
            save_section_layout(&connection, section, layout).unwrap();
        }
        for (section, layout) in choices {
            assert_eq!(saved_section_layout(&connection, section), layout);
        }
    }

    #[test]
    fn folders_and_albums_each_share_a_section_preference() {
        let connection = connection();
        save_section_layout(&connection, SidebarFilter::Folder(1), PhotoLayout::Masonry).unwrap();
        save_section_layout(&connection, SidebarFilter::Album(1), PhotoLayout::PhotoWall).unwrap();
        assert_eq!(
            saved_section_layout(&connection, SidebarFilter::Folder(42)),
            PhotoLayout::Masonry
        );
        assert_eq!(
            saved_section_layout(&connection, SidebarFilter::Album(42)),
            PhotoLayout::PhotoWall
        );
    }

    #[test]
    fn legacy_preference_is_a_stable_fallback() {
        let connection = connection();
        assert_eq!(
            saved_section_layout(&connection, SidebarFilter::All),
            PhotoLayout::Grid
        );
        db::set_setting(&connection, "photo_layout", "masonry").unwrap();
        save_section_layout(&connection, SidebarFilter::Folder(1), PhotoLayout::Grid).unwrap();
        assert_eq!(
            saved_section_layout(&connection, SidebarFilter::All),
            PhotoLayout::Masonry
        );
        assert_eq!(
            saved_section_layout(&connection, SidebarFilter::Folder(2)),
            PhotoLayout::Grid
        );
        assert_eq!(
            db::setting(&connection, "photo_layout").unwrap().as_deref(),
            Some("masonry")
        );
    }

    #[test]
    #[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
    fn restoring_sections_syncs_gallery_and_switch_without_saving() {
        use gtk4::prelude::*;
        use std::{
            cell::{Cell, RefCell},
            rc::Rc,
        };

        gtk4::init().unwrap();
        let connection = Rc::new(RefCell::new(connection()));
        let gallery = Rc::new(crate::grid::Gallery::new(
            &[],
            100,
            |_| {},
            |_, _, _| {},
            |_, _, _, _| {},
            |_, _| {},
            |_| {},
        ));
        let info = crate::infobar::InfoBar::new();
        let filter = Rc::new(Cell::new(SidebarFilter::All));
        let changes = Rc::new(Cell::new(0));
        {
            let connection = connection.clone();
            let gallery = gallery.clone();
            let filter = filter.clone();
            let changes = changes.clone();
            info.connect_photo_layout(move |layout| {
                gallery.set_layout(layout);
                save_section_layout(&connection.borrow(), filter.get(), layout).unwrap();
                changes.set(changes.get() + 1);
            });
        }
        save_section_layout(
            &connection.borrow(),
            SidebarFilter::All,
            PhotoLayout::PhotoWall,
        )
        .unwrap();
        save_section_layout(
            &connection.borrow(),
            SidebarFilter::Folder(1),
            PhotoLayout::Grid,
        )
        .unwrap();
        for (section, layout, tooltip) in [
            (
                SidebarFilter::All,
                PhotoLayout::PhotoWall,
                "Switch to Masonry",
            ),
            (
                SidebarFilter::Folder(2),
                PhotoLayout::Grid,
                "Switch to Photo Wall",
            ),
            (
                SidebarFilter::All,
                PhotoLayout::PhotoWall,
                "Switch to Masonry",
            ),
        ] {
            filter.set(section);
            restore_section_layout(&connection.borrow(), &gallery, &info, section);
            assert_eq!(gallery.layout(), layout);
            assert_eq!(info.view_toggle.tooltip_text().as_deref(), Some(tooltip));
            assert_eq!(changes.get(), 0);
        }
        // The next click must advance from the restored section's layout.
        info.view_toggle.emit_clicked();
        assert_eq!(gallery.layout(), PhotoLayout::Masonry);
        assert_eq!(changes.get(), 1);
        assert_eq!(
            saved_section_layout(&connection.borrow(), SidebarFilter::All),
            PhotoLayout::Masonry
        );
        assert_eq!(
            saved_section_layout(&connection.borrow(), SidebarFilter::Folder(1)),
            PhotoLayout::Grid
        );
    }
}
