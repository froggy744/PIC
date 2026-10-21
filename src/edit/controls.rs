#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FilterTileEffect {
    Preset(FilterPreset),
    BlackWhite,
    Sepia,
}

impl FilterTileEffect {
    fn all() -> Vec<Self> {
        std::iter::once(Self::Preset(FilterPreset::None))
            .chain(FilterPreset::ALL.map(Self::Preset))
            .chain([Self::BlackWhite, Self::Sepia])
            .collect()
    }

    fn label(self) -> &'static str {
        match self {
            Self::Preset(FilterPreset::None) => "Original",
            Self::Preset(preset) => preset.label(),
            Self::BlackWhite => "B&W",
            Self::Sepia => "Sepia",
        }
    }

    fn recipe(self, base: &EditRecipe) -> EditRecipe {
        let mut recipe = base.clone();
        // The picker is a catalogue of recognisable full-strength looks.
        // Otherwise setting Amount to 0% makes every preview tile look
        // identical, even though choosing a filter remains possible.
        recipe.filter_amount = 1.0;
        recipe.filter = FilterPreset::None;
        recipe.black_white = false;
        recipe.sepia = false;
        match self {
            Self::Preset(preset) => recipe.filter = preset,
            Self::BlackWhite => recipe.black_white = true,
            Self::Sepia => recipe.sepia = true,
        }
        recipe
    }
}

type FilterThumbnailPixels = (u32, u32, Vec<u8>);

fn render_filter_thumbnails(
    path: &str,
    rotation: i32,
    recipe: &EditRecipe,
) -> anyhow::Result<Vec<FilterThumbnailPixels>> {
    // Decode at twice the square on-screen preview size. The widget's fixed
    // size_request (FILTER_TILE_PREVIEW_*) owns the tile's natural size, so the
    // larger decode cannot inflate the grid; ContentFit::Cover then minifies
    // the texture ~2:1 with the GPU's mipmapped filtering.
    let base = super::render::decode_base_for_viewer(
        path,
        rotation,
        (FILTER_TILE_PREVIEW_WIDTH.max(1) as u32) * FILTER_TILE_SUPERSAMPLE,
        (FILTER_TILE_PREVIEW_HEIGHT.max(1) as u32) * FILTER_TILE_SUPERSAMPLE,
    )?;
    Ok(FilterTileEffect::all()
        .into_iter()
        .map(|effect| {
            let image = super::render::apply_recipe(base.clone(), &effect.recipe(recipe));
            let image = cover_crop_to_tile_size(image);
            (image.width(), image.height(), image.into_raw())
        })
        .collect())
}

// Center-crop to the tile preview's aspect ratio and downscale to its exact
// size — the same aspect-fill result ContentFit::Cover produces on the GPU,
// but resampled in one averaging step from a supersampled source instead of
// being displayed from a 1x texture.
fn cover_crop_to_tile_size(image: image::RgbaImage) -> image::RgbaImage {
    use image::imageops::FilterType;

    let target_ratio = FILTER_TILE_PREVIEW_WIDTH as f64 / FILTER_TILE_PREVIEW_HEIGHT as f64;
    let ratio = image.width() as f64 / image.height() as f64;
    let cropped = if ratio > target_ratio {
        let new_width =
            ((image.height() as f64 * target_ratio).round() as u32).clamp(1, image.width());
        let x = (image.width() - new_width) / 2;
        image::imageops::crop_imm(&image, x, 0, new_width, image.height()).to_image()
    } else {
        let new_height =
            ((image.width() as f64 / target_ratio).round() as u32).clamp(1, image.height());
        let y = (image.height() - new_height) / 2;
        image::imageops::crop_imm(&image, 0, y, image.width(), new_height).to_image()
    };
    image::imageops::resize(
        &cropped,
        FILTER_TILE_PREVIEW_WIDTH.max(1) as u32,
        FILTER_TILE_PREVIEW_HEIGHT.max(1) as u32,
        FilterType::Triangle,
    )
}

#[derive(Clone)]
struct Controls {
    straighten: gtk::Scale,
    exposure: gtk::Scale,
    contrast: gtk::Scale,
    fill_light: gtk::Scale,
    highlights: gtk::Scale,
    shadows: gtk::Scale,
    whites: gtk::Scale,
    blacks: gtk::Scale,
    temperature: gtk::Scale,
    tint: gtk::Scale,
    saturation: gtk::Scale,
    vibrance: gtk::Scale,
    sharpen: gtk::Scale,
    clarity: gtk::Scale,
    smooth: gtk::Scale,
    grain: gtk::Scale,
    vignette: gtk::Scale,
    auto_contrast: gtk::ToggleButton,
    auto_color: gtk::ToggleButton,
    filter_amount: gtk::Scale,
    black_white: gtk::ToggleButton,
    sepia: gtk::ToggleButton,
    filters: Vec<(FilterPreset, gtk::ToggleButton)>,
}

impl Controls {
    fn sync(&self, recipe: &EditRecipe) {
        self.straighten.set_value(recipe.straighten as f64);
        self.exposure.set_value(recipe.exposure as f64);
        self.contrast.set_value(recipe.contrast as f64);
        self.fill_light.set_value(recipe.fill_light as f64);
        self.highlights.set_value(recipe.highlights as f64);
        self.shadows.set_value(recipe.shadows as f64);
        self.whites.set_value(recipe.whites as f64);
        self.blacks.set_value(recipe.blacks as f64);
        self.temperature.set_value(recipe.temperature as f64);
        self.tint.set_value(recipe.tint as f64);
        self.saturation.set_value(recipe.saturation as f64);
        self.vibrance.set_value(recipe.vibrance as f64);
        self.sharpen.set_value(recipe.sharpen as f64);
        self.clarity.set_value(recipe.clarity as f64);
        self.smooth.set_value(recipe.smooth as f64);
        self.grain.set_value(recipe.grain as f64);
        self.vignette.set_value(recipe.vignette as f64);
        self.auto_contrast.set_active(recipe.auto_contrast);
        self.auto_color.set_active(recipe.auto_color);
        self.filter_amount.set_value((recipe.filter_amount * 100.0) as f64);
        self.filter_amount.set_sensitive(
            recipe.filter != FilterPreset::None || recipe.black_white || recipe.sepia
        );
        self.black_white.set_active(recipe.black_white);
        self.sepia.set_active(recipe.sepia);
        for (preset, button) in &self.filters {
            button.set_active(recipe.filter == *preset);
        }
    }
}

fn add_section_label(parent: &gtk::Box, text: &str) {
    let label = gtk::Label::new(Some(text));
    label.set_xalign(0.0);
    label.set_hexpand(true);
    label.set_margin_top(8);
    label.add_css_class("heading");
    label.add_css_class("edit-section-label");
    parent.append(&label);
}

const FILTER_TILE_WIDTH: i32 = 88;
#[cfg(test)]
const FILTER_TILE_MIN_WIDTH: i32 = FILTER_TILE_WIDTH;
const FILTER_TILE_PREVIEW_WIDTH: i32 = FILTER_TILE_WIDTH - 4;
const FILTER_TILE_PREVIEW_HEIGHT: i32 = FILTER_TILE_PREVIEW_WIDTH;
// Tile previews decode at this multiple of their on-screen size and are then
// CPU-downscaled to the exact tile box; see render_filter_thumbnails.
const FILTER_TILE_SUPERSAMPLE: u32 = 2;

/// CPU thumbnail renderer is used only in CPU mode or when the GPU tile
/// renderer cannot initialize. In GPU mode normal tiles use shader GLAreas.
fn queue_cpu_filter_thumbnails(
    path: String,
    rotation: i32,
    recipe: EditRecipe,
    pictures: Vec<gtk::Picture>,
) {
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = sender.send(render_filter_thumbnails(&path, rotation, &recipe));
    });
    glib::timeout_add_local(Duration::from_millis(25), move || {
        match receiver.try_recv() {
            Ok(Ok(thumbnails)) => {
                for (picture, (width, height, pixels)) in pictures.iter().zip(thumbnails) {
                    let bytes = glib::Bytes::from_owned(pixels);
                    let texture = gtk::gdk::MemoryTexture::new(
                        width as i32, height as i32,
                        gtk::gdk::MemoryFormat::R8g8b8a8,
                        &bytes, width as usize * 4,
                    );
                    picture.set_paintable(Some(&texture));
                }
                glib::ControlFlow::Break
            }
            Ok(Err(error)) => {
                eprintln!("PIC_EDIT_FILTERS CPU thumbnail error: {error}");
                glib::ControlFlow::Break
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
            Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
        }
    });
}

fn filter_tile(
    effect: FilterTileEffect,
    gpu_thumbnail: bool,
) -> (gtk::ToggleButton, gtk::Picture, Option<LiveGpuCanvas>) {
    let button = gtk::ToggleButton::new();
    button.add_css_class("filter-tile");
    button.set_tooltip_text(Some(effect.label()));
    button.set_size_request(FILTER_TILE_WIDTH, -1);
    button.set_hexpand(false);
    button.set_halign(gtk::Align::Start);
    button.set_valign(gtk::Align::Start);

    let content = gtk::Box::new(gtk::Orientation::Vertical, 5);
    content.set_hexpand(false);
    content.set_halign(gtk::Align::Fill);

    // A square, compact preview keeps filter tiles from forcing the sidebar
    // wider than the editing controls need.
    let picture = gtk::Picture::new();
    picture.set_content_fit(gtk::ContentFit::Cover);
    picture.set_can_shrink(true);
    picture.set_hexpand(true);
    picture.set_vexpand(true);
    picture.set_halign(gtk::Align::Fill);
    picture.set_valign(gtk::Align::Fill);
    picture.set_size_request(FILTER_TILE_PREVIEW_WIDTH, FILTER_TILE_PREVIEW_HEIGHT);
    picture.add_css_class("filter-tile-preview");
    picture.set_overflow(gtk::Overflow::Hidden);

    // Same OpenGL effect shaders as live Edit; the Picture stays available
    // underneath for the CPU renderer or an OpenGL initialization failure.
    let gpu_canvas = if gpu_thumbnail {
        let stage = gtk::Overlay::new();
        stage.set_size_request(FILTER_TILE_PREVIEW_WIDTH, FILTER_TILE_PREVIEW_HEIGHT);
        // Keep the GL framebuffer exactly the viewport passed to
        // set_thumbnail_cover; FlowBox may allocate extra width to the button.
        stage.set_hexpand(false);
        stage.set_vexpand(false);
        stage.set_halign(gtk::Align::Center);
        stage.set_valign(gtk::Align::Start);
        stage.set_child(Some(&picture));
        let canvas = LiveGpuCanvas::new();
        stage.add_overlay(&canvas.widget());
        content.append(&stage);
        Some(canvas)
    } else {
        content.append(&picture);
        None
    };

    let label = gtk::Label::new(Some(effect.label()));
    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    label.set_justify(gtk::Justification::Center);
    label.set_max_width_chars(18);
    label.add_css_class("filter-tile-label");
    content.append(&label);
    button.set_child(Some(&content));
    (button, picture, gpu_canvas)
}

fn add_slider(
    parent: &gtk::Box,
    text: &str,
    min: f64,
    max: f64,
    step: f64,
    digits: i32,
) -> gtk::Scale {
    let row = gtk::Box::new(gtk::Orientation::Vertical, 2);
    row.add_css_class("edit-adjustment-row");

    let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let label = gtk::Label::new(Some(text));
    label.set_xalign(0.0);
    label.set_hexpand(true);
    let value = gtk::Label::new(None);
    value.set_xalign(1.0);
    value.add_css_class("dim-label");
    value.add_css_class("edit-adjustment-value");
    header.append(&label);
    header.append(&value);

    let scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, min, max, step);
    scale.set_digits(digits);
    scale.set_draw_value(false);
    scale.set_hexpand(true);
    scale.add_css_class("edit-adjustment-scale");

    let precision = digits.max(0) as usize;
    value.set_text(&format!("{:.*}", precision, scale.value()));
    {
        let value = value.clone();
        scale.connect_value_changed(move |scale| {
            value.set_text(&format!("{:.*}", precision, scale.value()));
        });
    }

    row.append(&header);
    row.append(&scale);
    parent.append(&row);
    scale
}

fn add_scale_default_affordance(scale: &gtk::Scale, default_value: f64) {
    let adjustment = scale.adjustment();
    let is_center_default = default_value > adjustment.lower() && default_value < adjustment.upper();
    if is_center_default {
        // A small visible detent marks the neutral/default position.
        scale.add_mark(default_value, gtk::PositionType::Bottom, None);
        scale.set_tooltip_text(Some(
            "Drag through the centre mark to snap to the default; right-click to reset this slider",
        ));
    } else {
        scale.set_tooltip_text(Some("Right-click to reset this slider to its default"));
    }

    let reset = gtk::GestureClick::new();
    reset.set_button(3);
    reset.set_propagation_phase(gtk::PropagationPhase::Capture);
    let scale_for_reset = scale.clone();
    reset.connect_pressed(move |gesture, _, _, _| {
        scale_for_reset.set_value(default_value);
        gesture.set_state(gtk::EventSequenceState::Claimed);
    });
    scale.add_controller(reset);
}

fn slider_detent_value(
    value: f64,
    default_value: f64,
    lower: f64,
    upper: f64,
    step: f64,
) -> Option<f64> {
    let is_center_default = default_value > lower && default_value < upper;
    let detent_radius = step.abs() * 0.51;
    (is_center_default && (value - default_value).abs() <= detent_radius)
        .then_some(default_value)
}

fn connect_scale(
    scale: &gtk::Scale,
    session: Rc<RefCell<EditSession>>,
    syncing: Rc<Cell<bool>>,
    queue_preview: Rc<dyn Fn()>,
    update_history: Rc<dyn Fn()>,
    default_value: f64,
    assign: impl Fn(&mut EditRecipe, f32) + 'static,
) {
    let dragging = Rc::new(Cell::new(false));
    let action_start_recipe = Rc::new(RefCell::new(None::<EditRecipe>));
    let finish_action: Rc<dyn Fn()> = {
        let session = session.clone();
        let dragging = dragging.clone();
        let action_start_recipe = action_start_recipe.clone();
        let update_history = update_history.clone();
        Rc::new(move || {
            if dragging.replace(false) {
                let start = action_start_recipe.borrow_mut().take();
                let changed = scale_action_changed(start, &session.borrow().recipe);
                session.borrow_mut().end_action();
                update_history();
                // value_changed already queued the final full-resolution recipe.
                // Releasing the slider only closes its undo action.
                if std::env::var_os("PICASA_TRACE").is_some() {
                    eprintln!("PIC_EDIT slider_action_end changed={changed}");
                }
            }
        })
    };
    add_scale_default_affordance(scale, default_value);

    let press = gtk::GestureClick::new();
    press.set_button(1);
    press.set_propagation_phase(gtk::PropagationPhase::Capture);
    {
        let session = session.clone();
        let dragging = dragging.clone();
        let action_start_recipe = action_start_recipe.clone();
        press.connect_pressed(move |_, _, _, _| {
            dragging.set(true);
            action_start_recipe.replace(Some(session.borrow().recipe.clone()));
            session.borrow_mut().begin_action();
            if std::env::var_os("PICASA_TRACE").is_some() {
                eprintln!("PIC_EDIT slider_action_begin");
            }
        });
    }
    {
        let finish_action = finish_action.clone();
        press.connect_released(move |_, _, _, _| {
            finish_action();
        });
    }
    scale.add_controller(press);

    // GtkScale's internal drag gesture can cancel a sibling GestureClick once
    // the pointer moves. This capture controller still sees the physical
    // release, ensuring the draft settles to a source-resolution render.
    let release = gtk::EventControllerLegacy::new();
    release.set_propagation_phase(gtk::PropagationPhase::Capture);
    {
        let finish_action = finish_action.clone();
        release.connect_event(move |_, event| {
            if event.event_type() == gtk::gdk::EventType::ButtonRelease
                && event
                    .downcast_ref::<gtk::gdk::ButtonEvent>()
                    .is_some_and(|button| button.button() == 1)
            {
                finish_action();
            }
            glib::Propagation::Proceed
        });
    }
    scale.add_controller(release);

    scale.connect_value_changed(move |scale| {
        if syncing.get() {
            return;
        }
        if dragging.get() {
            let adjustment = scale.adjustment();
            if let Some(snapped) = slider_detent_value(
                scale.value(),
                default_value,
                adjustment.lower(),
                adjustment.upper(),
                adjustment.step_increment(),
            ) {
                if (scale.value() - snapped).abs() > f64::EPSILON {
                    // Set the exact neutral value when the thumb crosses its
                    // detent. The nested value-changed signal applies that value
                    // to the edit recipe and queues the matching preview.
                    scale.set_value(snapped);
                    return;
                }
            }
        }
        let value = scale.value() as f32;
        if dragging.get() {
            session
                .borrow_mut()
                .mutate_active(|recipe| assign(recipe, value));
            update_history();
        } else {
            // Keyboard steps and deliberate wheel steps remain normal single
            // undoable actions; only a continuous mouse drag is coalesced.
            session.borrow_mut().mutate(|recipe| assign(recipe, value));
            update_history();
        }
        queue_preview();
    });
}

fn scale_action_changed(start_recipe: Option<EditRecipe>, current_recipe: &EditRecipe) -> bool {
    start_recipe.is_some_and(|start| start != *current_recipe)
}

#[cfg(test)]
mod scale_action_tests {
    use super::*;

    #[test]
    fn detent_snaps_only_internal_defaults_within_half_a_step() {
        assert_eq!(slider_detent_value(0.04, 0.0, -1.0, 1.0, 0.1), Some(0.0));
        assert_eq!(slider_detent_value(-0.04, 0.0, -1.0, 1.0, 0.1), Some(0.0));
        assert_eq!(slider_detent_value(0.06, 0.0, -1.0, 1.0, 0.1), None);
        assert_eq!(slider_detent_value(0.0, 0.0, 0.0, 1.0, 0.1), None);
        assert_eq!(slider_detent_value(0.0, 0.0, -1.0, 1.0, 0.0), Some(0.0));
    }

    #[test]
    fn settle_render_is_requested_only_after_a_real_slider_change() {
        let initial = EditRecipe::default();
        assert!(!scale_action_changed(Some(initial.clone()), &initial));
        assert!(!scale_action_changed(None, &initial));

        let mut changed = initial.clone();
        changed.exposure = -0.03;
        assert!(scale_action_changed(Some(initial), &changed));
    }
}

fn configure_scale_scroll(scale: &gtk::Scale, tools_scroll: &gtk::ScrolledWindow) {
    // One mouse-wheel click over the slider track changes the value by this
    // much (wheel up increases, wheel down decreases). Fractional deltas from
    // smooth-scroll devices produce proportional micro-steps.
    const SLIDER_WHEEL_STEP: f64 = 0.05;
    let pointer_y = Rc::new(Cell::new(f64::NAN));
    let motion = gtk::EventControllerMotion::new();
    {
        let pointer_y = pointer_y.clone();
        motion.connect_motion(move |_, _, y| pointer_y.set(y));
    }
    {
        let pointer_y = pointer_y.clone();
        motion.connect_leave(move |_| pointer_y.set(f64::NAN));
    }
    scale.add_controller(motion);

    let controller = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
    controller.set_propagation_phase(gtk::PropagationPhase::Capture);
    let scale_for_scroll = scale.clone();
    let tools_scroll = tools_scroll.clone();
    controller.connect_scroll(move |_, _, dy| {
        // GTK Scale reacts to wheel input across its full allocation, including
        // value text and padding, and steps the adjustment by its own coarse
        // increments. Take over the narrow central track band and step the
        // value by SLIDER_WHEEL_STEP per click instead; everywhere else
        // scrolls the editing tools normally.
        let y = pointer_y.get();
        let center = scale_for_scroll.height() as f64 / 2.0;
        if y.is_finite() && (y - center).abs() <= 7.0 {
            if dy != 0.0 {
                let adjustment = scale_for_scroll.adjustment();
                // set_value clamps to [lower, upper]
                adjustment.set_value(adjustment.value() - dy * SLIDER_WHEEL_STEP);
            }
            return glib::Propagation::Stop;
        }

        if dy != 0.0 {
            let adjustment = tools_scroll.vadjustment();
            let max = (adjustment.upper() - adjustment.page_size()).max(adjustment.lower());
            let step = adjustment.step_increment().max(24.0);
            adjustment.set_value(
                (adjustment.value() + dy.signum() * step * 2.0).clamp(adjustment.lower(), max),
            );
        }
        glib::Propagation::Stop
    });
    scale.add_controller(controller);
}

fn connect_toggle(
    button: &gtk::ToggleButton,
    session: Rc<RefCell<EditSession>>,
    syncing: Rc<Cell<bool>>,
    queue_preview: Rc<dyn Fn()>,
    update_history: Rc<dyn Fn()>,
    assign: impl Fn(&mut EditRecipe, bool) + 'static,
) {
    button.connect_toggled(move |button| {
        if syncing.get() {
            return;
        }
        let value = button.is_active();
        session.borrow_mut().mutate(|recipe| assign(recipe, value));
        update_history();
        queue_preview();
    });
}
