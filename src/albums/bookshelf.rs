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
    rows: gtk::Box,
    coverflow: gtk::Box,
    focused_album_id: Rc<Cell<Option<i64>>>,
    focused_index: Rc<Cell<usize>>,
    last_columns: Rc<Cell<usize>>,
    last_row_count: Rc<Cell<usize>>,
    last_row_height: Rc<Cell<i32>>,
    last_card_width: Rc<Cell<i32>>,
    album_count: Rc<Cell<usize>>,
    selected_theme: Rc<RefCell<Option<BookshelfTheme>>>,
    standard_width: Rc<Cell<i32>>,
    last_standard_width: Rc<Cell<i32>>,
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
            // The class lands on the shelf-rows box, whose top edge is the top
            // of the first row and whose height is a whole number of rows. A
            // tile that is exactly one row tall and starts at the box top is
            // therefore drawn at the same y as the row it sits behind, and it
            // scrolls with the rows instead of staying fixed in the viewport.
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

fn bookshelf_columns_for_width(width: i32, _card_width: i32) -> usize {
    if width <= 0 {
        return BOOKSHELF_MAX_COLUMNS;
    }

    let usable_width = (width - BOOKSHELF_SIDE_PADDING * 2).max(BOOKSHELF_MIN_CARD_WIDTH);
    let per_card = BOOKSHELF_MIN_CARD_WIDTH + BOOKSHELF_COLUMN_GAP;
    ((usable_width + BOOKSHELF_COLUMN_GAP) / per_card).clamp(1, BOOKSHELF_MAX_COLUMNS as i32)
        as usize
}

fn bookshelf_target_card_width(width: i32, columns: usize) -> i32 {
    if width <= 0 || columns == 0 {
        return BOOKSHELF_MIN_CARD_WIDTH;
    }

    let usable_width = (width - BOOKSHELF_SIDE_PADDING * 2).max(BOOKSHELF_MIN_CARD_WIDTH);
    let slot_width = usable_width / columns as i32;
    ((slot_width as f64 * BOOKSHELF_CARD_FILL_RATIO).round() as i32)
        .clamp(BOOKSHELF_MIN_CARD_WIDTH, BOOKSHELF_MAX_CARD_WIDTH)
}

fn bookshelf_row(theme: &BookshelfTheme) -> gtk::Overlay {
    let row = gtk::Overlay::new();
    row.set_hexpand(true);
    row.set_vexpand(false);
    row.set_height_request(theme.row_height);
    row.set_size_request(-1, theme.row_height);
    row.set_overflow(gtk::Overflow::Hidden);
    row.add_css_class("albums-bookshelf-row");

    // A Picture as the main child requests height from its image aspect ratio.
    // On wide/tall windows GTK can grant that natural height, stretching the
    // shelf artwork away from the fixed surface_y used by cards and reflections.
    let surface = gtk::Box::new(gtk::Orientation::Vertical, 0);
    surface.set_height_request(theme.row_height);
    row.set_child(Some(&surface));

    let background = gtk::Picture::new();
    background.set_filename(Some(&theme.image_path));
    background.set_content_fit(gtk::ContentFit::Fill);
    background.set_can_shrink(true);
    background.set_hexpand(true);
    background.set_vexpand(true);
    background.set_halign(gtk::Align::Fill);
    background.set_valign(gtk::Align::Fill);
    background.set_can_target(false);
    row.add_overlay(&background);
    row.set_measure_overlay(&background, false);
    row
}

fn bookshelf_row_count(card_count: usize, columns: usize) -> usize {
    let columns = columns.max(1);
    let required = card_count.div_ceil(columns);
    required.max(BOOKSHELF_MIN_ROWS)
}

fn bookshelf_rows_for_viewport(card_count: usize, columns: usize, available_height: i32) -> usize {
    // Do not manufacture empty shelves just to fill a tall viewport.
    // The existing row artwork remains untouched; only populated rows (and
    // the minimum visual pair) participate in the adaptive layout.
    let _ = available_height;
    bookshelf_row_count(card_count, columns)
}

fn bookshelf_available_height(scrolled: &gtk::ScrolledWindow, rows: &gtk::Box) -> i32 {
    let page_height = scrolled.vadjustment().page_size().round() as i32;
    let Some(content) = rows.parent().and_downcast::<gtk::Box>() else {
        return 0;
    };

    let rows_widget = rows.upcast_ref::<gtk::Widget>();
    let mut reserved = content.margin_top()
        + content.margin_bottom()
        + rows.margin_top()
        + rows.margin_bottom();
    let mut previous_visible = false;
    let mut child = content.first_child();
    while let Some(widget) = child {
        if std::ptr::eq(widget.as_ptr(), rows_widget.as_ptr()) {
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

fn bookshelf_row_height_for_viewport(
    row_count: usize,
    available_height: i32,
    row_spacing: i32,
    theme: &BookshelfTheme,
) -> i32 {
    if row_count < BOOKSHELF_MAX_VIEWPORT_ROWS {
        return theme.row_height;
    }
    let gaps = row_spacing.max(0) * row_count.saturating_sub(1) as i32;
    ((available_height - gaps).max(1) / row_count as i32)
        .clamp(BOOKSHELF_MIN_EXPANDED_ROW_HEIGHT, theme.row_height)
}

fn bookshelf_theme_with_row_height(theme: &BookshelfTheme, row_height: i32) -> BookshelfTheme {
    let surface_y = (theme.surface_y as f64 * row_height as f64 / theme.row_height.max(1) as f64)
        .round() as i32;
    BookshelfTheme {
        row_height,
        surface_y: surface_y.clamp(0, row_height),
        ..theme.clone()
    }
}

fn bookshelf_card_width_for_row_height(
    target_width: i32,
    row_height: i32,
    base_row_height: i32,
) -> i32 {
    ((target_width as f64 * row_height as f64 / base_row_height.max(1) as f64).round() as i32)
        .clamp(140, target_width)
}

#[cfg(test)]
mod bookshelf_responsive_sizing_tests {
    use super::*;

    fn theme() -> BookshelfTheme {
        BookshelfTheme {
            id: "test".into(),
            image_path: PathBuf::new(),
            kind: BookshelfThemeKind::Row,
            row_height: BOOKSHELF_ROW_HEIGHT,
            surface_y: BOOKSHELF_SURFACE_Y,
            reflection_strength: 0,
        }
    }

    #[test]
    fn four_rows_fit_the_available_bookshelf_height() {
        let theme = theme();
        assert_eq!(bookshelf_row_height_for_viewport(4, 820, 0, &theme), 205);
        assert_eq!(bookshelf_row_height_for_viewport(3, 820, 0, &theme), 288);
        assert_eq!(bookshelf_card_width_for_row_height(230, 205, 288), 164);
    }

    #[test]
    fn four_shelves_wait_until_each_can_keep_a_readable_book_size() {
        assert_eq!(bookshelf_rows_for_viewport(8, 4, 819), 2);
        assert_eq!(bookshelf_rows_for_viewport(8, 4, 820), 2);
        assert_eq!(bookshelf_row_height_for_viewport(4, 820, 0, &theme()), 205);
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

fn build_bookshelf_rows(
    rows: &gtk::Box,
    cards: &[gtk::Button],
    columns: usize,
    row_count: usize,
    theme: &BookshelfTheme,
) {
    clear_box(rows);
    let columns = columns.clamp(1, BOOKSHELF_MAX_COLUMNS);
    let row_count = row_count.max(bookshelf_row_count(cards.len(), columns));

    for row_index in 0..row_count {
        let start = (row_index * columns).min(cards.len());
        let end = (start + columns).min(cards.len());
        let chunk = &cards[start..end];

        let row = bookshelf_row(theme);
        let reflection_height = bookshelf_reflection_height(theme);
        if reflection_height > 0 {
            let reflections = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            reflections.set_homogeneous(true);
            reflections.set_hexpand(true);
            reflections.set_halign(gtk::Align::Fill);
            reflections.set_valign(gtk::Align::Start);
            reflections.set_height_request(theme.row_height);
            reflections.set_can_target(false);
            for index in 0..columns {
                let slot = gtk::Box::new(gtk::Orientation::Vertical, 0);
                slot.set_hexpand(true);
                slot.set_halign(gtk::Align::Fill);
                if let Some(card) = chunk.get(index) {
                    if let Some(cover) = find_descendant_with_css_class(
                        card.upcast_ref(), "album-cover") {
                        let reflection = ShelfReflection::new(
                            &cover, theme.reflection_strength, reflection_height,
                        );
                        reflection.set_margin_top(theme.surface_y + 4);
                        slot.append(&reflection);
                    }
                }
                reflections.append(&slot);
            }
            row.add_overlay(&reflections);
        }
        let slots = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        slots.set_homogeneous(true);
        slots.set_hexpand(true);
        slots.set_vexpand(false);
        slots.set_halign(gtk::Align::Fill);
        slots.set_valign(gtk::Align::Start);
        slots.set_height_request(theme.row_height);

        for index in 0..columns {
            let slot = gtk::Box::new(gtk::Orientation::Vertical, 0);
            slot.set_hexpand(true);
            slot.set_vexpand(false);
            slot.set_halign(gtk::Align::Fill);
            slot.set_valign(gtk::Align::Start);
            if let Some(card) = chunk.get(index) {
                // Align the book and its reflection in the same centered slot;
                // never stretch a sparse last row across the entire shelf.
                card.set_halign(gtk::Align::Center);
                slot.append(card);
            }
            slots.append(&slot);
        }

        row.add_overlay(&slots);
        rows.append(&row);
    }
}

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

fn resize_bookshelf_cards(rows: &gtk::Box, target_width: i32, theme: &BookshelfTheme) {
    let mut cards = Vec::new();
    collect_bookshelf_cards(rows.upcast_ref(), &mut cards);

    for card in cards {
        let Some(cover) = find_descendant_with_css_class(card.upcast_ref(), "album-cover")
            .and_then(|widget| widget.downcast::<gtk::Overlay>().ok())
        else {
            continue;
        };

        let current_width = cover.width_request();
        let current_height = cover.height_request();
        if current_width <= 0 || current_height <= 0 || current_width == target_width {
            continue;
        }

        let opening = unsafe {
            cover
                .data::<PhotoOpening>(FRAME_OPENING_KEY)
                .map(|opening| *opening.as_ref())
        };

        let scale_x = target_width as f64 / current_width as f64;
        let card_width = target_width;
        let target_height =
            ((current_height as f64 * scale_x).round() as i32).clamp(1, theme.surface_y.max(1));

        cover.set_width_request(card_width);
        cover.set_height_request(target_height);
        cover.set_size_request(card_width, target_height);
        if let Some(surface) = cover.child() {
            surface.set_size_request(card_width, target_height);
        }

        // Framed covers place the photo in a transparent window in the PNG.
        // Recompute its inset as the bookshelf resizes the complete book.
        if let Some(opening) = opening {
            let (left, top, right, bottom) = photo_margins(opening, card_width, target_height);
            for class_name in ["thumbnail", "album-cover-placeholder"] {
                if let Some(widget) = find_descendant_with_css_class(cover.upcast_ref(), class_name)
                {
                    widget.set_margin_start(left);
                    widget.set_margin_top(top);
                    widget.set_margin_end(right);
                    widget.set_margin_bottom(bottom);
                }
            }
        }

        card.set_width_request(card_width);
        card.set_margin_top(bookshelf_card_margin_top(theme.surface_y, target_height));

        if let Some(content) = card.child() {
            content.set_width_request(card_width);
            content.set_size_request(card_width, -1);
            let mut child = content.first_child();
            while let Some(widget) = child {
                // Preserve the cover's height request for subsequent resizes.
                if widget.is::<gtk::Label>() {
                    widget.set_width_request(card_width);
                }
                child = widget.next_sibling();
            }
        }
    }
}

fn reflow_bookshelf_rows(
    runtime: &BookshelfRuntime,
    columns: usize,
    row_count: usize,
    theme: &BookshelfTheme,
) {
    let mut cards = Vec::new();
    collect_bookshelf_cards(runtime.rows.upcast_ref(), &mut cards);
    if cards.is_empty() {
        runtime.last_columns.set(columns);
        runtime.last_row_count.set(row_count);
        return;
    }

    // Keep strong references while detaching cards from their old slot boxes.
    for card in &cards {
        if let Some(parent) = card.parent().and_downcast::<gtk::Box>() {
            parent.remove(card);
        }
    }
    build_bookshelf_rows(&runtime.rows, &cards, columns, row_count, theme);
    runtime.last_columns.set(columns);
    runtime.last_row_count.set(row_count);

    
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
