//! Photo screensaver: a Picasa-inspired mosaic/picture pile and an iPhoto-inspired
//! cinematic slideshow. Decode/rendering runs off the GTK thread. All original
//! images and non-destructive edits remain untouched.
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet, VecDeque};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use gtk::prelude::*;
use gtk4 as gtk;
use libadwaita as adw;
use rusqlite::Connection;

use crate::{db, edit, thumbnail};
mod gpu;

const STYLE_KEY: &str = "screensaver-style";
const SOURCE_KEY: &str = "screensaver-source";
const DURATION_KEY: &str = "screensaver-duration";
const SHUFFLE_KEY: &str = "screensaver-shuffle";
const SHUFFLE_TILES_KEY: &str = "screensaver-shuffle-tiles";
const MOSAIC_TILES_KEY: &str = "screensaver-mosaic-tiles";
const PICASA_TRANSITION_KEY: &str = "screensaver-picasa-transition";
const DEFAULT_MOSAIC_TILES: usize = 6;
const PICTURE_PILE_SLOTS: usize = 5;
const FLOATING_SLOTS: usize = 6;
const TRANSITION_KEY: &str = "screensaver-transition-ms";
const CAPTION_KEY: &str = "screensaver-captions";
const CLOCK_KEY: &str = "screensaver-clock";
const PREFETCH: usize = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Style { Mosaic, Grid, PicturePile, Floating, IPhoto, Fade, Surprise }
impl Style {
    const CHOICES: [Self; 7] = [
        Self::Mosaic, Self::Grid, Self::PicturePile, Self::Floating,
        Self::IPhoto, Self::Fade, Self::Surprise,
    ];
    fn from_saved(v: &str) -> Self {
        match v {
            "grid" => Self::Grid,
            "pile" => Self::PicturePile,
            "floating" => Self::Floating,
            "iphoto" => Self::IPhoto,
            "fade" => Self::Fade,
            "surprise" => Self::Surprise,
            _ => Self::Mosaic,
        }
    }
    fn key(self) -> &'static str {
        match self {
            Self::Mosaic => "mosaic", Self::Grid => "grid",
            Self::PicturePile => "pile", Self::Floating => "floating",
            Self::IPhoto => "iphoto", Self::Fade => "fade",
            Self::Surprise => "surprise",
        }
    }
    fn dropdown_index(self) -> u32 {
        Self::CHOICES.iter().position(|style| *style == self).unwrap_or(0) as u32
    }
    fn resolved(self) -> Self {
        if self != Self::Surprise { return self; }
        Self::CHOICES[(random_seed() as usize % (Self::CHOICES.len() - 1))]
    }
    fn single_photo(self) -> bool { matches!(self, Self::IPhoto | Self::Fade) }
    fn configurable_tiles(self) -> bool { matches!(self, Self::Mosaic | Self::Grid | Self::Surprise) }
    fn label(self) -> &'static str {
        match self {
            Self::Mosaic => "Mosaic",
            Self::Grid => "Changing Grid",
            Self::PicturePile => "Picture Pile",
            Self::Floating => "Floating Prints",
            Self::IPhoto => "Ken Burns",
            Self::Fade => "Fullscreen Fade",
            Self::Surprise => "Surprise Me",
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
            Self::Surprise => "Random style on launch",
        }
    }
    fn icon(self) -> &'static str {
        match self {
            Self::Mosaic => "view-grid-symbolic",
            Self::Grid => "view-grid-symbolic",
            Self::PicturePile => "view-paged-symbolic",
            Self::Floating => "image-x-generic-symbolic",
            Self::IPhoto => "image-x-generic-symbolic",
            Self::Fade => "view-fullscreen-symbolic",
            Self::Surprise => "media-playlist-shuffle-symbolic",
        }
    }
}

/// Picasa-specific tile transitions; iPhoto always uses its cinematic fade.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PicasaTransition { Flip, Crossfade, Cut }
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
    fn dropdown_index(self) -> u32 {
        match self {
            Self::Flip => 0,
            Self::Crossfade => 1,
            Self::Cut => 2,
        }
    }
    fn from_dropdown(value: u32) -> Self {
        match value {
            0 => Self::Flip,
            2 => Self::Cut,
            _ => Self::Crossfade,
        }
    }
    /// A two-sided horizontal card turn, edge-on at midpoint.
    fn flip_width(self, progress: f64) -> f64 {
        if self != Self::Flip { return 1.0; }
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
            Self::Selected(ids) => ids.iter()
                .map(|id| db::photo(connection, *id))
                .collect::<anyhow::Result<Vec<_>>>()
                .map(|rows| rows.into_iter().flatten().filter(|photo| !photo.trashed).collect()),
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
    shuffle_tiles: bool,
    mosaic_tiles: usize,
    picasa_transition: PicasaTransition,
    transition_ms: u32,
    captions: bool,
    clock: bool,
}
impl Preferences {
    fn load(connection: &Connection) -> Self {
        let setting = |key| db::setting(connection, key).ok().flatten().unwrap_or_default();
        Self {
            style: Style::from_saved(&setting(STYLE_KEY)),
            source: setting(SOURCE_KEY),
            seconds: setting(DURATION_KEY).parse::<u32>().unwrap_or(6).clamp(3, 30),
            shuffle: setting(SHUFFLE_KEY) != "false",
            shuffle_tiles: setting(SHUFFLE_TILES_KEY) != "false",
            mosaic_tiles: setting(MOSAIC_TILES_KEY).parse::<usize>()
                .ok().filter(|count| *count > 0).unwrap_or(DEFAULT_MOSAIC_TILES),
            picasa_transition: PicasaTransition::from_saved(&setting(PICASA_TRANSITION_KEY)),
            transition_ms: setting(TRANSITION_KEY).parse::<u32>().unwrap_or(900).clamp(200, 2500),
            captions: setting(CAPTION_KEY) == "true",
            clock: setting(CLOCK_KEY) == "true",
        }
    }
    fn save(&self, connection: &Connection) {
        for (key, value) in [
            (STYLE_KEY, self.style.key().to_owned()),
            (SOURCE_KEY, self.source.clone()),
            (DURATION_KEY, self.seconds.to_string()),
            (SHUFFLE_KEY, self.shuffle.to_string()),
            (SHUFFLE_TILES_KEY, self.shuffle_tiles.to_string()),
            (MOSAIC_TILES_KEY, self.mosaic_tiles.to_string()),
            (PICASA_TRANSITION_KEY, self.picasa_transition.key().to_string()),
            (TRANSITION_KEY, self.transition_ms.to_string()),
            (CAPTION_KEY, self.captions.to_string()),
            (CLOCK_KEY, self.clock.to_string()),
        ] {
            if let Err(error) = db::set_setting(connection, key, &value) {
                eprintln!("PIC screensaver: could not save {key}: {error}");
            }
        }
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
        let caption = if date.is_empty() { filename } else { format!("{filename}  ·  {date}") };
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

fn field(label: &str, control: &impl IsA<gtk::Widget>) -> gtk::Box {
    let line = gtk::Box::new(gtk::Orientation::Horizontal, 9);
    let title = gtk::Label::new(Some(label));
    title.set_xalign(0.0);
    title.set_width_chars(11);
    control.set_hexpand(true);
    line.append(&title);
    line.append(control);
    line
}

/// Small local, vector-only illustrations: no photo decoding or NFS I/O
/// occurs while opening the style menu, even with an offline library.
fn style_preview(style: Style) -> gtk::DrawingArea {
    let area = gtk::DrawingArea::new();
    area.set_content_width(66);
    area.set_content_height(42);
    area.set_draw_func(move |_, cr, _, _| {
        let rect = |x: f64, y: f64, w: f64, h: f64, rgb: (f64, f64, f64)| {
            cr.set_source_rgb(rgb.0, rgb.1, rgb.2);
            cr.rectangle(x, y, w, h);
            let _ = cr.fill();
        };
        let photo = |x: f64, y: f64, w: f64, h: f64, tint: usize| {
            let sky = [
                (0.42, 0.68, 0.80), (0.78, 0.61, 0.47),
                (0.49, 0.66, 0.63), (0.57, 0.57, 0.79),
            ][tint % 4];
            let ground = [
                (0.25, 0.42, 0.38), (0.55, 0.42, 0.28),
                (0.29, 0.45, 0.37), (0.40, 0.38, 0.58),
            ][tint % 4];
            rect(x, y, w, h, sky);
            rect(x, y + h * 0.64, w, h * 0.36, ground);
            rect(x + w * 0.68, y + h * 0.16, w * 0.12, h * 0.13, (0.98, 0.86, 0.61));
        };
        rect(0.0, 0.0, 66.0, 42.0, (0.12, 0.16, 0.21));
        match style {
            Style::Mosaic => {
                for row in 0..2 {
                    for col in 0..3 {
                        photo(3.0 + col as f64 * 21.0, 3.0 + row as f64 * 19.0,
                            18.0, 16.0, row * 3 + col);
                    }
                }
            }
            Style::Grid => {
                for row in 0..2 {
                    for col in 0..3 {
                        photo(col as f64 * 22.0, row as f64 * 21.0, 21.0, 20.0,
                            row * 3 + col);
                    }
                }
            }
            Style::Floating => {
                for (x, y, tilt, tint) in [(6., 4., -0.09, 0), (36., 3., 0.08, 1),
                    (3., 22., 0.10, 2), (32., 21., -0.1, 3)]
                {
                    let _ = cr.save();
                    cr.translate(x+13.,y+9.);
                    cr.rotate(tilt);
                    rect(-13., -9., 26., 18., (0.95,0.95,0.92));
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
                rect(9., 6., 48., 30., (0.22,0.27,0.33));
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
    });
    area
}

/// Compact style selector with miniature previews in its popover. Selection
/// is stored in a Cell and propagated to related controls by the callback.
fn style_picker(initial: Style, on_change: impl Fn(Style) + 'static)
    -> (gtk::MenuButton, Rc<Cell<Style>>)
{
    let chosen = Rc::new(Cell::new(initial));
    let menu = gtk::MenuButton::new();
    menu.set_hexpand(true);
    menu.set_tooltip_text(Some("Choose a screensaver presentation"));
    let selected_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let selected_icon = gtk::Image::from_icon_name(initial.icon());
    let selected_title = gtk::Label::new(Some(initial.label()));
    selected_title.set_hexpand(true);
    selected_title.set_xalign(0.0);
    let arrow = gtk::Image::from_icon_name("pan-down-symbolic");
    selected_row.append(&selected_icon);
    selected_row.append(&selected_title);
    selected_row.append(&arrow);
    menu.set_child(Some(&selected_row));

    let popover = gtk::Popover::new();
    popover.set_has_arrow(false);
    let options = gtk::Box::new(gtk::Orientation::Vertical, 2);
    options.set_margin_top(5);
    options.set_margin_bottom(5);
    options.set_margin_start(5);
    options.set_margin_end(5);
    let checks: Rc<RefCell<Vec<gtk::Image>>> = Rc::new(RefCell::new(Vec::new()));
    let on_change: Rc<dyn Fn(Style)> = Rc::new(on_change);
    for candidate in Style::CHOICES {
        let option = gtk::Button::new();
        option.add_css_class("flat");
        option.set_tooltip_text(Some(candidate.subtitle()));
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 11);
        row.set_margin_top(3);
        row.set_margin_bottom(3);
        row.set_margin_start(5);
        row.set_margin_end(5);
        row.append(&style_preview(candidate));
        let text = gtk::Box::new(gtk::Orientation::Vertical, 1);
        text.set_valign(gtk::Align::Center);
        text.set_hexpand(true);
        let name = gtk::Label::new(Some(candidate.label()));
        name.set_xalign(0.0);
        let detail = gtk::Label::new(Some(candidate.subtitle()));
        detail.set_xalign(0.0);
        detail.add_css_class("dim-label");
        detail.add_css_class("caption");
        text.append(&name);
        text.append(&detail);
        row.append(&text);
        let check = gtk::Image::from_icon_name("object-select-symbolic");
        check.set_visible(candidate == initial);
        row.append(&check);
        checks.borrow_mut().push(check);
        option.set_child(Some(&row));

        let chosen = Rc::clone(&chosen);
        let checks = Rc::clone(&checks);
        let selected_title = selected_title.clone();
        let selected_icon = selected_icon.clone();
        let on_change = Rc::clone(&on_change);
        let weak_popover = popover.downgrade();
        option.connect_clicked(move |_| {
            chosen.set(candidate);
            selected_title.set_text(candidate.label());
            selected_icon.set_icon_name(Some(candidate.icon()));
            for (i, check) in checks.borrow().iter().enumerate() {
                check.set_visible(i == candidate.dropdown_index() as usize);
            }
            on_change(candidate);
            if let Some(popover) = weak_popover.upgrade() {
                popover.popdown();
            }
        });
        options.append(&option);
    }
    let scroller = gtk::ScrolledWindow::new();
    scroller.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    scroller.set_min_content_height(260);
    scroller.set_max_content_height(420);
    scroller.set_child(Some(&options));
    popover.set_child(Some(&scroller));
    menu.set_popover(Some(&popover));
    (menu, chosen)
}

/// Opens the settings/launch dialog without changing PIC's current viewer.
pub fn open(parent: &adw::ApplicationWindow, connection: Rc<RefCell<Connection>>, selected_ids: Vec<i64>) {
    let current = Preferences::load(&connection.borrow());
    let mut sources = vec![
        ("All Photos".to_owned(), Source::All),
        ("Favourites".to_owned(), Source::Favorites),
    ];
    if !selected_ids.is_empty() {
        sources.insert(0, (
            format!("Selected Photos ({})", selected_ids.len()),
            Source::Selected(selected_ids),
        ));
    }
    if let Ok(albums) = db::albums(&connection.borrow()) {
        sources.extend(
            albums.into_iter().map(|album| (format!("Album · {}", album.name), Source::Album(album.id))),
        );
    }
    if let Ok(folders) = db::folders(&connection.borrow()) {
        sources.extend(
            folders.into_iter()
                .map(|folder| (format!("Folder · {}", folder.name), Source::Folder(folder.id))),
        );
    }

    let dialog = gtk::Window::builder()
        .title("PIC Screensaver")
        .default_width(444)
        .resizable(false)
        .modal(true)
        .transient_for(parent)
        .build();
    let body = gtk::Box::new(gtk::Orientation::Vertical, 10);
    body.set_margin_top(18);
    body.set_margin_bottom(18);
    body.set_margin_start(18);
    body.set_margin_end(18);

    let heading = gtk::Label::new(Some("Your photos on display"));
    heading.add_css_class("title-2");
    heading.set_xalign(0.0);
    body.append(&heading);
    let explanation = gtk::Label::new(Some(
        "Choose a style and collection. Press Esc or move the pointer to exit."
    ));
    explanation.set_xalign(0.0);
    explanation.set_wrap(true);
    explanation.add_css_class("dim-label");
    body.append(&explanation);

    let picasa_transition = gtk::DropDown::from_strings(&["Flip", "Crossfade", "Cut"]);
    picasa_transition.set_selected(current.picasa_transition.dropdown_index());
    picasa_transition.set_tooltip_text(Some("Picasa Mosaic and Picture Pile only"));

    // Keep large mosaic counts as integers; GTK spin adjustment uses f64.
    let mosaic_tiles = gtk::Entry::new();
    mosaic_tiles.set_text(&current.mosaic_tiles.to_string());
    mosaic_tiles.set_width_chars(8);
    mosaic_tiles.set_input_purpose(gtk::InputPurpose::Digits);
    mosaic_tiles.set_tooltip_text(Some("Positive whole number of square mosaic tiles"));

    let shuffle_tiles_button = gtk::CheckButton::with_label("Shuffle tiles");
    shuffle_tiles_button.set_active(current.shuffle_tiles);
    shuffle_tiles_button.set_tooltip_text(Some("Change tile positions in a shuffled order"));

    let transition_for_style = picasa_transition.clone();
    let mosaic_for_style = mosaic_tiles.clone();
    let shuffle_for_style = shuffle_tiles_button.clone();
    let (style_picker, style_selected) = style_picker(current.style, move |style| {
        transition_for_style.set_sensitive(!style.single_photo());
        mosaic_for_style.set_sensitive(style.configurable_tiles());
        shuffle_for_style.set_sensitive(!style.single_photo());
    });
    picasa_transition.set_sensitive(!current.style.single_photo());
    mosaic_tiles.set_sensitive(current.style.configurable_tiles());
    shuffle_tiles_button.set_sensitive(!current.style.single_photo());
    body.append(&field("Style", &style_picker));

    let source_names: Vec<String> = sources.iter().map(|(name, _)| name.clone()).collect();
    let names: Vec<&str> = source_names.iter().map(String::as_str).collect();
    let source = gtk::DropDown::from_strings(&names);
    // Playing an explicit selection should default to precisely those images,
    // even if the saved source was an album or All Photos.
    let selected = if matches!(sources.first().map(|(_, kind)| kind), Some(Source::Selected(_))) {
        0
    } else {
        sources.iter()
            .position(|(_, kind)| kind.key() == current.source)
            .unwrap_or(0)
    };
    source.set_selected(selected as u32);
    body.append(&field("Photos", &source));

    let duration = gtk::SpinButton::with_range(3.0, 30.0, 1.0);
    duration.set_value(current.seconds as f64);
    duration.set_tooltip_text(Some("Seconds per photo or photo-wall change"));

    // Keep duration and tile count together without increasing the dialog
    // height or introducing another full-width settings row.
    let timing_row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let duration_group = gtk::Box::new(gtk::Orientation::Horizontal, 7);
    duration_group.set_hexpand(true);
    let duration_label = gtk::Label::new(Some("Duration"));
    duration_group.append(&duration_label);
    duration.set_hexpand(true);
    duration_group.append(&duration);
    let tiles_group = gtk::Box::new(gtk::Orientation::Horizontal, 7);
    tiles_group.set_hexpand(true);
    let tiles_label = gtk::Label::new(Some("Mosaic tiles"));
    tiles_group.append(&tiles_label);
    mosaic_tiles.set_hexpand(true);
    tiles_group.append(&mosaic_tiles);
    timing_row.append(&duration_group);
    timing_row.append(&tiles_group);
    body.append(&timing_row);

    // A second, independent monotonic timer controls the amount of each
    // change. Do not tie animation duration to the photo hold interval.
    let transition_time = gtk::SpinButton::with_range(0.2, 2.5, 0.1);
    transition_time.set_digits(1);
    transition_time.set_value(current.transition_ms as f64 / 1000.0);
    transition_time.set_tooltip_text(Some("Seconds to blend/flip between photos"));
    let timing_options = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let transition_mode = field("Transition", &picasa_transition);
    transition_mode.set_hexpand(true);
    timing_options.append(&transition_mode);
    let animation_time = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let animation_label = gtk::Label::new(Some("Fade (s)"));
    animation_time.append(&animation_label);
    animation_time.append(&transition_time);
    timing_options.append(&animation_time);
    body.append(&timing_options);

    let shuffle_row = gtk::Box::new(gtk::Orientation::Horizontal, 16);
    let shuffle_button = gtk::CheckButton::with_label("Shuffle photos");
    shuffle_button.set_active(current.shuffle);
    shuffle_row.append(&shuffle_button);
    shuffle_row.append(&shuffle_tiles_button);
    body.append(&shuffle_row);

    let overlays_row = gtk::Box::new(gtk::Orientation::Horizontal, 16);
    let captions_button = gtk::CheckButton::with_label("Photo captions");
    captions_button.set_active(current.captions);
    captions_button.set_tooltip_text(Some("Show photo filename and date from the library"));
    let clock_button = gtk::CheckButton::with_label("Clock");
    clock_button.set_active(current.clock);
    clock_button.set_tooltip_text(Some("Show a discreet local-time clock in the top-right corner"));
    overlays_row.append(&captions_button);
    overlays_row.append(&clock_button);
    body.append(&field("Overlays", &overlays_row));

    let notice = gtk::Label::new(None);
    notice.set_visible(false);
    notice.set_wrap(true);
    notice.set_xalign(0.0);
    notice.add_css_class("dim-label");
    body.append(&notice);

    let footer = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    footer.set_margin_top(6);
    footer.set_halign(gtk::Align::End);
    let cancel = gtk::Button::with_label("Cancel");
    let start = gtk::Button::with_label("Start Screensaver");
    start.add_css_class("suggested-action");
    footer.append(&cancel);
    footer.append(&start);
    body.append(&footer);
    dialog.set_child(Some(&body));
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
        let requested_tiles = match mosaic_tiles.text().trim().parse::<usize>() {
            Ok(count) if count > 0 => count,
            _ => {
                notice.set_text("Mosaic tiles must be a positive whole number.");
                notice.set_visible(true);
                return;
            }
        };
        let prefs = Preferences {
            style: style_selected.get(),
            source: photo_source.key(),
            seconds: duration.value_as_int().clamp(3, 30) as u32,
            shuffle: shuffle_button.is_active(),
            shuffle_tiles: shuffle_tiles_button.is_active(),
            mosaic_tiles: requested_tiles,
            picasa_transition: PicasaTransition::from_dropdown(picasa_transition.selected()),
            transition_ms: (transition_time.value() * 1000.0).round() as u32,
            captions: captions_button.is_active(),
            clock: clock_button.is_active(),
        };
        let mut photos = match photo_source.photos(&connection.borrow()) {
            Ok(photos) => photos,
            Err(error) => {
                notice.set_text(&format!("Could not load photos: {error}"));
                notice.set_visible(true);
                return;
            }
        };
        if photos.is_empty() {
            notice.set_text("This collection has no photos. Choose another source.");
            notice.set_visible(true);
            return;
        }
        prefs.save(&connection.borrow());
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
    Pixels { width: width as i32, height: height as i32, data: image.into_raw() }
}

/// Decode the already-generated local cache without touching the original.
/// Remote NFS/SMB reads MUST NOT run on this worker: a reachable server with
/// missing exports can block source reads indefinitely.
fn decode_cached_preview(slide: &Slide) -> Option<Pixels> {
    let path = thumbnail::existing_cache_path(
        &slide.path, slide.mtime, slide.size_bytes,
    ).ok().flatten()?;
    let recipe = edit::EditRecipe::decode(&slide.edit_recipe);
    edit::render::render_thumbnail(
        path.to_str()?, slide.rotation, &recipe,
    ).ok().map(texture_pixels)
}

fn decode_original(
    slide: &Slide,
    dimensions: (u32, u32),
    cancelled: &AtomicBool,
) -> anyhow::Result<Pixels> {
    let decoded = thumbnail::decode_for_viewer_with_cancel(
        &slide.path, dimensions.0, dimensions.1, None,
        || cancelled.load(Ordering::Relaxed),
    )?;
    anyhow::ensure!(!cancelled.load(Ordering::Relaxed), "screensaver decode cancelled");
    let recipe = edit::EditRecipe::decode(&slide.edit_recipe);
    Ok(texture_pixels(edit::render::apply_recipe(
        edit::render::apply_library_rotation(decoded, slide.rotation), &recipe,
    )))
}

/// Keep cached previews responsive even when an in-flight remote RAW read is
/// hung. Original upgrades run on separate local and network workers.
fn start_decode_worker(
    slides: Vec<Slide>,
    style: Style,
    mosaic_tiles: usize,
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
                if cancelled.load(Ordering::Relaxed) { break; }
                let Some(photo) = slides.get(index) else { continue; };
                let dimensions = match style {
                    Style::Mosaic | Style::Grid => {
                        let edge = if mosaic_tiles <= 6 { 1536 }
                            else if mosaic_tiles <= 12 { 1152 }
                            else if mosaic_tiles <= 30 { 896 }
                            else if mosaic_tiles <= 100 { 640 }
                            else if mosaic_tiles <= 400 { 384 }
                            else if mosaic_tiles <= 1600 { 192 }
                            else if mosaic_tiles <= 6400 { 96 }
                            else { 48 };
                        (edge, edge)
                    }
                    Style::PicturePile => (1920, 1280),
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
                if cancelled.load(Ordering::Relaxed) { break; }
                if std::env::var_os("PIC_SCREENSAVER_TRACE").is_some() {
                    eprintln!(
                        "PIC_SCREEN decode kind=original network={is_remote} index={index} ok={} ms={}",
                        decoded.is_ok(), started.elapsed().as_millis(),
                    );
                }
                if let Err(error) = &decoded {
                    eprintln!("PIC screensaver: original unavailable {}: {error}", photo.path);
                }
                if results.send(ImageResult {
                    index,
                    high_quality: true,
                    pixels: decoded.ok(),
                }).is_err() { break; }
            }
        });
    }

    // The preview worker never opens an original or tests an NFS mount.
    // It can still fill the photo wall when the network decoder is stalled.
    std::thread::spawn(move || {
        while let Ok(index) = requests.recv() {
            if cancelled.load(Ordering::Relaxed) { break; }
            let Some(photo) = slides.get(index) else { continue; };
            let started = Instant::now();
            let cached = decode_cached_preview(photo);
            if cancelled.load(Ordering::Relaxed) { break; }
            if std::env::var_os("PIC_SCREENSAVER_TRACE").is_some() {
                eprintln!(
                    "PIC_SCREEN decode kind=cache index={index} hit={} ms={}",
                    cached.is_some(), started.elapsed().as_millis(),
                );
            }
            if results.send(ImageResult {
                index, high_quality: false, pixels: cached,
            }).is_err() { break; }

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
    transition: Duration,
    switched_at: Instant,
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
    fn new(style: Style, count: usize, seconds: u32, shuffle_tiles: bool, mosaic_tiles: usize) -> Self {
        let desired_slots = match style {
            Style::Mosaic | Style::Grid => mosaic_tiles.max(1),
            Style::PicturePile => PICTURE_PILE_SLOTS,
            Style::Floating => FLOATING_SLOTS,
            _ => 0,
        };
        let slots = (0..desired_slots.min(count)).collect::<Vec<_>>();
        let mut tile_order = (0..slots.len()).collect::<Vec<_>>();
        if shuffle_tiles {
            shuffle(&mut tile_order);
        }
        Self {
            style, count,
            interval: Duration::from_secs(seconds as u64),
            transition: Duration::from_millis(900),
            switched_at: Instant::now(),
            paused_at: None,
            next: if style.single_photo() { 1 } else { slots.len() },
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
        if self.count == 0 { return; }
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
        if self.count == 0 { return; }
        if self.style.single_photo() {
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
            let blank_slots = self.slots.iter()
                .filter(|id| self.failed.contains(id) && !self.frames.contains_key(id))
                .count();
            let mut available = blank_slots.saturating_sub(self.backfill_pending.len());
            while available > 0 && self.backfill_cursor < self.count {
                let index = self.backfill_cursor;
                self.backfill_cursor += 1;
                if self.slots.contains(&index) || self.requested.contains(&index)
                    || self.frames.contains_key(&index) || self.failed.contains(&index)
                {
                    continue;
                }
                self.backfill_pending.insert(index);
                self.request(index, tx);
                available -= 1;
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
        self.slots.iter().position(|id| {
            self.failed.contains(id) && !self.frames.contains_key(id)
        })
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
        if self.frames.get(&result.index).is_some_and(|f| f.high_quality && !result.high_quality) {
            return;
        }
        let bytes = glib::Bytes::from_owned(pixels.data);
        let texture: gtk::gdk::Texture = gtk::gdk::MemoryTexture::new(
            pixels.width, pixels.height, gtk::gdk::MemoryFormat::R8g8b8a8,
            &bytes, pixels.width as usize * 4
        ).upcast();
        self.next_revision = self.next_revision.wrapping_add(1);
        self.frames.insert(result.index, Surface {
            texture, high_quality: result.high_quality,
            revision: self.next_revision,
        });
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
        let max_frames = if self.style.single_photo() { 5 }
            else { self.slots.len() + PREFETCH + 2 };
        while self.frames.len() > max_frames {
            let protected = |id: usize| id == self.current
                || self.previous == Some(id)
                || self.slots.contains(&id)
                || self.fading_slot.is_some_and(|(_, old)| old == id);
            let Some(pos) = self.order.iter().position(|id| !protected(*id)) else { break };
            let Some(old) = self.order.remove(pos) else { break };
            self.frames.remove(&old);
            // Do not clear requested: the original might still be decoding.
            // Clearing it here requeues the same network RAW every frame.
        }
    }
        fn available(&self, index: usize) -> bool {
        self.frames.contains_key(&index)
    }
    fn update(&mut self, tx: &Sender<usize>) {
        // Do not keep an obsolete outgoing image/slot alive after its fade.
        // This also allows the LRU to release old GPU textures.
        if self.progress() >= 1.0 {
            self.previous = None;
            self.fading_slot = None;
        }
        self.prefetch(tx);
        if self.paused_at.is_some() || self.count == 0 || self.elapsed() < self.interval {
            return;
        }
        // If every selected photo is already on screen, do not duplicate it
        // in Mosaic/Grid/Floating. Picture Pile instead cycles one existing
        // print to the front so a small scrapbook remains animated.
        if !self.style.single_photo() && self.count <= self.slots.len() {
            if self.style == Style::PicturePile && !self.slots.is_empty() {
                let slot = self.tile_order[self.changes % self.tile_order.len()];
                let id = self.slots[slot];
                self.fading_slot = Some((slot, id));
                self.switched_at = Instant::now();
                self.changes += 1;
            }
            return;
        }
        if self.style.single_photo() {
            for _ in 0..self.count {
                let index = self.next % self.count;
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
                if self.available(index) {
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
        self.paused_at.unwrap_or_else(Instant::now).duration_since(self.switched_at)
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
        (self.elapsed().as_secs_f64() / self.transition.as_secs_f64().max(0.2))
            .clamp(0.0, 1.0)
    }

}

// Smooth start/finish and alternate pan directions per photograph.
fn smootherstep(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
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
    if slides.is_empty() { return; }
    let style = prefs.style.resolved();
    let player = Rc::new(RefCell::new(Player::new(
        style, slides.len(), prefs.seconds, prefs.shuffle_tiles, prefs.mosaic_tiles,
    )));
    player.borrow_mut().transition =
        Duration::from_millis(prefs.transition_ms.clamp(200, 2500) as u64);
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
    start_decode_worker(slides, style, active_mosaic_tiles, request_rx, result_tx, Arc::clone(&cancelled));

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
        style, player.borrow().slots.len(), prefs.picasa_transition,
    )));
    stage.borrow().root.set_focusable(true);
    let stage_overlay = gtk::Overlay::new();
    stage_overlay.set_child(Some(&stage.borrow().root));
    let caption = gtk::Label::new(None);
    caption.add_css_class("screensaver-caption");
    caption.set_halign(gtk::Align::Center);
    caption.set_valign(gtk::Align::End);
    caption.set_margin_bottom(26);
    caption.set_margin_start(18);
    caption.set_margin_end(18);
    caption.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    caption.set_visible(false);
    stage_overlay.add_overlay(&caption);

    let clock = gtk::Label::new(None);
    clock.add_css_class("screensaver-clock");
    clock.set_halign(gtk::Align::End);
    clock.set_valign(gtk::Align::Start);
    clock.set_margin_top(22);
    clock.set_margin_end(24);
    clock.set_can_target(false);
    clock.set_visible(prefs.clock);
    if prefs.clock {
        clock.set_label(&local_clock_text());
    }
    stage_overlay.add_overlay(&clock);
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
        if closing.replace(true) { return; }
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
        if matches!(key, gtk::gdk::Key::space | gtk::gdk::Key::p | gtk::gdk::Key::P) {
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
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() / 60
    );
    // The GTK frame clock drives composited fades at the display refresh
    // rate. A 50ms GLib timer was limited to ~20fps and caused visible steps.
    // The callback is owned by the fullscreen widget and naturally stops
    // when that widget is no longer being presented.
    screen.add_tick_callback(move |_, _| {
        if cancelled.load(Ordering::Relaxed) {
            return glib::ControlFlow::Break;
        }
        let Some(screen) = weak_screen.upgrade() else {
            return glib::ControlFlow::Break;
        };
        if !screen.is_visible() { return glib::ControlFlow::Break; }
        {
            let mut player = player.borrow_mut();
            for _ in 0..2 {
                match result_rx.try_recv() {
                    Ok(result) => player.receive(result),
                    Err(_) => break,
                }
            }
            player.update(&request_tx);
            stage.borrow_mut().sync(&player);
            if prefs.clock {
                let minute = SystemTime::now().duration_since(UNIX_EPOCH)
                    .unwrap_or_default().as_secs() / 60;
                if minute != last_clock_minute.get() {
                    last_clock_minute.set(minute);
                    clock.set_label(&local_clock_text());
                }
            }
            if prefs.captions {
                // For a wall, show the newest tile's label; for fullscreen,
                // show the current single photograph.
                let active_id = if style.single_photo() { Some(player.current) }
                    else { player.fading_slot.map(|(slot, _)| player.slots[slot]) };
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
    if let Some(monitor) = parent.native()
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
    fn last_screensaver_settings_round_trip_in_library_db() {
        let connection = Connection::open_in_memory().unwrap();
        connection.execute_batch(
            "CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);"
        ).unwrap();
        let prefs = Preferences {
            style: Style::Floating,
            source: Source::Album(42).key(),
            seconds: 14,
            shuffle: false,
            shuffle_tiles: true,
            mosaic_tiles: 24,
            picasa_transition: PicasaTransition::Crossfade,
            transition_ms: 1700,
            captions: true,
            clock: true,
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
        assert!(restored.captions);
        assert!(restored.clock);
        let another = Preferences { clock: false, style: Style::Fade, ..prefs };
        another.save(&connection);
        let restored = Preferences::load(&connection);
        assert_eq!(restored.style, Style::Fade);
        assert!(!restored.clock);
        assert!(restored.captions);
    }

    #[test]
    fn compact_style_picker_labels_and_previews_are_distinct() {
        let styles = Style::CHOICES;
        let mut labels = HashSet::new();
        for style in styles {
            assert!(!style.label().is_empty());
            assert!(!style.subtitle().is_empty());
            assert!(!style.icon().is_empty());
            assert!(labels.insert(style.label()));
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
        assert_eq!(PicasaTransition::from_saved("crossfade"), PicasaTransition::Crossfade);
        assert_eq!(PicasaTransition::from_saved("cut"), PicasaTransition::Cut);
        assert_eq!(PicasaTransition::from_saved("unknown"), PicasaTransition::Crossfade);
        for (index, mode) in [(0, PicasaTransition::Flip), (1, PicasaTransition::Crossfade),
                              (2, PicasaTransition::Cut)] {
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
        player.receive(ImageResult { index: 0, high_quality: false, pixels: None });
        assert!(!player.failed.contains(&0));
        player.receive(ImageResult { index: 0, high_quality: true, pixels: None });
        assert!(player.failed.contains(&0));
    }

    #[test]
    fn mosaic_loading_does_not_scan_ahead_while_previews_are_pending() {
        let mut player = Player::new(Style::Mosaic, 5400, 6, false, 10);
        let (tx, rx) = mpsc::channel();
        for _ in 0..200 { player.prefetch(&tx); }
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
        player.receive(ImageResult { index: 3, high_quality: true, pixels: None });
        for _ in 0..120 { player.prefetch(&tx); }
        assert_eq!(rx.try_iter().collect::<Vec<_>>(), vec![13]);
        assert_eq!(player.backfill_pending.len(), 1);
        assert_eq!(player.backfill_cursor, 14);
        // Once candidate 13 finishes without a preview, allow precisely one
        // subsequent replacement request rather than a library-wide scan.
        player.receive(ImageResult { index: 13, high_quality: true, pixels: None });
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
        player.receive(ImageResult { index: 107, high_quality: false, pixels: None });
        for _ in 0..100 { player.request(107, &tx); }
        assert_eq!(rx.try_iter().collect::<Vec<_>>(), vec![107]);
        assert!(player.requested.contains(&107));
        player.receive(ImageResult { index: 107, high_quality: true, pixels: None });
        assert!(!player.requested.contains(&107));
        assert!(player.failed.contains(&107));
        player.request(107, &tx);
        assert_eq!(rx.try_iter().count(), 0);
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
    fn picture_pile_reorders_when_all_photos_are_already_on_screen() {
        let mut pile = Player::new(Style::PicturePile, 4, 3, false, 10);
        let (tx, _rx) = mpsc::channel();
        pile.switched_at -= Duration::from_secs(5);
        pile.update(&tx);
        assert_eq!(pile.changes, 1);
        assert_eq!(pile.fading_slot, Some((0, 0)));
        pile.switched_at -= Duration::from_secs(5);
        pile.update(&tx);
        assert_eq!(pile.changes, 2);
        assert_eq!(pile.fading_slot, Some((1, 1)));
        assert_eq!(pile.slots, vec![0, 1, 2, 3]);
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

    #[test]
    fn player_paces_changes() {
        let player = Player::new(Style::IPhoto, 5, 6, true, DEFAULT_MOSAIC_TILES);
        assert_eq!(player.current, 0);
        assert_eq!(player.next, 1);
        assert_eq!(player.interval, Duration::from_secs(6));
        assert_eq!(Player::new(Style::Mosaic, 20, 3, true, DEFAULT_MOSAIC_TILES).slots.len(), 6);
        let ordered = Player::new(Style::Mosaic, 20, 3, false, DEFAULT_MOSAIC_TILES);
        assert_eq!(ordered.tile_order, (0..6).collect::<Vec<_>>());
        let shuffled = Player::new(Style::PicturePile, 20, 3, true, DEFAULT_MOSAIC_TILES);
        let mut positions = shuffled.tile_order.clone();
        positions.sort_unstable();
        assert_eq!(positions, (0..PICTURE_PILE_SLOTS).collect::<Vec<_>>());
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
            assert_eq!(pile.slots.len(), PICTURE_PILE_SLOTS);
        }
        let few = Player::new(Style::Mosaic, 3, 5, false, 30);
        assert_eq!(few.slots.len(), 3);
        assert_eq!(few.next, 3);
        assert_eq!(Player::new(Style::Mosaic, 100, 5, false, 1).slots.len(), 1);
        assert_eq!(Player::new(Style::Grid, 200, 5, false, 12).slots.len(), 12);
        assert_eq!(Player::new(Style::Floating, 200, 5, false, 12).slots.len(), FLOATING_SLOTS);
        assert!(Player::new(Style::Fade, 200, 5, false, 12).slots.is_empty());
        assert_eq!(Player::new(Style::PicturePile, 200, 5, false, 12).slots.len(), PICTURE_PILE_SLOTS);
        assert_eq!(Player::new(Style::Mosaic, 2000, 5, false, 1500).slots.len(), 1500);
        assert_eq!(Player::new(Style::Mosaic, 100, 5, false, usize::MAX).slots.len(), 100);
    }
}
