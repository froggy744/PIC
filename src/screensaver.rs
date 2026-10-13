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
const PICTURE_PILE_SLOTS: usize = 6;
const PREFETCH: usize = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Style { Mosaic, PicturePile, IPhoto, Surprise }
impl Style {
    fn from_saved(v: &str) -> Self {
        match v { "pile" => Self::PicturePile, "iphoto" => Self::IPhoto,
            "surprise" => Self::Surprise, _ => Self::Mosaic }
    }
    fn key(self) -> &'static str {
        match self { Self::Mosaic => "mosaic", Self::PicturePile => "pile",
            Self::IPhoto => "iphoto", Self::Surprise => "surprise" }
    }
    fn dropdown_index(self) -> u32 {
        match self { Self::Mosaic => 0, Self::PicturePile => 1,
            Self::IPhoto => 2, Self::Surprise => 3 }
    }
    fn from_dropdown(v: u32) -> Self {
        match v { 1 => Self::PicturePile, 2 => Self::IPhoto,
            3 => Self::Surprise, _ => Self::Mosaic }
    }
    fn resolved(self) -> Self {
        if self != Self::Surprise { return self; }
        match random_seed() % 3 { 0 => Self::Mosaic, 1 => Self::PicturePile, _ => Self::IPhoto }
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
}
impl From<db::Photo> for Slide {
    fn from(photo: db::Photo) -> Self {
        Self {
            path: photo.path,
            mtime: photo.mtime,
            size_bytes: photo.size_bytes,
            rotation: photo.rotation,
            edit_recipe: photo.edit_recipe,
        }
    }
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
    let line = gtk::Box::new(gtk::Orientation::Horizontal, 14);
    let title = gtk::Label::new(Some(label));
    title.set_xalign(0.0);
    title.set_width_chars(13);
    control.set_hexpand(true);
    line.append(&title);
    line.append(control);
    line
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
        .default_width(490)
        .resizable(false)
        .modal(true)
        .transient_for(parent)
        .build();
    let body = gtk::Box::new(gtk::Orientation::Vertical, 16);
    body.set_margin_top(24);
    body.set_margin_bottom(24);
    body.set_margin_start(24);
    body.set_margin_end(24);

    let heading = gtk::Label::new(Some("Your photographs, on display"));
    heading.add_css_class("title-2");
    heading.set_xalign(0.0);
    body.append(&heading);
    let explanation = gtk::Label::new(Some(
        "Choose a Picasa photo wall or an iPhoto cinematic slideshow.\nPress Esc or move the pointer to finish."
    ));
    explanation.set_xalign(0.0);
    explanation.set_wrap(true);
    explanation.add_css_class("dim-label");
    body.append(&explanation);

    let style = gtk::DropDown::from_strings(&[
        "Picasa · Mosaic",
        "Picasa · Picture Pile",
        "iPhoto · Cinematic Ken Burns",
        "Surprise me · Random style on start",
    ]);
    style.set_selected(current.style.dropdown_index());
    body.append(&field("Presentation", &style));

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
    body.append(&field("Seconds per change", &duration));

    let picasa_transition = gtk::DropDown::from_strings(&[
        "Flip", "Crossfade", "Cut",
    ]);
    picasa_transition.set_selected(current.picasa_transition.dropdown_index());
    picasa_transition.set_tooltip_text(Some(
        "Picasa Mosaic and Picture Pile only. iPhoto keeps its cinematic transition.",
    ));
    picasa_transition.set_sensitive(style.selected() != 2);
    let transition_for_style = picasa_transition.clone();
    style.connect_selected_notify(move |dropdown| {
        transition_for_style.set_sensitive(dropdown.selected() != 2);
    });
    body.append(&field("Picasa transition", &picasa_transition));

    // An entry (rather than GtkSpinButton) avoids imposing an arbitrary
    // maximum or losing large integer precision through its f64 adjustment.
    let mosaic_tiles = gtk::Entry::new();
    mosaic_tiles.set_text(&current.mosaic_tiles.to_string());
    mosaic_tiles.set_width_chars(10);
    mosaic_tiles.set_input_purpose(gtk::InputPurpose::Digits);
    mosaic_tiles.set_tooltip_text(Some(
        "Enter any positive whole number. Limited only by available photos and hardware; landscape photos crop to square tiles.",
    ));
    mosaic_tiles.set_sensitive(matches!(style.selected(), 0 | 3));
    let mosaic_tiles_for_style = mosaic_tiles.clone();
    style.connect_selected_notify(move |dropdown| {
        mosaic_tiles_for_style.set_sensitive(matches!(dropdown.selected(), 0 | 3));
    });
    body.append(&field("Mosaic tiles", &mosaic_tiles));

    let shuffle_row = gtk::Box::new(gtk::Orientation::Horizontal, 16);
    let shuffle_button = gtk::CheckButton::with_label("Shuffle photographs");
    shuffle_button.set_active(current.shuffle);
    let shuffle_tiles_button = gtk::CheckButton::with_label("Shuffle tiles");
    shuffle_tiles_button.set_active(current.shuffle_tiles);
    shuffle_tiles_button.set_tooltip_text(Some(
        "Replace mosaic and picture-pile positions in a shuffled order instead of left to right",
    ));
    shuffle_tiles_button.set_sensitive(style.selected() != 2);
    let tiles_for_style = shuffle_tiles_button.clone();
    style.connect_selected_notify(move |dropdown| {
        tiles_for_style.set_sensitive(dropdown.selected() != 2);
    });
    shuffle_row.append(&shuffle_button);
    shuffle_row.append(&shuffle_tiles_button);
    body.append(&shuffle_row);

    let notice = gtk::Label::new(None);
    notice.set_wrap(true);
    notice.set_xalign(0.0);
    notice.add_css_class("dim-label");
    body.append(&notice);

    let footer = gtk::Box::new(gtk::Orientation::Horizontal, 8);
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
                return;
            }
        };
        let prefs = Preferences {
            style: Style::from_dropdown(style.selected()),
            source: photo_source.key(),
            seconds: duration.value_as_int().clamp(3, 30) as u32,
            shuffle: shuffle_button.is_active(),
            shuffle_tiles: shuffle_tiles_button.is_active(),
            mosaic_tiles: requested_tiles,
            picasa_transition: PicasaTransition::from_dropdown(picasa_transition.selected()),
        };
        let mut photos = match photo_source.photos(&connection.borrow()) {
            Ok(photos) => photos,
            Err(error) => {
                notice.set_text(&format!("Could not load photos: {error}"));
                return;
            }
        };
        if photos.is_empty() {
            notice.set_text("This collection has no photos. Choose another source.");
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
                    Style::Mosaic => {
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
    switched_at: Instant,
    next: usize,
    current: usize,
    previous: Option<usize>,
    slots: Vec<usize>,
    fading_slot: Option<(usize, usize)>,
    changes: usize,
    shuffle_tiles: bool,
    tile_order: Vec<usize>,
    preload_cursor: usize,
    frames: HashMap<usize, Surface>,
    order: VecDeque<usize>,
    requested: HashSet<usize>,
    failed: HashSet<usize>,
    next_revision: u64,
}
impl Player {
    fn new(style: Style, count: usize, seconds: u32, shuffle_tiles: bool, mosaic_tiles: usize) -> Self {
        let desired_slots = match style {
            Style::Mosaic => mosaic_tiles.max(1),
            Style::PicturePile => PICTURE_PILE_SLOTS,
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
            switched_at: Instant::now(),
            next: if style == Style::IPhoto { 1 } else { slots.len() },
            current: 0,
            previous: None,
            slots,
            fading_slot: None,
            changes: 0,
            shuffle_tiles,
            tile_order,
            preload_cursor: 0,
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
        if self.requested.insert(index) {
            let _ = tx.send(index);
        }
    }
    fn prefetch(&mut self, tx: &Sender<usize>) {
        if self.count == 0 { return; }
        if self.style == Style::IPhoto {
            for i in 0..PREFETCH {
                self.request((self.current + i) % self.count, tx);
            }
        } else {
            // Queue every original tile once, then look for locally cached
            // substitutes if network-only originals cannot be decoded.
            // Work stays bounded per GTK frame for very large mosaics.
            if self.preload_cursor < self.slots.len() {
                let end = (self.preload_cursor + 24).min(self.slots.len());
                for slot in self.preload_cursor..end {
                    self.request(self.slots[slot], tx);
                }
                self.preload_cursor = end;
            } else if self.slots.iter().any(|id| !self.frames.contains_key(id)) {
                // Cache scans are cheap and never touch the remote shares.
                for _ in 0..16.min(self.count) {
                    let candidate = self.preload_cursor % self.count;
                    self.preload_cursor = self.preload_cursor.wrapping_add(1);
                    self.request(candidate, tx);
                }
            }
            for i in 0..PREFETCH {
                self.request((self.next + i) % self.count, tx);
            }
        }
    }
    fn receive(&mut self, result: ImageResult) {
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
        if self.style != Style::IPhoto && !self.slots.contains(&result.index) {
            // Replace a blank tile with the next successfully decoded image.
            // This also handles the case where the original is offline but
            // another photo in the library has a cached preview.
            if let Some(blank_slot) = self.slots.iter().position(|id| {
                !self.frames.contains_key(id)
            }) {
                self.slots[blank_slot] = result.index;
            }
        }
        self.order.retain(|index| *index != result.index);
        self.order.push_back(result.index);
        // Preserve every active tile plus a small prefetch margin.
        let max_frames = if self.style == Style::IPhoto { 5 }
            else { self.slots.len() + PREFETCH + 2 };
        while self.frames.len() > max_frames {
            let protected = |id: usize| id == self.current
                || self.previous == Some(id)
                || self.slots.contains(&id)
                || self.fading_slot.is_some_and(|(_, old)| old == id);
            let Some(pos) = self.order.iter().position(|id| !protected(*id)) else { break };
            let Some(old) = self.order.remove(pos) else { break };
            self.frames.remove(&old);
            self.requested.remove(&old);
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
        if self.count == 0 || self.switched_at.elapsed() < self.interval {
            return;
        }
        // If the chosen collection has fewer photos than square positions,
        // every photo is already visible: cycling would create duplicates.
        if self.style != Style::IPhoto && self.count <= self.slots.len() {
            return;
        }
        if self.style == Style::IPhoto {
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
    fn progress(&self) -> f64 {
        let fade = self.interval.as_secs_f64().min(1.4).max(0.4);
        (self.switched_at.elapsed().as_secs_f64() / fade).clamp(0.0, 1.0)
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
    screen.set_child(Some(&stage.borrow().root));

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
    keys.connect_key_pressed(move |_, _, _, _| {
        close_for_keys();
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
    fn style_and_source_keys_are_stable() {
        assert_eq!(Style::from_saved("iphoto"), Style::IPhoto);
        assert_eq!(Style::from_saved("picasa"), Style::Mosaic);
        assert_eq!(Style::from_saved("pile"), Style::PicturePile);
        assert_eq!(Style::Surprise.key(), "surprise");
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
        assert_eq!(positions, (0..6).collect::<Vec<_>>());
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
        assert_eq!(Player::new(Style::Mosaic, 2000, 5, false, 1500).slots.len(), 1500);
        assert_eq!(Player::new(Style::Mosaic, 100, 5, false, usize::MAX).slots.len(), 100);
    }
}
