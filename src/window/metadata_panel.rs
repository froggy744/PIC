//! Native, theme-inheriting metadata controls. All facet queries run off-thread.
use super::*;
use db::filters::{Facet, Field, Filters, Scope};
use std::sync::{
    atomic::{AtomicBool, Ordering as AtomicOrdering},
    Arc,
};

const VISIBLE_FILTERS_SETTING: &str = "photo-metadata-visible-filters";
const EXPLORER_COLUMNS_SETTING: &str = "photo-metadata-explorer-columns";
const EXPLORER_WIDTHS_SETTING: &str = "photo-metadata-explorer-widths";
const DEFAULT_VISIBLE_FILTERS: &[Field] = &[
    Field::Camera,
    Field::CameraMake,
    Field::CaptureDate,
    Field::FileType,
];

#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct VisibleFilterSelection {
    fields: Vec<Field>,
    rating: bool,
}

fn load_visible_filters(connection: &Connection) -> VisibleFilterSelection {
    db::setting(connection, VISIBLE_FILTERS_SETTING)
        .ok()
        .flatten()
        .and_then(|raw| serde_json::from_str::<VisibleFilterSelection>(&raw).ok())
        .map(|mut selection| {
            if selection.fields
                == [
                    Field::Camera,
                    Field::Lens,
                    Field::Iso,
                    Field::CaptureDate,
                    Field::FileType,
                ]
            {
                selection.fields = DEFAULT_VISIBLE_FILTERS.to_vec();
            }
            selection
        })
        .unwrap_or_else(|| VisibleFilterSelection {
            fields: DEFAULT_VISIBLE_FILTERS.to_vec(),
            rating: true,
        })
}

fn load_explorer_fields(connection: &Connection) -> Vec<Field> {
    db::setting(connection, EXPLORER_COLUMNS_SETTING)
        .ok()
        .flatten()
        .and_then(|raw| serde_json::from_str::<Vec<Field>>(&raw).ok())
        .map(|fields| {
            let fields = fields
                .into_iter()
                .filter(|field| Field::ALL.contains(field))
                .take(8)
                .collect::<Vec<_>>();
            if fields == [Field::Camera, Field::Lens, Field::Iso, Field::FileType] {
                vec![Field::Camera, Field::Aperture, Field::Shutter]
            } else {
                fields
            }
        })
        .filter(|fields: &Vec<Field>| !fields.is_empty())
        .unwrap_or_else(|| vec![Field::Camera, Field::Aperture, Field::Shutter])
}

struct ExplorerColumn {
    root: gtk::Box,
    choose: gtk::DropDown,
    search: gtk::SearchEntry,
    list: gtk::Box,
    width: Rc<Cell<i32>>,
    add: gtk::Button,
    remove: gtk::Button,
}

pub(super) struct MetadataPanel {
    pub button: gtk::ToggleButton,
    pub root: gtk::Box,
    explorer_revealer: gtk::Revealer,
    all_button: gtk::ToggleButton,
    camera_control: gtk::Box,
    individual_controls: gtk::ScrolledWindow,
    lightbox_open: Cell<bool>,
    connection: Rc<RefCell<Connection>>,
    scope: Rc<dyn Fn() -> Scope>,
    changed: Rc<dyn Fn()>,
    rating: Rc<dyn Fn(&str)>,
    state: RefCell<Filters>,
    chips: gtk::Box,
    chips_scroll: gtk::ScrolledWindow,
    indicator: gtk::Label,
    photo_count: gtk::Label,
    active_count: gtk::Label,
    status: gtk::Label,
    indexing: gtk::Box,
    index_button: gtk::Button,
    refresh_button: gtk::Button,
    refresh_action: RefCell<Option<Rc<dyn Fn()>>>,
    cancel_button: gtk::Button,
    cancel: RefCell<Option<Arc<AtomicBool>>>,
    database: RefCell<Option<std::path::PathBuf>>,
    revision: Cell<u64>,
    request: Cell<u64>,
    count_request: Cell<u64>,
    filter_selection: RefCell<VisibleFilterSelection>,
    filter_options: RefCell<Vec<(Option<Field>, gtk::CheckButton)>>,
    filter_buttons: RefCell<Vec<(Field, gtk::MenuButton)>>,
    explorer_columns: gtk::Box,
    explorer: RefCell<Vec<ExplorerColumn>>,
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
        button.add_css_class("metadata-header-toggle");
        button.set_tooltip_text(Some("Show or hide photo filters"));
        button.update_property(&[gtk::accessible::Property::Label("Filters")]);
        // Icon-only beside Sort and the existing rating button in the top bar.
        let title = gtk::Box::new(gtk::Orientation::Horizontal, 3);
        // Prefer the standard GTK icon when available. The bundled symbolic
        // filter icon is a reliable fallback on themes without view-filter.
        let filter_icon_name = gtk::gdk::Display::default()
            .map(|display| gtk::IconTheme::for_display(&display).has_icon("view-filter-symbolic"))
            .unwrap_or(false);
        title.append(&gtk::Image::from_icon_name(if filter_icon_name {
            "view-filter-symbolic"
        } else {
            "metadata-filter-symbolic"
        }));
        let indicator = gtk::Label::new(None);
        indicator.add_css_class("caption");
        title.append(&indicator);
        button.set_child(Some(&title));
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.add_css_class("metadata-filter-panel");
        // Metadata options never occupy photo-grid space until the header toggle is used.
        root.set_visible(false);
        let body = gtk::Box::new(gtk::Orientation::Vertical, 8);
        body.set_margin_start(12);
        body.set_margin_end(12);
        body.set_margin_top(8);
        body.set_margin_bottom(10);
        root.append(&body);

        let toolbar = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        toolbar.add_css_class("metadata-filter-toolbar");
        // ALL is always the first button, followed by Camera when individual
        // filters are visible. The summary remains anchored at the right.
        let camera_control = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        camera_control.add_css_class("metadata-filter-controls");
        let all_button = gtk::ToggleButton::with_label("ALL");
        all_button.add_css_class("metadata-filter-all");
        all_button.set_tooltip_text(Some("Show all metadata columns"));
        all_button.update_property(&[gtk::accessible::Property::Label(
            "Switch between individual filters and all metadata columns",
        )]);
        toolbar.append(&all_button);
        toolbar.append(&camera_control);
        // Reserve the flexible space between ALL and the right-aligned summary.
        let middle = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        middle.add_css_class("metadata-filter-middle");
        middle.set_hexpand(true);
        let controls_scroll = gtk::ScrolledWindow::new();
        controls_scroll.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Never);
        controls_scroll.set_propagate_natural_width(false);
        controls_scroll.set_min_content_width(1);
        controls_scroll.set_max_content_height(42);
        controls_scroll.set_hexpand(true);
        let controls = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        controls.add_css_class("metadata-filter-controls");
        let filters_button = gtk::MenuButton::new();
        filters_button.add_css_class("metadata-filter-choice");
        filters_button.set_icon_name("list-add-symbolic");
        filters_button
            .update_property(&[gtk::accessible::Property::Label("Choose filter categories")]);
        filters_button.set_tooltip_text(Some("Choose filter categories to show"));
        let filters_popover = gtk::Popover::new();
        let filters_list = gtk::Box::new(gtk::Orientation::Vertical, 2);
        filters_list.set_margin_start(10);
        filters_list.set_margin_end(10);
        filters_list.set_margin_top(8);
        filters_list.set_margin_bottom(8);
        let filters_scroll = gtk::ScrolledWindow::new();
        filters_scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        filters_scroll.set_min_content_width(220);
        filters_scroll.set_min_content_height(240);
        filters_scroll.set_max_content_height(300);
        filters_scroll.set_propagate_natural_width(true);
        filters_scroll.set_propagate_natural_height(true);
        filters_scroll.set_child(Some(&filters_list));
        filters_popover.set_child(Some(&filters_scroll));
        filters_button.set_popover(Some(&filters_popover));
        controls_scroll.set_child(Some(&controls));
        middle.append(&controls_scroll);
        let chips = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        chips.add_css_class("metadata-selected-chips");
        let chips_scroll = gtk::ScrolledWindow::new();
        chips_scroll.add_css_class("metadata-chips-scroll");
        chips_scroll.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Never);
        chips_scroll.set_propagate_natural_width(false);
        chips_scroll.set_min_content_width(1);
        chips_scroll.set_max_content_height(42);
        chips_scroll.set_hexpand(true);
        chips_scroll.set_child(Some(&chips));
        chips_scroll.set_visible(false);
        middle.append(&chips_scroll);
        toolbar.append(&middle);

        let summary = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        summary.add_css_class("metadata-filter-summary");
        let photo_count = gtk::Label::new(Some("— photos"));
        photo_count.add_css_class("dim-label");
        summary.append(&photo_count);
        let active_count = gtk::Label::new(None);
        active_count.add_css_class("metadata-filter-active-count");
        summary.append(&active_count);
        let clear = gtk::Button::with_label("Clear all");
        clear.add_css_class("metadata-filter-clear");
        clear.set_tooltip_text(Some("Clear all filters"));
        clear.update_property(&[gtk::accessible::Property::Label("Clear all filters")]);
        summary.append(&clear);
        let refresh_button = gtk::Button::from_icon_name("view-refresh-symbolic");
        refresh_button.add_css_class("metadata-filter-utility");
        refresh_button.set_tooltip_text(Some("Refresh library"));
        refresh_button.update_property(&[gtk::accessible::Property::Label("Refresh library")]);
        let index_button = gtk::Button::from_icon_name("network-transmit-receive-symbolic");
        index_button.add_css_class("metadata-filter-utility");
        index_button.set_tooltip_text(Some("Index missing metadata"));
        index_button.update_property(&[gtk::accessible::Property::Label("Index missing metadata")]);
        summary.append(&index_button);
        summary.append(&refresh_button);
        toolbar.append(&summary);
        body.append(&toolbar);
        let explorer_scroll = gtk::ScrolledWindow::new();
        explorer_scroll.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Never);
        explorer_scroll.set_propagate_natural_width(false);
        explorer_scroll.set_min_content_width(1);
        explorer_scroll.set_hexpand(true);
        explorer_scroll.set_min_content_height(205);
        explorer_scroll.set_max_content_height(235);
        explorer_scroll.set_propagate_natural_height(true);
        let columns = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        columns.set_hexpand(true);
        explorer_scroll.set_child(Some(&columns));
        explorer_scroll.add_css_class("metadata-explorer");
        let explorer_revealer = gtk::Revealer::new();
        explorer_revealer.set_transition_type(gtk::RevealerTransitionType::SlideDown);
        explorer_revealer.set_transition_duration(150);
        explorer_revealer.set_reveal_child(false);
        explorer_revealer.set_child(Some(&explorer_scroll));
        explorer_revealer.set_child_visible(false);
        body.append(&explorer_revealer);
        let indexing = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        indexing.set_visible(false);
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
        let visible_filters = load_visible_filters(&connection.borrow());
        let explorer_fields = load_explorer_fields(&connection.borrow());
        let panel = Rc::new(Self {
            button,
            root,
            explorer_revealer,
            all_button: all_button.clone(),
            camera_control,
            // Hide only the buttons when ALL is active, not their expanding
            // scroll host. That host reserves flexible width so counts and
            // Clear all never jump left next to ALL.
            individual_controls: controls_scroll,
            lightbox_open: Cell::new(false),
            connection,
            scope,
            changed,
            rating,
            state: RefCell::new(state),
            chips,
            chips_scroll,
            indicator,
            photo_count,
            active_count,
            status,
            indexing,
            index_button,
            refresh_button,
            refresh_action: RefCell::new(None),
            cancel_button,
            cancel: RefCell::new(None),
            database: RefCell::new(database),
            revision: Cell::new(0),
            request: Cell::new(0),
            count_request: Cell::new(0),
            filter_selection: RefCell::new(visible_filters),
            filter_options: RefCell::new(Vec::new()),
            filter_buttons: RefCell::new(Vec::new()),
            explorer_columns: columns,
            explorer: RefCell::new(Vec::new()),
        });
        for field in Field::ALL {
            let menu = gtk::MenuButton::new();
            menu.add_css_class("metadata-filter-choice");
            menu.set_label(if field == Field::CameraMake {
                "Manufacturer"
            } else {
                field.label()
            });
            menu.set_visible(panel.filter_selection.borrow().fields.contains(&field));
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
            if field == Field::Camera {
                panel.camera_control.append(&menu);
            } else {
                controls.append(&menu);
            }
            panel.filter_buttons.borrow_mut().push((field, menu));

            let option = gtk::CheckButton::with_label(field.label());
            option.set_active(panel.filter_selection.borrow().fields.contains(&field));
            filters_list.append(&option);
            let weak = Rc::downgrade(&panel);
            option.connect_toggled(move |check| {
                if let Some(panel) = weak.upgrade() {
                    let mut selection = panel.filter_selection.borrow_mut();
                    if check.is_active() {
                        if !selection.fields.contains(&field) {
                            selection.fields.push(field);
                        }
                    } else {
                        selection.fields.retain(|selected| *selected != field);
                    }
                    drop(selection);
                    panel.sync_filter_visibility();
                    panel.save_visible_filters();
                }
            });
            panel
                .filter_options
                .borrow_mut()
                .push((Some(field), option));
        }
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
        controls.append(&filters_button);
        // Indexing belongs with the toolbar utilities, not the filter chips.
        let rating_option = gtk::CheckButton::with_label("Rating");
        let rating_visible = panel.filter_selection.borrow().rating;
        rating_button.set_visible(rating_visible);
        rating_option.set_active(rating_visible);
        filters_list.append(&rating_option);
        let weak = Rc::downgrade(&panel);
        rating_option.connect_toggled(move |check| {
            if let Some(panel) = weak.upgrade() {
                panel.filter_selection.borrow_mut().rating = check.is_active();
                panel.sync_filter_visibility();
                panel.save_visible_filters();
            }
        });
        panel
            .filter_options
            .borrow_mut()
            .push((None, rating_option));
        let weak = Rc::downgrade(&panel);
        filters_popover.connect_show(move |_| {
            if let Some(panel) = weak.upgrade() {
                for (field, option) in panel.filter_options.borrow().iter() {
                    let active = match field {
                        Some(field) => panel.filter_selection.borrow().fields.contains(field),
                        None => panel.filter_selection.borrow().rating,
                    };
                    if option.is_active() != active {
                        option.set_active(active);
                    }
                }
            }
        });
        for field in explorer_fields {
            panel.add_explorer_column(field);
        }
        let weak = Rc::downgrade(&panel);
        panel.button.connect_toggled(move |button| {
            if let Some(panel) = weak.upgrade() {
                // Every new opening starts with the individual filter buttons.
                // Hiding for the lightbox does not toggle this button, so it
                // preserves the current ALL/individual mode on return.
                if !button.is_active() {
                    panel.all_button.set_active(false);
                }
                panel.sync_view();
                if button.is_active() {
                    panel.sync();
                }
            }
        });
        let weak = Rc::downgrade(&panel);
        panel.all_button.connect_toggled(move |button| {
            if let Some(panel) = weak.upgrade() {
                panel.sync_view();
                if button.is_active() && panel.root.is_visible() {
                    panel.refresh_explorer();
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
        panel.refresh_button.connect_clicked(move |_| {
            if let Some(panel) = weak.upgrade() {
                if let Some(refresh) = panel.refresh_action.borrow().as_ref() {
                    refresh();
                }
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
                panel
                    .filter_selection
                    .replace(load_visible_filters(&panel.connection.borrow()));
                panel.sync_filter_visibility();
                let fields = load_explorer_fields(&panel.connection.borrow());
                while let Some(column) = panel.explorer_columns.first_child() {
                    panel.explorer_columns.remove(&column);
                }
                panel.explorer.borrow_mut().clear();
                for field in fields {
                    panel.add_explorer_column(field);
                }
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
    pub fn set_refresh_action(&self, action: Rc<dyn Fn()>) {
        self.refresh_action.replace(Some(action));
    }
    /// Temporarily hide the filter interface behind the fullscreen viewer.
    /// The header toggle and ALL choice stay unchanged and restore on return.
    pub fn set_lightbox_open(&self, open: bool) {
        self.lightbox_open.set(open);
        self.sync_view();
    }
    fn sync_view(&self) {
        let show_panel = self.button.is_active() && !self.lightbox_open.get();
        self.root.set_visible(show_panel);
        let show_all = self.all_button.is_active();
        self.camera_control.set_visible(!show_all);
        self.individual_controls.set_visible(!show_all);
        self.chips_scroll.set_visible(show_all && self.chips.first_child().is_some());
        self.explorer_revealer.set_child_visible(show_all);
        self.explorer_revealer.set_reveal_child(show_all);
        self.all_button.set_tooltip_text(Some(if show_all {
            "Show individual filters"
        } else {
            "Show all metadata columns"
        }));
    }
    fn sync_filter_visibility(&self) {
        let selection = self.filter_selection.borrow().clone();
        for (field, button) in self.filter_buttons.borrow().iter() {
            button.set_visible(selection.fields.contains(field));
        }
        for (field, option) in self.filter_options.borrow().iter() {
            let active = match field {
                Some(field) => selection.fields.contains(field),
                None => selection.rating,
            };
            if option.is_active() != active {
                option.set_active(active);
            }
        }
    }
    fn save_visible_filters(&self) {
        if let Ok(value) = serde_json::to_string(&*self.filter_selection.borrow()) {
            let _ = db::set_setting(&self.connection.borrow(), VISIBLE_FILTERS_SETTING, &value);
        }
    }
    fn save_explorer_columns(&self) {
        let fields = self
            .explorer
            .borrow()
            .iter()
            .map(|column| Field::ALL[column.choose.selected() as usize])
            .collect::<Vec<_>>();
        if let Ok(value) = serde_json::to_string(&fields) {
            let _ = db::set_setting(&self.connection.borrow(), EXPLORER_COLUMNS_SETTING, &value);
        }
        for column in self.explorer.borrow().iter() {
            column.add.set_sensitive(fields.len() < 8);
            column.remove.set_sensitive(fields.len() > 1);
        }
        self.save_explorer_widths();
    }
    fn save_explorer_widths(&self) {
        let widths = self
            .explorer
            .borrow()
            .iter()
            .map(|column| column.width.get())
            .collect::<Vec<_>>();
        if let Ok(value) = serde_json::to_string(&widths) {
            let _ = db::set_setting(&self.connection.borrow(), EXPLORER_WIDTHS_SETTING, &value);
        }
    }
    fn add_explorer_column(self: &Rc<Self>, field: Field) {
        if self.explorer.borrow().len() >= 8 {
            return;
        }
        let root = gtk::Box::new(gtk::Orientation::Vertical, 4);
        root.add_css_class("metadata-explorer-column");
        let header = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        header.add_css_class("metadata-column-header");
        let width_index = self.explorer.borrow().len();
        let saved_widths = db::setting(&self.connection.borrow(), EXPLORER_WIDTHS_SETTING)
            .ok()
            .flatten()
            .and_then(|raw| serde_json::from_str::<Vec<i32>>(&raw).ok())
            .unwrap_or_default();
        let initial_width = saved_widths
            .get(width_index)
            .copied()
            .unwrap_or(200)
            .clamp(165, 420);
        let width = Rc::new(Cell::new(initial_width));
        let names = Field::ALL
            .iter()
            .map(|item| item.label())
            .collect::<Vec<_>>();
        let choose = gtk::DropDown::from_strings(&names);
        choose.add_css_class("metadata-column-choice");
        choose.set_hexpand(true);
        choose.set_selected(
            Field::ALL
                .iter()
                .position(|item| *item == field)
                .unwrap_or(0) as u32,
        );
        header.append(&choose);
        // Keep column management available without a row of floating + / - icons.
        let actions = gtk::MenuButton::new();
        actions.set_icon_name("view-more-symbolic");
        actions.add_css_class("metadata-column-menu");
        actions.set_tooltip_text(Some("Column options"));
        actions.update_property(&[gtk::accessible::Property::Label("Column options")]);
        let actions_popover = gtk::Popover::new();
        let actions_box = gtk::Box::new(gtk::Orientation::Vertical, 4);
        actions_box.add_css_class("metadata-column-actions-popover");
        actions_box.set_margin_start(8);
        actions_box.set_margin_end(8);
        actions_box.set_margin_top(8);
        actions_box.set_margin_bottom(8);
        let add = gtk::Button::with_label("Add column");
        add.add_css_class("metadata-column-action");
        let remove = gtk::Button::with_label("Remove column");
        remove.add_css_class("metadata-column-action");
        actions_box.append(&add);
        actions_box.append(&remove);
        actions_popover.set_child(Some(&actions_box));
        actions.set_popover(Some(&actions_popover));
        header.append(&actions);
        // The thin grab area preserves adjustable widths without a chevron.
        let resize = gtk::Box::new(gtk::Orientation::Vertical, 0);
        resize.add_css_class("metadata-column-resize");
        resize.set_width_request(8);
        resize.set_tooltip_text(Some("Drag to resize column"));
        resize.set_cursor_from_name(Some("col-resize"));
        header.append(&resize);
        root.append(&header);
        root.set_hexpand(true);
        root.set_size_request(initial_width, -1);
        let search = gtk::SearchEntry::new();
        search.add_css_class("metadata-column-search");
        search.set_placeholder_text(Some(&format!("Search {}…", field.label().to_lowercase())));
        root.append(&search);
        let list = gtk::Box::new(gtk::Orientation::Vertical, 2);
        let scroll = gtk::ScrolledWindow::new();
        scroll.set_min_content_height(120);
        scroll.set_max_content_height(145);
        scroll.set_propagate_natural_height(true);
        scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        scroll.set_child(Some(&list));
        root.append(&scroll);
        self.explorer_columns.append(&root);
        self.explorer.borrow_mut().push(ExplorerColumn {
            root: root.clone(),
            choose: choose.clone(),
            search: search.clone(),
            list: list.clone(),
            width: width.clone(),
            add: add.clone(),
            remove: remove.clone(),
        });

        let weak = Rc::downgrade(self);
        let content = list.clone();
        let search_for_choice = search.clone();
        choose.connect_selected_notify(move |choice| {
            if let Some(panel) = weak.upgrade() {
                let field = Field::ALL[choice.selected() as usize];
                panel.populate_column_choices(field, &search_for_choice, &content);
                search_for_choice.set_text("");
                search_for_choice.set_placeholder_text(Some(&format!(
                    "Search {}…",
                    field.label().to_lowercase()
                )));
                panel.save_explorer_columns();
            }
        });
        let content = list.clone();
        search.connect_search_changed(move |entry| {
            Self::filter_column_values(&content, &entry.text());
        });
        let weak = Rc::downgrade(self);
        let popover_for_add = actions_popover.clone();
        add.connect_clicked(move |_| {
            popover_for_add.popdown();
            if let Some(panel) = weak.upgrade() {
                let current = panel
                    .explorer
                    .borrow()
                    .iter()
                    .map(|column| Field::ALL[column.choose.selected() as usize])
                    .collect::<Vec<_>>();
                let next = Field::ALL
                    .iter()
                    .copied()
                    .find(|field| !current.contains(field))
                    .unwrap_or(Field::Camera);
                panel.add_explorer_column(next);
            }
        });
        let drag = gtk::GestureDrag::new();
        resize.add_controller(drag.clone());
        let drag_start_width = Rc::new(Cell::new(initial_width));
        let start_width = drag_start_width.clone();
        let stored_width = width.clone();
        let root_for_drag = root.clone();
        drag.connect_drag_begin(move |_, _, _| {
            start_width.set(stored_width.get());
        });
        let weak = Rc::downgrade(self);
        let start_width = drag_start_width;
        let stored_width = width;
        drag.connect_drag_update(move |_, delta_x, _| {
            let new_width = (start_width.get() + delta_x as i32).clamp(165, 420);
            stored_width.set(new_width);
            root_for_drag.set_size_request(new_width, -1);
            if let Some(panel) = weak.upgrade() {
                panel.save_explorer_widths();
            }
        });
        let weak = Rc::downgrade(self);
        let popover_for_remove = actions_popover;
        remove.connect_clicked(move |_| {
            popover_for_remove.popdown();
            if let Some(panel) = weak.upgrade() {
                panel.explorer_columns.remove(&root);
                panel
                    .explorer
                    .borrow_mut()
                    .retain(|column| column.root != root);
                panel.save_explorer_columns();
            }
        });
        self.save_explorer_columns();
        if self.explorer_revealer.reveals_child() {
            self.populate_column_choices(field, &search, &list);
        }
    }
    fn filter_column_values(list: &gtk::Box, query: &str) {
        let query = query.trim().to_lowercase();
        let mut child = list.first_child();
        while let Some(widget) = child {
            child = widget.next_sibling();
            if let Some(check) = widget.downcast_ref::<gtk::CheckButton>() {
                let label = check.label().unwrap_or_default().to_lowercase();
                check.set_visible(query.is_empty() || label.contains(&query));
            } else if let Some(button) = widget.downcast_ref::<gtk::Button>() {
                let label = button.label().unwrap_or_default().to_lowercase();
                button.set_visible(
                    query.is_empty() || label.starts_with("show more") || label.contains(&query),
                );
            }
        }
    }
    fn refresh_explorer(self: &Rc<Self>) {
        for column in self.explorer.borrow().iter() {
            self.populate_column_choices(
                Field::ALL[column.choose.selected() as usize],
                &column.search,
                &column.list,
            );
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
        if active == 0 {
            self.button.remove_css_class("metadata-filter-active");
        } else {
            self.button.add_css_class("metadata-filter-active");
        }
        if active == 0 {
            self.active_count.set_text("");
        } else {
            self.active_count.set_text(&format!("{active} filters active"));
        }
        self.button
            .set_tooltip_text(Some(&format!("Filter photos · {active} active categories")));
        for (field, selection) in state.categories {
            for value in selection.values {
                let chip = gtk::Button::with_label(&format!(
                    "{}: {} ×",
                    field.label(),
                    field.display(&value)
                ));
                chip.add_css_class("metadata-active-chip");
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
                self.chips.append(&chip);
            }
            if selection.min.is_some() || selection.max.is_some() {
                let chip = gtk::Button::with_label(&format!(
                    "{}: {} – {} ×",
                    field.label(),
                    selection.min.as_deref().unwrap_or("Any"),
                    selection.max.as_deref().unwrap_or("Any")
                ));
                chip.add_css_class("metadata-active-chip");
                let weak = Rc::downgrade(self);
                chip.connect_clicked(move |_| {
                    if let Some(panel) = weak.upgrade() {
                        let _ = panel.state.borrow_mut().range(field, "", "");
                        panel.apply();
                    }
                });
                self.chips.append(&chip);
            }
        }
        if !matches!(rating.as_str(), "" | "all") {
            let chip = gtk::Button::with_label(&format!("Rating: {rating} ×"));
            chip.add_css_class("metadata-active-chip");
            let weak = Rc::downgrade(self);
            chip.connect_clicked(move |_| {
                if let Some(panel) = weak.upgrade() {
                    (panel.rating)("all");
                    panel.sync();
                }
            });
            self.chips.append(&chip);
        }
        self.chips_scroll.set_visible(self.all_button.is_active() && active > 0);
        self.refresh_count();
        if self.explorer_revealer.reveals_child() {
            self.refresh_explorer();
        }
    }
    fn refresh_count(self: &Rc<Self>) {
        self.count_request
            .set(self.count_request.get().wrapping_add(1));
        let request = self.count_request.get();
        let Some(path) = db::connection_path(&self.connection.borrow()).ok() else {
            self.photo_count.set_text("— photos");
            return;
        };
        let mut filters = self.state.borrow().clone();
        let scope = (self.scope)();
        filters.rating = scope.rating.clone();
        let scope_for_query = scope.clone();
        let expected_path = path.clone();
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let result = db::open_existing(&path).and_then(|connection| {
                db::filters::count_matching(&connection, &scope_for_query, &filters)
            });
            let _ = sender.send(result);
        });
        let weak = Rc::downgrade(self);
        glib::timeout_add_local(Duration::from_millis(25), move || {
            let Some(panel) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            match receiver.try_recv() {
                Ok(Ok(count)) => {
                    if request == panel.count_request.get()
                        && (panel.scope)() == scope
                        && db::connection_path(&panel.connection.borrow())
                            .ok()
                            .as_ref()
                            == Some(&expected_path)
                    {
                        panel.photo_count.set_text(&format!("{count} photos"));
                    }
                    glib::ControlFlow::Break
                }
                Ok(Err(_)) => {
                    if request == panel.count_request.get() {
                        panel.photo_count.set_text("— photos");
                    }
                    glib::ControlFlow::Break
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(_) => glib::ControlFlow::Break,
            }
        });
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
    fn populate_choices(self: &Rc<Self>, field: Field, list: &gtk::Box) {
        self.populate_choices_with_search(field, list, None);
    }
    fn populate_column_choices(
        self: &Rc<Self>,
        field: Field,
        search: &gtk::SearchEntry,
        list: &gtk::Box,
    ) {
        self.populate_choices_with_search(field, list, Some(search.clone()));
    }
    fn populate_choices_with_search(
        self: &Rc<Self>,
        field: Field,
        list: &gtk::Box,
        search: Option<gtk::SearchEntry>,
    ) {
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
                        panel.populate_choices_with_search(field, &list, search.clone());
                        return glib::ControlFlow::Break;
                    }
                    while let Some(child) = list.first_child() {
                        list.remove(&child);
                    }
                    match result {
                        Ok(rows) => panel.fill_choices(field, &list, rows, search.as_ref()),
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
    fn fill_choices(
        self: &Rc<Self>,
        field: Field,
        list: &gtk::Box,
        mut rows: Vec<Facet>,
        search: Option<&gtk::SearchEntry>,
    ) {
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
        self.append_choice_page(field, list, rows, selected, search.cloned());
    }
    // Reorder existing checkbox widgets immediately, including inside open
    // popovers, so selected items do not wait for a fresh database facet query.
    fn move_selected_choices_first(list: &gtk::Box) {
        let mut selected_widgets: Vec<gtk::Widget> = Vec::new();
        let mut child = list.first_child();
        while let Some(widget) = child {
            child = widget.next_sibling();
            if widget
                .downcast_ref::<gtk::CheckButton>()
                .is_some_and(|check| check.is_active())
            {
                selected_widgets.push(widget);
            }
        }
        let mut previous: Option<gtk::Widget> = None;
        for widget in selected_widgets {
            list.reorder_child_after(&widget, previous.as_ref());
            previous = Some(widget);
        }
    }
    fn append_choice_page(
        self: &Rc<Self>,
        field: Field,
        list: &gtk::Box,
        mut rows: Vec<Facet>,
        selected: Vec<String>,
        search: Option<gtk::SearchEntry>,
    ) {
        let query = search
            .as_ref()
            .map(|entry| entry.text().to_string().to_lowercase())
            .unwrap_or_default();
        if !query.is_empty() {
            rows.retain(|row| field.display(&row.value).to_lowercase().contains(&query));
        }
        // Stable ordering keeps the original facet order within each group.
        // Apply before paging so selected values beyond the first 100 are visible.
        rows.sort_by_key(|row| !selected.contains(&row.value));
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
            check.set_visible(
                query.is_empty() || field.display(&row.value).to_lowercase().contains(&query),
            );
            // Long lens names must not force the application wider.
            if let Some(label) = check.child().and_then(|w| w.downcast::<gtk::Label>().ok()) {
                label.set_ellipsize(gtk::pango::EllipsizeMode::End);
                label.set_max_width_chars(32);
            }
            check.set_tooltip_text(Some(&field.display(&row.value)));
            let weak = Rc::downgrade(self);
            let list_for_reorder = list.downgrade();
            let search_for_selection = search.clone();
            check.connect_toggled(move |b| {
                if let Some(panel) = weak.upgrade() {
                    // In the ALL explorer, selecting a searched value must
                    // return to the complete facet list. Otherwise the next
                    // refresh recreates only search-matching checkboxes and
                    // makes additional selections appear impossible.
                    if b.is_active() {
                        if let Some(entry) = search_for_selection.as_ref() {
                            if !entry.text().is_empty() {
                                entry.set_text("");
                            }
                        }
                    }
                    if let Some(list) = list_for_reorder.upgrade() {
                        Self::move_selected_choices_first(&list);
                    }
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
            let search_weak = search.as_ref().map(gtk::SearchEntry::downgrade);
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
                    let search = search_weak.as_ref().and_then(|entry| entry.upgrade());
                    panel.append_choice_page(field, &list, rows, selected, search);
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
        self.indexing.set_visible(true);
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
    fn save_snapshot(window: &gtk::Window, path: &std::path::Path) {
        let snapshot = gtk::Snapshot::new();
        gtk::WidgetPaintable::new(Some(window)).snapshot(
            &snapshot,
            f64::from(window.width()),
            f64::from(window.height()),
        );
        window
            .renderer()
            .unwrap()
            .render_texture(snapshot.to_node().unwrap(), None)
            .save_to_png(path)
            .unwrap();
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
        assert_eq!(panel.explorer.borrow().len(), 3);
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
        header.pack_end(&tools);
        body.append(&header);
        body.append(&panel.root);
        let photos = gtk::Label::new(Some("Photo browsing area"));
        photos.set_vexpand(true);
        body.append(&photos);
        window.set_child(Some(&body));
        window.present();
        settle();
        assert!(panel.button.parent().is_some());
        panel.button.set_active(true);
        settle();
        assert!(panel.explorer_revealer.reveals_child());
        panel
            .state
            .borrow_mut()
            .toggle(Field::Camera, "nikon z 5".into(), true);
        panel.apply();
        assert_eq!(changes.get(), 1);
        panel.button.set_active(false);
        settle();
        assert!(!panel.explorer_revealer.reveals_child());
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
            if let Some(path) =
                std::env::var_os("PIC_METADATA_SCREENSHOTS").map(std::path::PathBuf::from)
            {
                std::fs::create_dir_all(&path).unwrap();
                save_snapshot(&window, &path.join(format!("standard-gtk4-{width}.png")));
            }
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
        filter.set_active(true);
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
