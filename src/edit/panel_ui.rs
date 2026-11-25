// Editor fragment included by editor.rs. Shared presentation pieces for the
// inspector panels built on the Draw panel's design language (Draw, Text).
// Their styling lives in src/css/edit_mode.css under `.pic-editor-panel`.

/// Colour palette swatch names and the CSS classes that paint them. Each
/// panel pairs these with its own stored colour values.
const EDITOR_COLOUR_SWATCHES: [(&str, &str); 8] = [
    ("White", "text-swatch-white"),
    ("Black", "text-swatch-black"),
    ("Red", "text-swatch-red"),
    ("Orange", "text-swatch-orange"),
    ("Yellow", "text-swatch-yellow"),
    ("Green", "text-swatch-green"),
    ("Blue", "text-swatch-blue"),
    ("Purple", "text-swatch-purple"),
];

fn editor_section_heading(parent: &gtk::Box, text: &str) {
    let heading = gtk::Label::new(Some(text));
    heading.set_xalign(0.0);
    heading.add_css_class("editor-section-heading");
    parent.append(&heading);
}

fn editor_section_divider(parent: &gtk::Box) {
    let divider = gtk::Separator::new(gtk::Orientation::Horizontal);
    divider.add_css_class("editor-section-divider");
    parent.append(&divider);
}

/// A vertical section: heading followed by its controls.
fn editor_section(parent: &gtk::Box, heading: &str) -> gtk::Box {
    let section = gtk::Box::new(gtk::Orientation::Vertical, 6);
    section.set_hexpand(true);
    section.set_halign(gtk::Align::Fill);
    editor_section_heading(&section, heading);
    parent.append(&section);
    section
}

/// Flat toggle used for tool and formatting choices.
fn editor_tool_button(icon: &gtk::Image, label: &str) -> gtk::ToggleButton {
    let button = gtk::ToggleButton::new();
    button.set_child(Some(icon));
    button.set_size_request(0, 34);
    button.set_tooltip_text(Some(label));
    button.update_property(&[gtk::accessible::Property::Label(label)]);
    button.add_css_class("flat");
    button.add_css_class("editor-tool-button");
    button
}

/// Equal-width buttons laid out `columns` per row.
fn editor_button_group<W: IsA<gtk::Widget>>(columns: usize, buttons: &[&W]) -> gtk::Grid {
    let grid = gtk::Grid::new();
    grid.set_hexpand(true);
    grid.set_halign(gtk::Align::Fill);
    grid.set_column_homogeneous(true);
    grid.set_row_homogeneous(true);
    grid.set_column_spacing(6);
    grid.set_row_spacing(6);
    grid.add_css_class("editor-tool-group");
    for (index, button) in buttons.iter().enumerate() {
        button.set_hexpand(true);
        grid.attach(
            *button,
            (index % columns) as i32,
            (index / columns) as i32,
            1,
            1,
        );
    }
    grid
}

/// Swatch row for the shared palette plus a trailing custom-colour button.
fn editor_colour_palette<T: Copy>(
    values: [T; 8],
) -> (gtk::Box, Vec<(gtk::Button, T)>, gtk::Button) {
    let swatches = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    swatches.set_hexpand(true);
    swatches.set_halign(gtk::Align::Fill);
    swatches.set_homogeneous(true);
    swatches.add_css_class("text-colour-swatches");
    let mut buttons = Vec::with_capacity(values.len());
    for ((name, class), value) in EDITOR_COLOUR_SWATCHES.into_iter().zip(values) {
        let swatch = gtk::Button::new();
        swatch.set_size_request(0, 28);
        swatch.add_css_class("text-colour-swatch");
        swatch.add_css_class(class);
        swatch.set_tooltip_text(Some(name));
        swatches.append(&swatch);
        buttons.push((swatch, value));
    }
    let custom = gtk::Button::new();
    custom.set_size_request(0, 28);
    custom.add_css_class("text-colour-swatch");
    custom.set_tooltip_text(Some("Choose custom colour"));
    let custom_icon = gtk::Image::from_icon_name("color-select-symbolic");
    custom_icon.set_pixel_size(16);
    custom.set_child(Some(&custom_icon));
    swatches.append(&custom);
    (swatches, buttons, custom)
}

/// Row of equally expanding control columns (e.g. two sliders side by side).
fn editor_control_columns(parent: &gtk::Box) -> gtk::Box {
    let columns = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    columns.set_hexpand(true);
    parent.append(&columns);
    columns
}

/// A labelled control column: title (and optional value) above the control.
fn editor_control_column(
    parent: &gtk::Box,
    label: &str,
    value: Option<&gtk::Label>,
    control: &impl IsA<gtk::Widget>,
) {
    let column = gtk::Box::new(gtk::Orientation::Vertical, 2);
    column.set_hexpand(true);
    column.add_css_class("editor-slider-column");
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    let title = gtk::Label::new(Some(label));
    title.set_xalign(0.0);
    title.set_hexpand(true);
    row.append(&title);
    if let Some(value) = value {
        row.append(value);
    }
    column.append(&row);
    column.append(control);
    parent.append(&column);
}

fn editor_slider_column(
    parent: &gtk::Box,
    label: &str,
    range: (f64, f64, f64),
    initial: f64,
    format: impl Fn(f64) -> String + 'static,
) -> (gtk::Scale, gtk::Label) {
    let (min, max, step) = range;
    let scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, min, max, step);
    scale.set_value(initial);
    scale.set_draw_value(false);
    scale.set_hexpand(true);
    scale.set_tooltip_text(Some(label));
    scale.add_css_class("edit-adjustment-scale");
    let value = gtk::Label::new(Some(&format(initial)));
    value.set_xalign(1.0);
    value.add_css_class("editor-slider-value");
    {
        let value_label = value.clone();
        scale.connect_value_changed(move |scale| value_label.set_text(&format(scale.value())));
    }
    editor_control_column(parent, label, Some(&value), &scale);
    (scale, value)
}

/// Quiet destructive action (icon + label) aligned to the panel start.
fn editor_destructive_button(label: &str) -> gtk::Button {
    let button = gtk::Button::new();
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    content.append(&gtk::Image::from_icon_name("user-trash-symbolic"));
    content.append(&gtk::Label::new(Some(label)));
    button.set_child(Some(&content));
    button.update_property(&[gtk::accessible::Property::Label(label)]);
    button.add_css_class("editor-destructive-button");
    button.set_halign(gtk::Align::Start);
    button
}

/// "Offset" column of X/Y spin buttons in photo pixels, beside a position grid.
fn editor_offset_column(parent: &gtk::Box) -> (gtk::SpinButton, gtk::SpinButton) {
    let offset_x = gtk::SpinButton::with_range(-100000.0, 100000.0, 1.0);
    let offset_y = gtk::SpinButton::with_range(-100000.0, 100000.0, 1.0);
    offset_x.set_tooltip_text(Some(
        "Horizontal offset from the selected position, in photo pixels",
    ));
    offset_y.set_tooltip_text(Some(
        "Vertical offset from the selected position, in photo pixels",
    ));
    let rows = gtk::Box::new(gtk::Orientation::Vertical, 4);
    for (axis, spin) in [("X", &offset_x), ("Y", &offset_y)] {
        spin.set_hexpand(true);
        spin.set_width_chars(3);
        spin.set_max_width_chars(3);
        spin.add_css_class("edit-offset-spin");
        let cell = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        cell.set_hexpand(true);
        let label = gtk::Label::new(Some(axis));
        label.set_xalign(0.0);
        label.set_width_chars(1);
        cell.append(&label);
        cell.append(spin);
        rows.append(&cell);
    }
    editor_control_column(parent, "Offset", None, &rows);
    (offset_x, offset_y)
}
