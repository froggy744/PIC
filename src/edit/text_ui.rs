// Editor fragment included by editor.rs. Provides the Text panel tab for
// creating and editing text layers.

fn font_family_display_name(name: &str) -> String {
    let mut chars = name.chars();
    let display = chars.by_ref().take(18).collect::<String>();
    if chars.next().is_some() {
        format!("{display}…")
    } else {
        display
    }
}

#[allow(clippy::too_many_arguments)]
fn build_text_panel(
    parent: &gtk::Box,
    session: Rc<RefCell<EditSession>>,
    selected: Rc<Cell<Option<usize>>>,
    syncing: Rc<Cell<bool>>,
    update_history_buttons: Rc<dyn Fn()>,
    redraw_canvas: Rc<dyn Fn()>,
    update_canvas_input: Rc<dyn Fn()>,
    text_toggle: gtk::ToggleButton,
    photo_dimensions: Rc<Cell<(i32, i32)>>,
) -> Rc<dyn Fn()> {
    use gtk::pango::prelude::*;
    let selected_section = gtk::Box::new(gtk::Orientation::Vertical, 10);
    selected_section.set_hexpand(true);
    selected_section.set_halign(gtk::Align::Fill);
    selected_section.add_css_class("overlay-selected-section");
    parent.append(&selected_section);

    let content_section = editor_section(&selected_section, "TEXT");
    let text_scroll = gtk::ScrolledWindow::new();
    text_scroll.add_css_class("pic-text-editor-surface");
    text_scroll.set_hexpand(true);
    text_scroll.set_halign(gtk::Align::Fill);
    text_scroll.set_direction(gtk::TextDirection::Ltr);
    text_scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    text_scroll.set_min_content_height(76);
    text_scroll.set_min_content_width(1);
    text_scroll.set_propagate_natural_width(false);
    text_scroll.connect_notify_local(Some("width"), |scroll, _| {
        let scroll = scroll.clone();
        glib::idle_add_local_once(move || {
            let adjustment = scroll.hadjustment();
            adjustment.set_value(adjustment.lower());
        });
    });
    let text_view = gtk::TextView::new();
    text_view.set_wrap_mode(gtk::WrapMode::WordChar);
    text_view.set_size_request(1, -1);
    text_view.set_hexpand(true);
    text_view.set_top_margin(6);
    text_view.set_bottom_margin(6);
    text_view.set_left_margin(6);
    text_view.set_right_margin(6);
    text_scroll.set_child(Some(&text_view));
    content_section.append(&text_scroll);
    editor_section_divider(&selected_section);

    // Typography: family, then the style and alignment toggles in one group.
    let typography_section = editor_section(&selected_section, "TYPOGRAPHY");
    let font_map = pangocairo::FontMap::new();
    let mut families = font_map
        .list_families()
        .into_iter()
        .map(|family| family.name().to_string())
        .collect::<Vec<_>>();
    families.sort_by_key(|name| name.to_lowercase());
    let font_labels = families
        .iter()
        .map(|family| font_family_display_name(family))
        .collect::<Vec<_>>();
    let font_model =
        gtk::StringList::new(&font_labels.iter().map(String::as_str).collect::<Vec<_>>());
    let font_names = Rc::new(RefCell::new(families));
    let font_combo = gtk::DropDown::new(Some(font_model.clone()), None::<&gtk::Expression>);
    font_combo.set_hexpand(true);
    font_combo.set_size_request(0, -1);
    font_combo.add_css_class("text-font-picker");
    font_combo.set_tooltip_text(Some("Font family"));
    font_combo.set_selected(0);
    typography_section.append(&font_combo);
    let format_icon = |name: &str| {
        let icon = gtk::Image::from_icon_name(name);
        icon.set_pixel_size(16);
        icon
    };
    let format_letter = |markup: &str| {
        let letter = gtk::Label::new(None);
        letter.set_markup(markup);
        letter
    };
    let bold_button = editor_tool_button(&format_letter("<b>B</b>"), "Bold");
    let italic_button = editor_tool_button(&format_letter("<i>I</i>"), "Italic");
    let left_button = editor_tool_button(&format_icon("format-justify-left-symbolic"), "Left");
    left_button.set_active(true);
    let center_button =
        editor_tool_button(&format_icon("format-justify-center-symbolic"), "Centre");
    center_button.set_group(Some(&left_button));
    let right_button = editor_tool_button(&format_icon("format-justify-right-symbolic"), "Right");
    right_button.set_group(Some(&left_button));
    let format_group = editor_button_group(
        5,
        &[
            &bold_button,
            &italic_button,
            &left_button,
            &center_button,
            &right_button,
        ],
    );
    format_group.add_css_class("editor-format-group");
    typography_section.append(&format_group);
    let align_buttons = vec![
        (TextAlign::Left, left_button.clone()),
        (TextAlign::Center, center_button),
        (TextAlign::Right, right_button),
    ];
    editor_section_divider(&selected_section);

    let colour_row = editor_section(&selected_section, "COLOUR");
    let colour_button = gtk::ColorDialogButton::new(Some(gtk::ColorDialog::new()));
    colour_button.set_tooltip_text(Some("Choose custom colour"));
    colour_button.set_rgba(&gtk::gdk::RGBA::new(1.0, 1.0, 1.0, 1.0));
    let (colour_swatches, swatch_buttons, custom_swatch) = editor_colour_palette([
        (1.0, 1.0, 1.0),
        (0.05, 0.05, 0.05),
        (0.95, 0.12, 0.12),
        (1.0, 0.48, 0.0),
        (1.0, 0.9, 0.05),
        (0.12, 0.72, 0.28),
        (0.15, 0.42, 1.0),
        (0.68, 0.3, 0.88),
    ]);
    colour_row.append(&colour_swatches);
    editor_section_divider(&selected_section);

    // Size and opacity side by side.
    let appearance_columns = editor_control_columns(&selected_section);
    let (size_slider, _size_value) =
        editor_slider_column(&appearance_columns, "Size", (0.5, 50.0, 0.5), 4.5, |n| {
            format!("{n:.1}%")
        });
    size_slider.set_tooltip_text(Some("Font size as a percentage of the photo height"));
    let (opacity_row, _opacity_value) =
        editor_slider_column(&appearance_columns, "Opacity", (0.0, 1.0, 0.01), 1.0, |n| {
            format!("{:.0}%", n * 100.0)
        });
    opacity_row.set_digits(2);
    add_scale_default_affordance(&opacity_row, 1.0);
    editor_section_divider(&selected_section);

    // Position: anchor grid with its offsets, then the placement shortcuts.
    let position_section = editor_section(&selected_section, "POSITION");
    let (offset_x, offset_y) = editor_offset_row(&position_section);
    let anchor_buttons = editor_position_buttons(&position_section);
    let fit_width = gtk::Button::with_label("Fit Width");
    fit_width.set_tooltip_text(Some("Scale text to span the photo width"));
    fit_width.add_css_class("crop-reset-button");
    let fit_screen = gtk::Button::with_label("Fit Screen");
    fit_screen.set_tooltip_text(Some("Scale text to fit inside the photo"));
    fit_screen.add_css_class("crop-reset-button");
    let reset_placement = gtk::Button::with_label("Reset");
    reset_placement.set_tooltip_text(Some("Centre this text and reset its position offsets"));
    reset_placement.add_css_class("crop-reset-button");
    let fit_row = editor_button_group(3, &[&fit_width, &fit_screen, &reset_placement]);
    fit_row.add_css_class("edit-fit-row");
    position_section.append(&fit_row);

    // Layers: choose which text layer the controls above edit.
    editor_section_divider(parent);
    let layers_section = editor_section(parent, "TEXT LAYERS");
    let list = gtk::ListBox::new();
    list.set_hexpand(true);
    list.set_halign(gtk::Align::Fill);
    list.set_selection_mode(gtk::SelectionMode::Single);
    list.add_css_class("overlay-list");
    layers_section.append(&list);

    let add_text = gtk::Button::with_label("+ Add text");
    add_text.set_tooltip_text(Some("Add a text layer on top of this photo"));
    let remove = editor_destructive_button("Delete selected");
    remove.set_sensitive(false);
    remove.set_tooltip_text(Some("Delete this text layer from the photo"));
    editor_action_row(parent, &[&add_text, &remove]);

    let sync: Rc<dyn Fn()> = {
        let session = session.clone();
        let selected = selected.clone();
        let syncing = syncing.clone();
        let list = list.clone();
        let selected_section = selected_section.clone();
        let text_view = text_view.clone();
        let font_combo = font_combo.clone();
        let font_model = font_model.clone();
        let font_names = font_names.clone();
        let colour_button = colour_button.clone();
        let size_slider = size_slider.clone();
        let bold_button = bold_button.clone();
        let italic_button = italic_button.clone();
        let align_buttons = align_buttons.clone();
        let opacity_row = opacity_row.clone();
        let anchor_buttons = anchor_buttons.clone();
        let remove = remove.clone();
        let offset_x = offset_x.clone();
        let offset_y = offset_y.clone();
        let photo_dimensions = photo_dimensions.clone();
        let redraw_canvas = redraw_canvas.clone();
        let update_canvas_input = update_canvas_input.clone();
        Rc::new(move || {
            syncing.set(true);
            let layers = session.borrow().recipe.text_layers.clone();
            if selected.get().is_some_and(|index| index >= layers.len()) {
                selected.set(if layers.is_empty() { None } else { Some(0) });
            }
            if selected.get().is_none() && !layers.is_empty() {
                selected.set(Some(0));
            }
            let active = selected.get();
            remove.set_sensitive(active.is_some());
            selected_section.set_visible(true);
            selected_section.set_sensitive(active.is_some());
            list.set_visible(true);

            while let Some(child) = list.first_child() {
                list.remove(&child);
            }
            for (index, layer) in layers.iter().enumerate() {
                let row = gtk::ListBoxRow::new();
                let label = gtk::Label::new(None);
                label.set_xalign(0.0);
                label.set_hexpand(true);
                label.set_single_line_mode(true);
                label.set_max_width_chars(24);
                label.set_ellipsize(gtk::pango::EllipsizeMode::End);
                let first_line = layer.text.lines().next().unwrap_or("");
                let mut preview = first_line.chars().take(40).collect::<String>();
                if first_line.chars().count() > 40 {
                    preview.push('…');
                }
                let name = if preview.is_empty() {
                    "Text".to_string()
                } else {
                    format!("Text: {preview}")
                };
                let visibility = if layer.visible { "" } else { " · hidden" };
                label.set_text(&format!("{name}{visibility}"));
                row.set_child(Some(&label));
                list.append(&row);
                if Some(index) == active {
                    list.select_row(Some(&row));
                }
            }

            if let Some(layer) = active.and_then(|index| layers.get(index)) {
                let buffer = text_view.buffer();
                let start = buffer.start_iter();
                let end = buffer.end_iter();
                let current = buffer.text(&start, &end, false);
                if current.as_str() != layer.text {
                    buffer.set_text(&layer.text);
                }
                size_slider.set_value(f64::from(layer.size) * 100.0);
                bold_button.set_active(layer.bold);
                italic_button.set_active(layer.italic);
                for (align, button) in &align_buttons {
                    button.set_active(*align == layer.align);
                }
                let existing_font = font_names
                    .borrow()
                    .iter()
                    .position(|name| name == &layer.font_family);
                let font_index = existing_font.unwrap_or_else(|| {
                    font_model.append(&font_family_display_name(&layer.font_family));
                    let mut names = font_names.borrow_mut();
                    names.push(layer.font_family.clone());
                    names.len() - 1
                });
                font_combo.set_selected(font_index as u32);
                let (red, green, blue, alpha) = layer.color_rgba();
                colour_button.set_rgba(&gtk::gdk::RGBA::new(red, green, blue, alpha));
                opacity_row.set_value(f64::from(layer.opacity));
                for (anchor, button) in &anchor_buttons {
                    let (width, height) = photo_dimensions.get();
                    let (col, row, x_offset, y_offset) = text_grid_position(layer, width, height);
                    button.set_active((col, row) == *anchor);
                    offset_x.set_value(x_offset);
                    offset_y.set_value(y_offset);
                }
            } else {
                colour_button.set_rgba(&gtk::gdk::RGBA::new(1.0, 1.0, 1.0, 1.0));
                opacity_row.set_value(1.0);
                for ((col, row), button) in &anchor_buttons {
                    button.set_active((*col, *row) == (1, 1));
                }
                offset_x.set_value(0.0);
                offset_y.set_value(0.0);
            }
            syncing.set(false);
            redraw_canvas();
            update_canvas_input();
        })
    };

    {
        let selected = selected.clone();
        let syncing = syncing.clone();
        let sync = sync.clone();
        list.connect_row_selected(move |_, row| {
            if syncing.get() {
                return;
            }
            // Rebuilding list rows during sync can emit a delayed `None`
            // notification. A single-selection list cannot be user-cleared,
            // so only a real row should update the active text layer.
            let Some(row) = row else {
                if std::env::var_os("PICASA_TRACE").is_some() {
                    eprintln!(
                        "PIC_TEXT list_selection_cleared_preserved selected={:?}",
                        selected.get()
                    );
                }
                return;
            };
            let index = Some(row.index() as usize);
            if selected.get() == index {
                return;
            }
            selected.set(index);
            sync();
        });
    }

    {
        let session = session.clone();
        let selected = selected.clone();
        let update_history_buttons = update_history_buttons.clone();
        let sync = sync.clone();
        let text_toggle = text_toggle.clone();
        add_text.connect_clicked(move |_| {
            session.borrow_mut().mutate(|recipe| {
                recipe.text_layers.push(TextLayerSpec::new_default());
            });
            let index = session.borrow().recipe.text_layers.len().saturating_sub(1);
            selected.set(Some(index));
            text_toggle.set_active(true);
            update_history_buttons();
            sync();
        });
    }

    {
        let session = session.clone();
        let selected = selected.clone();
        let update_history_buttons = update_history_buttons.clone();
        let sync = sync.clone();
        remove.connect_clicked(move |_| {
            let Some(index) = selected.get() else {
                return;
            };
            session.borrow_mut().mutate(move |recipe| {
                if index < recipe.text_layers.len() {
                    recipe.text_layers.remove(index);
                }
            });
            selected.set(None);
            update_history_buttons();
            sync();
        });
    }

    {
        let session = session.clone();
        let selected = selected.clone();
        let syncing = syncing.clone();
        let update_history_buttons = update_history_buttons.clone();
        let redraw_canvas = redraw_canvas.clone();
        let sync = sync.clone();
        let typing = Rc::new(Cell::new(false));
        let typing_source: Rc<RefCell<Option<glib::SourceId>>> = Rc::new(RefCell::new(None));
        text_view.buffer().connect_changed(move |buffer| {
            if syncing.get() {
                return;
            }
            let Some(index) = selected.get() else {
                return;
            };
            let text = {
                let start = buffer.start_iter();
                let end = buffer.end_iter();
                buffer.text(&start, &end, false).to_string()
            };
            {
                let mut session = session.borrow_mut();
                if !typing.get() {
                    typing.set(true);
                    session.begin_action();
                }
                session.mutate_active(move |recipe| {
                    if let Some(layer) = recipe.text_layers.get_mut(index) {
                        layer.text = text.clone();
                    }
                });
            }
            redraw_canvas();
            if let Some(previous) = typing_source.borrow_mut().take() {
                previous.remove();
            }
            let session = session.clone();
            let typing = typing.clone();
            let typing_source_for_timer = typing_source.clone();
            let update_history_buttons = update_history_buttons.clone();
            let sync = sync.clone();
            let source = glib::timeout_add_local(Duration::from_millis(700), move || {
                if typing.replace(false) {
                    session.borrow_mut().end_action();
                    update_history_buttons();
                    sync();
                }
                let _ = typing_source_for_timer.borrow_mut().take();
                glib::ControlFlow::Break
            });
            typing_source.borrow_mut().replace(source);
        });
    }

    {
        let session = session.clone();
        let selected = selected.clone();
        let syncing = syncing.clone();
        let update_history_buttons = update_history_buttons.clone();
        let redraw_canvas = redraw_canvas.clone();
        let font_names = font_names.clone();
        font_combo.connect_selected_notify(move |combo| {
            if syncing.get() {
                return;
            }
            let Some(index) = selected.get() else {
                return;
            };
            let Some(family) = font_names.borrow().get(combo.selected() as usize).cloned() else {
                return;
            };
            session.borrow_mut().mutate(move |recipe| {
                if let Some(layer) = recipe.text_layers.get_mut(index) {
                    layer.font_family = family.clone();
                }
            });
            update_history_buttons();
            redraw_canvas();
        });
    }

    {
        let session = session.clone();
        let selected = selected.clone();
        let syncing = syncing.clone();
        let update_history_buttons = update_history_buttons.clone();
        let redraw_canvas = redraw_canvas.clone();
        // A slider drag is one undo step, like the opacity slider.
        let dragging = Rc::new(Cell::new(false));
        let press = gtk::GestureClick::new();
        press.set_button(1);
        press.set_propagation_phase(gtk::PropagationPhase::Capture);
        {
            let session = session.clone();
            let selected = selected.clone();
            let dragging = dragging.clone();
            press.connect_pressed(move |_, _, _, _| {
                if selected.get().is_some() {
                    dragging.set(true);
                    session.borrow_mut().begin_action();
                }
            });
        }
        {
            let session = session.clone();
            let dragging = dragging.clone();
            let update_history_buttons = update_history_buttons.clone();
            let redraw_canvas = redraw_canvas.clone();
            press.connect_released(move |_, _, _, _| {
                if dragging.replace(false) {
                    session.borrow_mut().end_action();
                    update_history_buttons();
                    redraw_canvas();
                }
            });
        }
        size_slider.add_controller(press);
        size_slider.connect_value_changed(move |slider| {
            if syncing.get() {
                return;
            }
            let Some(index) = selected.get() else {
                return;
            };
            let size = (slider.value() / 100.0) as f32;
            let size = size.clamp(TextLayerSpec::MIN_SIZE, TextLayerSpec::MAX_SIZE);
            let was_dragging = dragging.get();
            {
                let mut session = session.borrow_mut();
                let apply = move |recipe: &mut EditRecipe| {
                    if let Some(layer) = recipe.text_layers.get_mut(index) {
                        layer.size = size;
                    }
                };
                if was_dragging {
                    session.mutate_active(apply);
                } else {
                    session.mutate(apply);
                }
            }
            if !was_dragging {
                update_history_buttons();
            }
            redraw_canvas();
        });
    }

    {
        let session = session.clone();
        let selected = selected.clone();
        let syncing = syncing.clone();
        let update_history_buttons = update_history_buttons.clone();
        let redraw_canvas = redraw_canvas.clone();
        colour_button.connect_rgba_notify(move |button| {
            if syncing.get() {
                return;
            }
            let Some(index) = selected.get() else {
                return;
            };
            let rgba = button.rgba();
            let (red, green, blue, alpha) = (rgba.red(), rgba.green(), rgba.blue(), rgba.alpha());
            session.borrow_mut().mutate(move |recipe| {
                if let Some(layer) = recipe.text_layers.get_mut(index) {
                    layer.set_color_rgba(red, green, blue, alpha);
                }
            });
            update_history_buttons();
            redraw_canvas();
        });
    }

    for (swatch, color) in swatch_buttons {
        let colour_button = colour_button.clone();
        swatch.connect_clicked(move |_| {
            colour_button.set_rgba(&gtk::gdk::RGBA::new(color.0, color.1, color.2, 1.0));
        });
    }

    {
        let colour_button = colour_button.clone();
        let chooser = gtk::ColorDialog::new();
        let colour_row = colour_row.clone();
        custom_swatch.connect_clicked(move |_| {
            let initial = colour_button.rgba();
            let dialog = chooser.clone();
            let result_button = colour_button.clone();
            let parent_window = colour_row
                .root()
                .and_then(|root| root.downcast::<gtk::Window>().ok());
            if let Some(parent_window) = parent_window.as_ref() {
                dialog.choose_rgba(
                    Some(parent_window),
                    Some(&initial),
                    None::<&gtk::gio::Cancellable>,
                    move |result| {
                        if let Ok(color) = result {
                            result_button.set_rgba(&color);
                        }
                    },
                );
            } else {
                dialog.choose_rgba(
                    None::<&gtk::Window>,
                    Some(&initial),
                    None::<&gtk::gio::Cancellable>,
                    move |result| {
                        if let Ok(color) = result {
                            result_button.set_rgba(&color);
                        }
                    },
                );
            }
        });
    }

    for (button, apply) in [(bold_button.clone(), true), (italic_button.clone(), false)] {
        let session = session.clone();
        let selected = selected.clone();
        let syncing = syncing.clone();
        let update_history_buttons = update_history_buttons.clone();
        let redraw_canvas = redraw_canvas.clone();
        button.connect_toggled(move |button| {
            if syncing.get() {
                return;
            }
            let Some(index) = selected.get() else {
                return;
            };
            let active = button.is_active();
            session.borrow_mut().mutate(move |recipe| {
                let Some(layer) = recipe.text_layers.get_mut(index) else {
                    return;
                };
                if apply {
                    layer.bold = active;
                } else {
                    layer.italic = active;
                }
            });
            update_history_buttons();
            redraw_canvas();
        });
    }

    for (align, button) in &align_buttons {
        let align = *align;
        let session = session.clone();
        let selected = selected.clone();
        let syncing = syncing.clone();
        let update_history_buttons = update_history_buttons.clone();
        let redraw_canvas = redraw_canvas.clone();
        button.connect_toggled(move |button| {
            if syncing.get() || !button.is_active() {
                return;
            }
            let Some(index) = selected.get() else {
                return;
            };
            session.borrow_mut().mutate(move |recipe| {
                if let Some(layer) = recipe.text_layers.get_mut(index) {
                    layer.align = align;
                }
            });
            update_history_buttons();
            redraw_canvas();
        });
    }

    {
        let session = session.clone();
        let selected = selected.clone();
        let syncing = syncing.clone();
        let update_history_buttons = update_history_buttons.clone();
        let redraw_canvas = redraw_canvas.clone();
        let dragging = Rc::new(Cell::new(false));
        let press = gtk::GestureClick::new();
        press.set_button(1);
        press.set_propagation_phase(gtk::PropagationPhase::Capture);
        {
            let session = session.clone();
            let selected = selected.clone();
            let dragging = dragging.clone();
            press.connect_pressed(move |_, _, _, _| {
                if selected.get().is_some() {
                    dragging.set(true);
                    session.borrow_mut().begin_action();
                }
            });
        }
        {
            let session = session.clone();
            let dragging = dragging.clone();
            let update_history_buttons = update_history_buttons.clone();
            let redraw_canvas = redraw_canvas.clone();
            press.connect_released(move |_, _, _, _| {
                if dragging.replace(false) {
                    session.borrow_mut().end_action();
                    update_history_buttons();
                    redraw_canvas();
                }
            });
        }
        opacity_row.add_controller(press);
        opacity_row.connect_value_changed(move |scale| {
            if syncing.get() {
                return;
            }
            let Some(index) = selected.get() else {
                return;
            };
            let value = (scale.value() as f32).clamp(0.0, 1.0);
            let was_dragging = dragging.get();
            {
                let mut session = session.borrow_mut();
                if was_dragging {
                    session.mutate_active(move |recipe| {
                        if let Some(layer) = recipe.text_layers.get_mut(index) {
                            layer.opacity = value;
                        }
                    });
                } else {
                    session.mutate(move |recipe| {
                        if let Some(layer) = recipe.text_layers.get_mut(index) {
                            layer.opacity = value;
                        }
                    });
                }
            }
            if !was_dragging {
                update_history_buttons();
            }
            redraw_canvas();
        });
    }

    for ((col, row), button) in &anchor_buttons {
        let (col, row) = (*col, *row);
        let session = session.clone();
        let selected = selected.clone();
        let syncing = syncing.clone();
        let update_history_buttons = update_history_buttons.clone();
        let redraw_canvas = redraw_canvas.clone();
        let photo_dimensions = photo_dimensions.clone();
        button.connect_toggled(move |button| {
            if syncing.get() || !button.is_active() {
                return;
            }
            let Some(index) = selected.get() else {
                return;
            };
            let dims = photo_dimensions.get();
            session.borrow_mut().mutate(move |recipe| {
                let Some(layer) = recipe.text_layers.get_mut(index) else {
                    return;
                };
                place_text_grid(layer, dims, col, row, 0.0, 0.0);
            });
            update_history_buttons();
            redraw_canvas();
        });
    }

    for (spin, horizontal) in [(offset_x.clone(), true), (offset_y.clone(), false)] {
        let session = session.clone();
        let selected = selected.clone();
        let syncing = syncing.clone();
        let update_history_buttons = update_history_buttons.clone();
        let redraw_canvas = redraw_canvas.clone();
        let photo_dimensions = photo_dimensions.clone();
        let offset_x = offset_x.clone();
        let offset_y = offset_y.clone();
        spin.connect_value_changed(move |spin| {
            if syncing.get() {
                return;
            }
            let Some(index) = selected.get() else { return };
            let dims = photo_dimensions.get();
            let x_offset = if horizontal {
                spin.value()
            } else {
                offset_x.value()
            };
            let y_offset = if horizontal {
                offset_y.value()
            } else {
                spin.value()
            };
            session.borrow_mut().mutate(move |recipe| {
                if let Some(layer) = recipe.text_layers.get_mut(index) {
                    let (col, row, _, _) = text_grid_position(layer, dims.0, dims.1);
                    place_text_grid(layer, dims, col, row, x_offset, y_offset);
                }
            });
            update_history_buttons();
            redraw_canvas();
        });
    }

    for (button, fit_screen_mode) in [(&fit_width, false), (&fit_screen, true)] {
        let session = session.clone();
        let selected = selected.clone();
        let dimensions = photo_dimensions.clone();
        let update_history_buttons = update_history_buttons.clone();
        let sync = sync.clone();
        button.connect_clicked(move |_| {
            let Some(index) = selected.get() else { return };
            let (photo_w, photo_h) = dimensions.get();
            if photo_w <= 0 || photo_h <= 0 {
                return;
            }
            session.borrow_mut().mutate(move |recipe| {
                let Some(layer) = recipe.text_layers.get_mut(index) else {
                    return;
                };
                let Some(metrics) = super::text_render::measure_text_px(layer, photo_h as f64)
                else {
                    return;
                };
                if metrics.width <= 0 || metrics.height <= 0 {
                    return;
                }
                let scale_x = photo_w as f64 / metrics.width as f64;
                let scale_y = photo_h as f64 / metrics.height as f64;
                let scale = if fit_screen_mode {
                    scale_x.min(scale_y)
                } else {
                    scale_x
                };
                layer.size = (f64::from(layer.size) * scale).clamp(
                    f64::from(TextLayerSpec::MIN_SIZE),
                    f64::from(TextLayerSpec::MAX_SIZE),
                ) as f32;
            });
            update_history_buttons();
            sync();
        });
    }

    {
        let session = session.clone();
        let selected = selected.clone();
        let update_history_buttons = update_history_buttons.clone();
        let sync = sync.clone();
        reset_placement.connect_clicked(move |_| {
            let Some(index) = selected.get() else {
                return;
            };
            session.borrow_mut().mutate(move |recipe| {
                if let Some(layer) = recipe.text_layers.get_mut(index) {
                    layer.anchor = super::model::OverlayAnchor::Center;
                    layer.x = 0.5;
                    layer.y = 0.5;
                }
            });
            update_history_buttons();
            sync();
        });
    }

    sync();
    sync
}

fn text_grid_position(layer: &TextLayerSpec, photo_w: i32, photo_h: i32) -> (u8, u8, f64, f64) {
    let (col, row) = layer.anchor.grid_position();
    let targets = [0.0, 0.5, 1.0];
    (
        col,
        row,
        f64::from(layer.x - targets[usize::from(col)]) * f64::from(photo_w.max(1)),
        f64::from(layer.y - targets[usize::from(row)]) * f64::from(photo_h.max(1)),
    )
}

fn place_text_grid(
    layer: &mut TextLayerSpec,
    dims: (i32, i32),
    col: u8,
    row: u8,
    x_offset: f64,
    y_offset: f64,
) {
    let col = col.min(2);
    let row = row.min(2);
    let targets = [0.0, 0.5, 1.0];
    layer.anchor = OverlayAnchor::from_grid(col, row);
    layer.x = targets[usize::from(col)] + (x_offset / f64::from(dims.0.max(1))) as f32;
    layer.y = targets[usize::from(row)] + (y_offset / f64::from(dims.1.max(1))) as f32;
}

#[cfg(test)]
mod text_position_tests {
    use super::*;

    #[test]
    fn all_nine_position_cells_place_and_report_the_text_layer() {
        let dims = (2400, 1600);
        let targets = [0.0, 0.5, 1.0];
        for row in 0..3 {
            for col in 0..3 {
                let mut layer = TextLayerSpec::new_default();
                place_text_grid(&mut layer, dims, col, row, 0.0, 0.0);
                let rect = super::super::text_render::text_rect(
                    &layer,
                    f64::from(dims.0),
                    f64::from(dims.1),
                )
                .unwrap();
                let x = [rect.left, rect.left + rect.width * 0.5, rect.right()][col as usize];
                let y = [rect.top, rect.top + rect.height * 0.5, rect.bottom()][row as usize];
                assert!((x - targets[col as usize]).abs() < 0.001);
                assert!((y - targets[row as usize]).abs() < 0.001);
                assert_eq!(
                    (
                        text_grid_position(&layer, dims.0, dims.1).0,
                        text_grid_position(&layer, dims.0, dims.1).1
                    ),
                    (col, row)
                );
            }
        }
    }

    #[test]
    fn text_anchors_persist_when_pasted_to_portrait() {
        for row in 0..3 {
            for col in 0..3 {
                let mut source = TextLayerSpec::new_default();
                place_text_grid(&mut source, (6000, 4000), col, row, 0.0, 0.0);
                let recipe = EditRecipe {
                    text_layers: vec![source],
                    ..Default::default()
                };
                let pasted = EditRecipe::decode(&recipe.encode());
                let layer = &pasted.text_layers[0];
                assert_eq!(layer.anchor, OverlayAnchor::from_grid(col, row));
                let rect = super::super::text_render::text_rect(layer, 4000.0, 6000.0).unwrap();
                let (ax, ay) = layer.anchor.factors();
                let targets = [0.0, 0.5, 1.0];
                assert!((rect.left + rect.width * ax - targets[col as usize]).abs() < 1e-5);
                assert!((rect.top + rect.height * ay - targets[row as usize]).abs() < 1e-5);
            }
        }
    }

    #[test]
    fn offsets_are_photo_relative_and_survive_position_sync() {
        let dims = (2400, 1600);
        let mut layer = TextLayerSpec::new_default();
        place_text_grid(&mut layer, dims, 2, 0, 120.0, -50.0);
        let (col, row, x, y) = text_grid_position(&layer, dims.0, dims.1);
        assert_eq!((col, row), (2, 0));
        assert!((x - 120.0).abs() < 1.0);
        assert!((y + 50.0).abs() < 1.0);
    }
}
