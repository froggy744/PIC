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

#[derive(Default)]
struct DrawGesture {
    start: (f32, f32),
    current: (f32, f32),
    points: Vec<(f32, f32)>,
    before: Option<super::model::DrawAnnotation>,
    selected: Option<usize>,
    resizing: bool,
}

fn draw_tool_button(label: &str, tooltip: &str, first: &gtk::ToggleButton) -> gtk::ToggleButton {
    let button = gtk::ToggleButton::with_label(label);
    button.set_group(Some(first));
    button.set_tooltip_text(Some(tooltip));
    button.add_css_class("flat");
    button
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
    let gesture = Rc::new(RefCell::new(DrawGesture::default()));
    let tool_row = gtk::Box::new(gtk::Orientation::Horizontal, 2);
    tool_row.add_css_class("linked");
    let tool_row_second = gtk::Box::new(gtk::Orientation::Horizontal, 2);
    tool_row_second.add_css_class("linked");
    let select = gtk::ToggleButton::with_label("Select");
    select.set_active(false);
    select.set_tooltip_text(Some("Select, move, resize, or delete annotations"));
    select.add_css_class("flat");
    let pen = draw_tool_button("Pen", "Freehand pen", &select);
    pen.set_active(true);
    let line = draw_tool_button("Line", "Straight line", &select);
    let arrow = draw_tool_button("Arrow", "Arrow", &select);
    let rect = draw_tool_button("▱", "Rectangle", &select);
    let ellipse = draw_tool_button("◯", "Ellipse", &select);
    let highlighter = draw_tool_button("Highlight", "Transparent highlighter", &select);
    let eraser = draw_tool_button("Erase", "Erase annotations by dragging", &select);
    for b in [&select, &pen, &line, &arrow] {
        tool_row.append(b);
    }
    parent.append(&tool_row);
    for b in [&rect, &ellipse, &highlighter, &eraser] {
        tool_row_second.append(b);
    }
    parent.append(&tool_row_second);
    let arrow_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let arrow_start = gtk::CheckButton::with_label("Start head");
    let arrow_end = gtk::CheckButton::with_label("End head");
    arrow_end.set_active(true);
    arrow_row.append(&arrow_start);
    arrow_row.append(&arrow_end);
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

    let swatches = gtk::Box::new(gtk::Orientation::Horizontal, 5);
    swatches.set_margin_top(8);
    let mut color_buttons = Vec::new();
    for (name, color) in [
        ("Black", 0x17191fff),
        ("White", 0xffffffff),
        ("Red", 0xf04438ff),
        ("Orange", 0xf79009ff),
        ("Yellow", 0xfec84bff),
        ("Green", 0x12b76aff),
        ("Blue", 0x2e90faff),
        ("Purple", 0x875bf7ff),
    ] {
        let b = gtk::Button::new();
        b.set_size_request(24, 24);
        b.add_css_class("draw-color-swatch");
        b.set_tooltip_text(Some(name));
        let area = gtk::DrawingArea::new();
        area.set_content_width(16);
        area.set_content_height(16);
        area.set_draw_func(move |_, cr, w, h| {
            let c = color;
            cr.set_source_rgba(
                ((c >> 24) & 255) as f64 / 255.,
                ((c >> 16) & 255) as f64 / 255.,
                ((c >> 8) & 255) as f64 / 255.,
                1.,
            );
            cr.arc(w as f64 / 2., h as f64 / 2., 7., 0., std::f64::consts::TAU);
            let _ = cr.fill();
        });
        b.set_child(Some(&area));
        swatches.append(&b);
        color_buttons.push((b, color));
    }
    let custom = gtk::ColorDialogButton::new(Some(gtk::ColorDialog::new()));
    custom.set_tooltip_text(Some("Choose custom annotation colour"));
    swatches.append(&custom);
    parent.append(&swatches);

    let width = gtk::Scale::with_range(gtk::Orientation::Horizontal, 1.0, 36.0, 1.0);
    width.set_value(5.0);
    width.set_draw_value(true);
    width.set_hexpand(true);
    width.set_tooltip_text(Some("Stroke thickness"));
    parent.append(&gtk::Label::new(Some("Thickness")));
    parent.append(&width);
    let opacity = gtk::Scale::with_range(gtk::Orientation::Horizontal, 5.0, 100.0, 5.0);
    opacity.set_value(100.0);
    opacity.set_draw_value(true);
    opacity.set_hexpand(true);
    opacity.set_tooltip_text(Some("Stroke opacity"));
    parent.append(&gtk::Label::new(Some("Opacity")));
    parent.append(&opacity);
    let delete = gtk::Button::with_label("Delete selected");
    delete.set_sensitive(false);
    parent.append(&delete);
    let helper = gtk::Label::new(Some(
        "Drag on the photo to annotate. Escape cancels a gesture.",
    ));
    helper.set_wrap(true);
    helper.add_css_class("dim-label");
    parent.append(&helper);

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
        b.connect_toggled(move |button| {
            if button.is_active() {
                tools.set(tool);
                layer.set_cursor_from_name(Some(match tool {
                    DrawTool::Select => "default",
                    DrawTool::Eraser => "not-allowed",
                    _ => "crosshair",
                }));
                layer.queue_draw();
            }
        });
    }
    let color = Rc::new(Cell::new(0xf04438ffu32));
    let style_sync = Rc::new(Cell::new(false));
    let apply_selected_color = Rc::new({
        let session = session.clone();
        let selected = selected.clone();
        let update = update_history_buttons.clone();
        let layer = layer.clone();
        move |rgba: u32| {
            if let Some(i) = selected.get() {
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
        let color = color.clone();
        let apply_selected_color = apply_selected_color.clone();
        custom.connect_rgba_notify(move |button| {
            let c = button.rgba();
            let q = |v: f32| (v.clamp(0., 1.) * 255.).round() as u32;
            let rgba =
                (q(c.red()) << 24) | (q(c.green()) << 16) | (q(c.blue()) << 8) | (q(c.alpha()));
            color.set(rgba);
            apply_selected_color(rgba);
        });
    }
    {
        let session = session.clone();
        let selected = selected.clone();
        let width = width.clone();
        let style_sync = style_sync.clone();
        let update = update_history_buttons.clone();
        let layer = layer.clone();
        width.connect_value_changed(move |scale| {
            if style_sync.get() {
                return;
            }
            if let Some(i) = selected.get() {
                let value = (scale.value() as f32 / 1000.).clamp(0.001, 0.04);
                session.borrow_mut().mutate(|r| {
                    if let Some(item) = r.drawings.get_mut(i) {
                        item.width = value;
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
        let opacity = opacity.clone();
        let style_sync = style_sync.clone();
        let update = update_history_buttons.clone();
        let layer = layer.clone();
        opacity.connect_value_changed(move |scale| {
            if style_sync.get() {
                return;
            }
            if let Some(i) = selected.get() {
                let value = (scale.value() / 100.) as f32;
                session.borrow_mut().mutate(|r| {
                    if let Some(item) = r.drawings.get_mut(i) {
                        item.opacity = value;
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
        let arrowheads = arrowheads.clone();
        let display = display.clone();
        layer.set_draw_func(move |widget, cr, w, h| {
            let d = display(w as f64, h as f64);
            let (pw, ph) = preview_dimensions.get();
            let recipe = session.borrow();
            for (i, item) in recipe.recipe.drawings.iter().enumerate() {
                draw_annotation_on_canvas(cr, item, d, pw, ph);
                if selected.get() == Some(i) {
                    draw_selection_box(cr, item, d, pw, ph);
                }
            }
            let state = gesture.borrow();
            if state.points.len() > 1 || state.current != state.start {
                let (start_head, end_head) = arrowheads.get();
                let preview = DrawAnnotation {
                    kind: tool_kind(tools.get()),
                    points: if tools.get() == DrawTool::Pen || tools.get() == DrawTool::Highlighter
                    {
                        state.points.clone()
                    } else {
                        vec![state.start, state.current]
                    },
                    color: color.get(),
                    width: (width.value() as f32 / 1000.).clamp(0.001, 0.04),
                    opacity: (opacity.value() / 100.) as f32,
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
    let delete_button = delete.clone();
    delete.connect_clicked(move |_| {
        if let Some(i) = selected_delete.take() {
            session_delete.borrow_mut().mutate(|r| {
                if i < r.drawings.len() {
                    r.drawings.remove(i);
                }
            });
            update_history();
            layer_for_delete.queue_draw();
            delete_button.set_sensitive(false);
        }
    });

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
                selected.set(hit);
                delete.set_sensitive(hit.is_some());
                if let Some(i) = hit {
                    state.selected = Some(i);
                    state.before = session.borrow().recipe.drawings.get(i).cloned();
                    if let Some(item) = &state.before {
                        style_sync.set(true);
                        width.set_value((item.width * 1000.).clamp(1., 36.) as f64);
                        opacity.set_value((item.opacity * 100.).clamp(5., 100.) as f64);
                        color.set(item.color);
                        style_sync.set(false);
                        let (mx, my) = item
                            .points
                            .iter()
                            .fold((0f32, 0f32), |(x, y), p| (x.max(p.0), y.max(p.1)));
                        state.resizing = (p.0 - mx).abs() < 0.025 && (p.1 - my).abs() < 0.025;
                    }
                    session.borrow_mut().begin_action();
                }
            } else if tool == DrawTool::Eraser {
                selected.set(None);
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
                if let Some(i) = hit {
                    state.selected = Some(i);
                    session.borrow_mut().begin_action();
                    session.borrow_mut().mutate_active(|r| {
                        if i < r.drawings.len() {
                            r.drawings.remove(i);
                        }
                    });
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
                    if let Some(i) = s.selected {
                        if let Some(before) = &s.before {
                            let ox = s.current.0 - s.start.0;
                            let oy = s.current.1 - s.start.1;
                            if let Some(item) = session.borrow_mut().recipe.drawings.get_mut(i) {
                                if s.resizing {
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
        let width = width.clone();
        let opacity = opacity.clone();
        let color = color.clone();
        let arrowheads = arrowheads.clone();
        let delete = delete.clone();
        drag.connect_drag_end(move |_, _, _| {
            let s = gesture.borrow_mut();
            let tool = tools.get();
            if matches!(
                tool,
                DrawTool::Pen
                    | DrawTool::Line
                    | DrawTool::Arrow
                    | DrawTool::Rectangle
                    | DrawTool::Ellipse
                    | DrawTool::Highlighter
            ) {
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
                    delete.set_sensitive(true);
                    update();
                }
            } else if session.borrow().action_active() {
                session.borrow_mut().end_action();
                update();
            }
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
        key.connect_key_pressed(move |_, key, _, _| {
            if key == gtk::gdk::Key::Escape {
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
            layer_for_update.set_cursor_from_name(Some("crosshair"));
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
    cr.rectangle(x + maxx as f64 * w - 4., y + maxy as f64 * h - 4., 8., 8.);
    cr.set_source_rgba(0.2, 0.55, 1., 1.);
    let _ = cr.fill();
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
