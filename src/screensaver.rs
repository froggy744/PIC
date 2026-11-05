//! Photo screensaver: a Picasa-inspired mosaic/picture pile and an iPhoto-inspired
//! cinematic slideshow. Decode/rendering runs off the GTK thread. All original
//! images and non-destructive edits remain untouched.
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet, VecDeque};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use gtk::prelude::*;
use gtk4 as gtk;
use libadwaita as adw;
use rusqlite::Connection;

use crate::{db, edit, thumbnail};
mod gpu;
mod matrix;

const STYLE_KEY: &str = "screensaver-style";
const TERMINAL_STYLE_KEY: &str = "screensaver-terminal-style";
const SOURCE_KEY: &str = "screensaver-source";
const DURATION_KEY: &str = "screensaver-duration";
const SHUFFLE_KEY: &str = "screensaver-shuffle";
const EXCLUDE_PORTRAITS_KEY: &str = "screensaver-exclude-portraits";
const SHUFFLE_TILES_KEY: &str = "screensaver-shuffle-tiles";
const MOSAIC_TILES_KEY: &str = "screensaver-mosaic-tiles";
const PICASA_TRANSITION_KEY: &str = "screensaver-picasa-transition";
const DEFAULT_MOSAIC_TILES: usize = 6;
const MAX_WALL_TILES: usize = 10_000;
/// Largest count with hand-placed layouts in print_rect(); beyond these the
/// prints are scattered over a computed grid so any tile count works.
const PICTURE_PILE_SLOTS: usize = 5;
const FLOATING_SLOTS: usize = 6;
/// Picture Pile / Floating Prints are painted with shadows every frame, so the
/// user-chosen tile count is capped (walls have no such cap).
const PRINT_MAX_SLOTS: usize = 64;
/// Flying Photos keeps a bounded field of prints in the air. Every print in
/// flight owns a live texture, so the count is capped more tightly than a
/// still wall.
const FLY_MAX_SLOTS: usize = 32;
const TRANSITION_KEY: &str = "screensaver-transition-ms";
const MOVEMENT_SPEED_KEY: &str = "screensaver-movement-speed";
const CAPTION_KEY: &str = "screensaver-captions";
const CLOCK_KEY: &str = "screensaver-clock";
const BACKGROUND_COLOUR_KEY: &str = "screensaver-background-colour";
const PREFETCH: usize = 3;
const HQ_PATIENCE: Duration = Duration::from_secs(3);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Style {
    Mosaic,
    Grid,
    PicturePile,
    Floating,
    IPhoto,
    Fade,
    CoverFlow,
    Flying,
    PhotoDrift,
    Surprise,
    MatrixRain,
    MatrixKenBurns,
}
impl Style {
    fn default_tiles(self) -> usize {
        match self {
            Self::Flying | Self::PhotoDrift => 30,
            Self::Floating => 12,
            _ => DEFAULT_MOSAIC_TILES,
        }
    }
    const CHOICES: [Self; 12] = [
        Self::Mosaic,
        Self::Grid,
        Self::PicturePile,
        Self::Floating,
        Self::IPhoto,
        Self::Fade,
        Self::CoverFlow,
        Self::Flying,
        Self::PhotoDrift,
        Self::Surprise,
        Self::MatrixRain,
        Self::MatrixKenBurns,
    ];
    fn from_saved(v: &str) -> Self {
        match v {
            "grid" => Self::Grid,
            "pile" => Self::PicturePile,
            "floating" => Self::Floating,
            "iphoto" => Self::IPhoto,
            "fade" => Self::Fade,
            "coverflow" => Self::CoverFlow,
            "flying" => Self::Flying,
            "photo-drift" => Self::PhotoDrift,
            "surprise" => Self::Surprise,
            "matrix-rain" => Self::MatrixRain,
            "matrix-ken-burns" => Self::MatrixKenBurns,
            _ => Self::Mosaic,
        }
    }
    fn key(self) -> &'static str {
        match self {
            Self::Mosaic => "mosaic",
            Self::Grid => "grid",
            Self::PicturePile => "pile",
            Self::Floating => "floating",
            Self::IPhoto => "iphoto",
            Self::Fade => "fade",
            Self::CoverFlow => "coverflow",
            Self::Flying => "flying",
            Self::PhotoDrift => "photo-drift",
            Self::Surprise => "surprise",
            Self::MatrixRain => "matrix-rain",
            Self::MatrixKenBurns => "matrix-ken-burns",
        }
    }
    fn resolved(self) -> Self {
        if self != Self::Surprise {
            return self;
        }
        // Surprise stays within the ordinary photo styles in every theme:
        // every CHOICES entry except Surprise itself and the Terminal-only
        // Matrix modes.
        let ordinary = Self::CHOICES.len() - 3;
        Self::CHOICES[random_seed() as usize % ordinary]
    }
    fn single_photo(self) -> bool {
        matches!(
            self,
            Self::IPhoto | Self::Fade | Self::CoverFlow | Self::MatrixRain | Self::MatrixKenBurns
        )
    }
    fn matrix(self) -> bool {
        matches!(self, Self::MatrixRain | Self::MatrixKenBurns)
    }
    fn uses_photos(self) -> bool {
        self != Self::MatrixRain
    }
    // Easter egg: follow the white rabbit; Matrix modes only appear in Terminal.
    fn available(self, terminal: bool) -> bool {
        terminal || !self.matrix()
    }
    fn for_theme(self, terminal: bool) -> Self {
        if self.available(terminal) {
            self
        } else {
            Self::Mosaic
        }
    }
    fn label(self) -> &'static str {
        match self {
            Self::Mosaic => "Mosaic",
            Self::Grid => "Changing Grid",
            Self::PicturePile => "Picture Pile",
            Self::Floating => "Floating Prints",
            Self::IPhoto => "Ken Burns",
            Self::Fade => "Fullscreen Fade",
            Self::CoverFlow => "Cover Flow",
            Self::Flying => "Flying Photos",
            Self::PhotoDrift => "Photo Drift",
            Self::Surprise => "Surprise Me",
            Self::MatrixRain => "Matrix Rain",
            Self::MatrixKenBurns => "Matrix Ken Burns",
        }
    }
    fn subtitle(self) -> &'static str {
        match self {
            Self::Mosaic => "Square Picasa photo wall",
            Self::Grid => "Full-screen tiled gallery",
            Self::PicturePile => "Layered scrapbook photographs",
            Self::Floating => "Scattered framed pictures",
            Self::IPhoto => "Cinematic pan and zoom",
            Self::Fade => "Gentle full-screen slideshow",
            Self::CoverFlow => "Browse photos as a reflective Cover Flow",
            Self::Flying => "Photographs fly in from the distance and past you",
            Self::PhotoDrift => "Photos gently drift across a scattered, layered field",
            Self::Surprise => "Random style on launch",
            Self::MatrixRain => "Animated Katakana rain on black, no photos",
            Self::MatrixKenBurns => "Green photo with fixed Katakana streams and slow pan and zoom",
        }
    }
}

/// Picasa-specific tile transitions; iPhoto always uses its cinematic fade.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PicasaTransition {
    Flip,
    Crossfade,
    Cut,
}
impl PicasaTransition {
    fn from_saved(value: &str) -> Self {
        match value {
            "flip" => Self::Flip,
            "cut" => Self::Cut,
            _ => Self::Crossfade, // Existing screensavers retain old behaviour.
        }
    }
    fn key(self) -> &'static str {
        match self {
            Self::Flip => "flip",
            Self::Crossfade => "crossfade",
            Self::Cut => "cut",
        }
    }
    #[cfg(test)]
    fn dropdown_index(self) -> u32 {
        match self {
            Self::Flip => 0,
            Self::Crossfade => 1,
            Self::Cut => 2,
        }
    }
    #[cfg(test)]
    fn from_dropdown(value: u32) -> Self {
        match value {
            0 => Self::Flip,
            2 => Self::Cut,
            _ => Self::Crossfade,
        }
    }
    /// A two-sided horizontal card turn, edge-on at midpoint.
    fn flip_width(self, progress: f64) -> f64 {
        if self != Self::Flip {
            return 1.0;
        }
        let eased = smootherstep(progress.clamp(0.0, 1.0));
        ((std::f64::consts::PI * eased).cos()).abs().max(0.025)
    }
}

#[derive(Clone, Debug)]
enum Source {
    All,
    Favorites,
    Selected(Vec<i64>),
    Album(i64),
    Folder(i64),
}
impl Source {
    fn key(&self) -> String {
        match self {
            Self::All => "all".into(),
            Self::Favorites => "favorites".into(),
            Self::Selected(_) => "selected".into(),
            Self::Album(id) => format!("album:{id}"),
            Self::Folder(id) => format!("folder:{id}"),
        }
    }
    fn photos(&self, connection: &Connection) -> anyhow::Result<Vec<db::Photo>> {
        match self {
            Self::All => db::photos(connection, None, false, None),
            Self::Favorites => db::photos(connection, None, true, None),
            Self::Selected(ids) => ids
                .iter()
                .map(|id| db::photo(connection, *id))
                .collect::<anyhow::Result<Vec<_>>>()
                .map(|rows| {
                    rows.into_iter()
                        .flatten()
                        .filter(|photo| !photo.trashed)
                        .collect()
                }),
            Self::Album(id) => db::photos_in_album(connection, *id, None),
            Self::Folder(id) => db::photos(connection, Some(*id), false, None),
        }
    }
}

#[derive(Clone)]
struct Preferences {
    style: Style,
    source: String,
    seconds: u32,
    shuffle: bool,
    exclude_portraits: bool,
    shuffle_tiles: bool,
    mosaic_tiles: usize,
    picasa_transition: PicasaTransition,
    transition_ms: u32,
    movement_speed: f64,
    captions: bool,
    clock: bool,
    background_colour: Option<[u8; 3]>,
}
impl Preferences {
    fn load(connection: &Connection) -> Self {
        let setting = |key| {
            db::setting(connection, key)
                .ok()
                .flatten()
                .unwrap_or_default()
        };
        let style = Style::from_saved(&setting(STYLE_KEY));
        Self {
            style,
            source: setting(SOURCE_KEY),
            seconds: setting(DURATION_KEY)
                .parse::<u32>()
                .unwrap_or(6)
                .clamp(3, 30),
            shuffle: setting(SHUFFLE_KEY) != "false",
            exclude_portraits: setting(EXCLUDE_PORTRAITS_KEY) == "true",
            shuffle_tiles: setting(SHUFFLE_TILES_KEY) != "false",
            mosaic_tiles: setting(MOSAIC_TILES_KEY)
                .parse::<usize>()
                .ok()
                .filter(|count| *count > 0)
                .unwrap_or(style.default_tiles()),
            picasa_transition: PicasaTransition::from_saved(&setting(PICASA_TRANSITION_KEY)),
            transition_ms: setting(TRANSITION_KEY)
                .parse::<u32>()
                .unwrap_or(900)
                .clamp(200, 2500),
            movement_speed: setting(MOVEMENT_SPEED_KEY)
                .parse::<f64>()
                .ok()
                .filter(|speed| speed.is_finite())
                .unwrap_or(1.0)
                .clamp(0.25, 3.0),
            captions: setting(CAPTION_KEY) == "true",
            clock: setting(CLOCK_KEY) == "true",
            background_colour: parse_background_colour(&setting(BACKGROUND_COLOUR_KEY)),
        }
    }
    fn save(&self, connection: &Connection) {
        for (key, value) in [
            (STYLE_KEY, self.style.key().to_owned()),
            (SOURCE_KEY, self.source.clone()),
            (DURATION_KEY, self.seconds.to_string()),
            (SHUFFLE_KEY, self.shuffle.to_string()),
            (EXCLUDE_PORTRAITS_KEY, self.exclude_portraits.to_string()),
            (SHUFFLE_TILES_KEY, self.shuffle_tiles.to_string()),
            (MOSAIC_TILES_KEY, self.mosaic_tiles.to_string()),
            (
                PICASA_TRANSITION_KEY,
                self.picasa_transition.key().to_string(),
            ),
            (TRANSITION_KEY, self.transition_ms.to_string()),
            (
                MOVEMENT_SPEED_KEY,
                format!("{:.2}", self.movement_speed.clamp(0.25, 3.0)),
            ),
            (CAPTION_KEY, self.captions.to_string()),
            (CLOCK_KEY, self.clock.to_string()),
            (
                BACKGROUND_COLOUR_KEY,
                self.background_colour
                    .map(|[r, g, b]| format!("#{r:02x}{g:02x}{b:02x}"))
                    .unwrap_or_default(),
            ),
        ] {
            if let Err(error) = db::set_setting(connection, key, &value) {
                eprintln!("PIC screensaver: could not save {key}: {error}");
            }
        }
    }
}

fn parse_background_colour(value: &str) -> Option<[u8; 3]> {
    let hex = value.strip_prefix('#')?;
    if hex.len() != 6 || !hex.is_ascii() {
        return None;
    }
    Some([
        u8::from_str_radix(&hex[0..2], 16).ok()?,
        u8::from_str_radix(&hex[2..4], 16).ok()?,
        u8::from_str_radix(&hex[4..6], 16).ok()?,
    ])
}

fn default_background_colour(style: Style, widget: &gtk::Widget) -> [u8; 3] {
    match style {
        Style::Flying => [230, 230, 224],
        Style::PhotoDrift => [235, 232, 224],
        Style::MatrixRain | Style::MatrixKenBurns => [0, 4, 1],
        Style::CoverFlow => crate::coverflow::lookup_theme_color(widget, "theme_bg_color")
            .or_else(|| crate::coverflow::lookup_theme_color(widget, "window_bg_color"))
            .map(|colour| {
                [
                    (colour.red() * 255.0).round() as u8,
                    (colour.green() * 255.0).round() as u8,
                    (colour.blue() * 255.0).round() as u8,
                ]
            })
            .unwrap_or(if adw::StyleManager::default().is_dark() {
                [31, 31, 31]
            } else {
                [247, 247, 247]
            }),
        _ => [9, 11, 16],
    }
}

#[derive(Clone)]
struct Slide {
    path: String,
    mtime: Option<i64>,
    size_bytes: Option<i64>,
    rotation: i32,
    edit_recipe: String,
    caption: String,
}
impl From<db::Photo> for Slide {
    fn from(photo: db::Photo) -> Self {
        let filename = crate::source::filename(&photo.path);
        let date = photo.taken_at.as_deref().unwrap_or_default();
        let date = date.get(..10).unwrap_or(date);
        let caption = if date.is_empty() {
            filename
        } else {
            format!("{filename}  ·  {date}")
        };
        Self {
            path: photo.path,
            mtime: photo.mtime,
            size_bytes: photo.size_bytes,
            rotation: photo.rotation,
            edit_recipe: photo.edit_recipe,
            caption,
        }
    }
}

/// Classify the edited image as the viewer presents it. Unknown metadata is
/// checked again after decode so it cannot put a portrait on screen.
fn displayed_is_portrait(
    width: Option<i64>,
    height: Option<i64>,
    rotation: i32,
    recipe: &str,
) -> Option<bool> {
    let mut width = u32::try_from(width?).ok().filter(|value| *value > 0)?;
    let mut height = u32::try_from(height?).ok().filter(|value| *value > 0)?;
    if rotation.rem_euclid(180) == 90 {
        std::mem::swap(&mut width, &mut height);
    }
    let recipe = edit::EditRecipe::decode(recipe);
    let (width, height) = edit::render::estimated_output_dimensions(width, height, &recipe);
    Some(height > width)
}

fn accept_orientation(pixels: Pixels, exclude_portraits: bool) -> Option<Pixels> {
    if exclude_portraits && pixels.height > pixels.width {
        None
    } else {
        Some(pixels)
    }
}

/// Read local time only when the displayed minute changes. The overlay is
/// updated from the GTK frame clock, with no extra GLib timer or network I/O.
fn local_clock_text() -> String {
    glib::DateTime::now_local()
        .ok()
        .and_then(|date| date.format("%H:%M").ok())
        .map(|text| text.to_string())
        .unwrap_or_else(|| "--:--".to_string())
}

fn random_seed() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64
}
fn shuffle<T>(items: &mut [T]) {
    let mut seed = random_seed() | 1;
    for i in (1..items.len()).rev() {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        items.swap(i, (seed as usize) % (i + 1));
    }
}

fn option_cell(label: &str, control: &impl IsA<gtk::Widget>) -> gtk::Box {
    let cell = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    let title = gtk::Label::new(Some(label));
    title.set_xalign(0.0);
    title.set_hexpand(true);
    control.set_halign(gtk::Align::End);
    control.set_hexpand(false);
    cell.append(&title);
    cell.append(control);
    cell
}

fn inspector_group(title: &str) -> (gtk::Box, gtk::Box) {
    let group = gtk::Box::new(gtk::Orientation::Vertical, 4);
    let heading = gtk::Label::new(Some(title));
    heading.set_xalign(0.0);
    heading.add_css_class("caption");
    heading.add_css_class("dim-label");
    let card = gtk::Box::new(gtk::Orientation::Vertical, 0);
    card.add_css_class("card");
    group.append(&heading);
    group.append(&card);
    (group, card)
}

fn relayout_inspector_group(card: &gtk::Box, rows: &[gtk::Widget], visible: &[bool]) -> bool {
    while let Some(child) = card.first_child() {
        card.remove(&child);
    }
    let mut shown = 0;
    for (row, show) in rows.iter().zip(visible.iter().copied()) {
        row.set_visible(show);
        if show {
            if shown > 0 {
                let separator = gtk::Separator::new(gtk::Orientation::Horizontal);
                separator.add_css_class("screensaver-inspector-separator");
                card.append(&separator);
            }
            row.set_margin_start(10);
            row.set_margin_end(10);
            row.set_margin_top(3);
            row.set_margin_bottom(3);
            card.append(row);
            shown += 1;
        }
    }
    shown > 0
}

fn option_flags(style: Style) -> [bool; 7] {
    let multi = !style.single_photo() && style != Style::MatrixRain;
    [
        !style.single_photo() && style != Style::Flying,
        style != Style::MatrixRain,
        style == Style::PhotoDrift,
        multi,
        style.uses_photos(),
        multi,
        style.uses_photos(),
    ]
}

fn transition_picker(initial: PicasaTransition) -> (gtk::Box, Rc<Cell<PicasaTransition>>) {
    let selected = Rc::new(Cell::new(initial));
    let group = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    group.add_css_class("linked");
    let choices = [
        (PicasaTransition::Flip, "Flip"),
        (PicasaTransition::Crossfade, "Fade"),
        (PicasaTransition::Cut, "Cut"),
    ];
    let buttons: Rc<RefCell<Vec<(PicasaTransition, gtk::ToggleButton)>>> =
        Rc::new(RefCell::new(Vec::new()));
    for (mode, label) in choices {
        let button = gtk::ToggleButton::with_label(label);
        button.set_active(mode == initial);
        let selected = Rc::clone(&selected);
        let buttons_for_click = Rc::clone(&buttons);
        button.connect_clicked(move |_| {
            selected.set(mode);
            for (candidate, button) in buttons_for_click.borrow().iter() {
                button.set_active(*candidate == mode);
            }
        });
        buttons.borrow_mut().push((mode, button.clone()));
        group.append(&button);
    }
    (group, selected)
}

/// Local vector illustrations scale into both the style list and the larger
/// preview without decoding photos or touching remote storage.
fn style_preview(style: Style, width: i32, height: i32) -> gtk::DrawingArea {
    let area = gtk::DrawingArea::new();
    area.set_content_width(width);
    area.set_content_height(height);
    area.set_hexpand(true);
    area.set_vexpand(true);
    area.set_draw_func(move |_, cr, actual_width, actual_height| {
        let _ = cr.save();
        cr.scale(actual_width as f64 / 66.0, actual_height as f64 / 42.0);
        let rect = |x: f64, y: f64, w: f64, h: f64, rgb: (f64, f64, f64)| {
            cr.set_source_rgb(rgb.0, rgb.1, rgb.2);
            cr.rectangle(x, y, w, h);
            let _ = cr.fill();
        };
        let photo = |x: f64, y: f64, w: f64, h: f64, tint: usize| {
            let sky = [
                (0.42, 0.68, 0.80),
                (0.78, 0.61, 0.47),
                (0.49, 0.66, 0.63),
                (0.57, 0.57, 0.79),
            ][tint % 4];
            let ground = [
                (0.25, 0.42, 0.38),
                (0.55, 0.42, 0.28),
                (0.29, 0.45, 0.37),
                (0.40, 0.38, 0.58),
            ][tint % 4];
            rect(x, y, w, h, sky);
            rect(x, y + h * 0.64, w, h * 0.36, ground);
            rect(
                x + w * 0.68,
                y + h * 0.16,
                w * 0.12,
                h * 0.13,
                (0.98, 0.86, 0.61),
            );
        };
        rect(0.0, 0.0, 66.0, 42.0, (0.12, 0.16, 0.21));
        match style {
            Style::Mosaic => {
                for row in 0..2 {
                    for col in 0..3 {
                        photo(
                            3.0 + col as f64 * 21.0,
                            3.0 + row as f64 * 19.0,
                            18.0,
                            16.0,
                            row * 3 + col,
                        );
                    }
                }
            }
            Style::Grid => {
                for row in 0..2 {
                    for col in 0..3 {
                        photo(
                            col as f64 * 22.0,
                            row as f64 * 21.0,
                            21.0,
                            20.0,
                            row * 3 + col,
                        );
                    }
                }
            }
            Style::Floating => {
                for (x, y, tilt, tint) in [
                    (6., 4., -0.09, 0),
                    (36., 3., 0.08, 1),
                    (3., 22., 0.10, 2),
                    (32., 21., -0.1, 3),
                ] {
                    let _ = cr.save();
                    cr.translate(x + 13., y + 9.);
                    cr.rotate(tilt);
                    rect(-13., -9., 26., 18., (0.95, 0.95, 0.92));
                    photo(-11., -7., 22., 14., tint);
                    let _ = cr.restore();
                }
            }
            Style::PicturePile => {
                for (i, angle, x) in [(0, -0.15, 11.0), (1, 0.13, 23.0), (2, -0.08, 35.0)] {
                    let _ = cr.save();
                    cr.translate(x + 9.0, 20.0);
                    cr.rotate(angle);
                    rect(-10.0, -15.0, 22.0, 31.0, (0.95, 0.94, 0.89));
                    photo(-8.0, -13.0, 18.0, 22.0, i);
                    let _ = cr.restore();
                }
            }
            Style::Fade => {
                photo(2., 0., 62., 42., 2);
                rect(9., 6., 48., 30., (0.22, 0.27, 0.33));
                photo(11., 8., 44., 26., 0);
            }
            Style::IPhoto => {
                rect(5.0, 4.0, 56.0, 34.0, (0.84, 0.85, 0.83));
                photo(7.0, 6.0, 52.0, 30.0, 0);
                cr.set_source_rgba(1.0, 1.0, 1.0, 0.84);
                cr.set_line_width(1.0);
                cr.rectangle(13.5, 9.5, 39.0, 23.0);
                let _ = cr.stroke();
            }
            Style::CoverFlow => {
                for (x, y, width, tint) in [
                    (5.0, 8.0, 17.0, 2),
                    (22.0, 4.0, 25.0, 0),
                    (47.0, 8.0, 14.0, 1),
                ] {
                    photo(x, y, width, 24.0, tint);
                    cr.set_source_rgba(0.7, 0.8, 0.9, 0.18);
                    cr.rectangle(x, y + 25.0, width, 7.0);
                    let _ = cr.fill();
                }
            }
            Style::MatrixRain | Style::MatrixKenBurns => {
                rect(0., 0., 66., 42., (0.0, 0.025, 0.008));
                let layout = pangocairo::functions::create_layout(cr);
                layout.set_font_description(Some(&gtk::pango::FontDescription::from_string(
                    "Sans 7",
                )));
                for col in 0..11 {
                    for row in 0..5 {
                        let bright = ((col * 3 + row * 7) % 9) as f64 / 10.0;
                        cr.set_source_rgb(0.02, 0.22 + bright * 0.7, 0.08);
                        cr.move_to(col as f64 * 6., row as f64 * 8. + 7.);
                        layout.set_text(
                            &matrix::KATAKANA[(col * 7 + row * 13) % matrix::KATAKANA.len()]
                                .to_string(),
                        );
                        pangocairo::functions::show_layout(cr, &layout);
                    }
                }
                if style == Style::MatrixKenBurns {
                    cr.set_source_rgb(0.4, 1., 0.6);
                    cr.rectangle(12., 8., 42., 26.);
                    let _ = cr.stroke();
                }
            }
            Style::Flying => {
                // Perspective, miniaturised: the vanishing point in the middle,
                // prints growing as they approach and drifting out to the edges.
                cr.set_source_rgba(1.0, 1.0, 1.0, 0.12);
                cr.set_line_width(0.8);
                for (dx, dy) in [(-26., -14.), (26., -12.), (-24., 15.), (24., 14.)] {
                    cr.move_to(33.0, 21.0);
                    cr.line_to(33.0 + dx, 21.0 + dy);
                    let _ = cr.stroke();
                }
                photo(31.5, 19.5, 5.0, 3.4, 2);
                photo(14.0, 7.0, 13.0, 9.0, 0);
                photo(40.0, 24.0, 22.0, 15.0, 1);
            }
            Style::PhotoDrift => {
                photo(5.0, 6.0, 9.0, 7.0, 1);
                photo(20.0, 14.0, 14.0, 11.0, 0);
                photo(43.0, 5.0, 8.0, 6.0, 2);
                photo(39.0, 25.0, 18.0, 13.0, 1);
            }
            Style::Surprise => {
                photo(4.0, 5.0, 24.0, 15.0, 1);
                photo(4.0, 22.0, 24.0, 15.0, 2);
                let _ = cr.save();
                cr.translate(45.0, 21.0);
                cr.rotate(0.15);
                rect(-13.0, -17.0, 27.0, 34.0, (0.95, 0.94, 0.89));
                photo(-11.0, -15.0, 23.0, 27.0, 3);
                let _ = cr.restore();
            }
        }
        let _ = cr.restore();
    });
    area
}

/// Five equal-width tiles keep the ten regular styles visible without scrolling.
fn style_picker(
    initial: Style,
    terminal: bool,
    on_change: impl Fn(Style) + 'static,
) -> (gtk::Grid, Rc<Cell<Style>>) {
    let chosen = Rc::new(Cell::new(initial));
    let cards = gtk::Grid::new();
    cards.set_column_homogeneous(true);
    cards.set_row_spacing(6);
    cards.set_column_spacing(6);
    let buttons: Rc<RefCell<Vec<(Style, gtk::ToggleButton)>>> = Rc::new(RefCell::new(Vec::new()));
    let on_change: Rc<dyn Fn(Style)> = Rc::new(on_change);
    for (index, candidate) in Style::CHOICES
        .into_iter()
        .filter(|style| style.available(terminal))
        .enumerate()
    {
        let card = gtk::ToggleButton::new();
        card.set_hexpand(true);
        card.set_size_request(82, 70);
        card.set_active(candidate == initial);
        card.set_tooltip_text(Some(candidate.subtitle()));
        card.add_css_class("screensaver-style-tile");
        let content = gtk::Box::new(gtk::Orientation::Vertical, 2);
        content.set_halign(gtk::Align::Center);
        content.append(&style_preview(candidate, 46, 28));
        let label = candidate.label();
        let tile_label = match label.split_once(' ') {
            Some((first, rest)) => format!("{first}\n{rest}"),
            None => label.to_owned(),
        };
        let title = gtk::Label::new(Some(&tile_label));
        title.set_justify(gtk::Justification::Center);
        title.add_css_class("caption");
        content.append(&title);
        card.set_child(Some(&content));
        buttons.borrow_mut().push((candidate, card.clone()));
        let buttons_for_click = Rc::clone(&buttons);
        let chosen = Rc::clone(&chosen);
        let on_change = Rc::clone(&on_change);
        card.connect_clicked(move |_| {
            chosen.set(candidate);
            for (style, button) in buttons_for_click.borrow().iter() {
                button.set_active(*style == candidate);
            }
            on_change(candidate);
        });
        cards.attach(&card, (index % 5) as i32, (index / 5) as i32, 1, 1);
    }
    (cards, chosen)
}

/// Opens the settings/launch dialog without changing PIC's current viewer.
pub fn open(
    parent: &adw::ApplicationWindow,
    connection: Rc<RefCell<Connection>>,
    selected_ids: Vec<i64>,
    theme_id: &str,
) {
    let terminal = theme_id.eq_ignore_ascii_case("Terminal");
    let mut current = Preferences::load(&connection.borrow());
    if terminal {
        if let Ok(Some(saved)) = db::setting(&connection.borrow(), TERMINAL_STYLE_KEY) {
            current.style = Style::from_saved(&saved);
        }
    }
    current.style = current.style.for_theme(terminal);
    let mut sources = vec![
        ("All Photos".to_owned(), Source::All),
        ("Favourites".to_owned(), Source::Favorites),
    ];
    if !selected_ids.is_empty() {
        sources.insert(
            0,
            (
                format!("Selected Photos ({})", selected_ids.len()),
                Source::Selected(selected_ids),
            ),
        );
    }
    if let Ok(albums) = db::albums(&connection.borrow()) {
        sources.extend(
            albums
                .into_iter()
                .map(|album| (format!("Album · {}", album.name), Source::Album(album.id))),
        );
    }
    if let Ok(folders) = db::folders(&connection.borrow()) {
        sources.extend(folders.into_iter().map(|folder| {
            (
                format!("Folder · {}", folder.name),
                Source::Folder(folder.id),
            )
        }));
    }

    let dialog = gtk::Window::builder()
        .title("PIC Screensaver")
        .default_width(920)
        .default_height(600)
        .resizable(false)
        .modal(true)
        .transient_for(parent)
        .build();
    dialog.add_css_class("screensaver-settings-window");
    let body = gtk::Box::new(gtk::Orientation::Vertical, 10);
    body.set_margin_top(16);
    body.set_margin_bottom(16);
    body.set_margin_start(16);
    body.set_margin_end(16);

    let heading = gtk::Label::new(Some("Screensaver"));
    heading.add_css_class("title-2");
    heading.add_css_class("screensaver-dialog-heading");
    heading.set_xalign(0.0);
    body.append(&heading);

    let source_names: Vec<String> = sources.iter().map(|(name, _)| name.clone()).collect();
    let names: Vec<&str> = source_names.iter().map(String::as_str).collect();
    let source = gtk::DropDown::from_strings(&names);
    // Explicit selections default to precisely those images.
    let selected = if matches!(
        sources.first().map(|(_, kind)| kind),
        Some(Source::Selected(_))
    ) {
        0
    } else {
        sources
            .iter()
            .position(|(_, kind)| kind.key() == current.source)
            .unwrap_or(0)
    };
    source.set_selected(selected as u32);

    let interval_scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, 3.0, 30.0, 1.0);
    interval_scale.set_value(current.seconds as f64);
    interval_scale.set_digits(0);
    interval_scale.set_draw_value(false);
    interval_scale.add_mark(6.0, gtk::PositionType::Bottom, None);
    interval_scale.set_hexpand(true);
    interval_scale.set_width_request(110);
    let interval_value = gtk::Label::new(None);
    interval_value.set_width_chars(4);
    interval_value.add_css_class("dim-label");
    let interval_control = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    interval_control.append(&interval_scale);
    interval_control.append(&interval_value);
    let source_row = option_cell("Photos", &source);
    let interval_row = option_cell("Change every", &interval_control);

    let (transition_control, transition_selected) = transition_picker(current.picasa_transition);
    transition_control.set_tooltip_text(Some("Tile transition style"));
    let transition_row = option_cell("Transition", &transition_control);

    let transition_time = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.2, 2.5, 0.1);
    transition_time.set_value(current.transition_ms as f64 / 1000.0);
    transition_time.set_digits(1);
    transition_time.set_draw_value(false);
    transition_time.add_mark(0.9, gtk::PositionType::Bottom, None);
    transition_time.set_width_request(110);
    let transition_time_value = gtk::Label::new(None);
    transition_time_value.set_width_chars(5);
    transition_time_value.add_css_class("dim-label");
    let transition_time_control = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    transition_time_control.append(&transition_time);
    transition_time_control.append(&transition_time_value);
    let animation_time = option_cell("Duration", &transition_time_control);

    let movement_speed = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.25, 3.0, 0.25);
    movement_speed.set_value(current.movement_speed);
    movement_speed.set_digits(2);
    movement_speed.set_draw_value(false);
    movement_speed.add_mark(1.0, gtk::PositionType::Bottom, None);
    movement_speed.set_width_request(110);
    let movement_speed_value = gtk::Label::new(None);
    movement_speed_value.set_width_chars(5);
    movement_speed_value.add_css_class("dim-label");
    let movement_control = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    movement_control.append(&movement_speed);
    movement_control.append(&movement_speed_value);
    let movement_row = option_cell("Speed", &movement_control);

    let mosaic_tiles = gtk::Scale::with_range(
        gtk::Orientation::Horizontal,
        1.0,
        MAX_WALL_TILES as f64,
        1.0,
    );
    mosaic_tiles.set_value(current.mosaic_tiles as f64);
    mosaic_tiles.set_digits(0);
    mosaic_tiles.set_draw_value(false);
    mosaic_tiles.set_width_request(94);
    let mosaic_tiles_value = gtk::SpinButton::new(Some(&mosaic_tiles.adjustment()), 1.0, 0);
    mosaic_tiles_value.set_width_chars(5);
    mosaic_tiles_value.set_tooltip_text(Some("Enter an exact tile count"));
    let tiles_control = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    tiles_control.append(&mosaic_tiles);
    tiles_control.append(&mosaic_tiles_value);
    let tiles_row = option_cell("Tiles", &tiles_control);
    mosaic_tiles.set_tooltip_text(Some(
        "Photos repeat when needed. Mosaic and Changing Grid support up to 10,000 tiles; animated print styles have smaller limits.",
    ));

    let captions_switch = gtk::Switch::new();
    captions_switch.set_active(current.captions);
    captions_switch.set_tooltip_text(Some("Show photo filename and date from the library"));
    let captions_row = option_cell("Captions", &captions_switch);
    let clock_switch = gtk::Switch::new();
    clock_switch.set_active(current.clock);
    clock_switch.set_tooltip_text(Some("Show a discreet local-time clock"));
    let clock_row = option_cell("Clock", &clock_switch);
    let background_colour = Rc::new(Cell::new(current.background_colour));
    let colour_style = Rc::new(Cell::new(current.style));
    let colour_button = gtk::Button::from_icon_name("color-select-symbolic");
    colour_button.add_css_class("screensaver-background-swatch");
    let control_size = gtk::SizeGroup::new(gtk::SizeGroupMode::Both);
    control_size.add_widget(&clock_switch);
    control_size.add_widget(&colour_button);
    colour_button.set_cursor_from_name(Some("pointer"));
    colour_button.set_tooltip_text(Some("Choose background colour"));
    let colour_css = gtk::CssProvider::new();
    #[allow(deprecated)]
    colour_button
        .style_context()
        .add_provider(&colour_css, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 1);
    let update_colour_swatch: Rc<dyn Fn()> = {
        let selected = Rc::clone(&background_colour);
        let style = Rc::clone(&colour_style);
        let button = colour_button.clone();
        Rc::new(move || {
            let [r, g, b] = selected
                .get()
                .unwrap_or_else(|| default_background_colour(style.get(), button.upcast_ref()));
            let ink = if 299 * r as u32 + 587 * g as u32 + 114 * b as u32 >= 128_000 {
                "#111111"
            } else {
                "#ffffff"
            };
            colour_css.load_from_string(&format!("button.screensaver-background-swatch {{ background-color: #{r:02x}{g:02x}{b:02x}; color: {ink}; }}"));
        })
    };
    let background_row = option_cell("Background colour", &colour_button);
    {
        let background_colour = Rc::clone(&background_colour);
        let colour_style = Rc::clone(&colour_style);
        let update_colour_swatch = Rc::clone(&update_colour_swatch);
        let dialog = dialog.downgrade();
        colour_button.connect_clicked(move |_| {
            let Some(parent) = dialog.upgrade() else {
                return;
            };
            let [r, g, b] = background_colour.get().unwrap_or_else(|| {
                default_background_colour(colour_style.get(), parent.upcast_ref())
            });
            let initial =
                gtk::gdk::RGBA::new(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0);
            let selected = Rc::clone(&background_colour);
            let update_colour_swatch = Rc::clone(&update_colour_swatch);
            let chooser = gtk::ColorDialog::new();
            chooser.set_with_alpha(false);
            chooser.choose_rgba(
                Some(&parent),
                Some(&initial),
                None::<&gtk::gio::Cancellable>,
                move |result| {
                    if let Ok(color) = result {
                        selected.set(Some([
                            (color.red() * 255.0).round() as u8,
                            (color.green() * 255.0).round() as u8,
                            (color.blue() * 255.0).round() as u8,
                        ]));
                        update_colour_swatch();
                    }
                },
            );
        });
    }
    let shuffle_photos_switch = gtk::Switch::new();
    shuffle_photos_switch.set_active(current.shuffle);
    let shuffle_photos_row = option_cell("Shuffle photos", &shuffle_photos_switch);
    let shuffle_tiles_switch = gtk::Switch::new();
    shuffle_tiles_switch.set_active(current.shuffle_tiles);
    shuffle_tiles_switch.set_tooltip_text(Some("Change tile positions in a shuffled order"));
    let shuffle_tiles_row = option_cell("Shuffle tiles", &shuffle_tiles_switch);
    let exclude_portraits_switch = gtk::Switch::new();
    exclude_portraits_switch.set_active(current.exclude_portraits);
    exclude_portraits_switch.set_tooltip_text(Some(
        "Skip photos taller than they are wide after rotation and crop",
    ));
    let exclude_portraits_row = option_cell("Skip portraits", &exclude_portraits_switch);

    let (source_group, source_card) = inspector_group("Source");
    let (animation_group, animation_card) = inspector_group("Animation");
    let (display_group, display_card) = inspector_group("Display");
    let source_rows = vec![source_row.clone().upcast(), interval_row.clone().upcast()];
    let animation_rows = vec![
        transition_row.clone().upcast(),
        animation_time.clone().upcast(),
        movement_row.clone().upcast(),
        tiles_row.clone().upcast(),
    ];
    let display_rows = vec![
        captions_row.clone().upcast(),
        clock_row.clone().upcast(),
        shuffle_photos_row.clone().upcast(),
        shuffle_tiles_row.clone().upcast(),
        exclude_portraits_row.clone().upcast(),
        background_row.clone().upcast(),
    ];
    let source_card_for_layout = source_card.clone();
    let source_group_for_layout = source_group.clone();
    let animation_card_for_layout = animation_card.clone();
    let animation_group_for_layout = animation_group.clone();
    let display_card_for_layout = display_card.clone();
    let display_group_for_layout = display_group.clone();
    let source_rows_for_layout = source_rows.clone();
    let animation_rows_for_layout = animation_rows.clone();
    let display_rows_for_layout = display_rows.clone();
    let relayout: Rc<dyn Fn(Style)> = Rc::new(move |style| {
        let flags = option_flags(style);
        let has_photos = style.uses_photos();
        let source_visible = relayout_inspector_group(
            &source_card_for_layout,
            &source_rows_for_layout,
            &[has_photos, has_photos],
        );
        source_group_for_layout.set_visible(source_visible);
        let animation_visible = relayout_inspector_group(
            &animation_card_for_layout,
            &animation_rows_for_layout,
            &flags[..4],
        );
        animation_group_for_layout.set_visible(animation_visible);
        let display_visible = relayout_inspector_group(
            &display_card_for_layout,
            &display_rows_for_layout,
            &[has_photos, true, flags[4], flags[5], flags[6], true],
        );
        display_group_for_layout.set_visible(display_visible);
    });

    let preview_frame = gtk::Frame::new(None);
    preview_frame.add_css_class("card");
    preview_frame.set_hexpand(true);
    let preview = gtk::Box::new(gtk::Orientation::Vertical, 0);
    preview.set_hexpand(true);
    preview.set_vexpand(true);
    preview.append(&style_preview(current.style, 520, 230));
    preview_frame.set_child(Some(&preview));
    let selected_title = gtk::Label::new(Some(current.style.label()));
    selected_title.add_css_class("title-3");
    selected_title.set_xalign(0.0);
    let selected_subtitle = gtk::Label::new(Some(current.style.subtitle()));
    selected_subtitle.add_css_class("dim-label");
    selected_subtitle.set_xalign(0.0);

    let style_heading = gtk::Label::new(Some("Styles"));
    style_heading.set_xalign(0.0);
    style_heading.add_css_class("caption");
    style_heading.add_css_class("dim-label");
    let gallery = gtk::Box::new(gtk::Orientation::Vertical, 5);
    gallery.append(&style_heading);

    let visual_column = gtk::Box::new(gtk::Orientation::Vertical, 8);
    visual_column.set_hexpand(true);
    visual_column.append(&preview_frame);
    visual_column.append(&selected_title);
    visual_column.append(&selected_subtitle);
    visual_column.append(&gallery);
    let inspector = gtk::Box::new(gtk::Orientation::Vertical, 9);
    inspector.set_width_request(300);
    inspector.append(&source_group);
    inspector.append(&animation_group);
    inspector.append(&display_group);

    let main_row = gtk::Box::new(gtk::Orientation::Horizontal, 18);
    main_row.set_vexpand(true);
    main_row.append(&visual_column);
    main_row.append(&inspector);
    body.append(&main_row);

    let preview_for_style = preview.clone();
    let title_for_style = selected_title.clone();
    let subtitle_for_style = selected_subtitle.clone();
    let relayout_for_style = Rc::clone(&relayout);
    let tiles_for_style = mosaic_tiles.clone();
    let update_style: Rc<dyn Fn(Style)> = Rc::new(move |style| {
        let style_changed = colour_style.get() != style;
        let limit = match style {
            Style::PicturePile | Style::Floating | Style::PhotoDrift => PRINT_MAX_SLOTS,
            Style::Flying => FLY_MAX_SLOTS,
            _ => MAX_WALL_TILES,
        };
        tiles_for_style.set_range(1.0, limit as f64);
        tiles_for_style.clear_marks();
        tiles_for_style.add_mark(
            style.default_tiles() as f64,
            gtk::PositionType::Bottom,
            None,
        );
        if style_changed {
            tiles_for_style.set_value(style.default_tiles() as f64);
        }
        colour_style.set(style);
        update_colour_swatch();
        if let Some(old) = preview_for_style.first_child() {
            preview_for_style.remove(&old);
        }
        preview_for_style.append(&style_preview(style, 520, 230));
        title_for_style.set_text(style.label());
        subtitle_for_style.set_text(style.subtitle());
        relayout_for_style(style);
    });
    let update_for_picker = Rc::clone(&update_style);
    let (style_picker, style_selected) = style_picker(current.style, terminal, move |style| {
        update_for_picker(style);
    });
    gallery.append(&style_picker);
    update_style(current.style);

    let notice = gtk::Label::new(None);
    notice.set_visible(false);
    notice.set_wrap(true);
    notice.set_xalign(0.0);
    notice.add_css_class("dim-label");
    body.append(&notice);

    let footer = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    footer.set_margin_top(6);
    let summary = gtk::Label::new(None);
    summary.set_xalign(0.0);
    summary.set_hexpand(true);
    summary.add_css_class("dim-label");
    let cancel = gtk::Button::with_label("Cancel");
    let reset = gtk::Button::with_label("Reset");
    let start = gtk::Button::with_label("Start");
    start.add_css_class("suggested-action");
    footer.append(&summary);
    footer.append(&cancel);
    footer.append(&reset);
    footer.append(&start);
    body.append(&footer);
    dialog.set_child(Some(&body));
    let update_summary = {
        let summary = summary.clone();
        let source_names = source_names.clone();
        let source = source.clone();
        let interval_scale = interval_scale.clone();
        Rc::new(move || {
            let source_name = source_names
                .get(source.selected() as usize)
                .map(String::as_str)
                .unwrap_or("Photos");
            let interval = interval_scale.value().round() as u32;
            summary.set_text(&format!("{source_name} · every {interval} s"));
        }) as Rc<dyn Fn()>
    };
    update_summary();
    let summary_for_source = Rc::clone(&update_summary);
    source.connect_selected_notify(move |_| summary_for_source());
    let interval_value_for_change = interval_value.clone();
    let summary_for_interval = Rc::clone(&update_summary);
    interval_scale.connect_value_changed(move |scale| {
        interval_value_for_change.set_text(&format!("{} s", scale.value().round() as u32));
        summary_for_interval();
    });
    interval_value.set_text(&format!("{} s", current.seconds));
    let fade_value_for_change = transition_time_value.clone();
    transition_time.connect_value_changed(move |scale| {
        fade_value_for_change.set_text(&format!("{:.1} s", scale.value()));
    });
    let speed_value_for_change = movement_speed_value.clone();
    movement_speed.connect_value_changed(move |scale| {
        speed_value_for_change.set_text(&format!("{:.2}", scale.value()));
    });
    transition_time_value.set_text(&format!("{:.1} s", current.transition_ms as f64 / 1000.0));
    movement_speed_value.set_text(&format!("{:.2}", current.movement_speed));

    {
        let source = source.clone();
        let interval_scale = interval_scale.clone();
        let transition_time = transition_time.clone();
        let movement_speed = movement_speed.clone();
        let mosaic_tiles = mosaic_tiles.clone();
        let captions_switch = captions_switch.clone();
        let clock_switch = clock_switch.clone();
        let shuffle_photos_switch = shuffle_photos_switch.clone();
        let shuffle_tiles_switch = shuffle_tiles_switch.clone();
        let exclude_portraits_switch = exclude_portraits_switch.clone();
        let background_colour = Rc::clone(&background_colour);
        let default_style = style_picker
            .child_at(0, 0)
            .unwrap()
            .downcast::<gtk::ToggleButton>()
            .unwrap();
        let default_transition = transition_control
            .first_child()
            .unwrap()
            .next_sibling()
            .unwrap()
            .downcast::<gtk::ToggleButton>()
            .unwrap();
        let notice = notice.clone();
        reset.connect_clicked(move |_| {
            background_colour.set(None);
            default_style.emit_clicked();
            default_transition.emit_clicked();
            source.set_selected(0);
            interval_scale.set_value(6.0);
            transition_time.set_value(0.9);
            movement_speed.set_value(1.0);
            mosaic_tiles.set_value(DEFAULT_MOSAIC_TILES as f64);
            captions_switch.set_active(false);
            clock_switch.set_active(false);
            shuffle_photos_switch.set_active(true);
            shuffle_tiles_switch.set_active(true);
            exclude_portraits_switch.set_active(false);
            notice.set_visible(false);
        });
    }

    let dialog_for_cancel = dialog.downgrade();
    cancel.connect_clicked(move |_| {
        if let Some(dialog) = dialog_for_cancel.upgrade() {
            dialog.close();
        }
    });

    let parent = parent.clone();
    let dialog_for_start = dialog.downgrade();
    start.connect_clicked(move |_| {
        let source_index = source.selected() as usize;
        let Some((_, photo_source)) = sources.get(source_index) else {
            return;
        };
        let requested_tiles = if style_selected.get().uses_photos() {
            mosaic_tiles.value().round().max(1.0) as usize
        } else {
            DEFAULT_MOSAIC_TILES
        };
        let prefs = Preferences {
            style: style_selected.get(),
            source: photo_source.key(),
            seconds: (interval_scale.value().round() as u32).clamp(3, 30),
            shuffle: shuffle_photos_switch.is_active(),
            exclude_portraits: exclude_portraits_switch.is_active(),
            shuffle_tiles: shuffle_tiles_switch.is_active(),
            mosaic_tiles: requested_tiles,
            picasa_transition: transition_selected.get(),
            transition_ms: (transition_time.value() * 1000.0).round() as u32,
            movement_speed: movement_speed.value().clamp(0.25, 3.0),
            captions: captions_switch.is_active(),
            clock: clock_switch.is_active(),
            background_colour: background_colour.get(),
        };
        if !prefs.style.uses_photos() {
            prefs.save(&connection.borrow());
            if terminal {
                if let Err(error) = db::set_setting(&connection.borrow(), TERMINAL_STYLE_KEY, prefs.style.key()) {
                    eprintln!("PIC screensaver: could not remember Terminal style: {error}");
                }
            }
            if let Some(dialog) = dialog_for_start.upgrade() { dialog.close(); }
            start_screensaver(&parent, Vec::new(), prefs);
            return;
        }
        let mut photos = match photo_source.photos(&connection.borrow()) {
            Ok(photos) => photos,
            Err(error) => {
                notice.set_text(&format!("Could not load photos: {error}"));
                notice.set_visible(true);
                return;
            }
        };
        if prefs.exclude_portraits {
            photos.retain(|photo| displayed_is_portrait(
                photo.width, photo.height, photo.rotation, &photo.edit_recipe,
            ) != Some(true));
        }
        if photos.is_empty() {
            notice.set_text(if prefs.exclude_portraits {
                "No landscape or square photos in this collection. Choose another source or turn off Exclude portrait photos."
            } else {
                "This collection has no photos. Choose another source."
            });
            notice.set_visible(true);
            return;
        }
        prefs.save(&connection.borrow());
        if terminal {
            if let Err(error) = db::set_setting(&connection.borrow(), TERMINAL_STYLE_KEY, prefs.style.key()) {
                eprintln!("PIC screensaver: could not remember Terminal style: {error}");
            }
        }
        if prefs.shuffle {
            shuffle(&mut photos);
        }
        let slides = photos.into_iter().map(Slide::from).collect();
        if let Some(dialog) = dialog_for_start.upgrade() {
            dialog.close();
        }
        start_screensaver(&parent, slides, prefs);
    });
    dialog.present();
}

struct Pixels {
    width: i32,
    height: i32,
    data: Vec<u8>,
}
struct ImageResult {
    index: usize,
    high_quality: bool,
    pixels: Option<Pixels>,
}
fn texture_pixels(image: image::RgbaImage) -> Pixels {
    let (width, height) = image.dimensions();
    Pixels {
        width: width as i32,
        height: height as i32,
        data: image.into_raw(),
    }
}

/// Decode the already-generated local cache without touching the original.
/// Remote NFS/SMB reads MUST NOT run on this worker: a reachable server with
/// missing exports can block source reads indefinitely.
fn decode_cached_preview(slide: &Slide) -> Option<Pixels> {
    let path = thumbnail::existing_cache_path(&slide.path, slide.mtime, slide.size_bytes)
        .ok()
        .flatten()?;
    let recipe = edit::EditRecipe::decode(&slide.edit_recipe);
    edit::render::render_thumbnail(path.to_str()?, slide.rotation, &recipe)
        .ok()
        .map(texture_pixels)
}

fn decode_original(
    slide: &Slide,
    dimensions: (u32, u32),
    cancelled: &AtomicBool,
) -> anyhow::Result<Pixels> {
    let decoded = thumbnail::decode_for_viewer_with_cancel(
        &slide.path,
        dimensions.0,
        dimensions.1,
        None,
        || cancelled.load(Ordering::Relaxed),
    )?;
    anyhow::ensure!(
        !cancelled.load(Ordering::Relaxed),
        "screensaver decode cancelled"
    );
    let recipe = edit::EditRecipe::decode(&slide.edit_recipe);
    Ok(texture_pixels(edit::render::apply_recipe(
        edit::render::apply_library_rotation(decoded, slide.rotation),
        &recipe,
    )))
}

/// Keep cached previews responsive even when an in-flight remote RAW read is
/// hung. Original upgrades run on separate local and network workers.
fn start_decode_worker(
    slides: Vec<Slide>,
    style: Style,
    mosaic_tiles: usize,
    exclude_portraits: bool,
    requests: Receiver<usize>,
    results: mpsc::SyncSender<ImageResult>,
    cancelled: Arc<AtomicBool>,
) {
    let slides = Arc::new(slides);
    let (local_tx, local_rx) = mpsc::channel::<usize>();
    let (remote_tx, remote_rx) = mpsc::channel::<usize>();

    for (receiver, is_remote) in [(local_rx, false), (remote_rx, true)] {
        let slides = Arc::clone(&slides);
        let results = results.clone();
        let cancelled = Arc::clone(&cancelled);
        std::thread::spawn(move || {
            while let Ok(index) = receiver.recv() {
                if cancelled.load(Ordering::Relaxed) {
                    break;
                }
                let Some(photo) = slides.get(index) else {
                    continue;
                };
                let dimensions = match style {
                    Style::Mosaic | Style::Grid => {
                        let edge = if mosaic_tiles <= 6 {
                            1536
                        } else if mosaic_tiles <= 12 {
                            1152
                        } else if mosaic_tiles <= 30 {
                            896
                        } else if mosaic_tiles <= 100 {
                            640
                        } else if mosaic_tiles <= 400 {
                            384
                        } else if mosaic_tiles <= 1600 {
                            192
                        } else if mosaic_tiles <= 6400 {
                            96
                        } else {
                            48
                        };
                        (edge, edge)
                    }
                    Style::PicturePile | Style::Floating | Style::PhotoDrift => {
                        // Prints are small when there are many of them.
                        if mosaic_tiles <= 6 {
                            (1920, 1280)
                        } else if mosaic_tiles <= 16 {
                            (1280, 860)
                        } else if mosaic_tiles <= 36 {
                            (960, 640)
                        } else {
                            (640, 430)
                        }
                    }
                    // Cover Flow shows a few large cards on screen, so decode
                    // source photos at display resolution instead of using
                    // the low-resolution preview path.
                    Style::CoverFlow => (2560, 1600),
                    // Flying prints grow until they pass the lens, so they are
                    // decoded close to display size. The flight field is capped,
                    // which keeps the number of live textures bounded.
                    Style::Flying => {
                        if mosaic_tiles <= 6 {
                            (1920, 1280)
                        } else {
                            (1440, 960)
                        }
                    }
                    _ => (2560, 1600),
                };
                let started = Instant::now();
                // Only the original worker may probe a source. In particular,
                // a local-looking path can itself be a stalled NFS mount.
                let decoded = if crate::source::cached_file_available(&photo.path) {
                    decode_original(photo, dimensions, &cancelled)
                } else {
                    Err(anyhow::anyhow!("original is currently offline"))
                };
                if cancelled.load(Ordering::Relaxed) {
                    break;
                }
                if std::env::var_os("PIC_SCREENSAVER_TRACE").is_some() {
                    eprintln!(
                        "PIC_SCREEN decode kind=original network={is_remote} index={index} ok={} ms={}",
                        decoded.is_ok(), started.elapsed().as_millis(),
                    );
                }
                if let Err(error) = &decoded {
                    eprintln!(
                        "PIC screensaver: original unavailable {}: {error}",
                        photo.path
                    );
                }
                if results
                    .send(ImageResult {
                        index,
                        high_quality: true,
                        pixels: decoded
                            .ok()
                            .and_then(|pixels| accept_orientation(pixels, exclude_portraits))
                            .map(|pixels| matrix::prepare(pixels, style)),
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
    }

    // Other styles can show a cached preview while the original loads. Cover
    // Flow skips that path so its large cards are always made from source
    // photos; the original worker remains responsible for source access.
    std::thread::spawn(move || {
        while let Ok(index) = requests.recv() {
            if cancelled.load(Ordering::Relaxed) {
                break;
            }
            let Some(photo) = slides.get(index) else {
                continue;
            };
            if style != Style::CoverFlow {
                let started = Instant::now();
                let cached = decode_cached_preview(photo);
                if cancelled.load(Ordering::Relaxed) {
                    break;
                }
                if std::env::var_os("PIC_SCREENSAVER_TRACE").is_some() {
                    eprintln!(
                        "PIC_SCREEN decode kind=cache index={index} hit={} ms={}",
                        cached.is_some(),
                        started.elapsed().as_millis(),
                    );
                }
                if results
                    .send(ImageResult {
                        index,
                        high_quality: false,
                        pixels: cached
                            .and_then(|pixels| accept_orientation(pixels, exclude_portraits))
                            .map(|pixels| matrix::prepare(pixels, style)),
                    })
                    .is_err()
                {
                    break;
                }
            }

            // Source availability checks belong to the other workers, not
            // this cache-only path (even mount-backed local paths may block).
            if photo.path.contains("://") {
                let _ = remote_tx.send(index);
            } else {
                let _ = local_tx.send(index);
            }
        }
    });
}

struct Surface {
    texture: gtk::gdk::Texture,
    high_quality: bool,
    revision: u64,
}
struct Player {
    style: Style,
    count: usize,
    interval: Duration,
    movement_speed: f64,
    transition: Duration,
    switched_at: Instant,
    started: bool,
    paused_at: Option<Instant>,
    next: usize,
    current: usize,
    previous: Option<usize>,
    slots: Vec<usize>,
    fading_slot: Option<(usize, usize)>,
    changes: usize,
    shuffle_tiles: bool,
    tile_order: Vec<usize>,
    preload_cursor: usize,
    backfill_cursor: usize,
    backfill_pending: HashSet<usize>,
    frames: HashMap<usize, Surface>,
    order: VecDeque<usize>,
    requested: HashSet<usize>,
    failed: HashSet<usize>,
    next_revision: u64,
}
impl Player {
    fn new(
        style: Style,
        count: usize,
        seconds: u32,
        shuffle_tiles: bool,
        mosaic_tiles: usize,
    ) -> Self {
        let desired_slots = match style {
            Style::Mosaic | Style::Grid => mosaic_tiles.clamp(1, MAX_WALL_TILES),
            Style::PicturePile | Style::Floating | Style::PhotoDrift => {
                mosaic_tiles.clamp(1, PRINT_MAX_SLOTS)
            }
            Style::Flying => mosaic_tiles.clamp(1, FLY_MAX_SLOTS),
            _ => 0,
        };
        let slots = if count == 0 {
            Vec::new()
        } else {
            (0..desired_slots)
                .map(|slot| slot % count)
                .collect::<Vec<_>>()
        };
        let mut tile_order = (0..slots.len()).collect::<Vec<_>>();
        if shuffle_tiles {
            shuffle(&mut tile_order);
        }
        Self {
            style,
            count,
            interval: Duration::from_secs(seconds as u64),
            movement_speed: 1.0,
            transition: Duration::from_millis(900),
            switched_at: Instant::now(),
            started: false,
            paused_at: None,
            next: if style.single_photo() {
                1
            } else {
                slots.len().min(count)
            },
            current: 0,
            previous: None,
            slots,
            fading_slot: None,
            changes: 0,
            shuffle_tiles,
            tile_order,
            preload_cursor: 0,
            backfill_cursor: desired_slots.min(count),
            backfill_pending: HashSet::new(),
            frames: HashMap::new(),
            order: VecDeque::new(),
            requested: HashSet::new(),
            failed: HashSet::new(),
            next_revision: 0,
        }
    }
    fn request(&mut self, index: usize, tx: &Sender<usize>) {
        if self.count == 0 {
            return;
        }
        let index = index % self.count;
        // A loaded image or a known-bad source must not be repeatedly queued.
        // requested stays set until its original upgrade completes, even
        // when a cached preview arrives or another frame is evicted.
        if self.frames.contains_key(&index) || self.failed.contains(&index) {
            return;
        }
        if self.requested.insert(index) {
            if tx.send(index).is_err() {
                self.requested.remove(&index);
            }
        }
    }
    fn prefetch(&mut self, tx: &Sender<usize>) {
        if self.count == 0 {
            return;
        }
        if self.style == Style::CoverFlow {
            for index in coverflow_neighbor_indices(self.current, self.count) {
                self.request(index, tx);
            }
            for offset in 1..=PREFETCH {
                self.request((self.current + offset) % self.count, tx);
            }
        } else if self.style.single_photo() {
            for i in 0..PREFETCH {
                self.request((self.current + i) % self.count, tx);
            }
        } else {
            // Initial tiles are loaded once. A pending preview is NOT blank:
            // the local cache worker and original worker may still finish.
            if self.preload_cursor < self.slots.len() {
                let end = self.preload_cursor.saturating_add(24).min(self.slots.len());
                for slot in self.preload_cursor..end {
                    self.request(self.slots[slot], tx);
                }
                self.preload_cursor = end;
            }

            // Look for substitutes only after a tile has definitively failed
            // both cache and original decoding, never just because it is slow.
            // Queue at most one candidate per failed slot, and wait for the
            // result before requesting another. This avoids scanning entire
            // libraries and flooding a stalled NFS share.
            let blank_slots = self
                .slots
                .iter()
                .filter(|id| self.failed.contains(id) && !self.frames.contains_key(id))
                .count();
            let mut available = blank_slots.saturating_sub(self.backfill_pending.len());
            while available > 0 && self.backfill_cursor < self.count {
                let index = self.backfill_cursor;
                self.backfill_cursor += 1;
                if self.slots.contains(&index)
                    || self.requested.contains(&index)
                    || self.frames.contains_key(&index)
                    || self.failed.contains(&index)
                {
                    continue;
                }
                self.backfill_pending.insert(index);
                self.request(index, tx);
                available -= 1;
            }
            // If every unused photo has been tried, reuse decoded photos for
            // failed slots, including repeated occurrences of the same photo.
            if self.backfill_cursor >= self.count && self.backfill_pending.is_empty() {
                let replacements: Vec<_> = self
                    .frames
                    .keys()
                    .copied()
                    .filter(|id| !self.failed.contains(id))
                    .collect();
                if !replacements.is_empty() {
                    let mut replaced = false;
                    for (slot, id) in self.slots.iter_mut().enumerate() {
                        if self.failed.contains(id) && !self.frames.contains_key(id) {
                            *id = replacements[slot % replacements.len()];
                            replaced = true;
                        }
                    }
                    if replaced {
                        self.next_revision = self.next_revision.wrapping_add(1);
                    }
                }
            }
            for i in 0..PREFETCH {
                self.request((self.next + i) % self.count, tx);
            }
        }
    }
    // Only explicit substitutes can replace an initial tile, and only
    // after that tile has failed both cache and original decoding.
    fn failed_slot_for_backfill(&self, candidate: usize) -> Option<usize> {
        if self.style.single_photo()
            || self.slots.contains(&candidate)
            || !self.backfill_pending.contains(&candidate)
        {
            return None;
        }
        self.slots
            .iter()
            .position(|id| self.failed.contains(id) && !self.frames.contains_key(id))
    }
    fn receive(&mut self, result: ImageResult) {
        // Remember the candidate before completing its original request.
        let replacement_slot = self.failed_slot_for_backfill(result.index);
        if result.high_quality {
            // An original result (success or failure) completes the request.
            // A cached result only completes the preview phase.
            self.requested.remove(&result.index);
            self.backfill_pending.remove(&result.index);
        }
        let Some(pixels) = result.pixels else {
            // A missing local preview isn't a failed photo: the independent
            // original worker may still succeed. Only a failed original with
            // no usable preview is considered unavailable.
            if result.high_quality && !self.frames.contains_key(&result.index) {
                self.failed.insert(result.index);
            }
            return;
        };
        if self
            .frames
            .get(&result.index)
            .is_some_and(|f| f.high_quality && !result.high_quality)
        {
            return;
        }
        let bytes = glib::Bytes::from_owned(pixels.data);
        let texture: gtk::gdk::Texture = gtk::gdk::MemoryTexture::new(
            pixels.width,
            pixels.height,
            gtk::gdk::MemoryFormat::R8g8b8a8,
            &bytes,
            pixels.width as usize * 4,
        )
        .upcast();
        self.next_revision = self.next_revision.wrapping_add(1);
        self.frames.insert(
            result.index,
            Surface {
                texture,
                high_quality: result.high_quality,
                revision: self.next_revision,
            },
        );
        self.failed.remove(&result.index);
        if let Some(blank_slot) = replacement_slot {
            // Never replace a tile merely because its decode is pending.
            // In particular, ordinary look-ahead images (100, 101, 102 ...)
            // must not displace the first 100 photos during mosaic startup.
            self.slots[blank_slot] = result.index;
            self.backfill_pending.remove(&result.index);
        }
        self.order.retain(|index| *index != result.index);
        self.order.push_back(result.index);
        // Preserve every active tile plus a small prefetch margin.
        let max_frames = if self.style == Style::CoverFlow {
            11
        } else if self.style.single_photo() {
            5
        } else {
            self.slots.len() + PREFETCH + 2
        };
        while self.frames.len() > max_frames {
            let protected = |id: usize| {
                id == self.current
                    || self.previous == Some(id)
                    || self.slots.contains(&id)
                    || self.fading_slot.is_some_and(|(_, old)| old == id)
            };
            let Some(pos) = self.order.iter().position(|id| !protected(*id)) else {
                break;
            };
            let Some(old) = self.order.remove(pos) else {
                break;
            };
            self.frames.remove(&old);
            // Do not clear requested: the original might still be decoding.
            // Clearing it here requeues the same network RAW every frame.
        }
    }
    fn available(&self, index: usize) -> bool {
        self.frames.contains_key(&index)
    }
    /// May `index` replace a tile now? Walls and prints wait for the original
    /// (or its definitive failure) so a tile never changes sharpness after it
    /// appears; if the original is slow they fall back to the preview.
    fn ready(&self, index: usize) -> bool {
        let Some(frame) = self.frames.get(&index) else {
            return false;
        };
        frame.high_quality
            || !self.requested.contains(&index)
            || self.elapsed() > self.interval + HQ_PATIENCE
    }
    fn update(&mut self, tx: &Sender<usize>) {
        // Do not keep an obsolete outgoing image/slot alive after its fade.
        // This also allows the LRU to release old GPU textures.
        if self.progress() >= 1.0 {
            self.previous = None;
            self.fading_slot = None;
        }
        self.prefetch(tx);
        if self.style.single_photo() && !self.started {
            // Begin the first slide only when it can actually be displayed.
            if !self.available(self.current) {
                if !self.failed.contains(&self.current) {
                    return;
                }
                let Some(index) = (1..self.count)
                    .map(|offset| (self.current + offset) % self.count)
                    .find(|index| !self.failed.contains(index))
                else {
                    return;
                };
                self.current = index;
                self.next = (index + 1) % self.count;
                self.request(index, tx);
                if !self.available(index) {
                    return;
                }
            }
            self.started = true;
            self.switched_at = self.paused_at.unwrap_or_else(Instant::now);
            return;
        }
        if self.paused_at.is_some() || self.count == 0 || self.elapsed() < self.interval {
            return;
        }
        // If every selected photo is already on screen, there is no new image
        // to fade in. Keep the pile still instead of promoting the same photo
        // again, which looks like a jump without an actual replacement.
        if !self.style.single_photo() && self.count <= self.slots.len() {
            return;
        }
        if self.style.single_photo() {
            for _ in 0..self.count {
                let index = self.next % self.count;
                if index == self.current {
                    self.next = (index + 1) % self.count;
                    continue;
                }
                if self.available(index) {
                    self.previous = Some(self.current);
                    self.current = index;
                    self.next = (index + 1) % self.count;
                    self.switched_at = Instant::now();
                    self.changes += 1;
                    break;
                }
                if !self.failed.contains(&index) {
                    self.request(index, tx);
                    break;
                }
                self.next = (index + 1) % self.count;
            }
        } else {
            for _ in 0..self.count {
                let index = self.next % self.count;
                if self.slots.contains(&index) {
                    self.next = (index + 1) % self.count;
                    continue;
                }
                if self.ready(index) {
                    let step = self.changes % self.tile_order.len().max(1);
                    if self.shuffle_tiles && step == 0 && self.changes > 0 {
                        // Refresh the permutation only after a complete pass:
                        // every tile changes once before one changes again.
                        shuffle(&mut self.tile_order);
                    }
                    let slot = self.tile_order[step];
                    let former = std::mem::replace(&mut self.slots[slot], index);
                    self.fading_slot = Some((slot, former));
                    self.next = (index + 1) % self.count;
                    self.switched_at = Instant::now();
                    self.changes += 1;
                    break;
                }
                if !self.failed.contains(&index) {
                    self.request(index, tx);
                    break;
                }
                self.next = (index + 1) % self.count;
            }
        }
        self.prefetch(tx);
    }
    fn elapsed(&self) -> Duration {
        self.paused_at
            .unwrap_or_else(Instant::now)
            .duration_since(self.switched_at)
    }
    fn pause_or_resume(&mut self) -> bool {
        if let Some(then) = self.paused_at.take() {
            self.switched_at += then.elapsed();
            false
        } else {
            self.paused_at = Some(Instant::now());
            true
        }
    }
    fn progress(&self) -> f64 {
        (self.elapsed().as_secs_f64() / self.transition.as_secs_f64().max(0.2)).clamp(0.0, 1.0)
    }
}

// Smooth start/finish and alternate pan directions per photograph.
fn smootherstep(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn coverflow_neighbor_indices(current: usize, count: usize) -> std::ops::Range<usize> {
    if count == 0 {
        return 0..0;
    }
    let current = current.min(count - 1);
    current.saturating_sub(crate::coverflow::VISIBLE_RADIUS)
        ..current
            .saturating_add(crate::coverflow::VISIBLE_RADIUS + 1)
            .min(count)
}

/// GPU camera movement in actual CSS pixels, never GTK's unsupported
/// percentage translations. Four directions give individual photos variety.
fn cinematic_motion(index: usize, progress: f64) -> (f64, f64, f64) {
    let eased = smootherstep(progress.clamp(0.0, 1.0));
    let scale = 1.08 + 0.08 * eased;
    let (from_x, from_y, to_x, to_y) = match index % 4 {
        0 => (-25.0, 20.0, 25.0, -20.0),
        1 => (25.0, -20.0, -25.0, 20.0),
        2 => (-28.0, -15.0, 28.0, 15.0),
        _ => (28.0, 15.0, -28.0, -15.0),
    };
    (
        scale,
        from_x + (to_x - from_x) * eased,
        from_y + (to_y - from_y) * eased,
    )
}

fn start_screensaver(parent: &adw::ApplicationWindow, slides: Vec<Slide>, prefs: Preferences) {
    let style = prefs.style.resolved();
    if slides.is_empty() && style.uses_photos() {
        return;
    }
    let player = Rc::new(RefCell::new(Player::new(
        style,
        slides.len(),
        prefs.seconds,
        prefs.shuffle_tiles,
        prefs.mosaic_tiles,
    )));
    player.borrow_mut().transition =
        Duration::from_millis(prefs.transition_ms.clamp(200, 2500) as u64);
    player.borrow_mut().movement_speed = prefs.movement_speed.clamp(0.25, 3.0);
    // Captions use catalog metadata and path basenames, never original EXIF.
    let captions: Vec<String> = slides.iter().map(|slide| slide.caption.clone()).collect();
    let (request_tx, request_rx) = mpsc::channel::<usize>();
    // Bound decoded-frame buffering. Unbounded RGBA frames could consume
    // hundreds of MB while GTK was busy rendering/closing.
    let (result_tx, result_rx) = mpsc::sync_channel::<ImageResult>(2);
    let cancelled = Arc::new(AtomicBool::new(false));
    // Render at the resolution of tiles actually displayed, not just the
    // requested count (e.g. 10,000 requested from a three-photo collection).
    let active_mosaic_tiles = player.borrow().slots.len().max(1);
    if style.uses_photos() {
        start_decode_worker(
            slides,
            style,
            active_mosaic_tiles,
            prefs.exclude_portraits,
            request_rx,
            result_tx,
            Arc::clone(&cancelled),
        );
    }

    // Fullscreen surfaces should be independent application windows on
    // Wayland, not transient dialogs attached to the main gallery. Closing
    // a fullscreen transient can confuse compositor focus and window teardown.
    let screen = gtk::Window::builder()
        .title("PIC Screensaver")
        .decorated(false)
        .build();
    screen.set_application(parent.application().as_ref());
    gpu::install_css();
    let stage = Rc::new(RefCell::new(gpu::Stage::new(
        style,
        player.borrow().slots.len(),
        prefs.picasa_transition,
    )));
    if let Some(colour) = prefs.background_colour {
        stage.borrow().set_background_colour(colour);
    }
    stage.borrow().root.set_focusable(true);
    let stage_overlay = gtk::Overlay::new();
    stage_overlay.set_child(Some(&stage.borrow().root));

    // A single footer owns both optional overlays. GtkCenterBox reserves a
    // dedicated end slot for the clock instead of allowing a long centered
    // filename to draw on top of it. It also keeps the clock consistently
    // inset from the bottom/right edges on differently sized monitors.
    let footer = gtk::CenterBox::new();
    footer.set_halign(gtk::Align::Fill);
    footer.set_valign(gtk::Align::End);
    footer.set_margin_start(28);
    footer.set_margin_end(30);
    footer.set_margin_bottom(28);
    footer.set_can_target(false);

    let caption = gtk::Label::new(None);
    caption.add_css_class("screensaver-caption");
    caption.set_halign(gtk::Align::Center);
    caption.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    caption.set_max_width_chars(48);
    caption.set_visible(false);
    caption.set_can_target(false);
    footer.set_center_widget(Some(&caption));

    let clock = gtk::Label::new(None);
    clock.add_css_class("screensaver-clock");
    clock.set_halign(gtk::Align::End);
    clock.set_margin_start(20);
    clock.set_can_target(false);
    clock.set_visible(prefs.clock && style.uses_photos());
    if prefs.clock && style.uses_photos() {
        clock.set_label(&local_clock_text());
    }
    footer.set_end_widget(Some(&clock));
    stage_overlay.add_overlay(&footer);
    screen.set_child(Some(&stage_overlay));

    // Controller callbacks must not own the window they are attached to:
    // a strong GtkWindow reference creates a cycle and leaves PIC waiting
    // for the old fullscreen window to be destroyed.
    // Key/mouse callbacks arrive inside GTK's event dispatch. Destruction
    // of an active fullscreen surface in that stack can stall Wayland/GTK.
    // Hide and cancel immediately, then close on the next idle iteration.
    // All callback captures are weak; never keep the closed window alive.
    let closing = Rc::new(Cell::new(false));
    let weak_for_exit = screen.downgrade();
    let cancel_for_exit = Arc::clone(&cancelled);
    let request_exit: Rc<dyn Fn()> = Rc::new(move || {
        if closing.replace(true) {
            return;
        }
        cancel_for_exit.store(true, Ordering::Relaxed);
        if let Some(window) = weak_for_exit.upgrade() {
            window.set_visible(false);
            glib::idle_add_local_once(move || {
                window.close();
            });
        }
    });
    let keys = gtk::EventControllerKey::new();
    let close_for_keys = request_exit.clone();
    let play_for_keys = Rc::clone(&player);
    keys.connect_key_pressed(move |_, key, _, _| {
        if matches!(
            key,
            gtk::gdk::Key::space | gtk::gdk::Key::p | gtk::gdk::Key::P
        ) {
            let paused = play_for_keys.borrow_mut().pause_or_resume();
            if std::env::var_os("PIC_SCREENSAVER_TRACE").is_some() {
                eprintln!("PIC_SCREEN playback paused={paused}");
            }
        } else {
            close_for_keys();
        }
        glib::Propagation::Stop
    });
    screen.add_controller(keys);

    let clicked = gtk::GestureClick::new();
    let close_for_click = request_exit.clone();
    clicked.connect_pressed(move |_, _, _, _| close_for_click());
    screen.add_controller(clicked);

    let motion = gtk::EventControllerMotion::new();
    let origin = Rc::new(RefCell::new(None::<(f64, f64)>));
    let first_frame = Instant::now();
    let close_for_motion = request_exit.clone();
    motion.connect_motion(move |_, x, y| {
        if first_frame.elapsed() < Duration::from_millis(850) {
            *origin.borrow_mut() = Some((x, y));
            return;
        }
        let mut position = origin.borrow_mut();
        if let Some((ox, oy)) = *position {
            if (x - ox).hypot(y - oy) > 26.0 {
                close_for_motion();
            }
        } else {
            *position = Some((x, y));
        }
    });
    screen.add_controller(motion);

    // Closing the window cancels queued work immediately. An in-flight RAW
    // decode can finish naturally off-thread, but no further photo is decoded.
    let cancel_on_close = Arc::clone(&cancelled);
    screen.connect_close_request(move |_| {
        cancel_on_close.store(true, Ordering::Relaxed);
        glib::Propagation::Proceed
    });
    let weak_screen = screen.downgrade();
    let caption_last = Cell::new(None::<usize>);
    let last_clock_minute = Cell::new(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
            / 60,
    );
    // The GTK frame clock drives composited fades at the display refresh
    // rate. A 50ms GLib timer was limited to ~20fps and caused visible steps.
    // The callback is owned by the fullscreen widget and naturally stops
    // when that widget is no longer being presented.
    screen.add_tick_callback(move |_, frame_clock| {
        if cancelled.load(Ordering::Relaxed) {
            return glib::ControlFlow::Break;
        }
        let Some(screen) = weak_screen.upgrade() else {
            return glib::ControlFlow::Break;
        };
        if !screen.is_visible() {
            return glib::ControlFlow::Break;
        }
        {
            let mut player = player.borrow_mut();
            for _ in 0..2 {
                match result_rx.try_recv() {
                    Ok(result) => player.receive(result),
                    Err(_) => break,
                }
            }
            player.update(&request_tx);
            stage.borrow_mut().sync(&player, frame_clock.frame_time());
            if prefs.clock && style.uses_photos() {
                let minute = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs()
                    / 60;
                if minute != last_clock_minute.get() {
                    last_clock_minute.set(minute);
                    clock.set_label(&local_clock_text());
                }
            }
            if prefs.captions {
                // For a wall, show the newest tile's label; for fullscreen,
                // show the current single photograph.
                let active_id = if style.single_photo() {
                    Some(player.current)
                } else {
                    player.fading_slot.map(|(slot, _)| player.slots[slot])
                };
                if active_id != caption_last.get() {
                    caption_last.set(active_id);
                    if let Some(text) = active_id.and_then(|id| captions.get(id)) {
                        caption.set_label(text);
                        caption.set_visible(true);
                    } else {
                        caption.set_visible(false);
                    }
                }
            }
        }
        glib::ControlFlow::Continue
    });
    // Fullscreen the same monitor that contains PIC's main window. Without
    // an explicit output, compositors may place the independent window on a
    // different monitor when multiple displays are connected.
    if let Some(monitor) = parent
        .native()
        .and_then(|native| native.surface())
        .and_then(|surface| gtk::prelude::WidgetExt::display(parent).monitor_at_surface(&surface))
    {
        screen.fullscreen_on_monitor(&monitor);
    } else {
        screen.fullscreen();
    }
    screen.present();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coverflow_style_round_trips_and_prefetches_a_bounded_neighbor_window() {
        assert_eq!(Style::from_saved("coverflow"), Style::CoverFlow);
        assert_eq!(Style::CoverFlow.key(), "coverflow");
        assert!(Style::CoverFlow.single_photo());
        assert!(Style::CoverFlow.available(false));
        assert_eq!(coverflow_neighbor_indices(0, 1), 0..1);
        assert_eq!(coverflow_neighbor_indices(0, 20), 0..5);
        assert_eq!(coverflow_neighbor_indices(10, 20), 6..15);
        assert_eq!(coverflow_neighbor_indices(19, 20), 15..20);
        let mut player = Player::new(Style::CoverFlow, 20, 5, false, 20);
        let (tx, rx) = mpsc::channel();
        player.prefetch(&tx);
        assert_eq!(rx.try_iter().collect::<Vec<_>>(), vec![0, 1, 2, 3, 4]);
        assert!(player.frames.len() <= 11);
        let mut end = Player::new(Style::CoverFlow, 20, 5, false, 20);
        end.current = 19;
        let (tx, rx) = mpsc::channel();
        end.prefetch(&tx);
        assert_eq!(
            rx.try_iter().collect::<Vec<_>>(),
            vec![15, 16, 17, 18, 19, 0, 1, 2]
        );
        for index in 0..20 {
            player.receive(ImageResult {
                index,
                high_quality: true,
                pixels: Some(Pixels {
                    width: 2,
                    height: 2,
                    data: vec![255; 16],
                }),
            });
        }
        assert!(player.frames.len() <= 11);
    }

    #[test]
    fn coverflow_screensaver_preference_survives_reopening_settings() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch("CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL)")
            .unwrap();
        let mut preferences = Preferences::load(&connection);
        preferences.style = Style::CoverFlow;
        preferences.source = Source::Album(37).key();
        preferences.save(&connection);
        let loaded = Preferences::load(&connection);
        assert_eq!(loaded.style, Style::CoverFlow);
        assert_eq!(loaded.source, "album:37");
    }

    #[test]
    fn coverflow_advances_past_a_failed_photo_and_wraps() {
        let mut player = Player::new(Style::CoverFlow, 3, 1, false, 6);
        let pixels = || Pixels {
            width: 2,
            height: 2,
            data: vec![255; 16],
        };
        player.receive(ImageResult {
            index: 0,
            high_quality: true,
            pixels: Some(pixels()),
        });
        player.receive(ImageResult {
            index: 1,
            high_quality: true,
            pixels: None,
        });
        player.receive(ImageResult {
            index: 2,
            high_quality: true,
            pixels: Some(pixels()),
        });
        let (tx, _) = mpsc::channel();
        player.update(&tx);
        assert_eq!(player.current, 0);
        player.switched_at -= Duration::from_secs(2);
        player.update(&tx);
        assert_eq!(player.current, 2);
        player.switched_at -= Duration::from_secs(2);
        player.update(&tx);
        assert_eq!(player.current, 0);
        assert!(player.frames.len() <= 11);
    }

    #[test]
    fn last_screensaver_settings_round_trip_in_library_db() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch("CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);")
            .unwrap();
        let prefs = Preferences {
            style: Style::Floating,
            source: Source::Album(42).key(),
            seconds: 14,
            shuffle: false,
            shuffle_tiles: true,
            mosaic_tiles: 24,
            picasa_transition: PicasaTransition::Crossfade,
            transition_ms: 1700,
            movement_speed: 1.75,
            captions: true,
            clock: true,
            background_colour: None,
            exclude_portraits: true,
        };
        prefs.save(&connection);
        let restored = Preferences::load(&connection);
        assert_eq!(restored.style, Style::Floating);
        assert_eq!(restored.source, "album:42");
        assert_eq!(restored.seconds, 14);
        assert!(!restored.shuffle);
        assert!(restored.shuffle_tiles);
        assert_eq!(restored.mosaic_tiles, 24);
        assert_eq!(restored.picasa_transition, PicasaTransition::Crossfade);
        assert_eq!(restored.transition_ms, 1700);
        assert!((restored.movement_speed - 1.75).abs() < f64::EPSILON);
        assert!(restored.captions);
        assert!(restored.clock);
        assert!(restored.exclude_portraits);
        let another = Preferences {
            clock: false,
            style: Style::Fade,
            ..prefs
        };
        another.save(&connection);
        let restored = Preferences::load(&connection);
        assert_eq!(restored.style, Style::Fade);
        assert!(!restored.clock);
        assert!(restored.captions);
    }

    #[test]
    fn tile_defaults_follow_the_saved_style_and_preserve_custom_counts() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch("CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL)")
            .unwrap();
        for (style, expected) in [
            ("flying", 30),
            ("photo-drift", 30),
            ("floating", 12),
            ("pile", 6),
            ("mosaic", 6),
            ("grid", 6),
        ] {
            db::set_setting(&connection, STYLE_KEY, style).unwrap();
            assert_eq!(
                Preferences::load(&connection).mosaic_tiles,
                expected,
                "{style}"
            );
        }
        db::set_setting(&connection, MOSAIC_TILES_KEY, "17").unwrap();
        assert_eq!(Preferences::load(&connection).mosaic_tiles, 17);
    }

    #[test]
    fn screensaver_background_colour_is_saved_and_defaults_to_style_background() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch("CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL)")
            .unwrap();
        let mut prefs = Preferences::load(&connection);
        assert_eq!(prefs.background_colour, None);
        prefs.background_colour = Some([18, 52, 86]);
        prefs.save(&connection);
        assert_eq!(
            Preferences::load(&connection).background_colour,
            Some([18, 52, 86])
        );
    }

    #[test]
    fn portrait_filter_uses_visible_rotation_and_crop() {
        assert_eq!(
            displayed_is_portrait(Some(4000), Some(6000), 0, ""),
            Some(true)
        );
        assert_eq!(
            displayed_is_portrait(Some(4000), Some(6000), 90, ""),
            Some(false)
        );
        assert_eq!(
            displayed_is_portrait(Some(4000), Some(4000), 0, ""),
            Some(false)
        );
        assert_eq!(displayed_is_portrait(None, Some(6000), 0, ""), None);
        let mut recipe = edit::EditRecipe::default();
        recipe.crop = edit::model::CropRect {
            left: 0.3,
            top: 0.0,
            right: 0.7,
            bottom: 1.0,
        };
        assert_eq!(
            displayed_is_portrait(Some(4000), Some(3000), 0, &recipe.encode()),
            Some(true)
        );
    }

    #[test]
    fn portrait_filter_checks_decoded_photos_when_metadata_is_unknown() {
        let pixels = |width, height| Pixels {
            width,
            height,
            data: vec![255; (width * height * 4) as usize],
        };
        assert!(accept_orientation(pixels(2, 3), true).is_none());
        assert!(accept_orientation(pixels(3, 2), true).is_some());
        assert!(accept_orientation(pixels(2, 2), true).is_some());
        assert!(accept_orientation(pixels(2, 3), false).is_some());
    }

    #[test]
    fn horizontal_style_picker_labels_and_previews_are_distinct() {
        let styles = Style::CHOICES;
        let mut labels = HashSet::new();
        for style in styles {
            assert!(!style.label().is_empty());
            assert!(!style.subtitle().is_empty());
            assert!(labels.insert(style.label()));
        }
    }

    #[test]
    #[ignore = "requires a GTK display"]
    fn screensaver_settings_measure_without_gtk_warnings() {
        libadwaita::init().unwrap();
        let parent = adw::ApplicationWindow::builder().build();
        let connection = Rc::new(RefCell::new(Connection::open_in_memory().unwrap()));
        open(&parent, connection, Vec::new(), "PIC-Default");
        let dialog = gtk::Window::list_toplevels()
            .into_iter()
            .filter_map(|widget| widget.downcast::<gtk::Window>().ok())
            .find(|window| window.title().as_deref() == Some("PIC Screensaver"))
            .unwrap();
        let body = dialog.child().unwrap();
        let main_row = body.first_child().unwrap().next_sibling().unwrap();
        for width in [840, 888, 920] {
            main_row.measure(gtk::Orientation::Vertical, width);
        }
        dialog.close();
    }

    #[test]
    #[ignore = "requires a GTK display"]
    fn background_colour_is_last_display_row_with_one_swatch() {
        fn find_label(widget: &gtk::Widget, text: &str) -> Option<gtk::Widget> {
            if widget
                .downcast_ref::<gtk::Label>()
                .is_some_and(|label| label.text() == text)
            {
                return Some(widget.clone());
            }
            let mut child = widget.first_child();
            while let Some(current) = child {
                if let Some(found) = find_label(&current, text) {
                    return Some(found);
                }
                child = current.next_sibling();
            }
            None
        }
        libadwaita::init().unwrap();
        let parent = adw::ApplicationWindow::builder().build();
        open(
            &parent,
            Rc::new(RefCell::new(Connection::open_in_memory().unwrap())),
            Vec::new(),
            "PIC-Default",
        );
        let dialog = gtk::Window::list_toplevels()
            .into_iter()
            .filter_map(|widget| widget.downcast::<gtk::Window>().ok())
            .find(|window| window.title().as_deref() == Some("PIC Screensaver"))
            .unwrap();
        let label = find_label(dialog.upcast_ref(), "Background colour").unwrap();
        let row = label.parent().unwrap();
        assert!(
            row.next_sibling().is_none(),
            "background must be the last display row"
        );
        let control = label.next_sibling().unwrap();
        assert!(
            control.is::<gtk::Button>(),
            "the colour swatch must be the only control"
        );
        assert!(control.next_sibling().is_none());
        let clock = find_label(dialog.upcast_ref(), "Clock")
            .unwrap()
            .next_sibling()
            .unwrap();
        for orientation in [gtk::Orientation::Horizontal, gtk::Orientation::Vertical] {
            assert_eq!(
                control.measure(orientation, -1).1,
                clock.measure(orientation, -1).1
            );
        }
        dialog.close();
    }

    #[test]
    #[ignore = "requires a GTK display"]
    fn reset_restores_dialog_defaults_without_saving() {
        fn find(
            widget: &gtk::Widget,
            matches: &dyn Fn(&gtk::Widget) -> bool,
        ) -> Option<gtk::Widget> {
            if matches(widget) {
                return Some(widget.clone());
            }
            let mut child = widget.first_child();
            while let Some(current) = child {
                if let Some(found) = find(&current, matches) {
                    return Some(found);
                }
                child = current.next_sibling();
            }
            None
        }
        libadwaita::init().unwrap();
        let connection = Rc::new(RefCell::new(Connection::open_in_memory().unwrap()));
        connection
            .borrow()
            .execute_batch("CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL)")
            .unwrap();
        db::set_setting(&connection.borrow(), STYLE_KEY, "fade").unwrap();
        db::set_setting(&connection.borrow(), CLOCK_KEY, "true").unwrap();
        db::set_setting(&connection.borrow(), DURATION_KEY, "20").unwrap();
        let parent = adw::ApplicationWindow::builder().build();
        open(&parent, Rc::clone(&connection), vec![42], "PIC-Default");
        let dialog = gtk::Window::list_toplevels()
            .into_iter()
            .filter_map(|widget| widget.downcast::<gtk::Window>().ok())
            .find(|window| window.title().as_deref() == Some("PIC Screensaver"))
            .unwrap();
        let root = dialog.upcast_ref();
        let reset = find(root, &|widget| {
            widget
                .downcast_ref::<gtk::Button>()
                .is_some_and(|button| button.label().as_deref() == Some("Reset"))
        })
        .unwrap()
        .downcast::<gtk::Button>()
        .unwrap();
        assert_eq!(
            reset
                .prev_sibling()
                .unwrap()
                .downcast::<gtk::Button>()
                .unwrap()
                .label()
                .as_deref(),
            Some("Cancel")
        );
        assert_eq!(
            reset
                .next_sibling()
                .unwrap()
                .downcast::<gtk::Button>()
                .unwrap()
                .label()
                .as_deref(),
            Some("Start")
        );
        reset.emit_clicked();
        let label = |text: &str| {
            find(root, &|widget| {
                widget
                    .downcast_ref::<gtk::Label>()
                    .is_some_and(|label| label.text() == text)
            })
            .unwrap()
        };
        let clock = label("Clock")
            .next_sibling()
            .unwrap()
            .downcast::<gtk::Switch>()
            .unwrap();
        assert!(!clock.is_active());
        let interval = label("Change every")
            .next_sibling()
            .unwrap()
            .first_child()
            .unwrap()
            .downcast::<gtk::Scale>()
            .unwrap();
        assert_eq!(interval.value(), 6.0);
        assert!(find(root, &|widget| widget
            .downcast_ref::<gtk::Label>()
            .is_some_and(|label| label.text() == "Square Picasa photo wall"))
        .is_some());
        let source = label("Photos")
            .next_sibling()
            .unwrap()
            .downcast::<gtk::DropDown>()
            .unwrap();
        assert_eq!(source.selected(), 0);
        for (style, expected) in [
            (Style::Flying, 30.0),
            (Style::PhotoDrift, 30.0),
            (Style::Floating, 12.0),
            (Style::PicturePile, 6.0),
            (Style::Grid, 6.0),
            (Style::Mosaic, 6.0),
        ] {
            let button = find(root, &|widget| {
                widget
                    .downcast_ref::<gtk::ToggleButton>()
                    .is_some_and(|button| {
                        button.tooltip_text().as_deref() == Some(style.subtitle())
                    })
            })
            .unwrap()
            .downcast::<gtk::ToggleButton>()
            .unwrap();
            button.emit_clicked();
            let tiles = label("Tiles")
                .next_sibling()
                .unwrap()
                .last_child()
                .unwrap()
                .downcast::<gtk::SpinButton>()
                .unwrap();
            assert_eq!(tiles.value(), expected, "{style:?}");
        }
        assert_eq!(
            db::setting(&connection.borrow(), STYLE_KEY)
                .unwrap()
                .as_deref(),
            Some("fade")
        );
        dialog.close();
    }

    #[test]
    fn matrix_rain_does_not_need_a_photo_source() {
        assert!(!Style::MatrixRain.uses_photos());
        assert!(Style::MatrixKenBurns.uses_photos());
    }

    #[test]
    fn matrix_styles_only_available_in_terminal() {
        for style in Style::CHOICES {
            assert!(style.available(true));
            assert_eq!(style.available(false), !style.matrix());
            assert!(!style.for_theme(false).matrix());
            assert_eq!(style.for_theme(true), style);
        }
        for _ in 0..100 {
            assert!(!Style::Surprise.resolved().matrix());
        }
    }

    #[test]
    fn matrix_style_keys_round_trip() {
        for key in ["matrix-rain", "matrix-ken-burns"] {
            let style = Style::from_saved(key);
            assert_eq!(style.key(), key);
            assert!(style.single_photo());
        }
    }

    #[test]
    fn style_and_source_keys_are_stable() {
        assert_eq!(Style::from_saved("iphoto"), Style::IPhoto);
        assert_eq!(Style::from_saved("picasa"), Style::Mosaic);
        assert_eq!(Style::from_saved("pile"), Style::PicturePile);
        assert_eq!(Style::Surprise.key(), "surprise");
        for style in Style::CHOICES {
            assert_eq!(Style::from_saved(style.key()), style);
            assert!(Style::CHOICES.contains(&style.resolved()));
        }
        assert!(!Style::Surprise.resolved().eq(&Style::Surprise));
        assert_eq!(Source::Selected(vec![1, 2]).key(), "selected");
        assert_eq!(Source::Album(42).key(), "album:42");
        assert_eq!(Source::Folder(3).key(), "folder:3");
    }
    #[test]
    fn picasa_transition_persists_and_flip_has_edge_on_midpoint() {
        assert_eq!(PicasaTransition::from_saved("flip"), PicasaTransition::Flip);
        assert_eq!(
            PicasaTransition::from_saved("crossfade"),
            PicasaTransition::Crossfade
        );
        assert_eq!(PicasaTransition::from_saved("cut"), PicasaTransition::Cut);
        assert_eq!(
            PicasaTransition::from_saved("unknown"),
            PicasaTransition::Crossfade
        );
        for (index, mode) in [
            (0, PicasaTransition::Flip),
            (1, PicasaTransition::Crossfade),
            (2, PicasaTransition::Cut),
        ] {
            assert_eq!(PicasaTransition::from_dropdown(index), mode);
            assert_eq!(PicasaTransition::from_saved(mode.key()), mode);
            assert_eq!(mode.dropdown_index(), index);
        }
        let flip = PicasaTransition::Flip;
        assert_eq!(flip.flip_width(0.0), 1.0);
        assert!(flip.flip_width(0.5) <= 0.026);
        assert_eq!(flip.flip_width(1.0), 1.0);
        assert_eq!(PicasaTransition::Cut.flip_width(0.5), 1.0);
    }

    #[test]
    fn missing_cached_preview_does_not_mark_original_failed() {
        let mut player = Player::new(Style::Mosaic, 12, 6, false, 6);
        player.receive(ImageResult {
            index: 0,
            high_quality: false,
            pixels: None,
        });
        assert!(!player.failed.contains(&0));
        player.receive(ImageResult {
            index: 0,
            high_quality: true,
            pixels: None,
        });
        assert!(player.failed.contains(&0));
    }

    #[test]
    fn mosaic_loading_does_not_scan_ahead_while_previews_are_pending() {
        let mut player = Player::new(Style::Mosaic, 5400, 6, false, 10);
        let (tx, rx) = mpsc::channel();
        for _ in 0..200 {
            player.prefetch(&tx);
        }
        let queued: Vec<usize> = rx.try_iter().collect();
        assert_eq!(player.preload_cursor, 10);
        assert_eq!(player.backfill_cursor, 10);
        assert_eq!(queued.len(), 13);
        assert_eq!(queued, (0..13).collect::<Vec<_>>());
        assert!(player.backfill_pending.is_empty());
    }

    #[test]
    fn failed_mosaic_tiles_backfill_one_request_at_a_time() {
        let mut player = Player::new(Style::Mosaic, 5400, 6, false, 10);
        let (tx, rx) = mpsc::channel();
        player.prefetch(&tx);
        assert_eq!(rx.try_iter().count(), 13);
        // This tile is definitively unavailable (cache AND original failed).
        player.receive(ImageResult {
            index: 3,
            high_quality: true,
            pixels: None,
        });
        for _ in 0..120 {
            player.prefetch(&tx);
        }
        assert_eq!(rx.try_iter().collect::<Vec<_>>(), vec![13]);
        assert_eq!(player.backfill_pending.len(), 1);
        assert_eq!(player.backfill_cursor, 14);
        // Once candidate 13 finishes without a preview, allow precisely one
        // subsequent replacement request rather than a library-wide scan.
        player.receive(ImageResult {
            index: 13,
            high_quality: true,
            pixels: None,
        });
        player.prefetch(&tx);
        assert_eq!(rx.try_iter().collect::<Vec<_>>(), vec![14]);
    }

    #[test]
    fn lookahead_photo_cannot_displace_pending_mosaic_tile() {
        let mut player = Player::new(Style::Mosaic, 250, 6, false, 100);
        // Initial tiles 0..99 have not completed loading, but are not failed.
        player.prefetch(&mpsc::channel().0);
        assert_eq!(player.failed_slot_for_backfill(100), None);
        assert_eq!(player.failed_slot_for_backfill(101), None);
        assert_eq!(player.failed_slot_for_backfill(102), None);
        assert_eq!(player.slots, (0..100).collect::<Vec<_>>());
    }

    #[test]
    fn replacement_requires_real_failure_and_explicit_backfill() {
        let mut player = Player::new(Style::Mosaic, 150, 6, false, 6);
        // A speculative decode of photo 6 is not permission to change a tile.
        player.failed.insert(2);
        assert_eq!(player.failed_slot_for_backfill(6), None);
        player.backfill_pending.insert(8);
        assert_eq!(player.failed_slot_for_backfill(8), Some(2));
        // A successful preview (if present) would protect the old tile.
        player.failed.remove(&2);
        assert_eq!(player.failed_slot_for_backfill(8), None);
    }

    #[test]
    fn original_upgrade_is_not_resubmitted_after_cache_phase() {
        let mut player = Player::new(Style::Mosaic, 200, 6, false, 10);
        let (tx, rx) = mpsc::channel();
        player.request(107, &tx);
        player.receive(ImageResult {
            index: 107,
            high_quality: false,
            pixels: None,
        });
        for _ in 0..100 {
            player.request(107, &tx);
        }
        assert_eq!(rx.try_iter().collect::<Vec<_>>(), vec![107]);
        assert!(player.requested.contains(&107));
        player.receive(ImageResult {
            index: 107,
            high_quality: true,
            pixels: None,
        });
        assert!(!player.requested.contains(&107));
        assert!(player.failed.contains(&107));
        player.request(107, &tx);
        assert_eq!(rx.try_iter().count(), 0);
    }

    #[test]
    fn picture_pile_waits_for_the_original_but_not_forever() {
        let mut pile = Player::new(Style::PicturePile, 20, 3, false, 6);
        let (tx, _rx) = mpsc::channel();
        pile.request(7, &tx);
        pile.receive(ImageResult {
            index: 7,
            high_quality: false,
            pixels: Some(Pixels {
                width: 1,
                height: 1,
                data: vec![0; 4],
            }),
        });
        assert!(
            pile.available(7) && !pile.ready(7),
            "preview only: still waiting for the original"
        );
        pile.switched_at -= Duration::from_secs(7);
        assert!(pile.ready(7), "slow original: fall back to the preview");
        pile.switched_at = Instant::now();
        pile.receive(ImageResult {
            index: 7,
            high_quality: false,
            pixels: None,
        });
        pile.receive(ImageResult {
            index: 7,
            high_quality: true,
            pixels: None,
        });
        assert!(
            pile.ready(7),
            "original failed for good: the preview is final"
        );
        let mosaic = {
            let mut m = Player::new(Style::Mosaic, 20, 3, false, 6);
            m.request(7, &tx);
            m.receive(ImageResult {
                index: 7,
                high_quality: false,
                pixels: Some(Pixels {
                    width: 1,
                    height: 1,
                    data: vec![0; 4],
                }),
            });
            m
        };
        assert!(!mosaic.ready(7), "walls wait for the original too");
    }

    #[test]
    fn shuffle_preserves_items() {
        let mut values: Vec<usize> = (0..100).collect();
        shuffle(&mut values);
        values.sort_unstable();
        assert_eq!(values, (0..100).collect::<Vec<_>>());
    }
    #[test]
    fn cinematic_motion_uses_pixel_scale_and_varied_directions() {
        for photo_index in 0..8 {
            let (first_scale, first_x, first_y) = cinematic_motion(photo_index, 0.0);
            let (last_scale, last_x, last_y) = cinematic_motion(photo_index, 1.0);
            assert!((first_scale - 1.08).abs() < 1e-8);
            assert!((last_scale - 1.16).abs() < 1e-8);
            assert!(first_x != last_x || first_y != last_y);
            for step in 0..100 {
                let (scale, x, y) = cinematic_motion(photo_index, step as f64 / 100.0);
                assert!((1.08..=1.16).contains(&scale));
                assert!(x.abs() <= 28.1 && y.abs() <= 20.1);
            }
        }
        assert_eq!(smootherstep(0.0), 0.0);
        assert_eq!(smootherstep(1.0), 1.0);
    }
    #[test]
    fn picture_pile_does_not_reorder_an_existing_photo_when_all_are_on_screen() {
        let mut pile = Player::new(Style::PicturePile, 4, 3, false, 10);
        let (tx, _rx) = mpsc::channel();
        pile.switched_at -= Duration::from_secs(5);
        pile.update(&tx);
        assert_eq!(pile.changes, 0);
        assert_eq!(pile.fading_slot, None);
        pile.switched_at -= Duration::from_secs(5);
        pile.update(&tx);
        assert_eq!(pile.changes, 0);
        assert_eq!(pile.fading_slot, None);
        assert_eq!(pile.slots, vec![0, 1, 2, 3, 0, 1, 2, 3, 0, 1]);
    }

    #[test]
    fn photo_interval_and_transition_timing_are_independent() {
        let mut player = Player::new(Style::Fade, 12, 15, false, 6);
        assert_eq!(player.interval, Duration::from_secs(15));
        player.transition = Duration::from_millis(2000);
        player.switched_at = Instant::now() - Duration::from_millis(1000);
        let progress = player.progress();
        assert!((0.45..0.55).contains(&progress));
        assert!(player.elapsed() < player.interval);
    }

    #[test]
    fn pause_freezes_and_resume_restores_player_timing() {
        let mut player = Player::new(Style::IPhoto, 6, 6, false, 6);
        player.switched_at = Instant::now() - Duration::from_millis(450);
        assert!(player.pause_or_resume());
        let elapsed = player.elapsed();
        // Pretend the pause has lasted 5s. Presentation time should still
        // be near 450ms once playback resumes, rather than jumping slides.
        // Advance the simulated clock by five seconds for *both* instants.
        // In real playback Instant::now() advances while paused; moving
        // only paused_at backwards would make switched_at appear in future.
        player.switched_at -= Duration::from_secs(5);
        player.paused_at = Some(Instant::now() - Duration::from_secs(5));
        assert!(!player.pause_or_resume());
        let resumed = player.elapsed();
        assert!(resumed.abs_diff(elapsed) < Duration::from_millis(100));
        assert!(player.paused_at.is_none());
    }

    fn load_cinema_test_frame(player: &mut Player, index: usize) {
        player.receive(ImageResult {
            index,
            high_quality: false,
            pixels: Some(Pixels {
                width: 1,
                height: 1,
                data: vec![255; 4],
            }),
        });
    }

    #[test]
    fn cinema_starts_its_interval_when_the_first_image_arrives() {
        let mut player = Player::new(Style::IPhoto, 3, 6, false, 6);
        let (tx, _rx) = mpsc::channel();
        player.switched_at -= Duration::from_secs(20);
        player.update(&tx);
        assert!(!player.started);
        load_cinema_test_frame(&mut player, 0);
        load_cinema_test_frame(&mut player, 1);
        player.update(&tx);
        assert!(player.started);
        assert_eq!(player.current, 0);
        assert!(player.elapsed() < Duration::from_secs(1));
        player.switched_at -= Duration::from_secs(7);
        player.update(&tx);
        assert_eq!(player.current, 1);
    }

    #[test]
    fn cinema_does_not_restart_the_only_usable_photo() {
        for count in [1, 4] {
            let mut player = Player::new(Style::IPhoto, count, 6, false, 6);
            let (tx, _rx) = mpsc::channel();
            load_cinema_test_frame(&mut player, 0);
            player.failed.extend(1..count);
            player.update(&tx);
            player.switched_at -= Duration::from_secs(7);
            let before = player.switched_at;
            for _ in 0..5 {
                player.update(&tx);
            }
            assert_eq!(player.switched_at, before);
            assert_eq!(player.changes, 0);
            assert_eq!(player.previous, None);
        }
    }

    #[test]
    fn cinema_skips_failed_initial_photos_without_consuming_display_time() {
        let mut player = Player::new(Style::IPhoto, 4, 6, false, 6);
        let (tx, _rx) = mpsc::channel();
        player.failed.extend([0, 1]);
        player.switched_at -= Duration::from_secs(20);
        load_cinema_test_frame(&mut player, 2);
        player.update(&tx);
        assert!(player.started);
        assert_eq!(player.current, 2);
        assert!(player.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn player_paces_changes() {
        let player = Player::new(Style::IPhoto, 5, 6, true, DEFAULT_MOSAIC_TILES);
        assert_eq!(player.current, 0);
        assert_eq!(player.next, 1);
        assert_eq!(player.interval, Duration::from_secs(6));
        assert_eq!(
            Player::new(Style::Mosaic, 20, 3, true, DEFAULT_MOSAIC_TILES)
                .slots
                .len(),
            6
        );
        let ordered = Player::new(Style::Mosaic, 20, 3, false, DEFAULT_MOSAIC_TILES);
        assert_eq!(ordered.tile_order, (0..6).collect::<Vec<_>>());
        let shuffled = Player::new(Style::PicturePile, 20, 3, true, DEFAULT_MOSAIC_TILES);
        let mut positions = shuffled.tile_order.clone();
        positions.sort_unstable();
        assert_eq!(positions, (0..DEFAULT_MOSAIC_TILES).collect::<Vec<_>>());
    }

    #[test]
    fn outgoing_frames_are_released_when_the_transition_finishes() {
        let mut mosaic = Player::new(Style::Mosaic, 20, 12, false, 6);
        mosaic.fading_slot = Some((2, 0));
        mosaic.switched_at = Instant::now() - Duration::from_secs(3);
        let (tx, _rx) = mpsc::channel();
        mosaic.update(&tx);
        assert!(mosaic.fading_slot.is_none());

        let mut cinema = Player::new(Style::IPhoto, 8, 12, true, 6);
        cinema.previous = Some(0);
        cinema.current = 1;
        cinema.switched_at = Instant::now() - Duration::from_secs(3);
        cinema.update(&tx);
        assert!(cinema.previous.is_none());
    }

    #[test]
    fn mosaic_tile_count_affects_mosaic_not_picture_pile() {
        for n in [1, 6, 9, 12, 18, 24, 30, 60, 100, 250, 1000] {
            let mosaic = Player::new(Style::Mosaic, 2000, 5, true, n);
            assert_eq!(mosaic.slots.len(), n);
            assert_eq!(mosaic.next, n);
            assert_eq!(mosaic.tile_order.len(), n);
            let pile = Player::new(Style::PicturePile, 100, 5, true, n);
            assert_eq!(pile.slots.len(), n.clamp(1, PRINT_MAX_SLOTS));
        }
        let few = Player::new(Style::Mosaic, 3, 5, false, 30);
        assert_eq!(few.slots.len(), 30);
        assert_eq!(few.next, 3);
        assert_eq!(Player::new(Style::Mosaic, 100, 5, false, 1).slots.len(), 1);
        assert_eq!(Player::new(Style::Grid, 200, 5, false, 12).slots.len(), 12);
        assert_eq!(
            Player::new(Style::Floating, 200, 5, false, 12).slots.len(),
            12
        );
        assert_eq!(
            Player::new(Style::Floating, 200, 5, false, 5000)
                .slots
                .len(),
            PRINT_MAX_SLOTS
        );
        assert_eq!(
            Player::new(Style::PhotoDrift, 200, 5, false, 5000)
                .slots
                .len(),
            PRINT_MAX_SLOTS
        );
        assert!(Player::new(Style::Fade, 200, 5, false, 12).slots.is_empty());
        assert_eq!(
            Player::new(Style::PicturePile, 200, 5, false, 12)
                .slots
                .len(),
            12
        );
        assert_eq!(
            Player::new(Style::PicturePile, 3, 5, false, 12).slots.len(),
            12
        );
        assert_eq!(
            Player::new(Style::Mosaic, 2000, 5, false, 1500).slots.len(),
            1500
        );
        assert_eq!(
            Player::new(Style::Mosaic, 100, 5, false, usize::MAX)
                .slots
                .len(),
            MAX_WALL_TILES
        );
    }

    #[test]
    fn multi_photo_styles_repeat_small_collections_to_fill_slots() {
        for style in [
            Style::Mosaic,
            Style::Grid,
            Style::PicturePile,
            Style::Floating,
            Style::PhotoDrift,
            Style::Flying,
        ] {
            let player = Player::new(style, 3, 5, false, 12);
            assert_eq!(
                player.slots,
                vec![0, 1, 2, 0, 1, 2, 0, 1, 2, 0, 1, 2],
                "{style:?}"
            );
        }
        assert_eq!(
            Player::new(Style::Mosaic, 1, 5, false, 10_000).slots.len(),
            10_000
        );
        assert!(Player::new(Style::Mosaic, 0, 5, false, 10_000)
            .slots
            .is_empty());
    }

    #[test]
    fn repeated_slots_replace_failed_photos_with_available_ones() {
        let mut player = Player::new(Style::Mosaic, 2, 5, false, 6);
        player.receive(ImageResult {
            index: 0,
            high_quality: true,
            pixels: None,
        });
        player.receive(ImageResult {
            index: 1,
            high_quality: true,
            pixels: Some(Pixels {
                width: 1,
                height: 1,
                data: vec![0, 0, 0, 255],
            }),
        });
        player.prefetch(&mpsc::channel().0);
        assert_eq!(player.slots, vec![1; 6]);
    }

    #[test]
    fn flying_photos_is_a_library_style_with_a_bounded_flight_field() {
        assert_eq!(Style::from_saved("flying"), Style::Flying);
        assert_eq!(Style::Flying.key(), "flying");
        assert_eq!(Style::Flying.label(), "Flying Photos");
        assert!(Style::Flying.uses_photos());
        assert!(!Style::Flying.single_photo());
        assert!(Style::Flying.available(false));
        assert_eq!(Style::Flying.for_theme(false), Style::Flying);
        // Player slots are the prints in the air, capped so live textures stay
        // bounded, with repeated photos when the library is smaller.
        assert_eq!(
            Player::new(Style::Flying, 500, 6, false, 999).slots.len(),
            FLY_MAX_SLOTS
        );
        assert_eq!(Player::new(Style::Flying, 500, 6, false, 4).slots.len(), 4);
        assert_eq!(Player::new(Style::Flying, 3, 6, false, 6).slots.len(), 6);
        // Surprise Me must be able to pick the new style.
        let mut picked = false;
        for _ in 0..400 {
            if Style::Surprise.resolved() == Style::Flying {
                picked = true;
                break;
            }
        }
        assert!(picked, "Surprise Me ignores Flying Photos");
    }
}
