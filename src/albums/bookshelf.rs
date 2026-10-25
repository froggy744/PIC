#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BookshelfThemeKind {
    /// A folder-backed theme containing one image that represents exactly one shelf row.
    Row,
    /// Compatibility mode for the older page-sized *-bookshelf.png/jpg assets.
    Legacy,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct BookshelfTheme {
    id: String,
    image_path: PathBuf,
    kind: BookshelfThemeKind,
    row_height: i32,
    surface_y: i32,
    reflection_strength: u8,
}

// These are scene widths after the sidebar has taken its space. The gap
// keeps a dragging window from switching sizes back and forth at one pixel.
const BOOKSHELF_LARGE_ENTER_WIDTH: i32 = 2040;
const BOOKSHELF_LARGE_EXIT_WIDTH: i32 = 1960;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum BookshelfCoverPreset {
    #[default]
    Standard,
    Large,
}

impl BookshelfCoverPreset {
    fn dimension(self, standard: i32) -> i32 {
        match self {
            Self::Standard => standard,
            Self::Large => (standard * 128 + 50) / 100,
        }
    }
}

fn bookshelf_cover_preset(width: i32, current: BookshelfCoverPreset) -> BookshelfCoverPreset {
    if width <= 0 { return current; }
    match current {
        BookshelfCoverPreset::Standard if width >= BOOKSHELF_LARGE_ENTER_WIDTH =>
            BookshelfCoverPreset::Large,
        BookshelfCoverPreset::Large if width < BOOKSHELF_LARGE_EXIT_WIDTH =>
            BookshelfCoverPreset::Standard,
        _ => current,
    }
}

fn bookshelf_reflection_strength(directory: &Path) -> u8 {
    std::fs::read_to_string(directory.join("theme.conf")).ok()
        .and_then(|text| text.lines().filter_map(|line| line.split_once('='))
            .find(|(key, _)| key.trim() == "reflection_strength")
            .and_then(|(_, value)| value.trim().parse::<f64>().ok()))
        .filter(|value| value.is_finite() && (0.0..=1.0).contains(value))
        .map(|value| (value * 100.0).round() as u8)
        .unwrap_or(0)
}

fn bookshelf_reflection_height(theme: &BookshelfTheme) -> i32 {
    if theme.kind != BookshelfThemeKind::Row || theme.reflection_strength == 0 { return 0; }
    (theme.row_height - theme.surface_y - 4).clamp(0, 56)
}

#[derive(Clone)]
struct BookshelfRuntime {
    scene: gtk::Overlay,
    layout: gtk::Fixed,
    items: Rc<RefCell<Vec<BookshelfItem>>>,
    motion: Rc<RefCell<Option<BookshelfMotion>>>,
    coverflow: gtk::Box,
    focused_album_id: Rc<Cell<Option<i64>>>,
    focused_index: Rc<Cell<usize>>,
    last_width: Rc<Cell<i32>>,
    cover_preset: Rc<Cell<BookshelfCoverPreset>>,
    selected_theme: Rc<RefCell<Option<BookshelfTheme>>>,
    standard_width: Rc<Cell<i32>>,
    last_standard_width: Rc<Cell<i32>>,
}

#[derive(Clone)]
struct BookshelfItem {
    card: gtk::Button,
    cover: gtk::Overlay,
    reflection: Option<ShelfReflection>,
    standard_width: i32,
    standard_cover_width: i32,
    standard_cover_height: i32,
    width: i32,
    cover_height: i32,
}

impl BookshelfItem {
    fn set_preset(&mut self, preset: BookshelfCoverPreset) {
        let width = preset.dimension(self.standard_width);
        let cover_width = preset.dimension(self.standard_cover_width);
        let cover_height = preset.dimension(self.standard_cover_height);
        self.card.set_width_request(width);
        if let Some(content) = self.card.child() {
            content.set_width_request(cover_width);
        }
        self.cover.set_size_request(cover_width, cover_height);
        if let Some(surface) = self.cover.child() {
            surface.set_size_request(cover_width, cover_height);
        }
        if let Some(name) = find_descendant_with_css_class(self.card.upcast_ref(), "album-name") {
            name.set_width_request(cover_width);
        }
        if let Some(opening) = unsafe { self.cover.data::<PhotoOpening>(FRAME_OPENING_KEY) } {
            let (left, top, right, bottom) = photo_margins(
                unsafe { *opening.as_ref() }, cover_width, cover_height,
            );
            for class in ["thumbnail", "album-cover-placeholder"] {
                if let Some(widget) = find_descendant_with_css_class(self.cover.upcast_ref(), class) {
                    widget.set_margin_start(left);
                    widget.set_margin_top(top);
                    widget.set_margin_end(right);
                    widget.set_margin_bottom(bottom);
                }
            }
        }
        if let Some(reflection) = &self.reflection {
            reflection.set_width_request(width);
        }
        self.width = width;
        self.cover_height = cover_height;
    }
}

struct BookshelfMotion {
    started: Instant,
    from: Vec<(f64, f64)>,
    to: Vec<(f64, f64)>,
}

fn bookshelf_theme_image_in(directory: &Path) -> Option<PathBuf> {
    let preferred = [
        "row.png",
        "row.jpg",
        "bookshelf.png",
        "bookshelf.jpg",
        "single-row-bookshelf.png",
        "single-row-bookshelf.jpg",
    ];
    for name in preferred {
        let path = directory.join(name);
        if path.is_file() {
            return Some(path);
        }
    }

    let mut images = png_or_jpg_paths_matching(directory, |_| true);
    images.sort();
    images.into_iter().next()
}

/// The shelf line the artwork itself draws: the strongest full-width horizontal
/// edge whose band below stays flat across the whole width. That band is the
/// shelf board, and its top edge is where albums and books stand.
///
/// Fallback only: shipped artwork states `surface_y` in `theme.conf`, because on
/// a photographed board the flat band under the measured edge is the shadow
/// beneath the board, a few pixels below the front edge covers should stand on.
/// See the measured edges in images/bookshelf-row-template.md.
///
/// Returned as a ratio of the image height, because the row is drawn by
/// stretching the whole canvas to `row_height`.
fn shelf_surface_ratio(path: &Path) -> Option<f64> {
    let decoded = image::open(path).ok()?.to_rgb8();
    let (width, height) = decoded.dimensions();
    if width < 8 || height < 8 {
        return None;
    }
    let (width, height) = (width as usize, height as usize);

    // Brightness scaled by 1000 keeps every comparison in whole numbers.
    let luma: Vec<usize> = decoded.pixels()
        .map(|pixel| 299 * pixel.0[0] as usize + 587 * pixel.0[1] as usize + 114 * pixel.0[2] as usize)
        .collect();
    if luma.len() < width * height {
        return None;
    }

    // Sampling a few hundred columns keeps a full-width shelf line visible
    // while decoding a page-sized photograph stays cheap enough to cache.
    let step = (width / 400_usize).max(1_usize);
    let columns = 1_usize + (width - 1_usize) / step;
    let at = |y: usize, column: usize| luma[y * width + column * step];
    let difference = |left: usize, right: usize|
        left.saturating_sub(right).max(right.saturating_sub(left));

    // edge[y - 1] counts the columns that change brightness at row y.
    let edge: Vec<usize> = (1_usize..height).map(|y| {
        (0_usize..columns).filter(|column|
            difference(at(y, *column), at(y - 1_usize, *column)) > 24_000_usize
        ).count()
    }).collect();

    // flat[y] counts the columns that match this row's own median colour.
    let flat: Vec<usize> = (0_usize..height).map(|y| {
        let mut row = (0_usize..columns).map(|column| at(y, column)).collect::<Vec<usize>>();
        row.sort();
        let median = row[columns / 2_usize];
        row.iter().filter(|value| difference(**value, median) < 12_000_usize).count()
    }).collect();

    let band = 32_usize;
    let candidate = (height * 2_usize / 5_usize..height.saturating_sub(band + 2_usize)).filter(|y| {
        let below = &flat[(y + 2_usize).min(height)..(y + band).min(height)];
        let steady = below.iter().filter(|value| *value * 100_usize >= columns * 85_usize).count();
        steady * 5_usize >= below.len() * 4_usize
            && edge[y - 1_usize] * 2_usize >= columns
    }).max_by_key(|y| edge[y - 1_usize])?;
    Some(candidate as f64 / height as f64)
}

/// Geometry for one folder-backed row theme: `theme.conf` wins, then the
/// artwork's own shelf line, then the shipped defaults.
fn bookshelf_theme_geometry(directory: &Path, image_path: &Path) -> (i32, i32) {
    let mut row_height = BOOKSHELF_ROW_HEIGHT;
    let mut configured: Option<i32> = None;
    if let Ok(text) = std::fs::read_to_string(directory.join("theme.conf")) {
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let Ok(value) = value.trim().parse::<i32>() else {
                continue;
            };
            match key.trim() {
                "row_height" if value > 0 => row_height = value,
                "surface_y" if value >= 0 => configured = Some(value),
                _ => {}
            }
        }
    }

    let surface_y = configured
        .or_else(|| shelf_surface_ratio(image_path)
            .map(|ratio| (row_height as f64 * ratio).round() as i32))
        .unwrap_or(BOOKSHELF_SURFACE_Y);
    (row_height, surface_y.clamp(0, row_height))
}

fn bookshelf_themes_in(directory: &Path) -> Vec<BookshelfTheme> {
    let mut themes = Vec::new();
    if let Ok(entries) = std::fs::read_dir(directory) {
        for path in entries.filter_map(Result::ok).map(|entry| entry.path()) {
            if path.is_dir() {
                if let Some(image_path) = bookshelf_theme_image_in(&path) {
                    let id = path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or("bookshelf")
                        .to_string();
                    let (row_height, surface_y) = bookshelf_theme_geometry(&path, &image_path);
                    themes.push(BookshelfTheme {
                        id,
                        image_path,
                        kind: BookshelfThemeKind::Row,
                        row_height,
                        surface_y,
                        reflection_strength: bookshelf_reflection_strength(&path),
                    });
                }
            } else if path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| {
                    name.ends_with("-bookshelf.png") || name.ends_with("-bookshelf.jpg")
                })
            {
                let id = path
                    .file_stem()
                    .and_then(|name| name.to_str())
                    .unwrap_or("bookshelf")
                    .to_string();
                themes.push(BookshelfTheme {
                    id,
                    image_path: path,
                    kind: BookshelfThemeKind::Legacy,
                    row_height: BOOKSHELF_ROW_HEIGHT,
                    surface_y: BOOKSHELF_SURFACE_Y,
                    reflection_strength: 0,
                });
            }
        }
    }

    themes.sort_by_key(|theme| {
        (
            theme.kind != BookshelfThemeKind::Row,
            theme.id != "single",
            theme.id.to_ascii_lowercase(),
        )
    });
    themes
}

fn bookshelf_background_index(themes: &[BookshelfTheme], saved_index: Option<usize>) -> usize {
    let fallback = themes
        .iter()
        .position(|theme| theme.id == "darkwood")
        .unwrap_or(0);
    saved_index.unwrap_or(fallback) % themes.len().max(1)
}

pub(crate) fn default_bookshelf_background_index() -> usize {
    let themes = bookshelf_themes_in(
        &crate::css::resolve_runtime_dir(Path::new(BOOKSHELF_THEME_DIRECTORY)),
    );
    bookshelf_background_index(&themes, None)
}

fn saved_bookshelf_background_index(connection: &Connection) -> Option<usize> {
    crate::db::setting(connection, crate::settings::ALBUM_BOOKSHELF_BACKGROUND_SETTING_KEY)
        .ok()
        .flatten()
        .and_then(|value| value.parse().ok())
}

fn bookshelf_theme(connection: &Connection) -> Option<BookshelfTheme> {
    let themes = bookshelf_themes_in(
        &crate::css::resolve_runtime_dir(Path::new(BOOKSHELF_THEME_DIRECTORY)),
    );
    (!themes.is_empty()).then(|| {
        themes[bookshelf_background_index(&themes, saved_bookshelf_background_index(connection))]
            .clone()
    })
}

#[cfg(test)]
fn bookshelf_background_path(directory: &Path, index: usize) -> Option<PathBuf> {
    let themes = bookshelf_themes_in(directory);
    (!themes.is_empty()).then(|| themes[index % themes.len()].image_path.clone())
}

fn bookshelf_card_margin_top(surface_y: i32, cover_height: i32) -> i32 {
    (surface_y - cover_height).max(0)
}

#[cfg(test)]
mod bookshelf_theme_defaults_tests {
    use super::*;

    #[test]
    fn bookshelf_defaults_to_darkwood_and_honors_saved_theme_index() {
        let themes = bookshelf_themes_in(Path::new(BOOKSHELF_THEME_DIRECTORY));
        let default = bookshelf_background_index(&themes, None);
        assert_eq!(themes[default].id, "darkwood");
        assert_eq!(bookshelf_background_index(&themes, Some(0)), 0);
    }
}

fn bookshelf_background_rules(directory: &Path) -> String {
    bookshelf_themes_in(directory)
        .into_iter()
        .enumerate()
        .map(|(index, theme)| {
            let uri = gio::File::for_path(theme.image_path).uri();
            // The scene begins at the first shelf and grows to the viewport or
            // the album content, whichever is taller. Its image tiles scroll
            // with the albums, including through the empty part of the scene.
            let size = match theme.kind {
                BookshelfThemeKind::Row => format!("100% {}px", theme.row_height),
                BookshelfThemeKind::Legacy => "100% auto".to_string(),
            };
            let position = "center top";
            format!(
                ".albums-bookshelf-{index} {{ background-image: url(\"{uri}\"); \
                 background-size: {size}; background-repeat: repeat-y; \
                 background-position: {position}; }}"
            )
        })
        .collect()
}

fn bookshelf_background_css_class(connection: &Connection) -> Option<String> {
    let themes = bookshelf_themes_in(
        &crate::css::resolve_runtime_dir(Path::new(BOOKSHELF_THEME_DIRECTORY)),
    );
    if themes.is_empty() {
        return None;
    }
    let index = bookshelf_background_index(&themes, saved_bookshelf_background_index(connection));
    Some(format!("albums-bookshelf-{index}"))
}

fn bookshelf_row_count(card_count: usize, columns: usize) -> usize {
    card_count.div_ceil(columns.max(1))
}

fn bookshelf_columns_for_width(width: i32, card_width: i32) -> usize {
    let pitch = card_width.max(1) + BOOKSHELF_COLUMN_GAP;
    ((width - 2 * BOOKSHELF_SIDE_PADDING + BOOKSHELF_COLUMN_GAP) / pitch)
        .clamp(1, BOOKSHELF_MAX_COLUMNS as i32) as usize
}

fn bookshelf_available_height(scrolled: &gtk::ScrolledWindow, scene: &gtk::Overlay) -> i32 {
    // The vertical adjustment reports a transient zero during GTK resize.
    // The actual widget allocation is the reliable fallback.
    let page_height = scrolled.height().max(scrolled.vadjustment().page_size().round() as i32);
    let Some(content) = scene.parent().and_downcast::<gtk::Box>() else {
        return 0;
    };

    let scene_widget = scene.upcast_ref::<gtk::Widget>();
    let mut reserved = content.margin_top()
        + content.margin_bottom()
        + scene.margin_top()
        + scene.margin_bottom();
    let mut previous_visible = false;
    let mut child = content.first_child();
    while let Some(widget) = child {
        if std::ptr::eq(widget.as_ptr(), scene_widget.as_ptr()) {
            if previous_visible {
                reserved += content.spacing();
            }
            break;
        }
        if widget.is_visible() {
            if previous_visible {
                reserved += content.spacing();
            }
            reserved += widget.height();
            previous_visible = true;
        }
        child = widget.next_sibling();
    }

    (page_height - reserved).max(0)
}

#[cfg(test)]
mod bookshelf_responsive_sizing_tests {
    use super::*;

    #[test]
    fn large_cover_preset_uses_content_width_with_hysteresis() {
        use BookshelfCoverPreset::{Large, Standard};
        assert_eq!(bookshelf_cover_preset(1490, Standard), Standard);
        assert_eq!(bookshelf_cover_preset(1920, Large), Standard);
        assert_eq!(bookshelf_cover_preset(2039, Standard), Standard);
        assert_eq!(bookshelf_cover_preset(2040, Standard), Large);
        assert_eq!(bookshelf_cover_preset(2000, Large), Large);
        assert_eq!(bookshelf_cover_preset(1959, Large), Standard);
        assert_eq!(bookshelf_cover_preset(0, Large), Large);
        assert_eq!(Large.dimension(220), 282);
        assert_eq!(Large.dimension(137), 175);
        assert_eq!(Standard.dimension(220), 220);
    }

    #[test]
    fn bookshelf_layout_reserves_only_occupied_rows() {
        assert_eq!(bookshelf_row_count(0, 4), 0);
        assert_eq!(bookshelf_row_count(8, 4), 2);
        assert_eq!(bookshelf_row_count(9, 4), 3);
    }
}

mod shelf_reflection_imp {
    use super::*;
    #[derive(Default)]
    pub struct ShelfReflection {
        pub paintable: RefCell<Option<gtk::WidgetPaintable>>,
        pub strength: Cell<f64>,
    }
    #[gtk::glib::object_subclass]
    impl ObjectSubclass for ShelfReflection {
        const NAME: &'static str = "PicShelfReflection";
        type Type = super::ShelfReflection;
        type ParentType = gtk::Widget;
    }
    impl ObjectImpl for ShelfReflection {}
    impl WidgetImpl for ShelfReflection {
        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let Some(paintable) = self.paintable.borrow().clone() else { return; };
            let Some(source) = paintable.widget() else { return; };
            let w = source.width().max(source.width_request()).max(1) as f32;
            let h = source.height().max(source.height_request()).max(1) as f32;
            let canvas_w = self.obj().width() as f32;
            let reflection_h = self.obj().height().max(0) as f32;
            if reflection_h <= 0.0 { return; }
            let x = (canvas_w - w) * 0.5;
            snapshot.save();
            snapshot.push_clip(&graphene::Rect::new(x, 0.0, w, reflection_h));
            snapshot.push_mask(gtk::gsk::MaskMode::Alpha);
            snapshot.append_linear_gradient(
                &graphene::Rect::new(x, 0.0, w, reflection_h),
                &graphene::Point::new(x, 0.0),
                &graphene::Point::new(x, reflection_h),
                &[
                    gtk::gsk::ColorStop::new(0.0, gtk::gdk::RGBA::new(1.0, 1.0, 1.0, self.strength.get() as f32)),
                    gtk::gsk::ColorStop::new(1.0, gtk::gdk::RGBA::new(1.0, 1.0, 1.0, 0.0)),
                ],
            );
            snapshot.pop();
            snapshot.translate(&graphene::Point::new(x, h));
            snapshot.scale(1.0, -1.0);
            paintable.snapshot(snapshot, w as f64, h as f64);
            snapshot.pop();
            snapshot.pop();
            snapshot.restore();
        }
    }
}
gtk::glib::wrapper! {
    pub struct ShelfReflection(ObjectSubclass<shelf_reflection_imp::ShelfReflection>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}
impl ShelfReflection {
    fn new(cover: &gtk::Widget, strength: u8, height: i32) -> Self {
        let reflection: Self = gtk::glib::Object::new();
        reflection.set_hexpand(true);
        reflection.set_halign(gtk::Align::Fill);
        reflection.set_height_request(height);
        reflection.set_can_target(false);
        reflection.add_css_class("albums-bookshelf-reflection");
        reflection.imp().strength.set(strength as f64 / 100.0);
        let paintable = gtk::WidgetPaintable::new(Some(cover));
        let weak = reflection.downgrade();
        paintable.connect_invalidate_contents(move |_| {
            if let Some(reflection) = weak.upgrade() { reflection.queue_draw(); }
        });
        let weak = reflection.downgrade();
        paintable.connect_invalidate_size(move |_| {
            if let Some(reflection) = weak.upgrade() { reflection.queue_draw(); }
        });
        reflection.imp().paintable.replace(Some(paintable));
        reflection
    }
}

// Album coordinates are independent of the backdrop and of GTK's row allocation.
fn bookshelf_card_position(
    viewport_width: i32,
    slot_width: i32,
    item_width: i32,
    columns: usize,
    index: usize,
    cover_height: i32,
    theme: &BookshelfTheme,
) -> (f64, f64) {
    let columns = columns.max(1);
    let used_width = columns as i32 * slot_width
        + (columns as i32 - 1) * BOOKSHELF_COLUMN_GAP;
    let left = ((viewport_width - used_width) / 2).max(0);
    let column = index % columns;
    let row = index / columns;
    let x = left + column as i32 * (slot_width + BOOKSHELF_COLUMN_GAP)
        + (slot_width - item_width) / 2;
    let y = row as i32 * theme.row_height
        + bookshelf_card_margin_top(theme.surface_y, cover_height);
    (x as f64, y as f64)
}

fn clear_bookshelf_layout(runtime: &BookshelfRuntime) {
    while let Some(child) = runtime.layout.first_child() {
        runtime.layout.remove(&child);
    }
    runtime.items.borrow_mut().clear();
    runtime.motion.borrow_mut().take();
    runtime.last_width.set(0);
    runtime.cover_preset.set(BookshelfCoverPreset::Standard);
}

fn install_bookshelf_cards(runtime: &BookshelfRuntime, cards: &[gtk::Button], theme: &BookshelfTheme) {
    let reflection_height = bookshelf_reflection_height(theme);
    let mut items = runtime.items.borrow_mut();
    for card in cards {
        let cover = find_descendant_with_css_class(card.upcast_ref(), "album-cover")
            .and_then(|cover| cover.downcast::<gtk::Overlay>().ok())
            .expect("an album card has a cover overlay");
        let width = card.width_request().max(cover.width_request());
        let cover_height = cover.height_request();
        card.set_margin_top(0);
        card.set_halign(gtk::Align::Start);
        let reflection = (reflection_height > 0).then(|| {
            let reflection = ShelfReflection::new(cover.upcast_ref(), theme.reflection_strength, reflection_height);
            reflection.set_width_request(width);
            runtime.layout.put(&reflection, 0.0, 0.0);
            reflection
        });
        runtime.layout.put(card, 0.0, 0.0);
        items.push(BookshelfItem {
            card: card.clone(), cover: cover.clone(), reflection,
            standard_width: width, standard_cover_width: cover.width_request(),
            standard_cover_height: cover_height, width, cover_height,
        });
    }
}

fn reflow_bookshelf_layout(
    runtime: &BookshelfRuntime,
    width: i32,
    available_height: i32,
    theme: &BookshelfTheme,
) {
    let preset = bookshelf_cover_preset(width, runtime.cover_preset.get());
    if preset != runtime.cover_preset.get() {
        for item in runtime.items.borrow_mut().iter_mut() {
            let (x, y) = runtime.layout.child_position(&item.card);
            let old_height = item.cover_height;
            item.set_preset(preset);
            let anchored_y = y + (old_height - item.cover_height) as f64;
            runtime.layout.move_(&item.card, x, anchored_y);
            if let Some(reflection) = &item.reflection {
                runtime.layout.move_(reflection, x, y + old_height as f64 + 4.0);
            }
        }
        runtime.cover_preset.set(preset);
    }
    let items = runtime.items.borrow();
    let slot_width = items.iter().map(|item| item.width).max().unwrap_or(1);
    let columns = bookshelf_columns_for_width(width, slot_width);
    let occupied_height = bookshelf_row_count(items.len(), columns) as i32 * theme.row_height;
    // The scene paints every tile through the visible viewport, even for an
    // empty library. Extra space leaves the last row's labels unclipped.
    runtime.scene.set_height_request(available_height.max(occupied_height + 56).max(theme.row_height));
    if width <= 0 || width == runtime.last_width.get() {
        return;
    }
    let to: Vec<_> = items.iter().enumerate().map(|(index, item)| {
        bookshelf_card_position(width, slot_width, item.width, columns, index, item.cover_height, theme)
    }).collect();
    let from: Vec<_> = items.iter().map(|item| runtime.layout.child_position(&item.card)).collect();
    let first_layout = runtime.last_width.replace(width) == 0;
    if first_layout {
        for (item, &(x, y)) in items.iter().zip(&to) {
            runtime.layout.move_(&item.card, x, y);
            if let Some(reflection) = &item.reflection {
                runtime.layout.move_(reflection, x, y + item.cover_height as f64 + 4.0);
            }
        }
        runtime.motion.borrow_mut().take();
    } else if from != to {
        runtime.motion.replace(Some(BookshelfMotion { started: Instant::now(), from, to }));
    }
}

fn advance_bookshelf_motion(runtime: &BookshelfRuntime) {
    let mut motion = runtime.motion.borrow_mut();
    let Some(active) = motion.as_ref() else { return; };
    let progress = (active.started.elapsed().as_secs_f64() / BOOKSHELF_MOVE_DURATION).min(1.0);
    let eased = progress * progress * (3.0 - 2.0 * progress);
    for (item, (&from, &to)) in runtime.items.borrow().iter()
        .zip(active.from.iter().zip(&active.to)) {
        let x = from.0 + (to.0 - from.0) * eased;
        let y = from.1 + (to.1 - from.1) * eased;
        runtime.layout.move_(&item.card, x, y);
        if let Some(reflection) = &item.reflection {
            runtime.layout.move_(reflection, x, y + item.cover_height as f64 + 4.0);
        }
    }
    if progress >= 1.0 { motion.take(); }
}

// Also used by the standard FlowBox sizing path.
fn collect_bookshelf_cards(root: &gtk::Widget, cards: &mut Vec<gtk::Button>) {
    if let Some(button) = root.downcast_ref::<gtk::Button>() {
        if button.has_css_class("album-card") {
            cards.push(button.clone());
            return;
        }
    }
    let mut child = root.first_child();
    while let Some(current) = child {
        collect_bookshelf_cards(&current, cards);
        child = current.next_sibling();
    }
}

fn insert_child(
    cards: &gtk::FlowBox,
    child: &impl IsA<gtk::Widget>,
    width: Option<i32>,
    height: Option<i32>,
) {
    cards.insert(child, -1);

    // GtkFlowBox automatically wraps every inserted widget in a
    // GtkFlowBoxChild. Prevent that wrapper from stretching the album card.
    if let Some(flow_child) = cards
        .last_child()
        .and_then(|child| child.downcast::<gtk::FlowBoxChild>().ok())
    {
        if let Some(width) = width {
            flow_child.set_width_request(width);
        }
        if let Some(height) = height {
            flow_child.set_height_request(height);
        }
        flow_child.set_hexpand(false);
        flow_child.set_vexpand(false);
        flow_child.set_halign(gtk::Align::Start);
        flow_child.set_valign(gtk::Align::Start);
    }
}
