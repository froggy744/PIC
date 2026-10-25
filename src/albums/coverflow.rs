// Albums Cover Flow adapter.
use gtk::subclass::prelude::*;
use gtk::{gdk, graphene, gsk};
use std::ops::Range;
pub(crate) struct AlbumFlowCanvas { gl: crate::coverflow::CoverflowGlArea }
impl AlbumFlowCanvas {
    fn new() -> Self {
        let gl = crate::coverflow::CoverflowGlArea::new("albums-coverflow-canvas", 430);
        gl.widget().set_can_target(true);
        gl.widget().set_focusable(true);
        Self { gl }
    }
    fn widget(&self) -> &gtk::GLArea { self.gl.widget() }
    fn set_scene(&self, count: usize, focus: f64, scene: Vec<(usize, gsk::RenderNode)>) {
        let items = scene.into_iter().map(|(index, node)| crate::coverflow::CoverflowItem {
            index, revision: index as u64, node,
        }).collect();
        self.gl.set_scene(count, focus, items, true);
    }
}

struct AlbumFlowState {
    albums: Vec<Album>,
    connection: Rc<RefCell<Connection>>,
    frames: Vec<FrameAsset>,
    appearance: AlbumAppearance,
    fit_whole_photo: bool,
    selected: usize,
    cached: HashMap<usize, gsk::RenderNode>,
    focused_album_id: Rc<Cell<Option<i64>>>,
    focused_index: Rc<Cell<usize>>,
}

fn paint_photo_inside(
    snapshot: &gtk::Snapshot,
    paintable: &gdk::Paintable,
    rect: graphene::Rect,
    fit_whole: bool,
) {
    let iw = paintable.intrinsic_width().max(1) as f32;
    let ih = paintable.intrinsic_height().max(1) as f32;
    let scale = if fit_whole {
        (rect.width() / iw).min(rect.height() / ih)
    } else {
        (rect.width() / iw).max(rect.height() / ih)
    };
    let width = iw * scale;
    let height = ih * scale;
    snapshot.save();
    snapshot.push_clip(&rect);
    snapshot.translate(&graphene::Point::new(
        rect.x() + (rect.width() - width) * 0.5,
        rect.y() + (rect.height() - height) * 0.5,
    ));
    paintable.snapshot(snapshot, width as f64, height as f64);
    snapshot.pop();
    snapshot.restore();
}

fn album_flow_node(state: &AlbumFlowState, album: &Album) -> gsk::RenderNode {
    let snapshot = gtk::Snapshot::new();
    let full = graphene::Rect::new(0.0, 0.0, 300.0, 300.0);
    snapshot.append_color(&gdk::RGBA::new(0.14, 0.15, 0.18, 1.0), &full);
    let root = crate::css::resolve_runtime_dir(Path::new(ALBUM_COVER_THEME_DIRECTORY));
    let frame_paths: Vec<_> = state
        .frames
        .iter()
        .map(|frame| frame.path.clone())
        .collect();
    let frame = selected_frame_index(album, &root, &frame_paths, state.appearance.cover_index)
        .and_then(|index| state.frames.get(index));
    let frame_rect = if let Some(frame) = frame {
        let ratio = frame.texture.width().max(1) as f32 / frame.texture.height().max(1) as f32;
        let height = (300.0 / ratio).min(300.0);
        graphene::Rect::new(0.0, (300.0 - height) * 0.5, 300.0, height)
    } else {
        full
    };
    let photo_rect = if let Some(frame) = frame {
        let o = frame.opening;
        graphene::Rect::new(
            frame_rect.x() + frame_rect.width() * o.x as f32,
            frame_rect.y() + frame_rect.height() * o.y as f32,
            frame_rect.width() * o.width as f32,
            frame_rect.height() * o.height as f32,
        )
    } else {
        full
    };
    let photo = {
        let connection = state.connection.borrow();
        let photos = db::browse_photos_in_album(&connection, album.id, None).unwrap_or_default();
        album_cover_photo(&connection, album, &photos)
    };
    if let Some(photo) = photo {
        let cache =
            crate::thumbnail::existing_cache_path(&photo.path, photo.mtime, photo.size_bytes)
                .ok()
                .flatten();
        if let Some(path) = cache {
            let path_text = path.to_string_lossy();
            let paintable = crate::photo_texture::edited_thumbnail(
                &path_text,
                photo.rotation,
                &photo.edit_recipe,
            )
            .or_else(|| {
                gdk::Texture::from_filename(&path)
                    .ok()
                    .map(|texture| texture.upcast())
            });
            if let Some(paintable) = paintable {
                paint_photo_inside(&snapshot, &paintable, photo_rect, state.fit_whole_photo);
            }
        }
    }
    if let Some(frame) = frame {
        snapshot.append_scaled_texture(&frame.texture, gsk::ScalingFilter::Trilinear, &frame_rect);
    }
    snapshot.to_node().expect("album cover node")
}

fn show_album_focus(
    state: &Rc<RefCell<AlbumFlowState>>,
    canvas: &AlbumFlowCanvas,
    name: &gtk::Label,
    count: &gtk::Label,
    index: usize,
) {
    let mut state = state.borrow_mut();
    if state.albums.is_empty() {
        return;
    }
    state.selected = index.min(state.albums.len() - 1);
    let selected = &state.albums[state.selected];
    state.focused_album_id.set(Some(selected.id));
    state.focused_index.set(state.selected);
    name.set_text(&selected.name);
    count.set_text(&format!(
        "{} photo{}",
        selected.photo_count,
        if selected.photo_count == 1 { "" } else { "s" }
    ));
    canvas.widget().update_property(&[gtk::accessible::Property::Label(&format!(
        "Album {} of {}: {}. {} photos. Arrow keys browse, Enter opens.",
        state.selected + 1,
        state.albums.len(),
        selected.name,
        selected.photo_count,
    ))]);
    let visible = visible_album_indices(state.selected, state.albums.len());
    state.cached.retain(|index, _| visible.contains(index));
    for index in visible.clone() {
        if !state.cached.contains_key(&index) {
            let node = album_flow_node(&state, &state.albums[index]);
            state.cached.insert(index, node);
        }
    }
    let scene = visible
        .map(|index| (index, state.cached[&index].clone()))
        .collect();
    canvas.set_scene(state.albums.len(), state.selected as f64, scene);
}

fn build_album_coverflow(
    albums: &[Album],
    connection: Rc<RefCell<Connection>>,
    frames: &[FrameAsset],
    appearance: AlbumAppearance,
    fit_whole_photo: bool,
    focus: usize,
    focused_album_id: Rc<Cell<Option<i64>>>,
    focused_index: Rc<Cell<usize>>,
    on_album: Rc<dyn Fn(i64)>,
) -> gtk::Box {
    let root = gtk::Box::new(gtk::Orientation::Vertical, 8);
    root.add_css_class("albums-coverflow");
    root.set_hexpand(true);
    let canvas = Rc::new(AlbumFlowCanvas::new());
    crate::settings::CornerStyle::saved(&connection.borrow()).apply(canvas.widget());
    root.append(canvas.widget());
    let name = gtk::Label::new(None);
    name.add_css_class("albums-coverflow-name");
    name.set_ellipsize(gtk::pango::EllipsizeMode::End);
    root.append(&name);
    let count = gtk::Label::new(None);
    count.add_css_class("albums-coverflow-count");
    root.append(&count);
    let open = gtk::Button::with_label("Open Album");
    open.set_halign(gtk::Align::Center);
    open.add_css_class("albums-coverflow-open");
    root.append(&open);

    if albums.is_empty() {
        name.set_text("Your albums start here");
        count.set_text("Choose New Album to make a collection.");
        canvas.widget().set_visible(false);
        open.set_visible(false);
        return root;
    }
    let state = Rc::new(RefCell::new(AlbumFlowState {
        albums: albums.to_vec(),
        connection,
        frames: frames.to_vec(),
        appearance,
        fit_whole_photo,
        selected: focus,
        cached: HashMap::new(),
        focused_album_id,
        focused_index,
    }));
    show_album_focus(&state, &canvas, &name, &count, focus);
    let state_for_open = state.clone();
    let on_open = on_album.clone();
    open.connect_clicked(move |_| {
        let state = state_for_open.borrow();
        on_open(state.albums[state.selected].id);
    });

    let keys = gtk::EventControllerKey::new();
    let state_for_keys = state.clone();
    let canvas_for_keys = Rc::downgrade(&canvas);
    let name_for_keys = name.downgrade();
    let count_for_keys = count.downgrade();
    let on_keys = on_album.clone();
    keys.connect_key_pressed(move |_, key, _, _| {
        let current = state_for_keys.borrow().selected;
        let len = state_for_keys.borrow().albums.len();
        let next = match key {
            gdk::Key::Left | gdk::Key::KP_Left => Some(current.saturating_sub(1)),
            gdk::Key::Right | gdk::Key::KP_Right => Some((current + 1).min(len - 1)),
            gdk::Key::Home => Some(0),
            gdk::Key::End => Some(len - 1),
            gdk::Key::Return | gdk::Key::KP_Enter => {
                on_keys(state_for_keys.borrow().albums[current].id);
                return gtk::glib::Propagation::Stop;
            }
            _ => None,
        };
        if let Some(next) = next {
            if let (Some(canvas), Some(name), Some(count)) = (
                canvas_for_keys.upgrade(),
                name_for_keys.upgrade(),
                count_for_keys.upgrade(),
            ) {
                show_album_focus(&state_for_keys, &canvas, &name, &count, next);
            }
            gtk::glib::Propagation::Stop
        } else {
            gtk::glib::Propagation::Proceed
        }
    });
    canvas.widget().add_controller(keys);

    let scroll = gtk::EventControllerScroll::new(
        gtk::EventControllerScrollFlags::VERTICAL | gtk::EventControllerScrollFlags::HORIZONTAL,
    );
    scroll.set_propagation_phase(gtk::PropagationPhase::Capture);
    let state_for_scroll = state.clone();
    let canvas_for_scroll = canvas.clone();
    let name_for_scroll = name.downgrade();
    let count_for_scroll = count.downgrade();
    let accumulator = Rc::new(RefCell::new(coverflow_input::ScrollNavigation::default()));
    let begin = accumulator.clone();
    scroll.connect_scroll_begin(move |_| begin.borrow_mut().reset());
    let end = accumulator.clone();
    scroll.connect_scroll_end(move |_| end.borrow_mut().reset());
    scroll.connect_scroll(move |controller, dx, dy| {
        let current = state_for_scroll.borrow().selected;
        let len = state_for_scroll.borrow().albums.len();
        let delta = if dy.abs() >= dx.abs() { dy } else { dx };
        let step = accumulator.borrow_mut().step(delta,
            controller.unit() == gdk::ScrollUnit::Surface,
            controller.current_event_time());
        let next = if step > 0 {
            (current + 1).min(len - 1)
        } else if step < 0 {
            current.saturating_sub(1)
        } else {
            current
        };
        if std::env::var_os("PIC_COVERFLOW_TRACE").is_some() {
            eprintln!("PIC_COVERFLOW input unit={:?} dx={dx:.3} dy={dy:.3} step={step} focus={current}->{next}", controller.unit());
        }
        if next == current { return gtk::glib::Propagation::Stop; }
        if let (Some(name), Some(count)) = (
            name_for_scroll.upgrade(),
            count_for_scroll.upgrade(),
        ) {
            show_album_focus(&state_for_scroll, &canvas_for_scroll, &name, &count, next);
        }
        gtk::glib::Propagation::Stop
    });
    root.add_controller(scroll);

    let click = gtk::GestureClick::new();
    let state_for_click = state.clone();
    let canvas_for_click = Rc::downgrade(&canvas);
    let name_for_click = name.downgrade();
    let count_for_click = count.downgrade();
    click.connect_pressed(move |_, _, x, y| {
        let Some(canvas) = canvas_for_click.upgrade() else {
            return;
        };
        canvas.widget().grab_focus();
        let state = state_for_click.borrow();
        let viewport = (canvas.widget().width() as f64, canvas.widget().height() as f64);
        let nearest = visible_album_indices(state.selected, state.albums.len())
            .filter_map(|index| {
                crate::coverflow::album_coverflow_pose(
                    index,
                    state.selected as f64,
                    state.albums.len(),
                    viewport,
                )
                .map(|pose| (index, pose))
            })
            .filter(|(_, pose)| y >= pose.y && y <= pose.y + pose.height)
            .min_by(|(_, a), (_, b)| {
                (x - a.x - a.width * 0.5)
                    .abs()
                    .total_cmp(&(x - b.x - b.width * 0.5).abs())
            })
            .map(|(index, _)| index);
        let current = state.selected;
        drop(state);
        if let Some(index) = nearest {
            if index == current {
                on_album(state_for_click.borrow().albums[index].id);
            } else if let (Some(name), Some(count)) =
                (name_for_click.upgrade(), count_for_click.upgrade())
            {
                show_album_focus(&state_for_click, &canvas, &name, &count, index);
            }
        }
    });
    canvas.widget().add_controller(click);
    root
}

fn visible_album_indices(focus: usize, count: usize) -> Range<usize> {
    if count == 0 {
        return 0..0;
    }
    let focus = focus.min(count - 1);
    focus.saturating_sub(crate::coverflow::VISIBLE_RADIUS)
        ..focus
            .saturating_add(crate::coverflow::VISIBLE_RADIUS + 1)
            .min(count)
}

fn restored_album_focus(albums: &[Album], album_id: Option<i64>, old_index: usize) -> usize {
    if albums.is_empty() {
        return 0;
    }
    album_id
        .and_then(|id| albums.iter().position(|album| album.id == id))
        .unwrap_or(old_index.min(albums.len() - 1))
}
