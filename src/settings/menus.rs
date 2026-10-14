use super::*;

#[derive(Clone, Default)]
pub(crate) struct MenuControls(Rc<RefCell<Vec<MenuControl>>>);

struct MenuControl {
    key: String,
    title: &'static str,
    group: &'static str,
    widget: glib::WeakRef<gtk::Widget>,
    enabled: Rc<Cell<bool>>,
}

impl MenuControls {
    pub(crate) fn register(
        &self,
        connection: &Connection,
        group: &'static str,
        id: &str,
        title: &'static str,
        widget: &impl IsA<gtk::Widget>,
    ) {
        let key = format!("menu_visible_{id}");
        let enabled = Rc::new(Cell::new(saved_bool(connection, &key).unwrap_or(true)));
        let widget = widget.upcast_ref::<gtk::Widget>();
        let enabled_for_notify = enabled.clone();
        widget.connect_visible_notify(move |widget| {
            if !enabled_for_notify.get() && widget.get_visible() {
                widget.set_visible(false);
            }
        });
        if !enabled.get() {
            widget.set_visible(false);
        }
        self.0.borrow_mut().push(MenuControl {
            key,
            title,
            group,
            widget: widget.downgrade(),
            enabled,
        });
    }

    pub(crate) fn reload(&self, connection: &Connection) {
        for control in self.0.borrow().iter() {
            let enabled = saved_bool(connection, &control.key).unwrap_or(true);
            control.enabled.set(enabled);
            if let Some(widget) = control.widget.upgrade() {
                widget.set_visible(enabled);
            }
        }
    }

    pub(super) fn page(&self, connection: Rc<RefCell<Connection>>) -> gtk::ScrolledWindow {
        let content = page_content(
            "Menus",
            "Choose the controls shown in the top bar and info card. Settings and More Photo Actions remain available so you can restore hidden controls.",
        );
        for group in ["Top bar", "Info card"] {
            let heading = gtk::Label::new(Some(group));
            heading.set_xalign(0.0);
            heading.add_css_class("heading");
            content.append(&heading);
            let list = settings_list();
            for control in self.0.borrow().iter().filter(|item| item.group == group) {
                let toggle = gtk::Switch::new();
                toggle.set_active(control.enabled.get());
                let connection = connection.clone();
                let key = control.key.clone();
                let enabled = control.enabled.clone();
                let widget = control.widget.clone();
                toggle.connect_active_notify(move |toggle| {
                    let active = toggle.is_active();
                    if let Err(error) = db_save(&connection.borrow(), &key, active) {
                        eprintln!("Could not save menu visibility: {error}");
                        if active != enabled.get() {
                            toggle.set_active(enabled.get());
                        }
                        return;
                    }
                    enabled.set(active);
                    if let Some(widget) = widget.upgrade() {
                        widget.set_visible(active);
                    }
                });
                append_row(&list, control.title, None, Some(toggle.upcast_ref()));
            }
            content.append(&list);
        }
        scroll_page(content)
    }
}

fn db_save(connection: &Connection, key: &str, enabled: bool) -> anyhow::Result<()> {
    crate::db::set_setting(connection, key, &enabled.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires a GTK display; run individually"]
    fn switches_apply_save_and_restore_visibility() {
        libadwaita::init().unwrap();
        let connection = Rc::new(RefCell::new(Connection::open_in_memory().unwrap()));
        connection
            .borrow()
            .execute_batch("CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL)")
            .unwrap();
        let controls = MenuControls::default();
        let button = gtk::Button::new();
        controls.register(&connection.borrow(), "Top bar", "sort", "Sort", &button);
        let page = controls.page(connection.clone());
        fn find_switch(widget: &gtk::Widget) -> Option<gtk::Switch> {
            if let Some(toggle) = widget.downcast_ref::<gtk::Switch>() {
                return Some(toggle.clone());
            }
            let mut child = widget.first_child();
            while let Some(widget) = child {
                if let Some(toggle) = find_switch(&widget) {
                    return Some(toggle);
                }
                child = widget.next_sibling();
            }
            None
        }
        let toggle = find_switch(page.upcast_ref()).unwrap();
        assert!(toggle.is_active());
        toggle.set_active(false);
        assert!(!button.get_visible());
        assert_eq!(
            saved_bool(&connection.borrow(), "menu_visible_sort"),
            Some(false)
        );
        // Existing responsive callbacks must not undo the saved choice.
        button.set_visible(true);
        assert!(!button.get_visible());
        let restored = gtk::Button::new();
        let restored_controls = MenuControls::default();
        restored_controls.register(&connection.borrow(), "Top bar", "sort", "Sort", &restored);
        assert!(!restored.get_visible());
        toggle.set_active(true);
        assert!(button.get_visible());
        restored_controls.reload(&connection.borrow());
        assert!(restored.get_visible());
    }
}
