//! Optional Compact workspace. This never changes the photo model or editing engine.
use std::rc::Rc;
use gtk4 as gtk;
use gtk::prelude::*;
use crate::sidebar::SidebarFilter;

pub(crate) const UI_LAYOUT_SETTING_KEY: &str = "ui-layout-style";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UiLayoutStyle {
    Standard,
    Compact,
}
impl UiLayoutStyle {
    pub(crate) fn from_setting(value: Option<&str>) -> Self {
        if value == Some("compact") { Self::Compact } else { Self::Standard }
    }
    pub(crate) fn saved(connection: &rusqlite::Connection) -> Self {
        Self::from_setting(
            crate::db::setting(connection, UI_LAYOUT_SETTING_KEY).ok().flatten().as_deref()
        )
    }
    pub(crate) fn setting_value(self) -> &'static str {
        match self { Self::Standard => "standard", Self::Compact => "compact" }
    }
    pub(crate) fn from_index(index: u32) -> Self {
        if index == 1 { Self::Compact } else { Self::Standard }
    }
    pub(crate) fn index(self) -> u32 {
        if self == Self::Compact { 1 } else { 0 }
    }
}

/// A fixed-width navigation rail. Its folder/album explorer is the *real*
/// sidebar temporarily moved into a popover, not an independent replica.
pub(crate) struct CompactRail {
    pub(crate) root: gtk::Box,
    pub(crate) explorer: gtk::Popover,
    nav_buttons: Vec<(SidebarFilter, gtk::Button)>,
    folders_button: gtk::Button,
    albums_button: gtk::Button,
}
impl CompactRail {
    pub(crate) fn new(
        navigate: Rc<dyn Fn(SidebarFilter)>,
        settings: Rc<dyn Fn()>,
    ) -> Self {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 4);
        root.add_css_class("compact-nav-rail");
        root.set_width_request(56);
        root.set_hexpand(false);
        root.set_vexpand(true);
        root.set_margin_top(4);
        root.set_margin_bottom(6);

        let mut nav_buttons = Vec::new();
        for (destination, icon, description) in [
            (SidebarFilter::Library, "user-home-symbolic", "Library home"),
            (SidebarFilter::All, "image-x-generic-symbolic", "All photos"),
            (SidebarFilter::Favorites, "emote-love-symbolic", "Favourites"),
            (SidebarFilter::RecentlyAdded, "appointment-soon-symbolic", "Recently added"),
            (SidebarFilter::History, "document-open-recent-symbolic", "History"),
            (SidebarFilter::Albums, "folder-pictures-symbolic", "Albums"),
        ] {
            let button = rail_button(icon, description);
            let go = navigate.clone();
            button.connect_clicked(move |_| go(destination));
            root.append(&button);
            nav_buttons.push((destination, button));
        }

        root.append(&gtk::Separator::new(gtk::Orientation::Horizontal));

        let explorer = gtk::Popover::new();
        explorer.add_css_class("compact-library-flyout");
        explorer.set_position(gtk::PositionType::Right);
        explorer.set_autohide(true);
        explorer.set_has_arrow(true);
        explorer.set_size_request(310, 510);

        let folders_button = rail_button("folder-symbolic", "Browse folders and albums");
        let albums_button = rail_button("network-workgroup-symbolic", "Browse network shares");
        for button in [&folders_button, &albums_button] {
            let pop = explorer.clone();
            button.connect_clicked(move |button| {
                pop.popdown();
                if pop.parent().is_some() {
                    pop.unparent();
                }
                pop.set_parent(button);
                pop.popup();
            });
            root.append(button);
        }

        let spacer = gtk::Box::new(gtk::Orientation::Vertical, 0);
        spacer.set_vexpand(true);
        root.append(&spacer);
        let settings_button = rail_button("emblem-system-symbolic", "Settings and appearance");
        settings_button.connect_clicked(move |_| settings());
        root.append(&settings_button);
        root.set_visible(false);

        Self { root, explorer, nav_buttons, folders_button, albums_button }
    }

    pub(crate) fn update_active(&self, current: SidebarFilter) {
        for (destination, button) in &self.nav_buttons {
            let active = *destination == current
                || (*destination == SidebarFilter::Albums && matches!(current, SidebarFilter::Album(_)));
            if active { button.add_css_class("compact-nav-active"); }
            else { button.remove_css_class("compact-nav-active"); }
        }
        if matches!(current, SidebarFilter::Folder(_)) {
            self.folders_button.add_css_class("compact-nav-active");
        } else {
            self.folders_button.remove_css_class("compact-nav-active");
        }
        // The network share icon always opens the shared explorer; a share is
        // represented by a folder filter and the source remains in the popup.
        self.albums_button.remove_css_class("compact-nav-active");
    }
}
fn rail_button(icon: &str, accessible_label: &str) -> gtk::Button {
    let button = gtk::Button::from_icon_name(icon);
    button.add_css_class("flat");
    button.add_css_class("compact-nav-button");
    button.set_size_request(40, 40);
    button.set_halign(gtk::Align::Center);
    button.set_focus_on_click(false);
    button.set_tooltip_text(Some(accessible_label));
    button.update_property(&[gtk::accessible::Property::Label(accessible_label)]);
    button
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn style_defaults_to_standard_and_is_separate_from_the_theme() {
        assert_eq!(UiLayoutStyle::from_setting(None), UiLayoutStyle::Standard);
        assert_eq!(UiLayoutStyle::from_setting(Some("unknown")), UiLayoutStyle::Standard);
        assert_eq!(UiLayoutStyle::from_setting(Some("compact")), UiLayoutStyle::Compact);
        assert_eq!(UiLayoutStyle::from_index(99), UiLayoutStyle::Standard);
    }

    #[test]
    fn style_preference_persists_without_a_schema_migration() {
        let db = rusqlite::Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL)").unwrap();
        assert_eq!(UiLayoutStyle::saved(&db), UiLayoutStyle::Standard);
        for style in [UiLayoutStyle::Compact, UiLayoutStyle::Standard] {
            crate::db::set_setting(&db, UI_LAYOUT_SETTING_KEY, style.setting_value()).unwrap();
            assert_eq!(UiLayoutStyle::saved(&db), style);
            assert_eq!(UiLayoutStyle::from_index(style.index()), style);
        }
        crate::db::set_setting(&db, UI_LAYOUT_SETTING_KEY, "something-unknown").unwrap();
        assert_eq!(UiLayoutStyle::saved(&db), UiLayoutStyle::Standard);
    }

    #[test]
    #[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
    fn compact_sidebar_reparents_the_single_navigation_widget() {
        gtk::init().unwrap();
        let rail = CompactRail::new(Rc::new(|_| {}), Rc::new(|| {}));
        let sidebar = gtk::ScrolledWindow::new();
        let standard = gtk::Box::new(gtk::Orientation::Vertical, 0);
        standard.append(&sidebar);
        assert_eq!(sidebar.parent().as_ref(), Some(standard.upcast_ref()));
        standard.remove(&sidebar);
        rail.explorer.set_child(Some(&sidebar));
        assert_eq!(rail.explorer.child().as_ref(), Some(sidebar.upcast_ref()));
        rail.explorer.set_child(None::<&gtk::Widget>);
        standard.append(&sidebar);
        assert_eq!(sidebar.parent().as_ref(), Some(standard.upcast_ref()));
    }
}
