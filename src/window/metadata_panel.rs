//! Native, theme-inheriting metadata controls. All facet queries run off-thread.
use super::*;
use db::filters::{Facet, Field, Filters, Scope};
use std::sync::{
    atomic::{AtomicBool, Ordering as AtomicOrdering},
    Arc,
};

pub(super) struct MetadataPanel {
    pub button: gtk::ToggleButton,
    pub root: gtk::Revealer,
    connection: Rc<RefCell<Connection>>,
    scope: Rc<dyn Fn() -> Scope>,
    changed: Rc<dyn Fn()>,
    rating: Rc<dyn Fn(&str)>,
    state: RefCell<Filters>,
    chips: gtk::FlowBox,
    indicator: gtk::Label,
    status: gtk::Label,
    index_button: gtk::Button,
    cancel_button: gtk::Button,
    cancel: RefCell<Option<Arc<AtomicBool>>>,
    database: RefCell<Option<std::path::PathBuf>>,
    revision: Cell<u64>,
    request: Cell<u64>,
    explorer: RefCell<Vec<(gtk::DropDown, gtk::Box)>>,
}
impl MetadataPanel {
    pub fn new(
        connection: Rc<RefCell<Connection>>,
        scope: Rc<dyn Fn() -> Scope>,
        changed: Rc<dyn Fn()>,
        rating: Rc<dyn Fn(&str)>,
    ) -> Rc<Self> {
        let button = gtk::ToggleButton::new();
        button.add_css_class("metadata-filter-toggle");
        button.set_tooltip_text(Some("Filter photos by metadata"));
        button.update_property(&[gtk::accessible::Property::Label("Filter")]);
        let title = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        title.append(&gtk::Image::from_icon_name("metadata-filter-symbolic"));
        let indicator = gtk::Label::new(None);
        indicator.add_css_class("caption");
        title.append(&indicator);
        button.set_child(Some(&title));
        let root = gtk::Revealer::new();
        root.set_transition_type(gtk::RevealerTransitionType::SlideDown);
        root.set_transition_duration(150);
        root.add_css_class("metadata-filter-panel");
        let body = gtk::Box::new(gtk::Orientation::Vertical, 6);
        for side in [0, 1] {
            if side == 0 {
                body.set_margin_start(12);
            } else {
                body.set_margin_end(12);
            }
        }
        body.set_margin_top(8);
        body.set_margin_bottom(8);
        let scroll = gtk::ScrolledWindow::new();
        scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        scroll.set_max_content_height(300);
        scroll.set_propagate_natural_height(true);
        scroll.set_child(Some(&body));
        root.set_child(Some(&scroll));
        let controls_scroll = gtk::ScrolledWindow::new();
        controls_scroll.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Never);
        controls_scroll.set_propagate_natural_width(false);
        controls_scroll.set_min_content_width(1);
        controls_scroll.set_max_content_height(42);
        let controls = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        controls.add_css_class("metadata-filter-controls");
        let clear = gtk::Button::from_icon_name("edit-clear-all-symbolic");
        clear.add_css_class("metadata-filter-clear");
        clear.set_tooltip_text(Some("Clear all filters"));
        clear.update_property(&[gtk::accessible::Property::Label("Clear all filters")]);
        controls.append(&clear);
        let explorer_button = gtk::ToggleButton::with_label("Metadata");
        explorer_button.add_css_class("metadata-filter-choice");
        explorer_button.set_tooltip_text(Some("Show metadata explorer"));
        controls.append(&explorer_button);
        controls_scroll.set_child(Some(&controls));
        body.append(&controls_scroll);
        let chips = gtk::FlowBox::new();
        chips.set_selection_mode(gtk::SelectionMode::None);
        chips.set_column_spacing(4);
        chips.set_row_spacing(4);
        chips.set_max_children_per_line(12);
        chips.set_min_children_per_line(1);
        body.append(&chips);
        let explorer_scroll = gtk::ScrolledWindow::new();
        explorer_scroll.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Never);
        explorer_scroll.set_propagate_natural_width(false);
        explorer_scroll.set_min_content_width(360);
        explorer_scroll.set_max_content_width(600);
        explorer_scroll.set_min_content_height(160);
        explorer_scroll.set_max_content_height(260);
        let columns = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        explorer_scroll.set_child(Some(&columns));
        let explorer_revealer = gtk::Revealer::new();
        explorer_revealer.set_transition_type(gtk::RevealerTransitionType::SlideDown);
        explorer_revealer.set_transition_duration(150);
        explorer_revealer.add_css_class("metadata-explorer");
        explorer_revealer.set_child(Some(&explorer_scroll));
        body.append(&explorer_revealer);
        let indexing = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let index_button = gtk::Button::from_icon_name("view-refresh-symbolic");
        index_button.set_tooltip_text(Some("Index missing metadata"));
        index_button.update_property(&[gtk::accessible::Property::Label("Index missing metadata")]);
        let cancel_button = gtk::Button::with_label("Cancel");
        cancel_button.set_visible(false);
        indexing.append(&cancel_button);
        let status = gtk::Label::new(None);
        status.set_visible(false);
        status.set_wrap(true);
        status.set_xalign(0.0);
        status.set_hexpand(true);
        status.add_css_class("dim-label");
        status.add_css_class("caption");
        indexing.append(&status);
        body.append(&indexing);
        let database = db::connection_path(&connection.borrow()).ok();
        let state = Filters::load(&connection.borrow());
        let panel = Rc::new(Self {
            button,
            root,
            connection,
            scope,
            changed,
            rating,
            state: RefCell::new(state),
            chips,
            indicator,
            status,
            index_button,
            cancel_button,
            cancel: RefCell::new(None),
            database: RefCell::new(database),
            revision: Cell::new(0),
            request: Cell::new(0),
            explorer: RefCell::new(Vec::new()),
        });
        for field in [
            Field::Camera,
            Field::Lens,
            Field::Iso,
            Field::Aperture,
            Field::Shutter,
            Field::FocalLength,
            Field::CaptureDate,
            Field::FileType,
        ] {
            let menu = gtk::MenuButton::new();
            menu.add_css_class("metadata-filter-choice");
            menu.set_label(field.label());
            let popover = gtk::Popover::new();
            let content = gtk::Box::new(gtk::Orientation::Vertical, 6);
            content.set_margin_start(10);
            content.set_margin_end(10);
            content.set_margin_top(10);
            content.set_margin_bottom(10);
            popover.set_child(Some(&content));
            menu.set_popover(Some(&popover));
            let weak = Rc::downgrade(&panel);
            popover.connect_show(move |_| {
                if let Some(panel) = weak.upgrade() {
                    panel.populate_editor(field, &content);
                }
            });
            controls.append(&menu);
        }
        let extra = gtk::MenuButton::new();
        extra.add_css_class("metadata-filter-choice");
        extra.set_label("More");
        let popover = gtk::Popover::new();
        let extra_box = gtk::Box::new(gtk::Orientation::Vertical, 6);
        let names = Field::ALL.iter().map(|f| f.label()).collect::<Vec<_>>();
        let choose = gtk::DropDown::from_strings(&names);
        choose.set_selected(1);
        extra_box.append(&choose);
        let editor = gtk::Box::new(gtk::Orientation::Vertical, 6);
        extra_box.append(&editor);
        popover.set_child(Some(&extra_box));
        extra.set_popover(Some(&popover));
        controls.append(&extra);
        let weak = Rc::downgrade(&panel);
        let content = editor.clone();
        choose.connect_selected_notify(move |choice| {
            if let Some(panel) = weak.upgrade() {
                if let Some(field) = Field::ALL.get(choice.selected() as usize) {
                    panel.populate_editor(*field, &content);
                }
            }
        });
        let weak = Rc::downgrade(&panel);
        popover.connect_show(move |_| {
            if let Some(panel) = weak.upgrade() {
                panel.populate_editor(Field::ALL[choose.selected() as usize], &editor);
            }
        });
        // Route to the existing rating state and controls, not a second rating model.
        let rating_button = gtk::MenuButton::new();
        rating_button.add_css_class("metadata-filter-choice");
        rating_button.set_label("Rating");
        let rating_popover = gtk::Popover::new();
        let ratings = gtk::Box::new(gtk::Orientation::Vertical, 2);
        for (label, key) in [
            ("Any rating", "all"),
            ("All Stars", "stars"),
            ("Unrated", "unrated"),
            ("1 star", "1"),
            ("2 stars", "2"),
            ("3 stars", "3"),
            ("4 stars", "4"),
            ("5 stars", "5"),
            ("Corrupt photos", "corrupt"),
        ] {
            let b = gtk::Button::with_label(label);
            b.add_css_class("flat");
            let weak = Rc::downgrade(&panel);
            let popup = rating_popover.downgrade();
            b.connect_clicked(move |_| {
                if let Some(panel) = weak.upgrade() {
                    (panel.rating)(key);
                    panel.sync();
                }
                if let Some(p) = popup.upgrade() {
                    p.popdown();
                }
            });
            ratings.append(&b);
        }
        rating_popover.set_child(Some(&ratings));
        rating_button.set_popover(Some(&rating_popover));
        controls.append(&rating_button);
        controls.append(&panel.index_button);
        for field in [Field::Camera, Field::Lens, Field::Iso, Field::FileType] {
            let column = gtk::Box::new(gtk::Orientation::Vertical, 4);
            column.set_hexpand(true);
            column.set_size_request(165, -1);
            let choose = gtk::DropDown::from_strings(&names);
            choose.set_selected(Field::ALL.iter().position(|f| *f == field).unwrap() as u32);
            column.append(&choose);
            let list = gtk::Box::new(gtk::Orientation::Vertical, 2);
            let sc = gtk::ScrolledWindow::new();
            sc.set_min_content_height(130);
            sc.set_max_content_height(160);
            sc.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
            sc.set_child(Some(&list));
            column.append(&sc);
            columns.append(&column);
            let weak = Rc::downgrade(&panel);
            let content = list.clone();
            choose.connect_selected_notify(move |choice| {
                if let Some(panel) = weak.upgrade() {
                    panel.populate_choices(Field::ALL[choice.selected() as usize], &content);
                }
            });
            panel.explorer.borrow_mut().push((choose, list));
        }
        let weak = Rc::downgrade(&panel);
        explorer_button.connect_toggled(move |button| {
            explorer_revealer.set_reveal_child(button.is_active());
            if button.is_active() {
                if let Some(panel) = weak.upgrade() {
                    panel.refresh_explorer();
                }
            }
        });
        let weak = Rc::downgrade(&panel);
        panel.button.connect_toggled(move |b| {
            if let Some(panel) = weak.upgrade() {
                panel.root.set_reveal_child(b.is_active());
                if b.is_active() {
                    panel.sync();
                }
            }
        });
        let weak = Rc::downgrade(&panel);
        clear.connect_clicked(move |_| {
            if let Some(panel) = weak.upgrade() {
                panel.state.replace(Filters::default());
                (panel.rating)("all");
                panel.apply();
            }
        });
        let weak = Rc::downgrade(&panel);
        panel.index_button.connect_clicked(move |_| {
            if let Some(panel) = weak.upgrade() {
                panel.start_index();
            }
        });
        let weak = Rc::downgrade(&panel);
        panel.cancel_button.connect_clicked(move |_| {
            if let Some(panel) = weak.upgrade() {
                panel.cancel_index();
                panel.status.set_text("Stopping after the current file…");
            }
        });
        let weak = Rc::downgrade(&panel);
        let mut previous = Scope::default();
        glib::timeout_add_local(Duration::from_millis(300), move || {
            let Some(panel) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            let path = db::connection_path(&panel.connection.borrow()).ok();
            let scope = (panel.scope)();
            if path != *panel.database.borrow() {
                panel.cancel_index();
                panel.database.replace(path);
                panel
                    .state
                    .replace(Filters::load(&panel.connection.borrow()));
                panel.revision.set(panel.revision.get() + 1);
                panel.sync();
            }
            if scope != previous {
                // The original toolbar's rating control and this panel share state.
                // Refresh Bookshelf counts as well as the photo grid for either entry point.
                if scope.bookshelf && scope.rating != previous.rating && !previous.rating.is_empty()
                {
                    (panel.changed)();
                }
                previous = scope;
                panel.revision.set(panel.revision.get() + 1);
                panel.sync();
            }
            glib::ControlFlow::Continue
        });
        panel.sync();
        panel
    }
    pub fn cancel_index(&self) {
        if let Some(cancel) = self.cancel.borrow().as_ref() {
            cancel.store(true, AtomicOrdering::Relaxed);
        }
    }
    fn apply(self: &Rc<Self>) {
        match self.state.borrow().save(&self.connection.borrow()) {
            Ok(()) => {}
            Err(error) => {
                self.status
                    .set_text(&format!("Could not apply filters: {error}"));
                return;
            }
        }
        self.revision.set(self.revision.get() + 1);
        self.sync();
        (self.changed)();
    }
    fn sync(self: &Rc<Self>) {
        while let Some(child) = self.chips.first_child() {
            self.chips.remove(&child);
        }
        let state = self.state.borrow().clone();
        let rating = (self.scope)().rating;
        let active = state.categories.len() + usize::from(!matches!(rating.as_str(), "" | "all"));
        self.indicator.set_text(if active == 0 { "" } else { "●" });
        self.button
            .set_tooltip_text(Some(&format!("Filter photos · {active} active categories")));
        for (field, selection) in state.categories {
            for value in selection.values {
                let chip = gtk::Button::with_label(&format!(
                    "{}: {} ×",
                    field.label(),
                    field.display(&value)
                ));
                chip.add_css_class("flat");
                chip.set_tooltip_text(Some(&format!(
                    "Remove {}: {}",
                    field.label(),
                    field.display(&value)
                )));
                if let Some(label) = chip.child().and_then(|w| w.downcast::<gtk::Label>().ok()) {
                    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
                    label.set_max_width_chars(28);
                }
                let weak = Rc::downgrade(self);
                chip.connect_clicked(move |_| {
                    if let Some(panel) = weak.upgrade() {
                        panel.state.borrow_mut().toggle(field, value.clone(), false);
                        panel.apply();
                    }
                });
                self.chips.insert(&chip, -1);
            }
            if selection.min.is_some() || selection.max.is_some() {
                let chip = gtk::Button::with_label(&format!(
                    "{}: {} – {} ×",
                    field.label(),
                    selection.min.as_deref().unwrap_or("Any"),
                    selection.max.as_deref().unwrap_or("Any")
                ));
                chip.add_css_class("flat");
                let weak = Rc::downgrade(self);
                chip.connect_clicked(move |_| {
                    if let Some(panel) = weak.upgrade() {
                        let _ = panel.state.borrow_mut().range(field, "", "");
                        panel.apply();
                    }
                });
                self.chips.insert(&chip, -1);
            }
        }
        if !matches!(rating.as_str(), "" | "all") {
            let chip = gtk::Button::with_label(&format!("Rating: {rating} ×"));
            let weak = Rc::downgrade(self);
            chip.connect_clicked(move |_| {
                if let Some(panel) = weak.upgrade() {
                    (panel.rating)("all");
                    panel.sync();
                }
            });
            self.chips.insert(&chip, -1);
        }
        self.chips.set_visible(active > 0);
        if self.root.reveals_child() {
            self.refresh_explorer();
        }
    }
    fn populate_editor(self: &Rc<Self>, field: Field, content: &gtk::Box) {
        while let Some(child) = content.first_child() {
            content.remove(&child);
        }
        if field.numeric() || field == Field::CaptureDate {
            let range = gtk::Box::new(gtk::Orientation::Horizontal, 4);
            let min = gtk::Entry::new();
            let max = gtk::Entry::new();
            min.set_width_chars(10);
            max.set_width_chars(10);
            min.set_placeholder_text(Some("Minimum"));
            max.set_placeholder_text(Some("Maximum"));
            let tooltip = if field == Field::CaptureDate {
                "YYYY-MM-DD (inclusive)"
            } else if field == Field::Shutter {
                "Seconds, e.g. 0.001 for 1/1000"
            } else {
                "Inclusive numeric range"
            };
            min.set_tooltip_text(Some(tooltip));
            max.set_tooltip_text(Some(tooltip));
            if let Some(s) = self.state.borrow().categories.get(&field) {
                min.set_text(s.min.as_deref().unwrap_or(""));
                max.set_text(s.max.as_deref().unwrap_or(""));
            }
            let apply = gtk::Button::with_label("Apply range");
            range.append(&min);
            range.append(&max);
            range.append(&apply);
            content.append(&range);
            let error = gtk::Label::new(None);
            error.set_wrap(true);
            content.append(&error);
            let weak = Rc::downgrade(self);
            apply.connect_clicked(move |_| {
                if let Some(panel) = weak.upgrade() {
                    let result = panel
                        .state
                        .borrow_mut()
                        .range(field, &min.text(), &max.text());
                    match result {
                        Ok(()) => {
                            error.set_text("");
                            panel.apply();
                        }
                        Err(e) => error.set_text(&e.to_string()),
                    }
                }
            });
        }
        let list = gtk::Box::new(gtk::Orientation::Vertical, 2);
        let scroll = gtk::ScrolledWindow::new();
        scroll.set_min_content_width(250);
        scroll.set_min_content_height(120);
        scroll.set_max_content_height(260);
        scroll.set_propagate_natural_height(true);
        scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        scroll.set_child(Some(&list));
        content.append(&scroll);
        self.populate_choices(field, &list);
    }
    fn refresh_explorer(self: &Rc<Self>) {
        for (choice, list) in self.explorer.borrow().iter() {
            self.populate_choices(Field::ALL[choice.selected() as usize], list);
        }
    }
    fn populate_choices(self: &Rc<Self>, field: Field, list: &gtk::Box) {
        while let Some(child) = list.first_child() {
            list.remove(&child);
        }
        list.append(&gtk::Label::new(Some("Loading…")));
        let Some(path) = db::connection_path(&self.connection.borrow()).ok() else {
            return;
        };
        let filters = self.state.borrow().clone();
        let scope = (self.scope)();
        let revision = self.revision.get();
        self.request.set(self.request.get().wrapping_add(1));
        let request = format!("metadata-request-{}", self.request.get());
        list.set_widget_name(&request);
        let expected_scope = scope.clone();
        let expected_path = path.clone();
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let start = Instant::now();
            let result = db::open_existing(&path)
                .and_then(|c| db::filters::facets(&c, &scope, &filters, field));
            if std::env::var_os("PIC_METADATA_TRACE").is_some() {
                eprintln!(
                    "PIC_METADATA stage=facets field={field:?} elapsed_ms={}",
                    start.elapsed().as_millis()
                );
            }
            let _ = sender.send(result);
        });
        let weak = Rc::downgrade(self);
        let list = list.downgrade();
        glib::timeout_add_local(Duration::from_millis(25), move || {
            let (Some(panel), Some(list)) = (weak.upgrade(), list.upgrade()) else {
                return glib::ControlFlow::Break;
            };
            match receiver.try_recv() {
                Ok(result) => {
                    if list.widget_name().as_str() != request {
                        return glib::ControlFlow::Break;
                    }
                    if revision != panel.revision.get()
                        || (panel.scope)() != expected_scope
                        || db::connection_path(&panel.connection.borrow())
                            .ok()
                            .as_ref()
                            != Some(&expected_path)
                    {
                        panel.populate_choices(field, &list);
                        return glib::ControlFlow::Break;
                    }
                    while let Some(child) = list.first_child() {
                        list.remove(&child);
                    }
                    match result {
                        Ok(rows) => panel.fill_choices(field, &list, rows),
                        Err(e) => list.append(&gtk::Label::new(Some(&format!(
                            "Could not load metadata: {e}"
                        )))),
                    };
                    glib::ControlFlow::Break
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(_) => glib::ControlFlow::Break,
            }
        });
    }
    fn fill_choices(self: &Rc<Self>, field: Field, list: &gtk::Box, mut rows: Vec<Facet>) {
        let selected = self
            .state
            .borrow()
            .categories
            .get(&field)
            .map(|s| s.values.clone())
            .unwrap_or_default();
        for value in &selected {
            if !rows.iter().any(|r| &r.value == value) {
                rows.push(Facet {
                    value: value.clone(),
                    count: 0,
                });
            }
        }
        if rows.is_empty() {
            list.append(&gtk::Label::new(Some("No values in this collection")));
        }
        self.append_choice_page(field, list, rows, selected);
    }
    fn append_choice_page(
        self: &Rc<Self>,
        field: Field,
        list: &gtk::Box,
        mut rows: Vec<Facet>,
        selected: Vec<String>,
    ) {
        let remaining = if rows.len() > 100 {
            rows.split_off(100)
        } else {
            Vec::new()
        };
        for row in rows {
            let check = gtk::CheckButton::with_label(&format!(
                "{} ({})",
                field.display(&row.value),
                row.count
            ));
            check.set_active(selected.contains(&row.value));
            // Long lens names must not force the application wider.
            if let Some(label) = check.child().and_then(|w| w.downcast::<gtk::Label>().ok()) {
                label.set_ellipsize(gtk::pango::EllipsizeMode::End);
                label.set_max_width_chars(32);
            }
            check.set_tooltip_text(Some(&field.display(&row.value)));
            let weak = Rc::downgrade(self);
            check.connect_toggled(move |b| {
                if let Some(panel) = weak.upgrade() {
                    panel
                        .state
                        .borrow_mut()
                        .toggle(field, row.value.clone(), b.is_active());
                    panel.apply();
                }
            });
            list.append(&check);
        }
        if !remaining.is_empty() {
            let more =
                gtk::Button::with_label(&format!("Show more ({} remaining)", remaining.len()));
            let weak = Rc::downgrade(self);
            let list_weak = list.downgrade();
            let remaining = RefCell::new(Some(remaining));
            more.connect_clicked(move |button| {
                if let (Some(panel), Some(list), Some(rows)) = (
                    weak.upgrade(),
                    list_weak.upgrade(),
                    remaining.borrow_mut().take(),
                ) {
                    list.remove(button);
                    let selected = panel
                        .state
                        .borrow()
                        .categories
                        .get(&field)
                        .map(|s| s.values.clone())
                        .unwrap_or_default();
                    panel.append_choice_page(field, &list, rows, selected);
                }
            });
            list.append(&more);
        }
    }
    fn start_index(self: &Rc<Self>) {
        if self.cancel.borrow().is_some() {
            return;
        }
        let Some(path) = self.database.borrow().clone() else {
            return;
        };
        let (cancel, receiver) = crate::metadata_index::start(path.clone());
        self.cancel.replace(Some(cancel));
        self.index_button.set_sensitive(false);
        self.cancel_button.set_visible(true);
        self.status.set_visible(true);
        let weak = Rc::downgrade(self);
        glib::timeout_add_local(Duration::from_millis(150), move || {
            let Some(panel) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            let mut latest = None;
            while let Ok(p) = receiver.try_recv() {
                latest = Some(p);
            }
            if let Some(p) = latest {
                if *panel.database.borrow() != Some(path.clone()) {
                    panel.cancel_index();
                    panel.cancel.take();
                    panel.index_button.set_sensitive(true);
                    panel.cancel_button.set_visible(false);
                    return glib::ControlFlow::Break;
                }
                panel.status.set_text(&format!(
                    "{} {} / {} · {} indexed · {} unavailable or changed",
                    if p.cancelled {
                        "Stopped"
                    } else if p.finished {
                        "Finished"
                    } else {
                        "Indexing"
                    },
                    p.examined,
                    p.total,
                    p.updated,
                    p.skipped
                ));
                if p.finished {
                    panel.cancel.take();
                    panel.index_button.set_sensitive(true);
                    panel.cancel_button.set_visible(false);
                    if let Some(error) = p.error {
                        panel.status.set_text(&error);
                    }
                    panel.revision.set(panel.revision.get() + 1);
                    panel.sync();
                    (panel.changed)();
                    return glib::ControlFlow::Break;
                }
            }
            glib::ControlFlow::Continue
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn settle() {
        let start = Instant::now();
        let context = glib::MainContext::default();
        while start.elapsed() < Duration::from_millis(350) {
            while context.pending() {
                context.iteration(false);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    fn descendants(widget: &gtk::Widget) -> Vec<gtk::Widget> {
        let mut result = vec![widget.clone()];
        let mut child = widget.first_child();
        while let Some(w) = child {
            result.extend(descendants(&w));
            child = w.next_sibling();
        }
        result
    }
    #[test]
    #[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
    fn metadata_panel_theme_layout_and_close_reopen() {
        adw::init().unwrap();
        crate::register_bundled_icons();
        gtk::IconTheme::for_display(&gtk::gdk::Display::default().unwrap())
            .add_resource_path("/picrs/icons");
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch(db::SCHEMA).unwrap();
        let c = Rc::new(RefCell::new(c));
        let changes = Rc::new(Cell::new(0));
        let calls = changes.clone();
        let panel = MetadataPanel::new(
            c.clone(),
            Rc::new(Scope::default),
            Rc::new(move || calls.set(calls.get() + 1)),
            Rc::new(|_| {}),
        );
        let window = gtk::Window::builder()
            .default_width(900)
            .default_height(550)
            .build();
        let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let header = gtk::HeaderBar::new();
        let tools = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        let sort = gtk::MenuButton::new();
        sort.set_label("Sort");
        tools.append(&sort);
        tools.append(&panel.button);
        header.pack_end(&tools);
        body.append(&header);
        body.append(&panel.root);
        let photos = gtk::Label::new(Some("Photo browsing area"));
        photos.set_vexpand(true);
        body.append(&photos);
        window.set_child(Some(&body));
        window.present();
        settle();
        assert_eq!(sort.next_sibling(), Some(panel.button.clone().upcast()));
        panel.button.set_active(true);
        settle();
        assert!(panel.root.reveals_child());
        panel
            .state
            .borrow_mut()
            .toggle(Field::Camera, "nikon z 5".into(), true);
        panel.apply();
        assert_eq!(changes.get(), 1);
        panel.button.set_active(false);
        settle();
        assert!(!panel.root.reveals_child());
        assert!(!Filters::load(&c.borrow()).is_empty());
        panel.button.set_active(true);
        settle();
        assert!(!panel.state.borrow().is_empty());
        assert_eq!(panel.indicator.text(), "●");
        let clear = descendants(panel.root.upcast_ref())
            .into_iter()
            .filter_map(|w| w.downcast::<gtk::Button>().ok())
            .find(|b| b.has_css_class("metadata-filter-clear"))
            .unwrap();
        clear.emit_clicked();
        assert!(Filters::load(&c.borrow()).is_empty());
        let display = gtk::gdk::Display::default().unwrap();
        crate::css::install_foundation(&display);
        adw::StyleManager::default().set_color_scheme(adw::ColorScheme::Default);
        for width in [900, 420] {
            window.set_default_size(width, 550);
            settle();
            assert!(
                window.width() <= width + 20,
                "Standard GTK4 forces width {} at {width}",
                window.width()
            );
            assert!(
                panel.root.height() < 400,
                "Filter panel consumes photo viewport"
            );
        }
        if let Some(path) =
            std::env::var_os("PIC_METADATA_SCREENSHOTS").map(std::path::PathBuf::from)
        {
            std::fs::create_dir_all(&path).unwrap();
            let snapshot = gtk::Snapshot::new();
            gtk::WidgetPaintable::new(Some(&window)).snapshot(
                &snapshot,
                f64::from(window.width()),
                f64::from(window.height()),
            );
            window
                .renderer()
                .unwrap()
                .render_texture(snapshot.to_node().unwrap(), None)
                .save_to_png(path.join("standard-gtk4.png"))
                .unwrap();
        }
        window.close();
    }
}

#[cfg(test)]
mod app_tests {
    use super::*;
    fn walk(w: &gtk::Widget) -> Vec<gtk::Widget> {
        let mut out = vec![w.clone()];
        let mut child = w.first_child();
        while let Some(c) = child {
            out.extend(walk(&c));
            child = c.next_sibling();
        }
        out
    }
    fn wait_until(mut predicate: impl FnMut() -> bool) {
        let start = Instant::now();
        let context = glib::MainContext::default();
        loop {
            while context.pending() {
                context.iteration(false);
            }
            if predicate() {
                break;
            }
            assert!(
                start.elapsed() < Duration::from_secs(12),
                "GTK condition timed out"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    #[test]
    #[ignore = "requires isolated GTK display; run individually with --ignored"]
    fn metadata_filters_actual_window_folder_album_and_layouts() {
        adw::init().unwrap();
        crate::register_bundled_icons();
        gtk::IconTheme::for_display(&gtk::gdk::Display::default().unwrap())
            .add_resource_path("/picrs/icons");
        let dir = std::env::temp_dir().join(format!("pic-metadata-app-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let c = db::open(&dir.join("library.db")).unwrap();
        let folder = db::insert_folder(&c, dir.to_str().unwrap()).unwrap();
        for (name, camera, iso) in [
            ("nikon-low.jpg", "NIKON Z 5", 100),
            ("nikon-high.jpg", "NIKON Z 5", 800),
            ("canon.jpg", "Canon R5", 100),
        ] {
            let path = dir.join(name);
            image::RgbImage::from_pixel(64, 48, image::Rgb([80, 140, 210]))
                .save(&path)
                .unwrap();
            let id = db::upsert_photo(
                &c,
                &path,
                Some(folder),
                &db::PhotoMetadata {
                    camera: Some(camera.into()),
                    iso: Some(iso),
                    width: Some(64),
                    height: Some(48),
                    searchable: db::metadata::SearchMetadata {
                        camera_model: Some(camera.into()),
                        has_exif: Some(true),
                        ..Default::default()
                    },
                    ..Default::default()
                },
            )
            .unwrap();
            if iso == 100 {
                db::set_rating(&c, id, 3).unwrap();
            }
        }
        let album = db::create_album(&c, "Metadata test").unwrap();
        c.execute(
            "INSERT INTO album_photos SELECT ?1,id FROM photos",
            [album.id],
        )
        .unwrap();
        for (key, value) in [
            (LAST_VIEW_SETTING_KEY, format!("folder:{folder}")),
            ("onboarding-never-show", "true".into()),
            ("thumbnail-file-names", "true".into()),
        ] {
            db::set_setting(&c, key, &value).unwrap();
        }
        let app = adw::Application::builder()
            .application_id("io.pic.MetadataFilterTest")
            .build();
        app.register(None::<&gio::Cancellable>).unwrap();
        let window = build(&app, c);
        window.present();
        let root = window.clone().upcast::<gtk::Widget>();
        let filter = walk(&root)
            .into_iter()
            .find(|w| w.has_css_class("metadata-filter-toggle"))
            .unwrap()
            .downcast::<gtk::ToggleButton>()
            .unwrap();
        assert_eq!(
            filter.prev_sibling().unwrap().tooltip_text().as_deref(),
            Some("Sort photos")
        );
        filter.set_active(true);
        let metadata = walk(&root)
            .into_iter()
            .filter_map(|w| w.downcast::<gtk::ToggleButton>().ok())
            .find(|b| b.label().as_deref() == Some("Metadata"))
            .unwrap();
        metadata.set_active(true);
        let explorer = walk(&root)
            .into_iter()
            .find(|w| w.has_css_class("metadata-explorer"))
            .unwrap();
        wait_until(|| {
            walk(&explorer)
                .iter()
                .filter_map(|w| w.downcast_ref::<gtk::CheckButton>())
                .any(|b| b.label().as_deref() == Some("nikon z 5 (2)"))
        });
        let camera = walk(&root)
            .into_iter()
            .filter_map(|w| w.downcast::<gtk::MenuButton>().ok())
            .find(|b| b.label().as_deref() == Some("Camera"))
            .unwrap();
        camera.popup();
        let popup = camera.popover().unwrap();
        wait_until(|| {
            walk(popup.upcast_ref())
                .iter()
                .filter_map(|w| w.downcast_ref::<gtk::CheckButton>())
                .any(|b| b.label().as_deref() == Some("nikon z 5 (2)"))
        });
        let choice = walk(popup.upcast_ref())
            .into_iter()
            .filter_map(|w| w.downcast::<gtk::CheckButton>().ok())
            .find(|b| b.label().as_deref() == Some("nikon z 5 (2)"))
            .unwrap();
        choice.set_active(true);
        popup.popdown();
        let query = db::open_existing(&dir.join("library.db")).unwrap();
        wait_until(|| db::browse_photos(&query, None, false, None).unwrap().len() == 2);
        filter.set_active(false);
        filter.set_active(true);
        assert!(!Filters::load(&query).is_empty());
        let iso = walk(&root)
            .into_iter()
            .filter_map(|w| w.downcast::<gtk::MenuButton>().ok())
            .find(|b| b.label().as_deref() == Some("ISO"))
            .unwrap();
        iso.popup();
        let popup = iso.popover().unwrap();
        wait_until(|| {
            walk(popup.upcast_ref())
                .iter()
                .filter_map(|w| w.downcast_ref::<gtk::CheckButton>())
                .any(|b| b.label().as_deref() == Some("100 (1)"))
        });
        walk(popup.upcast_ref())
            .into_iter()
            .filter_map(|w| w.downcast::<gtk::CheckButton>().ok())
            .find(|b| b.label().as_deref() == Some("100 (1)"))
            .unwrap()
            .set_active(true);
        popup.popdown();
        wait_until(|| db::browse_photos(&query, None, false, None).unwrap().len() == 1);
        assert_eq!(
            db::browse_photos_in_album(&query, album.id, None)
                .unwrap()
                .len(),
            1
        );
        assert_eq!(db::browse_albums(&query).unwrap()[0].photo_count, 1);
        let gallery = Rc::new(crate::grid::Gallery::new(
            &[],
            640,
            |_| {},
            |_, _, _| {},
            |_, _, _, _| {},
            |_, _| {},
            |_| {},
        ));
        let connection = Rc::new(RefCell::new(
            db::open_existing(&dir.join("library.db")).unwrap(),
        ));
        for layout in [
            crate::grid::PhotoLayout::Grid,
            crate::grid::PhotoLayout::PhotoWall,
            crate::grid::PhotoLayout::Masonry,
        ] {
            gallery.set_layout(layout);
            for scope in [
                sidebar::SidebarFilter::All,
                sidebar::SidebarFilter::Folder(folder),
                sidebar::SidebarFilter::Album(album.id),
            ] {
                refresh_grid_preserving_folder_viewport(
                    &connection,
                    scope,
                    "",
                    PhotoSort {
                        field: SortField::Name,
                        direction: SortDirection::Ascending,
                    },
                    &gallery,
                );
                wait_until(|| gallery.photo_objects().len() == 1);
                assert_eq!(
                    gallery.photo_objects()[0].camera().as_deref(),
                    Some("NIKON Z 5")
                );
            }
        }

        let clear = walk(&root)
            .into_iter()
            .filter_map(|w| w.downcast::<gtk::Button>().ok())
            .find(|b| b.has_css_class("metadata-filter-clear"))
            .unwrap();
        clear.emit_clicked();
        wait_until(|| db::browse_photos(&query, None, false, None).unwrap().len() == 3);
        // Wait for the asynchronous gallery to settle before destroying its surface.
        let start = Instant::now();
        wait_until(|| start.elapsed() > Duration::from_millis(500));
        window.close();
        drop(query);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
