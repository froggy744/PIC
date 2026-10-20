// Draw / annotation panel and interactive image-space canvas.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DrawTool {
    Select,
    Pen,
    Line,
    Arrow,
    Rectangle,
    Ellipse,
    Highlighter,
    Eraser,
}

#[derive(Clone, Default)]
struct DrawGesture {
    start: (f32, f32),
    current: (f32, f32),
    points: Vec<(f32, f32)>,
    before: Vec<(usize, super::model::DrawAnnotation)>,
    selected: Option<usize>,
    resizing: bool,
    selecting_box: bool,
    additive_selection: bool,
}

fn draw_tool_button(icon_name: &str, label: &str) -> gtk::ToggleButton {
    let button = gtk::ToggleButton::new();
    let icon = gtk::Image::from_icon_name(icon_name);
    icon.set_pixel_size(22);
    button.set_child(Some(&icon));
    button.set_size_request(40, 40);
    button.set_tooltip_text(Some(label));
    button.update_property(&[gtk::accessible::Property::Label(label)]);
    button.add_css_class("flat");
    button.add_css_class("draw-tool-button");
    button
}

fn draw_slider_row(
    parent: &gtk::Box,
    label: &str,
    min: f64,
    max: f64,
    step: f64,
    initial: f64,
    percent: bool,
) -> (gtk::Scale, gtk::Label) {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    row.set_hexpand(true);
    row.add_css_class("edit-adjustment-row");
    row.add_css_class("draw-adjustment-row");
    let title = gtk::Label::new(Some(label));
    title.set_width_chars(10);
    title.set_xalign(0.0);
    let scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, min, max, step);
    scale.set_value(initial);
    scale.set_draw_value(false);
    scale.set_hexpand(true);
    scale.set_tooltip_text(Some(label));
    scale.add_css_class("edit-adjustment-scale");
    let value = gtk::Label::new(None);
    value.set_width_chars(5);
    value.set_xalign(1.0);
    value.add_css_class("edit-adjustment-value");
    value.add_css_class("draw-adjustment-value");
    let format = move |n: f64| {
        if percent {
            format!("{n:.0}%")
        } else {
            format!("{n:.0}")
        }
    };
    value.set_text(&format(initial));
    {
        let value_label = value.clone();
        scale.connect_value_changed(move |scale| value_label.set_text(&format(scale.value())));
    }
    row.append(&title);
    row.append(&scale);
    row.append(&value);
    parent.append(&row);
    (scale, value)
}

#[allow(clippy::too_many_arguments)]
fn build_draw_panel(
    parent: &gtk::Box,
    layer: &gtk::DrawingArea,
    session: Rc<RefCell<EditSession>>,
    preview_dimensions: Rc<Cell<(i32, i32)>>,
    canvas_zoom: Rc<Cell<f64>>,
    native_one_to_one: Rc<Cell<bool>>,
    picture_scroll: gtk::ScrolledWindow,
    draw_toggle: gtk::ToggleButton,
    update_history_buttons: Rc<dyn Fn()>,
    queue_preview: Rc<dyn Fn()>,
) -> Rc<dyn Fn()> {
    use super::model::DrawAnnotation;
    let tools = Rc::new(Cell::new(DrawTool::Pen));
    let selected = Rc::new(Cell::new(None::<usize>));
    let selected_many = Rc::new(RefCell::new(Vec::<usize>::new()));
    let gesture = Rc::new(RefCell::new(DrawGesture::default()));
    let tool_grid = gtk::Grid::new();
    tool_grid.set_halign(gtk::Align::Center);
    tool_grid.set_column_homogeneous(true);
    tool_grid.set_row_homogeneous(true);
    tool_grid.set_column_spacing(4);
    tool_grid.set_row_spacing(4);
    tool_grid.add_css_class("draw-tool-grid");
    let select = draw_tool_button("draw-select-symbolic", "Select");
    select.set_active(false);
    let pen = draw_tool_button("draw-pen-symbolic", "Pen");
    pen.set_active(true);
    let line = draw_tool_button("draw-line-symbolic", "Line");
    let arrow = draw_tool_button("draw-arrow-symbolic", "Arrow");
    let rect = draw_tool_button("draw-rectangle-symbolic", "Rectangle");
    let ellipse = draw_tool_button("draw-ellipse-symbolic", "Ellipse");
    let highlighter = draw_tool_button("draw-highlighter-symbolic", "Highlighter");
    let eraser = draw_tool_button("draw-eraser-symbolic", "Eraser");
    for button in [&pen, &line, &arrow, &rect, &ellipse, &highlighter, &eraser] {
        button.set_group(Some(&select));
    }
    for (index, button) in [
        &select,
        &pen,
        &line,
        &arrow,
        &rect,
        &ellipse,
        &highlighter,
        &eraser,
    ]
    .into_iter()
    .enumerate()
    {
        tool_grid.attach(button, (index % 4) as i32, (index / 4) as i32, 1, 1);
    }
    parent.append(&tool_grid);
    let arrow_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let arrow_start = gtk::CheckButton::with_label("Start head");
    let arrow_end = gtk::CheckButton::with_label("End head");
    arrow_start.set_tooltip_text(Some("Add an arrowhead at the start"));
    arrow_end.set_tooltip_text(Some("Add an arrowhead at the end"));
    arrow_end.set_active(true);
    arrow_row.append(&arrow_start);
    arrow_row.append(&arrow_end);
    arrow_row.set_visible(false);
    parent.append(&arrow_row);
    let arrowheads = Rc::new(Cell::new((false, true)));
    {
        let heads = arrowheads.clone();
        arrow_start.connect_toggled(move |b| {
            let (_, end) = heads.get();
            heads.set((b.is_active(), end));
        });
    }
    {
        let heads = arrowheads.clone();
        arrow_end.connect_toggled(move |b| {
            let (start, _) = heads.get();
            heads.set((start, b.is_active()));
        });
    }

    let colour_row = gtk::Box::new(gtk::Orientation::Vertical, 6);
    colour_row.set_hexpand(true);
    colour_row.set_halign(gtk::Align::Fill);
    colour_row.set_margin_top(10);
    let colour_swatches = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    colour_swatches.set_hexpand(true);
    colour_swatches.set_halign(gtk::Align::Fill);
    colour_swatches.set_homogeneous(true);
    colour_swatches.add_css_class("text-colour-swatches");
    let mut color_buttons = Vec::new();
    for (name, class, rgba) in [
        ("White", "text-swatch-white", 0xffffffff),
        ("Black", "text-swatch-black", 0x111111ff),
        ("Red", "text-swatch-red", 0xf21f1fff),
        ("Orange", "text-swatch-orange", 0xff7a00ff),
        ("Yellow", "text-swatch-yellow", 0xffe60dff),
        ("Green", "text-swatch-green", 0x1fb848ff),
        ("Blue", "text-swatch-blue", 0x266bffff),
        ("Purple", "text-swatch-purple", 0xad4de0ff),
    ] {
        let b = gtk::Button::new();
        b.set_size_request(0, 30);
        b.add_css_class("text-colour-swatch");
        b.add_css_class(class);
        b.set_tooltip_text(Some(name));
        colour_swatches.append(&b);
        color_buttons.push((b, rgba));
    }
    let custom = gtk::Button::new();
    custom.set_size_request(0, 30);
    custom.add_css_class("text-colour-swatch");
    custom.set_tooltip_text(Some("Choose custom colour"));
    let custom_icon = gtk::Image::from_icon_name("color-select-symbolic");
    custom_icon.set_pixel_size(16);
    custom.set_child(Some(&custom_icon));
    colour_swatches.append(&custom);
    colour_row.append(&colour_swatches);
    parent.append(&colour_row);

    let (width, _width_value) = draw_slider_row(parent, "Thickness", 1.0, 36.0, 1.0, 5.0, false);
    let (opacity, _opacity_value) =
        draw_slider_row(parent, "Opacity", 5.0, 100.0, 5.0, 100.0, true);
    let delete = gtk::Button::with_label("Delete selected");
    delete.set_sensitive(false);
    parent.append(&delete);
    for (b, tool) in [
        (&select, DrawTool::Select),
        (&pen, DrawTool::Pen),
        (&line, DrawTool::Line),
        (&arrow, DrawTool::Arrow),
        (&rect, DrawTool::Rectangle),
        (&ellipse, DrawTool::Ellipse),
        (&highlighter, DrawTool::Highlighter),
        (&eraser, DrawTool::Eraser),
    ] {
        let tools = tools.clone();
        let layer = layer.clone();
        let arrow_row = arrow_row.clone();
        b.connect_toggled(move |button| {
            if button.is_active() {
                tools.set(tool);
                arrow_row.set_visible(tool == DrawTool::Arrow);
                layer.set_cursor_from_name(Some(cursor_for_tool(tool)));
                layer.queue_draw();
            }
        });
    }
    let color = Rc::new(Cell::new(0xf21f1fffu32));
    let style_sync = Rc::new(Cell::new(false));
    let apply_selected_color = Rc::new({
        let session = session.clone();
        let selected = selected.clone();
        let selected_many = selected_many.clone();
        let update = update_history_buttons.clone();
        let layer = layer.clone();
        move |rgba: u32| {
            let indices = selected_many.borrow().clone();
            if !indices.is_empty() {
                session.borrow_mut().mutate(|r| {
                    for i in &indices {
                        if let Some(item) = r.drawings.get_mut(*i) {
                            item.color = rgba;
                        }
                    }
                });
                update();
                layer.queue_draw();
            } else if let Some(i) = selected.get() {
                session.borrow_mut().mutate(|r| {
                    if let Some(item) = r.drawings.get_mut(i) {
                        item.color = rgba;
                    }
                });
                update();
                layer.queue_draw();
            }
        }
    });
    for (button, rgba) in color_buttons {
        let color = color.clone();
        let apply_selected_color = apply_selected_color.clone();
        button.connect_clicked(move |_| {
            color.set(rgba);
            apply_selected_color(rgba);
        });
    }
    {
        let colour_row = colour_row.clone();
        let color = color.clone();
        let apply_selected_color = apply_selected_color.clone();
        let chooser = gtk::ColorDialog::new();
        custom.connect_clicked(move |_| {
            let current = color.get();
            let initial = gtk::gdk::RGBA::new(
                ((current >> 24) & 255) as f32 / 255.,
                ((current >> 16) & 255) as f32 / 255.,
                ((current >> 8) & 255) as f32 / 255.,
                (current & 255) as f32 / 255.,
            );
            let dialog = chooser.clone();
            let color = color.clone();
            let apply_selected_color = apply_selected_color.clone();
            let parent_window = colour_row
                .root()
                .and_then(|root| root.downcast::<gtk::Window>().ok());
            dialog.choose_rgba(
                parent_window.as_ref(),
                Some(&initial),
                None::<&gtk::gio::Cancellable>,
                move |result| {
                    if let Ok(picked) = result {
                        let q = |v: f32| (v.clamp(0., 1.) * 255.).round() as u32;
                        let rgba = (q(picked.red()) << 24)
                            | (q(picked.green()) << 16)
                            | (q(picked.blue()) << 8)
                            | q(picked.alpha());
                        color.set(rgba);
                        apply_selected_color(rgba);
                    }
                },
            );
        });
    }
    {
        let session = session.clone();
        let selected = selected.clone();
        let selected_many = selected_many.clone();
        let width = width.clone();
        let style_sync = style_sync.clone();
        let update = update_history_buttons.clone();
        let layer = layer.clone();
        width.connect_value_changed(move |scale| {
            if style_sync.get() {
                return;
            }
            let indices = selected_many.borrow().clone();
            if !indices.is_empty() || selected.get().is_some() {
                let value = (scale.value() as f32 / 1000.).clamp(0.001, 0.04);
                session.borrow_mut().mutate(|r| {
                    for i in &indices {
                        if let Some(item) = r.drawings.get_mut(*i) {
                            item.width = value;
                        }
                    }
                    if indices.is_empty() {
                        if let Some(item) = selected.get().and_then(|i| r.drawings.get_mut(i)) {
                            item.width = value;
                        }
                    }
                });
                update();
                layer.queue_draw();
            }
        });
    }
    {
        let session = session.clone();
        let selected = selected.clone();
        let selected_many = selected_many.clone();
        let opacity = opacity.clone();
        let style_sync = style_sync.clone();
        let update = update_history_buttons.clone();
        let layer = layer.clone();
        opacity.connect_value_changed(move |scale| {
            if style_sync.get() {
                return;
            }
            let indices = selected_many.borrow().clone();
            if !indices.is_empty() || selected.get().is_some() {
                let value = (scale.value() / 100.) as f32;
                session.borrow_mut().mutate(|r| {
                    for i in &indices {
                        if let Some(item) = r.drawings.get_mut(*i) {
                            item.opacity = value;
                        }
                    }
                    if indices.is_empty() {
                        if let Some(item) = selected.get().and_then(|i| r.drawings.get_mut(i)) {
                            item.opacity = value;
                        }
                    }
                });
                update();
                layer.queue_draw();
            }
        });
    }

    let display = {
        let preview_dimensions = preview_dimensions.clone();
        let canvas_zoom = canvas_zoom.clone();
        let native_one_to_one = native_one_to_one.clone();
        let scroll = picture_scroll.clone();
        move |w: f64, h: f64| {
            let (pw, ph) = preview_dimensions.get();
            let (cw, ch) = if native_one_to_one.get() {
                (pw as f64, ph as f64)
            } else if canvas_zoom.get() > 0.0 {
                canvas_size_for_zoom(&scroll, (pw, ph), canvas_zoom.get())
            } else {
                (0., 0.)
            };
            overlay_display_rect(
                w,
                h,
                pw as f64,
                ph as f64,
                canvas_zoom.get(),
                native_one_to_one.get(),
                cw,
                ch,
                scroll.hadjustment().value(),
                scroll.vadjustment().value(),
            )
        }
    };
    {
        let layer = layer.clone();
        picture_scroll
            .hadjustment()
            .connect_value_changed(move |_| layer.queue_draw());
    }
    {
        let layer = layer.clone();
        picture_scroll
            .vadjustment()
            .connect_value_changed(move |_| layer.queue_draw());
    }
    {
        let session = session.clone();
        let gesture = gesture.clone();
        let selected = selected.clone();
        let width = width.clone();
        let opacity = opacity.clone();
        let color = color.clone();
        let tools = tools.clone();
        let preview_dimensions = preview_dimensions.clone();
        let layer = layer.clone();
        let selected_many = selected_many.clone();
        let arrowheads = arrowheads.clone();
        let display = display.clone();
        layer.set_draw_func(move |widget, cr, w, h| {
            let d = display(w as f64, h as f64);
            let (pw, ph) = preview_dimensions.get();
            let recipe = session.borrow();
            for (i, item) in recipe.recipe.drawings.iter().enumerate() {
                draw_annotation_on_canvas(cr, item, d, pw, ph);
                if selected_many.borrow().contains(&i) || selected.get() == Some(i) {
                    draw_selection_box(
                        cr,
                        item,
                        d,
                        pw,
                        ph,
                        selected_many.borrow().len() == 1 && selected.get() == Some(i),
                    );
                }
            }
            let state = gesture.borrow();
            if state.selecting_box {
                draw_marquee(cr, d, state.start, state.current);
            } else if should_preview_gesture(tools.get(), &state) {
                let (start_head, end_head) = arrowheads.get();
                let preview = DrawAnnotation {
                    kind: tool_kind(tools.get()),
                    points: if matches!(tools.get(), DrawTool::Pen | DrawTool::Highlighter) {
                        state.points.clone()
                    } else {
                        vec![state.start, state.current]
                    },
                    color: color.get(),
                    width: (width.value() as f32 / 1000.).clamp(0.001, 0.04),
                    opacity: if tools.get() == DrawTool::Highlighter {
                        (opacity.value() / 100.).min(0.35) as f32
                    } else {
                        (opacity.value() / 100.) as f32
                    },
                    arrow_start: start_head,
                    arrow_end: end_head,
                };
                draw_annotation_on_canvas(cr, &preview, d, pw, ph);
            }
            let _ = widget;
        });
    }

    let update_history = update_history_buttons.clone();
    let layer_for_delete = layer.clone();
    let session_delete = session.clone();
    let selected_delete = selected.clone();
    let selected_many_delete = selected_many.clone();
    let delete_button = delete.clone();
    let delete_selected = Rc::new(move || {
        let mut indices = selected_many_delete.borrow().clone();
        if indices.is_empty() {
            indices.extend(selected_delete.take());
        }
        if !indices.is_empty() {
            indices.sort_unstable();
            indices.dedup();
            session_delete.borrow_mut().mutate(|r| {
                for i in indices.iter().rev() {
                    if *i < r.drawings.len() {
                        r.drawings.remove(*i);
                    }
                }
            });
            selected_delete.set(None);
            selected_many_delete.borrow_mut().clear();
            update_history();
            layer_for_delete.queue_draw();
            delete_button.set_sensitive(false);
        }
    });
    {
        let delete_selected = delete_selected.clone();
        delete.connect_clicked(move |_| delete_selected());
    }

    let drag = gtk::GestureDrag::new();
    drag.set_button(1);
    drag.set_propagation_phase(gtk::PropagationPhase::Capture);
    {
        let gesture = gesture.clone();
        let tools = tools.clone();
        let session = session.clone();
        let selected = selected.clone();
        let layer = layer.clone();
        let display = display.clone();
        let preview_dimensions = preview_dimensions.clone();
        let delete = delete.clone();
        let style_sync = style_sync.clone();
        let width = width.clone();
        let opacity = opacity.clone();
        let color = color.clone();
        let selected_many = selected_many.clone();
        drag.connect_drag_begin(move |g, x, y| {
            let d = display(layer.width() as f64, layer.height() as f64);
            let (pw, ph) = preview_dimensions.get();
            let (nx, ny) = screen_to_normalized(d, x, y);
            let p = (nx as f32, ny as f32);
            let tool = tools.get();
            g.set_state(gtk::EventSequenceState::Claimed);
            layer.grab_focus();
            let mut state = DrawGesture {
                start: p,
                current: p,
                points: vec![p],
                additive_selection: g.current_event_state().intersects(
                    gtk::gdk::ModifierType::SHIFT_MASK | gtk::gdk::ModifierType::CONTROL_MASK,
                ),
                ..Default::default()
            };
            if tool == DrawTool::Select {
                let recipe = session.borrow();
                let hit = recipe
                    .recipe
                    .drawings
                    .iter()
                    .enumerate()
                    .rev()
                    .find(|(_, a)| annotation_hit(a, p, pw, ph))
                    .map(|(i, _)| i);
                drop(recipe);
                if let Some(i) = hit {
                    let mut indices = selected_many.borrow_mut();
                    if state.additive_selection {
                        if !indices.contains(&i) {
                            indices.push(i);
                        }
                    } else if !indices.contains(&i) {
                        indices.clear();
                        indices.push(i);
                    }
                    if indices.contains(&i) {
                        selected.set(Some(i));
                        state.selected = Some(i);
                        state.before = {
                            let recipe = session.borrow();
                            indices
                                .iter()
                                .filter_map(|index| {
                                    recipe
                                        .recipe
                                        .drawings
                                        .get(*index)
                                        .cloned()
                                        .map(|item| (*index, item))
                                })
                                .collect()
                        };
                    } else {
                        selected.set(indices.last().copied());
                    }
                    let only_one = indices.len() == 1;
                    drop(indices);
                    delete.set_sensitive(!selected_many.borrow().is_empty());
                    if state.selected.is_some() {
                        let item = session.borrow().recipe.drawings.get(i).cloned();
                        if let Some(item) = item {
                            style_sync.set(true);
                            width.set_value((item.width * 1000.).clamp(1., 36.) as f64);
                            opacity.set_value((item.opacity * 100.).clamp(5., 100.) as f64);
                            color.set(item.color);
                            style_sync.set(false);
                            let (mx, my) = item
                                .points
                                .iter()
                                .fold((0f32, 0f32), |(x, y), p| (x.max(p.0), y.max(p.1)));
                            state.resizing =
                                only_one && (p.0 - mx).abs() < 0.025 && (p.1 - my).abs() < 0.025;
                        }
                        session.borrow_mut().begin_action();
                    }
                } else {
                    if !state.additive_selection {
                        selected_many.borrow_mut().clear();
                        selected.set(None);
                    }
                    state.selecting_box = true;
                    delete.set_sensitive(!selected_many.borrow().is_empty());
                }
            } else if tool == DrawTool::Eraser {
                selected.set(None);
                let old_selection = selected_many.borrow().clone();
                selected_many.borrow_mut().clear();
                delete.set_sensitive(false);
                let recipe = session.borrow();
                let hit = recipe
                    .recipe
                    .drawings
                    .iter()
                    .enumerate()
                    .rev()
                    .find(|(_, a)| annotation_hit(a, p, pw, ph))
                    .map(|(i, _)| i);
                drop(recipe);
                let erase_indices = if hit.is_some_and(|i| old_selection.contains(&i)) {
                    old_selection
                } else {
                    hit.into_iter().collect()
                };
                if !erase_indices.is_empty() {
                    let mut erase_indices = erase_indices;
                    erase_indices.sort_unstable();
                    erase_indices.dedup();
                    session.borrow_mut().begin_action();
                    session.borrow_mut().mutate_active(|r| {
                        for i in erase_indices.iter().rev() {
                            if *i < r.drawings.len() {
                                r.drawings.remove(*i);
                            }
                        }
                    });
                } else {
                    session.borrow_mut().begin_action();
                }
            }
            gesture.replace(state);
            layer.queue_draw();
        });
    }
    {
        let gesture = gesture.clone();
        let tools = tools.clone();
        let layer = layer.clone();
        let display = display.clone();
        let preview_dimensions = preview_dimensions.clone();
        let session = session.clone();
        drag.connect_drag_update(move |_, dx, dy| {
            let d = display(layer.width() as f64, layer.height() as f64);
            let (pw, ph) = preview_dimensions.get();
            let (nx, ny) = screen_to_normalized(
                d,
                gesture.borrow().start.0 as f64 * d.2 + d.0 + dx,
                gesture.borrow().start.1 as f64 * d.3 + d.1 + dy,
            );
            let mut s = gesture.borrow_mut();
            s.current = (nx as f32, ny as f32);
            match tools.get() {
                DrawTool::Pen | DrawTool::Highlighter => {
                    let p = s.current;
                    if s.points
                        .last()
                        .is_none_or(|q| ((q.0 - p.0).powi(2) + (q.1 - p.1).powi(2)).sqrt() > 0.001)
                    {
                        s.points.push(p);
                    }
                }
                DrawTool::Select => {
                    if !s.selecting_box && !s.before.is_empty() {
                        let ox = s.current.0 - s.start.0;
                        let oy = s.current.1 - s.start.1;
                        let mut recipe = session.borrow_mut();
                        for (i, before) in &s.before {
                            if let Some(item) = recipe.recipe.drawings.get_mut(*i) {
                                if s.resizing && Some(*i) == s.selected {
                                    let (minx, maxx, miny, maxy) = before.points.iter().fold(
                                        (1f32, 0f32, 1f32, 0f32),
                                        |(a, b, c, d), p| {
                                            (a.min(p.0), b.max(p.0), c.min(p.1), d.max(p.1))
                                        },
                                    );
                                    let nw = (maxx - minx + ox).max(0.005);
                                    let nh = (maxy - miny + oy).max(0.005);
                                    item.points = before
                                        .points
                                        .iter()
                                        .map(|(x, y)| {
                                            (
                                                minx + (x - minx) * nw / (maxx - minx).max(0.005),
                                                miny + (y - miny) * nh / (maxy - miny).max(0.005),
                                            )
                                        })
                                        .collect();
                                } else {
                                    item.points = before
                                        .points
                                        .iter()
                                        .map(|(x, y)| (x + ox, y + oy))
                                        .collect();
                                }
                            }
                        }
                    }
                }
                DrawTool::Eraser => {
                    let p = s.current;
                    let hit = session
                        .borrow()
                        .recipe
                        .drawings
                        .iter()
                        .enumerate()
                        .rev()
                        .find(|(_, a)| annotation_hit(a, p, pw, ph))
                        .map(|(i, _)| i);
                    if let Some(i) = hit {
                        session.borrow_mut().mutate_active(|r| {
                            if i < r.drawings.len() {
                                r.drawings.remove(i);
                            }
                        });
                    }
                }
                _ => (),
            }
            layer.queue_draw();
        });
    }
    {
        let gesture = gesture.clone();
        let tools = tools.clone();
        let session = session.clone();
        let layer = layer.clone();
        let update = update_history_buttons.clone();
        let selected = selected.clone();
        let selected_many = selected_many.clone();
        let width = width.clone();
        let opacity = opacity.clone();
        let color = color.clone();
        let arrowheads = arrowheads.clone();
        let delete = delete.clone();
        drag.connect_drag_end(move |_, _, _| {
            let s = gesture.borrow().clone();
            let tool = tools.get();
            if is_drawing_tool(tool) {
                let points = if matches!(tool, DrawTool::Pen | DrawTool::Highlighter) {
                    s.points.clone()
                } else {
                    vec![s.start, s.current]
                };
                if points.len() > 1
                    && (s.start.0 - s.current.0).abs() + (s.start.1 - s.current.1).abs() > 0.001
                {
                    let width_norm = (width.value() as f32 / 1000.).clamp(0.001, 0.04);
                    let alpha = (opacity.value() / 100.) as f32;
                    let (arrow_start, arrow_end) = arrowheads.get();
                    let item = DrawAnnotation {
                        kind: tool_kind(tool),
                        points,
                        color: color.get(),
                        width: width_norm,
                        opacity: if tool == DrawTool::Highlighter {
                            alpha.min(0.35)
                        } else {
                            alpha
                        },
                        arrow_start: tool == DrawTool::Arrow && arrow_start,
                        arrow_end: tool == DrawTool::Arrow && arrow_end,
                    };
                    session.borrow_mut().mutate(|r| r.drawings.push(item));
                    selected.set(Some(session.borrow().recipe.drawings.len() - 1));
                    selected_many.borrow_mut().clear();
                    selected_many
                        .borrow_mut()
                        .push(session.borrow().recipe.drawings.len() - 1);
                    delete.set_sensitive(true);
                    update();
                }
            } else if tool == DrawTool::Select && s.selecting_box {
                let min = (s.start.0.min(s.current.0), s.start.1.min(s.current.1));
                let max = (s.start.0.max(s.current.0), s.start.1.max(s.current.1));
                let recipe = session.borrow();
                let found = recipe
                    .recipe
                    .drawings
                    .iter()
                    .enumerate()
                    .filter(|(_, item)| annotation_intersects_rect(item, min, max))
                    .map(|(i, _)| i)
                    .collect::<Vec<_>>();
                drop(recipe);
                let mut indices = selected_many.borrow_mut();
                if !s.additive_selection {
                    indices.clear();
                }
                for i in found {
                    if !indices.contains(&i) {
                        indices.push(i);
                    }
                }
                selected.set(indices.last().copied());
                delete.set_sensitive(!indices.is_empty());
                update();
            } else if session.borrow().action_active() {
                session.borrow_mut().end_action();
                update();
            }
            gesture.replace(DrawGesture::default());
            layer.queue_draw();
        });
    }
    layer.add_controller(drag);
    {
        let gesture = gesture.clone();
        let session = session.clone();
        let update = update_history_buttons.clone();
        let layer = layer.clone();
        let key = gtk::EventControllerKey::new();
        let layer_for_key = layer.clone();
        let draw_toggle_for_key = draw_toggle.clone();
        let delete_selected = delete_selected.clone();
        key.connect_key_pressed(move |_, key, _, _| {
            if (key == gtk::gdk::Key::Delete || key == gtk::gdk::Key::BackSpace)
                && draw_toggle_for_key.is_active()
            {
                delete_selected();
                layer_for_key.queue_draw();
                glib::Propagation::Stop
            } else if key == gtk::gdk::Key::Escape {
                if session.borrow().action_active() {
                    session.borrow_mut().undo();
                    update();
                }
                gesture.replace(DrawGesture::default());
                layer_for_key.queue_draw();
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
        layer.add_controller(key);
    }
    let _ = draw_toggle;
    let _ = queue_preview;
    let layer_for_update = layer.clone();
    Rc::new(move || {
        let active = draw_toggle.is_active();
        layer_for_update.set_visible(true);
        layer_for_update.set_can_target(active);
        if active {
            layer_for_update.set_cursor_from_name(Some(cursor_for_tool(tools.get())));
        } else {
            layer_for_update.set_cursor_from_name(None);
        }
        layer_for_update.queue_draw();
    })
}

fn tool_kind(tool: DrawTool) -> super::model::DrawKind {
    use super::model::DrawKind::*;
    match tool {
        DrawTool::Pen => Pen,
        DrawTool::Line => Line,
        DrawTool::Arrow => Arrow,
        DrawTool::Rectangle => Rectangle,
        DrawTool::Ellipse => Ellipse,
        DrawTool::Highlighter => Highlighter,
        _ => Pen,
    }
}

fn cursor_for_tool(tool: DrawTool) -> &'static str {
    match tool {
        DrawTool::Select => "default",
        DrawTool::Eraser => "cell",
        _ => "crosshair",
    }
}

fn is_drawing_tool(tool: DrawTool) -> bool {
    matches!(
        tool,
        DrawTool::Pen
            | DrawTool::Line
            | DrawTool::Arrow
            | DrawTool::Rectangle
            | DrawTool::Ellipse
            | DrawTool::Highlighter
    )
}

fn should_preview_gesture(tool: DrawTool, gesture: &DrawGesture) -> bool {
    is_drawing_tool(tool)
        && (gesture.points.len() > 1 || gesture.current != gesture.start)
        && !gesture.selecting_box
}

fn draw_marquee(
    cr: &gtk::cairo::Context,
    d: (f64, f64, f64, f64),
    start: (f32, f32),
    end: (f32, f32),
) {
    let (x, y, w, h) = d;
    let left = x + start.0.min(end.0) as f64 * w;
    let top = y + start.1.min(end.1) as f64 * h;
    let width = (end.0 - start.0).abs() as f64 * w;
    let height = (end.1 - start.1).abs() as f64 * h;
    cr.save().ok();
    cr.set_source_rgba(0.2, 0.55, 1., 0.12);
    cr.rectangle(left, top, width, height);
    let _ = cr.fill_preserve();
    cr.set_source_rgba(0.2, 0.55, 1., 0.95);
    cr.set_line_width(1.);
    cr.set_dash(&[4., 3.], 0.);
    let _ = cr.stroke();
    cr.restore().ok();
}

fn annotation_intersects_rect(
    item: &super::model::DrawAnnotation,
    min: (f32, f32),
    max: (f32, f32),
) -> bool {
    if item.points.len() < 2 {
        return false;
    }
    let (minx, maxx, miny, maxy) = item
        .points
        .iter()
        .fold((1f32, 0f32, 1f32, 0f32), |(a, b, c, d), p| {
            (a.min(p.0), b.max(p.0), c.min(p.1), d.max(p.1))
        });
    minx <= max.0 && maxx >= min.0 && miny <= max.1 && maxy >= min.1
}

#[cfg(test)]
mod draw_ui_tests {
    use super::*;

    #[test]
    fn selection_and_eraser_gestures_are_not_draw_previews() {
        let gesture = DrawGesture {
            start: (0.1, 0.2),
            current: (0.7, 0.8),
            points: vec![(0.1, 0.2), (0.7, 0.8)],
            ..Default::default()
        };
        assert!(!should_preview_gesture(DrawTool::Select, &gesture));
        assert!(!should_preview_gesture(DrawTool::Eraser, &gesture));
        assert!(should_preview_gesture(DrawTool::Ellipse, &gesture));
    }

    #[test]
    fn selection_marquee_intersects_annotation_bounds() {
        let annotation = super::super::model::DrawAnnotation {
            kind: super::super::model::DrawKind::Line,
            points: vec![(0.2, 0.3), (0.7, 0.8)],
            color: 0xff0000ff,
            width: 0.01,
            opacity: 1.0,
            arrow_start: false,
            arrow_end: false,
        };
        assert!(annotation_intersects_rect(
            &annotation,
            (0.65, 0.7),
            (0.8, 0.9)
        ));
        assert!(!annotation_intersects_rect(
            &annotation,
            (0.75, 0.85),
            (0.9, 0.95)
        ));
    }
}

fn draw_annotation_on_canvas(
    cr: &gtk::cairo::Context,
    item: &super::model::DrawAnnotation,
    d: (f64, f64, f64, f64),
    pw: i32,
    ph: i32,
) {
    use super::model::DrawKind;
    if item.points.len() < 2 {
        return;
    }
    let (x, y, w, h) = d;
    let (r, g, b, a) = (
        ((item.color >> 24) & 255) as f64 / 255.,
        ((item.color >> 16) & 255) as f64 / 255.,
        ((item.color >> 8) & 255) as f64 / 255.,
        (item.color & 255) as f64 / 255.,
    );
    cr.save().ok();
    cr.set_antialias(gtk::cairo::Antialias::Best);
    cr.set_line_cap(gtk::cairo::LineCap::Round);
    cr.set_line_join(gtk::cairo::LineJoin::Round);
    cr.set_source_rgba(r, g, b, a * item.opacity as f64);
    cr.set_line_width(item.width as f64 * pw.min(ph).max(1) as f64 * w.max(1.) / pw.max(1) as f64);
    let p0 = item.points[0];
    let p1 = *item.points.last().unwrap();
    let ax = x + p0.0 as f64 * w;
    let ay = y + p0.1 as f64 * h;
    let bx = x + p1.0 as f64 * w;
    let by = y + p1.1 as f64 * h;
    match item.kind {
        DrawKind::Pen | DrawKind::Highlighter => {
            let first = item.points[0];
            cr.move_to(x + first.0 as f64 * w, y + first.1 as f64 * h);
            for i in 1..item.points.len() - 1 {
                let (px, py) = item.points[i];
                let (nx, ny) = item.points[i + 1];
                cr.curve_to(
                    x + px as f64 * w,
                    y + py as f64 * h,
                    x + px as f64 * w,
                    y + py as f64 * h,
                    x + (px + nx) as f64 * w / 2.,
                    y + (py + ny) as f64 * h / 2.,
                );
            }
            cr.line_to(x + p1.0 as f64 * w, y + p1.1 as f64 * h);
            let _ = cr.stroke();
        }
        DrawKind::Line | DrawKind::Arrow => {
            cr.move_to(ax, ay);
            cr.line_to(bx, by);
            let _ = cr.stroke();
            if item.kind == DrawKind::Arrow {
                let stroke = item.width as f64 * pw.min(ph).max(1) as f64 * w / pw.max(1) as f64;
                if item.arrow_end {
                    draw_arrow_head(cr, ax, ay, bx, by, stroke);
                }
                if item.arrow_start {
                    draw_arrow_head(cr, bx, by, ax, ay, stroke);
                }
            }
        }
        DrawKind::Rectangle => {
            cr.rectangle(ax, ay, bx - ax, by - ay);
            let _ = cr.stroke();
        }
        DrawKind::Ellipse => {
            cr.save().ok();
            cr.translate((ax + bx) / 2., (ay + by) / 2.);
            cr.scale((bx - ax).abs().max(0.1) / 2., (by - ay).abs().max(0.1) / 2.);
            cr.arc(0., 0., 1., 0., std::f64::consts::TAU);
            cr.restore().ok();
            let _ = cr.stroke();
        }
    }
    cr.restore().ok();
}
fn draw_arrow_head(cr: &gtk::cairo::Context, ax: f64, ay: f64, bx: f64, by: f64, width: f64) {
    let ang = (by - ay).atan2(bx - ax);
    let len = (width * 4.).max(9.);
    cr.move_to(bx, by);
    cr.line_to(bx - len * (ang - 0.55).cos(), by - len * (ang - 0.55).sin());
    cr.move_to(bx, by);
    cr.line_to(bx - len * (ang + 0.55).cos(), by - len * (ang + 0.55).sin());
    let _ = cr.stroke();
}
fn draw_selection_box(
    cr: &gtk::cairo::Context,
    item: &super::model::DrawAnnotation,
    d: (f64, f64, f64, f64),
    _: i32,
    _: i32,
    show_resize_handle: bool,
) {
    let (x, y, w, h) = d;
    let pts = &item.points;
    let (minx, maxx, miny, maxy) = pts
        .iter()
        .fold((1f32, 0f32, 1f32, 0f32), |(a, b, c, d), p| {
            (a.min(p.0), b.max(p.0), c.min(p.1), d.max(p.1))
        });
    cr.save().ok();
    cr.set_source_rgba(0.2, 0.55, 1., 0.95);
    cr.set_line_width(1.);
    cr.set_dash(&[4., 3.], 0.);
    cr.rectangle(
        x + minx as f64 * w,
        y + miny as f64 * h,
        (maxx - minx) as f64 * w,
        (maxy - miny) as f64 * h,
    );
    let _ = cr.stroke();
    cr.set_dash(&[], 0.);
    if show_resize_handle {
        cr.rectangle(x + maxx as f64 * w - 4., y + maxy as f64 * h - 4., 8., 8.);
        cr.set_source_rgba(0.2, 0.55, 1., 1.);
        let _ = cr.fill();
    }
    cr.restore().ok();
}
fn annotation_hit(item: &super::model::DrawAnnotation, p: (f32, f32), _: i32, _: i32) -> bool {
    if item.points.len() < 2 {
        return false;
    }
    let (minx, maxx, miny, maxy) = item
        .points
        .iter()
        .fold((1f32, 0f32, 1f32, 0f32), |(a, b, c, d), p| {
            (a.min(p.0), b.max(p.0), c.min(p.1), d.max(p.1))
        });
    p.0 >= minx - 0.02 && p.0 <= maxx + 0.02 && p.1 >= miny - 0.02 && p.1 <= maxy + 0.02
}
