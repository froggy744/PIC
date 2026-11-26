//! Smart Album rules: the editable condition rows (pure) and the native rules
//! dialog that edits them. One condition per field, mapped one-to-one onto a
//! filter category, so a saved album keeps PIC's filter semantics: AND
//! between conditions, OR between the values chosen for one condition.
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use anyhow::Result;
use gtk4 as gtk;
use gtk4::glib;
use libadwaita as adw;
use libadwaita::prelude::*;
use rusqlite::Connection;

use crate::db::filters::{self, AlbumRules, Field, Filters, Selection};
use crate::db::{self, Album};

/// Comparison a condition applies; only those the filter predicate supports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuleOp {
    AnyOf,
    Between,
    AtLeast,
    AtMost,
}

impl RuleOp {
    pub fn label(self, field: Field) -> &'static str {
        match self {
            Self::AnyOf if is_flag(field) => "is",
            Self::AnyOf if field == Field::Tags => "has any of",
            Self::AnyOf => "is any of",
            Self::Between => "between",
            Self::AtLeast if field == Field::CaptureDate => "on or after",
            Self::AtLeast => "at least",
            Self::AtMost if field == Field::CaptureDate => "on or before",
            Self::AtMost => "at most",
        }
    }
    fn ranged(self) -> bool {
        self != Self::AnyOf
    }
}

fn is_flag(field: Field) -> bool {
    matches!(
        field,
        Field::Flash | Field::Gps | Field::Favorite | Field::Edited | Field::Metadata
    )
}

/// Fields a Smart Album can use: every metadata filter field except Files
/// (online, offline and corrupt are not stored metadata).
pub fn rule_fields() -> Vec<Field> {
    Field::ALL
        .into_iter()
        .filter(|field| *field != Field::Files)
        .collect()
}

pub fn operators(field: Field) -> &'static [RuleOp] {
    if field.numeric() || field == Field::CaptureDate {
        &[RuleOp::AnyOf, RuleOp::Between, RuleOp::AtLeast, RuleOp::AtMost]
    } else {
        &[RuleOp::AnyOf]
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Condition {
    pub field: Field,
    pub op: RuleOp,
    pub values: Vec<String>,
    pub min: String,
    pub max: String,
}

impl Condition {
    pub fn new(field: Field) -> Self {
        Self {
            field,
            op: RuleOp::AnyOf,
            values: Vec::new(),
            min: String::new(),
            max: String::new(),
        }
    }
    /// Switching between a value list and a range drops what the new
    /// operator cannot show, so nothing hidden is saved.
    pub fn set_op(&mut self, op: RuleOp) {
        if op == self.op {
            return;
        }
        self.op = op;
        if op.ranged() {
            self.values.clear();
        } else {
            self.min.clear();
            self.max.clear();
        }
        match op {
            RuleOp::AtLeast => self.max.clear(),
            RuleOp::AtMost => self.min.clear(),
            _ => {}
        }
    }
    fn selection(&self) -> Result<Selection> {
        let label = self.field.label();
        let (min, max) = (self.min.trim(), self.max.trim());
        match self.op {
            RuleOp::AnyOf => {
                anyhow::ensure!(!self.values.is_empty(), "Choose at least one {label} value")
            }
            RuleOp::Between => anyhow::ensure!(
                !min.is_empty() && !max.is_empty(),
                "Enter both {label} bounds"
            ),
            RuleOp::AtLeast => anyhow::ensure!(!min.is_empty(), "Enter the lowest {label}"),
            RuleOp::AtMost => anyhow::ensure!(!max.is_empty(), "Enter the highest {label}"),
        }
        let mut selection = Selection::default();
        if !min.is_empty() || !max.is_empty() {
            let mut check = Filters::default();
            check.range(self.field, min, max)?;
            selection = check.categories.remove(&self.field).unwrap_or_default();
        }
        selection.values = self.values.clone();
        Ok(selection)
    }
}

fn rating_values(rating: &str) -> Option<Vec<String>> {
    match rating {
        "unrated" => Some(vec!["0".into()]),
        "stars" => Some((1..=5).map(|n| n.to_string()).collect()),
        "0" | "1" | "2" | "3" | "4" | "5" => Some(vec![rating.into()]),
        _ => None,
    }
}

/// Rows for saved rules. The toolbar rating choice becomes the Rating row's
/// values (intersected with Rating values chosen in the Explorer, which is
/// what the two together match).
pub fn conditions_from_rules(rules: &AlbumRules) -> Vec<Condition> {
    let rating = rating_values(&rules.rating);
    let mut conditions = Vec::new();
    for (field, selection) in &rules.categories {
        let op = match (
            selection.values.is_empty(),
            selection.min.is_some(),
            selection.max.is_some(),
        ) {
            (false, _, _) => RuleOp::AnyOf,
            (true, true, true) => RuleOp::Between,
            (true, true, false) => RuleOp::AtLeast,
            (true, false, true) => RuleOp::AtMost,
            (true, false, false) => RuleOp::AnyOf,
        };
        let mut condition = Condition {
            field: *field,
            op,
            values: selection.values.clone(),
            min: selection.min.clone().unwrap_or_default(),
            max: selection.max.clone().unwrap_or_default(),
        };
        if *field == Field::Rating {
            if let Some(rating) = &rating {
                condition.values.retain(|value| rating.contains(value));
            }
        }
        conditions.push(condition);
    }
    if let Some(values) = rating {
        if !rules.categories.contains_key(&Field::Rating) {
            conditions.push(Condition {
                values,
                ..Condition::new(Field::Rating)
            });
        }
    }
    conditions
}

pub fn rules_from_conditions(conditions: &[Condition]) -> Result<AlbumRules> {
    anyhow::ensure!(!conditions.is_empty(), "Add at least one condition");
    let mut rules = AlbumRules::default();
    for condition in conditions {
        anyhow::ensure!(
            !rules.categories.contains_key(&condition.field),
            "Use one {} condition",
            condition.field.label()
        );
        rules
            .categories
            .insert(condition.field, condition.selection()?);
    }
    rules.validate()?;
    Ok(rules)
}

/// What the rules dialog shows around the conditions.
pub struct RulesDialogSpec {
    pub title: String,
    pub subtitle: Option<String>,
    /// `Some` shows an editable album name, prefilled with this text.
    pub name: Option<String>,
    pub accept_label: &'static str,
    /// A banner above the conditions, e.g. filters that could not be saved.
    pub notice: Option<String>,
}

type Accept = Rc<dyn Fn(&str, &AlbumRules) -> Result<()>>;

struct Editor {
    connection: Rc<RefCell<Connection>>,
    conditions: RefCell<Vec<Condition>>,
    list: gtk::ListBox,
    status: gtk::Label,
    error: gtk::Label,
    accept: gtk::Button,
    name: Option<adw::EntryRow>,
    pending_count: RefCell<Option<glib::SourceId>>,
}

/// Present the Smart Album rules dialog. `on_accept` gets the album name
/// (empty when the dialog has no name field) and validated rules; an error
/// it returns is shown in the dialog, which then stays open.
pub fn present_rules_dialog(
    parent: &gtk::Widget,
    connection: Rc<RefCell<Connection>>,
    spec: RulesDialogSpec,
    rules: Option<&AlbumRules>,
    on_accept: Accept,
) -> adw::Dialog {
    let dialog = adw::Dialog::builder()
        .title(&spec.title)
        .content_width(620)
        .content_height(520)
        .build();
    dialog.add_css_class("smart-album-rules-dialog");
    let header = adw::HeaderBar::new();
    header.set_show_start_title_buttons(false);
    header.set_show_end_title_buttons(false);
    if let Some(subtitle) = &spec.subtitle {
        header.set_title_widget(Some(&adw::WindowTitle::new(&spec.title, subtitle)));
    }
    let cancel = gtk::Button::with_label("Cancel");
    header.pack_start(&cancel);
    let accept = gtk::Button::with_label(spec.accept_label);
    accept.add_css_class("suggested-action");
    header.pack_end(&accept);
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    if let Some(notice) = &spec.notice {
        let banner = adw::Banner::new(notice);
        banner.set_revealed(true);
        toolbar.add_top_bar(&banner);
    }

    let page = adw::PreferencesPage::new();
    let name = spec.name.as_ref().map(|text| {
        let group = adw::PreferencesGroup::new();
        let row = adw::EntryRow::builder().title("Album name").text(text).build();
        group.add(&row);
        page.add(&group);
        row
    });

    let group = adw::PreferencesGroup::builder()
        .title("Conditions")
        .description("Photos must match every condition. Choosing several values for one condition matches any of them.")
        .build();
    let add = gtk::Button::builder()
        .child(
            &adw::ButtonContent::builder()
                .icon_name("list-add-symbolic")
                .label("Add Condition")
                .build(),
        )
        .valign(gtk::Align::Center)
        .build();
    add.add_css_class("flat");
    add.add_css_class("smart-album-add-condition");
    group.set_header_suffix(Some(&add));
    let list = gtk::ListBox::new();
    list.set_selection_mode(gtk::SelectionMode::None);
    list.add_css_class("boxed-list");
    list.add_css_class("smart-album-conditions");
    group.add(&list);
    page.add(&group);

    let footer = adw::PreferencesGroup::new();
    let status = gtk::Label::new(None);
    status.set_xalign(0.0);
    status.set_wrap(true);
    status.add_css_class("dim-label");
    status.add_css_class("smart-album-match-count");
    let error = gtk::Label::new(None);
    error.set_xalign(0.0);
    error.set_wrap(true);
    error.add_css_class("error");
    error.set_visible(false);
    let footer_box = gtk::Box::new(gtk::Orientation::Vertical, 6);
    footer_box.append(&status);
    footer_box.append(&error);
    footer.add(&footer_box);
    page.add(&footer);
    toolbar.set_content(Some(&page));
    dialog.set_child(Some(&toolbar));

    let editor = Rc::new(Editor {
        connection,
        conditions: RefCell::new(rules.map(conditions_from_rules).unwrap_or_default()),
        list,
        status,
        error,
        accept: accept.clone(),
        name: name.clone(),
        pending_count: RefCell::new(None),
    });
    if editor.conditions.borrow().is_empty() {
        editor
            .conditions
            .borrow_mut()
            .push(Condition::new(Field::Rating));
    }
    editor.render();
    editor.refresh_status_now();

    let weak = Rc::downgrade(&editor);
    add.connect_clicked(move |_| {
        let Some(editor) = weak.upgrade() else { return };
        let Some(field) = editor.unused_fields(None).first().copied() else {
            return;
        };
        editor.conditions.borrow_mut().push(Condition::new(field));
        editor.render();
        editor.schedule_status();
    });
    if let Some(name) = &name {
        let weak = Rc::downgrade(&editor);
        name.connect_changed(move |_| {
            if let Some(editor) = weak.upgrade() {
                editor.refresh_status_now();
            }
        });
    }
    let dialog_for_cancel = dialog.downgrade();
    cancel.connect_clicked(move |_| {
        if let Some(dialog) = dialog_for_cancel.upgrade() {
            dialog.close();
        }
    });
    let dialog_for_accept = dialog.downgrade();
    let accept_editor = editor.clone();
    accept.connect_clicked(move |_| {
        let name = accept_editor
            .name
            .as_ref()
            .map(|row| row.text().to_string())
            .unwrap_or_default();
        let result = rules_from_conditions(&accept_editor.conditions.borrow())
            .and_then(|rules| on_accept(name.trim(), &rules));
        match result {
            Ok(()) => {
                if let Some(dialog) = dialog_for_accept.upgrade() {
                    dialog.close();
                }
            }
            Err(error) => {
                accept_editor.error.set_text(&error.to_string());
                accept_editor.error.set_visible(true);
            }
        }
    });
    dialog.present(Some(parent));
    if let Some(name) = &name {
        name.grab_focus();
    }
    dialog
}

impl Editor {
    fn unused_fields(&self, keep: Option<Field>) -> Vec<Field> {
        let conditions = self.conditions.borrow();
        rule_fields()
            .into_iter()
            .filter(|field| {
                Some(*field) == keep || !conditions.iter().any(|c| c.field == *field)
            })
            .collect()
    }

    fn render(self: &Rc<Self>) {
        while let Some(child) = self.list.first_child() {
            self.list.remove(&child);
        }
        let count = self.conditions.borrow().len();
        for index in 0..count {
            let row = self.condition_row(index);
            self.list.append(&row);
        }
    }

    /// Rebuild after the current signal handler returns, so a dropdown is
    /// never removed while it is still emitting.
    fn render_later(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        glib::idle_add_local_once(move || {
            if let Some(editor) = weak.upgrade() {
                editor.render();
                editor.schedule_status();
            }
        });
    }

    fn condition_row(self: &Rc<Self>, index: usize) -> gtk::ListBoxRow {
        let condition = self.conditions.borrow()[index].clone();
        let content = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        content.set_margin_top(8);
        content.set_margin_bottom(8);
        content.set_margin_start(10);
        content.set_margin_end(8);

        let fields = self.unused_fields(Some(condition.field));
        let labels = fields.iter().map(|field| field.label()).collect::<Vec<_>>();
        let field_choice = gtk::DropDown::from_strings(&labels);
        field_choice.set_tooltip_text(Some("Metadata field"));
        field_choice.set_valign(gtk::Align::Center);
        if let Some(position) = fields.iter().position(|field| *field == condition.field) {
            field_choice.set_selected(position as u32);
        }
        let weak = Rc::downgrade(self);
        field_choice.connect_selected_notify(move |choice| {
            let Some(editor) = weak.upgrade() else { return };
            let Some(field) = fields.get(choice.selected() as usize).copied() else {
                return;
            };
            editor.conditions.borrow_mut()[index] = Condition::new(field);
            editor.render_later();
        });
        content.append(&field_choice);

        let ops = operators(condition.field);
        if ops.len() == 1 {
            let label = gtk::Label::new(Some(ops[0].label(condition.field)));
            label.add_css_class("dim-label");
            content.append(&label);
        } else {
            let labels = ops
                .iter()
                .map(|op| op.label(condition.field))
                .collect::<Vec<_>>();
            let op_choice = gtk::DropDown::from_strings(&labels);
            op_choice.set_tooltip_text(Some("Comparison"));
            op_choice.set_valign(gtk::Align::Center);
            if let Some(position) = ops.iter().position(|op| *op == condition.op) {
                op_choice.set_selected(position as u32);
            }
            let weak = Rc::downgrade(self);
            op_choice.connect_selected_notify(move |choice| {
                let Some(editor) = weak.upgrade() else { return };
                if let Some(op) = ops.get(choice.selected() as usize) {
                    editor.conditions.borrow_mut()[index].set_op(*op);
                    editor.render_later();
                }
            });
            content.append(&op_choice);
        }

        let value = if condition.op.ranged() {
            self.range_widget(index, &condition)
        } else {
            self.values_widget(index, &condition)
        };
        value.set_hexpand(true);
        content.append(&value);

        let remove = gtk::Button::from_icon_name("list-remove-symbolic");
        remove.add_css_class("flat");
        remove.add_css_class("circular");
        remove.set_valign(gtk::Align::Center);
        remove.set_tooltip_text(Some("Remove condition"));
        remove.update_property(&[gtk::accessible::Property::Label("Remove condition")]);
        let weak = Rc::downgrade(self);
        remove.connect_clicked(move |_| {
            let Some(editor) = weak.upgrade() else { return };
            editor.conditions.borrow_mut().remove(index);
            editor.render_later();
        });
        content.append(&remove);

        let row = gtk::ListBoxRow::new();
        row.set_activatable(false);
        row.set_child(Some(&content));
        row
    }

    fn range_widget(self: &Rc<Self>, index: usize, condition: &Condition) -> gtk::Widget {
        let placeholder = match condition.field {
            Field::CaptureDate => "YYYY-MM-DD",
            Field::Shutter => "Seconds",
            _ => "Value",
        };
        let content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let mut entry = |min: bool| {
            let entry = gtk::Entry::new();
            entry.set_placeholder_text(Some(placeholder));
            entry.set_width_chars(10);
            entry.set_hexpand(true);
            entry.set_text(if min { &condition.min } else { &condition.max });
            let weak = Rc::downgrade(self);
            entry.connect_changed(move |entry| {
                let Some(editor) = weak.upgrade() else { return };
                let text = entry.text().to_string();
                let mut conditions = editor.conditions.borrow_mut();
                if min {
                    conditions[index].min = text;
                } else {
                    conditions[index].max = text;
                }
                drop(conditions);
                editor.schedule_status();
            });
            entry
        };
        match condition.op {
            RuleOp::Between => {
                content.append(&entry(true));
                content.append(&gtk::Label::new(Some("and")));
                content.append(&entry(false));
            }
            RuleOp::AtLeast => content.append(&entry(true)),
            _ => content.append(&entry(false)),
        }
        content.upcast()
    }

    fn values_widget(self: &Rc<Self>, index: usize, condition: &Condition) -> gtk::Widget {
        let button = gtk::MenuButton::new();
        button.add_css_class("smart-album-values");
        button.set_valign(gtk::Align::Center);
        button.set_label(&values_summary(condition));
        let weak = Rc::downgrade(self);
        let field = condition.field;
        button.set_create_popup_func(move |button| {
            let Some(editor) = weak.upgrade() else { return };
            button.set_popover(Some(&editor.values_popover(index, field, button)));
        });
        button.upcast()
    }

    fn values_popover(
        self: &Rc<Self>,
        index: usize,
        field: Field,
        button: &gtk::MenuButton,
    ) -> gtk::Popover {
        const SHOWN: usize = 300;
        let mut choices = filters::facets(
            &self.connection.borrow(),
            &filters::Scope::default(),
            &Filters::default(),
            field,
        )
        .unwrap_or_default()
        .into_iter()
        .map(|facet| (facet.value, Some(facet.count)))
        .collect::<Vec<_>>();
        // Saved values the library no longer has stay visible and removable.
        for value in &self.conditions.borrow()[index].values {
            if !choices.iter().any(|(choice, _)| choice == value) {
                choices.push((value.clone(), None));
            }
        }
        let choices = Rc::new(choices);
        let popover = gtk::Popover::new();
        let content = gtk::Box::new(gtk::Orientation::Vertical, 6);
        content.set_margin_top(6);
        content.set_margin_bottom(6);
        content.set_margin_start(6);
        content.set_margin_end(6);
        let search = gtk::SearchEntry::new();
        search.set_placeholder_text(Some(&format!("Search {}", field.label().to_lowercase())));
        content.append(&search);
        let list = gtk::Box::new(gtk::Orientation::Vertical, 2);
        let scroll = gtk::ScrolledWindow::new();
        scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        scroll.set_propagate_natural_height(true);
        scroll.set_propagate_natural_width(true);
        scroll.set_max_content_height(320);
        scroll.set_min_content_width(240);
        scroll.set_child(Some(&list));
        content.append(&scroll);
        let more = gtk::Label::new(Some("Refine the search to see more values."));
        more.add_css_class("dim-label");
        more.add_css_class("caption");
        content.append(&more);
        popover.set_child(Some(&content));

        let populate: Rc<dyn Fn(&str)> = {
            let editor = Rc::downgrade(self);
            let list = list.clone();
            let more = more.clone();
            let choices = choices.clone();
            let button = button.downgrade();
            Rc::new(move |query: &str| {
                let Some(editor) = editor.upgrade() else { return };
                while let Some(child) = list.first_child() {
                    list.remove(&child);
                }
                let query = query.trim().to_lowercase();
                let selected = editor.conditions.borrow()[index].values.clone();
                let matching = choices
                    .iter()
                    .filter(|(value, _)| {
                        query.is_empty() || field.display(value).to_lowercase().contains(&query)
                    })
                    .collect::<Vec<_>>();
                more.set_visible(matching.len() > SHOWN);
                if matching.is_empty() {
                    let empty = gtk::Label::new(Some("No values"));
                    empty.add_css_class("dim-label");
                    list.append(&empty);
                }
                for (value, count) in matching.into_iter().take(SHOWN) {
                    let label = match count {
                        Some(count) => format!("{} ({count})", field.display(value)),
                        None => field.display(value),
                    };
                    let check = gtk::CheckButton::with_label(&label);
                    check.set_active(selected.contains(value));
                    let weak = Rc::downgrade(&editor);
                    let value = value.clone();
                    let button = button.clone();
                    check.connect_toggled(move |check| {
                        let Some(editor) = weak.upgrade() else { return };
                        let summary = {
                            let mut conditions = editor.conditions.borrow_mut();
                            let condition = &mut conditions[index];
                            condition.values.retain(|v| v != &value);
                            if check.is_active() {
                                condition.values.push(value.clone());
                            }
                            values_summary(condition)
                        };
                        if let Some(button) = button.upgrade() {
                            button.set_label(&summary);
                        }
                        editor.schedule_status();
                    });
                    list.append(&check);
                }
            })
        };
        populate("");
        search.connect_search_changed(move |search| populate(&search.text()));
        popover
    }

    fn schedule_status(self: &Rc<Self>) {
        if let Some(source) = self.pending_count.borrow_mut().take() {
            source.remove();
        }
        let weak = Rc::downgrade(self);
        let source = glib::timeout_add_local_once(Duration::from_millis(180), move || {
            if let Some(editor) = weak.upgrade() {
                editor.pending_count.borrow_mut().take();
                editor.refresh_status_now();
            }
        });
        self.pending_count.replace(Some(source));
    }

    /// Validate the conditions and show how many photos they match.
    fn refresh_status_now(&self) {
        self.error.set_visible(false);
        let named = self
            .name
            .as_ref()
            .map_or(true, |row| !row.text().trim().is_empty());
        match rules_from_conditions(&self.conditions.borrow()) {
            Ok(rules) => {
                let count = filters::count_matching(
                    &self.connection.borrow(),
                    &filters::Scope::default(),
                    &rules.to_filters(),
                );
                self.status.set_text(&match count {
                    Ok(1) => "1 matching photo".to_string(),
                    Ok(count) => format!("{count} matching photos"),
                    Err(_) => String::new(),
                });
                self.accept.set_sensitive(named);
            }
            Err(error) => {
                self.status.set_text(&error.to_string());
                self.accept.set_sensitive(false);
            }
        }
    }
}

fn values_summary(condition: &Condition) -> String {
    match condition.values.as_slice() {
        [] => "Choose…".to_string(),
        [value] => condition.field.display(value),
        values => format!("{} selected", values.len()),
    }
}

/// Edit a Smart Album's rules in place: its id, name, cover and presentation
/// settings are kept. Unreadable saved rules open as a fresh set to replace.
pub fn present_edit_rules(
    parent: &gtk::Widget,
    connection: Rc<RefCell<Connection>>,
    album: &Album,
    on_changed: Rc<dyn Fn()>,
) {
    let (rules, notice) = match db::album_rules(&connection.borrow(), album.id) {
        Ok(rules) => (rules, None),
        Err(_) => (
            None,
            Some("The saved rules could not be read. Choose new conditions.".to_string()),
        ),
    };
    let album_id = album.id;
    let save_connection = connection.clone();
    present_rules_dialog(
        parent,
        connection,
        RulesDialogSpec {
            title: "Edit Rules".into(),
            subtitle: Some(album.name.clone()),
            name: None,
            accept_label: "Save",
            notice,
        },
        rules.as_ref(),
        Rc::new(move |_, rules| {
            db::set_album_rules(&save_connection.borrow(), album_id, rules)?;
            on_changed();
            Ok(())
        }),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn values(field: Field, values: &[&str]) -> Condition {
        Condition {
            values: values.iter().map(|v| v.to_string()).collect(),
            ..Condition::new(field)
        }
    }

    #[test]
    fn operators_are_only_those_the_field_supports() {
        assert_eq!(operators(Field::Camera), &[RuleOp::AnyOf]);
        assert_eq!(operators(Field::Tags), &[RuleOp::AnyOf]);
        assert_eq!(operators(Field::Iso).len(), 4);
        assert_eq!(operators(Field::CaptureDate).len(), 4);
        assert_eq!(RuleOp::AnyOf.label(Field::Favorite), "is");
        assert_eq!(RuleOp::AtLeast.label(Field::CaptureDate), "on or after");
        assert!(!rule_fields().contains(&Field::Files));
    }

    #[test]
    fn conditions_round_trip_through_saved_rules() {
        let mut iso = Condition::new(Field::Iso);
        iso.set_op(RuleOp::Between);
        iso.min = "100".into();
        iso.max = "800".into();
        let mut date = Condition::new(Field::CaptureDate);
        date.set_op(RuleOp::AtLeast);
        date.min = "2024-01-01".into();
        let conditions = vec![
            values(Field::Camera, &["nikon z 5", "canon r5"]),
            values(Field::Favorite, &["1"]),
            iso,
            date,
        ];
        let rules = rules_from_conditions(&conditions).unwrap();
        assert_eq!(rules.categories[&Field::Iso].min.as_deref(), Some("100"));
        assert!(rules.rating.is_empty());
        let mut sorted = conditions.clone();
        sorted.sort_by_key(|condition| condition.field);
        assert_eq!(conditions_from_rules(&rules), sorted);
    }

    #[test]
    fn captured_rating_becomes_an_editable_rating_condition() {
        let rules = AlbumRules {
            rating: "stars".into(),
            ..Default::default()
        };
        let conditions = conditions_from_rules(&rules);
        assert_eq!(conditions, vec![values(Field::Rating, &["1", "2", "3", "4", "5"])]);
        // Saving it again matches the same photos through the Rating field.
        let saved = rules_from_conditions(&conditions).unwrap();
        assert!(saved.rating.is_empty());
        assert_eq!(saved.categories[&Field::Rating].values.len(), 5);

        let mut both = AlbumRules {
            rating: "stars".into(),
            ..Default::default()
        };
        both.categories.insert(
            Field::Rating,
            Selection {
                values: vec!["0".into(), "4".into()],
                ..Default::default()
            },
        );
        assert_eq!(conditions_from_rules(&both), vec![values(Field::Rating, &["4"])]);
    }

    #[test]
    fn incomplete_or_duplicate_conditions_are_rejected() {
        assert!(rules_from_conditions(&[]).is_err());
        assert!(rules_from_conditions(&[Condition::new(Field::Camera)]).is_err());
        let mut between = Condition::new(Field::Iso);
        between.set_op(RuleOp::Between);
        between.min = "100".into();
        assert!(rules_from_conditions(&[between.clone()]).is_err());
        between.max = "50".into();
        assert!(rules_from_conditions(&[between]).is_err());
        let mut date = Condition::new(Field::CaptureDate);
        date.set_op(RuleOp::AtMost);
        date.max = "2024-02-30".into();
        assert!(rules_from_conditions(&[date]).is_err());
        let camera = values(Field::Camera, &["a"]);
        assert!(rules_from_conditions(&[camera.clone(), camera]).is_err());
    }

    #[test]
    fn changing_operator_drops_what_it_cannot_show() {
        let mut condition = values(Field::Iso, &["100"]);
        condition.set_op(RuleOp::AtMost);
        assert!(condition.values.is_empty());
        condition.min = "1".into();
        condition.max = "200".into();
        condition.set_op(RuleOp::AtLeast);
        assert!(condition.max.is_empty());
        condition.set_op(RuleOp::AnyOf);
        assert!(condition.min.is_empty());
    }

    fn descendants<T: IsA<gtk::Widget>>(root: &gtk::Widget, out: &mut Vec<T>) {
        let mut child = root.first_child();
        while let Some(widget) = child {
            if let Ok(found) = widget.clone().downcast::<T>() {
                out.push(found);
            }
            descendants(&widget, out);
            child = widget.next_sibling();
        }
    }

    fn settle() {
        let context = glib::MainContext::default();
        let until = std::time::Instant::now() + Duration::from_millis(400);
        while std::time::Instant::now() < until {
            context.iteration(false);
        }
    }

    #[test]
    #[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
    fn rules_dialog_reviews_counts_and_saves_conditions() {
        adw::init().unwrap();
        let connection = Connection::open_in_memory().unwrap();
        connection.execute_batch(db::SCHEMA).unwrap();
        connection
            .execute_batch(
                "INSERT INTO photos(id,path,rating) VALUES (1,'/a.jpg',5),(2,'/b.jpg',3),(3,'/c.jpg',0);",
            )
            .unwrap();
        let connection = Rc::new(RefCell::new(connection));
        let window = adw::ApplicationWindow::builder().default_width(900).default_height(700).build();
        window.present();
        let saved: Rc<RefCell<Option<(String, AlbumRules)>>> = Rc::default();
        let saved_in = saved.clone();
        let dialog = present_rules_dialog(
            window.upcast_ref(),
            connection,
            RulesDialogSpec {
                title: "Save as Smart Album".into(),
                subtitle: None,
                name: Some("Rated".into()),
                accept_label: "Save",
                notice: Some("Online, offline and corrupt photo filters cannot be saved.".into()),
            },
            Some(&AlbumRules { rating: "stars".into(), ..Default::default() }),
            Rc::new(move |name, rules| {
                saved_in.replace(Some((name.to_string(), rules.clone())));
                Ok(())
            }),
        );
        settle();
        let root: gtk::Widget = dialog.clone().upcast();
        let mut labels = Vec::<gtk::Label>::new();
        descendants(&root, &mut labels);
        let status = labels
            .iter()
            .find(|label| label.has_css_class("smart-album-match-count"))
            .unwrap();
        assert_eq!(status.text(), "2 matching photos");
        let mut banners = Vec::<adw::Banner>::new();
        descendants(&root, &mut banners);
        assert!(banners[0].is_revealed());
        let mut buttons = Vec::<gtk::Button>::new();
        descendants(&root, &mut buttons);
        let remove = buttons
            .iter()
            .find(|button| button.tooltip_text().as_deref() == Some("Remove condition"))
            .unwrap();
        let save = buttons.iter().find(|button| button.label().as_deref() == Some("Save")).unwrap().clone();
        assert!(save.is_sensitive());

        // Removing the only condition leaves nothing to save.
        remove.emit_clicked();
        settle();
        assert!(!save.is_sensitive());
        let add = buttons
            .iter()
            .find(|button| button.has_css_class("smart-album-add-condition"))
            .unwrap();
        add.emit_clicked();
        settle();
        // A new condition takes the first free field; switch it to Rating.
        let mut drops = Vec::<gtk::DropDown>::new();
        descendants(&root, &mut drops);
        let field = drops
            .iter()
            .find(|drop| drop.tooltip_text().as_deref() == Some("Metadata field"))
            .unwrap();
        let rating = rule_fields().iter().position(|f| *f == Field::Rating).unwrap();
        field.set_selected(rating as u32);
        settle();
        let mut menus = Vec::<gtk::MenuButton>::new();
        descendants(&root, &mut menus);
        let values = menus.iter().find(|menu| menu.has_css_class("smart-album-values")).unwrap();
        values.popup();
        settle();
        let popover = values.popover().unwrap();
        let mut checks = Vec::<gtk::CheckButton>::new();
        descendants(popover.upcast_ref(), &mut checks);
        let five = checks
            .iter()
            .find(|check| check.label().as_deref() == Some("5 stars (1)"))
            .unwrap();
        five.set_active(true);
        values.popdown();
        settle();
        assert_eq!(status.text(), "1 matching photo");
        assert!(save.is_sensitive());
        save.emit_clicked();
        settle();
        let (name, rules) = saved.borrow().clone().unwrap();
        assert_eq!(name, "Rated");
        assert_eq!(rules.categories[&Field::Rating].values, vec!["5".to_string()]);
    }
}
