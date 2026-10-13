//! Photo screensaver: a Picasa-inspired mosaic/picture pile and an iPhoto-inspired
//! cinematic slideshow. Decode/rendering runs off the GTK thread. All original
//! images and non-destructive edits remain untouched.
use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use gtk::prelude::*;
use gtk4 as gtk;
use libadwaita as adw;
use rusqlite::Connection;

use crate::{db, edit, thumbnail};

const STYLE_KEY: &str = "screensaver-style";
const SOURCE_KEY: &str = "screensaver-source";
const DURATION_KEY: &str = "screensaver-duration";
const SHUFFLE_KEY: &str = "screensaver-shuffle";
const MOSAIC_SLOTS: usize = 6;
const MAX_FRAMES: usize = 15;
const PREFETCH: usize = 10;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Style {
    Picasa,
    IPhoto,
    Surprise,
}
impl Style {
    fn from_saved(s: &str) -> Self {
        match s {
            "iphoto" => Self::IPhoto,
            "surprise" => Self::Surprise,
            _ => Self::Picasa,
        }
    }
    fn key(self) -> &'static str {
        match self {
            Self::Picasa => "picasa",
            Self::IPhoto => "iphoto",
            Self::Surprise => "surprise",
        }
    }
    fn dropdown_index(self) -> u32 {
        match self {
            Self::Picasa => 0,
            Self::IPhoto => 1,
            Self::Surprise => 2,
        }
    }
    fn from_dropdown(index: u32) -> Self {
        match index {
            1 => Self::IPhoto,
            2 => Self::Surprise,
            _ => Self::Picasa,
        }
    }
    fn resolved(self) -> Self {
        if self != Self::Surprise {
            return self;
        }
        if random_seed() & 1 == 0 {
            Self::Picasa
        } else {
            Self::IPhoto
        }
    }
}

#[derive(Clone, Debug)]
enum Source {
    All,
    Favorites,
    Album(i64),
    Folder(i64),
}
impl Source {
    fn key(&self) -> String {
        match self {
            Self::All => "all".into(),
            Self::Favorites => "favorites".into(),
            Self::Album(id) => format!("album:{id}"),
            Self::Folder(id) => format!("folder:{id}"),
        }
    }
    fn photos(&self, connection: &Connection) -> anyhow::Result<Vec<db::Photo>> {
        match self {
            Self::All => db::photos(connection, None, false, None),
            Self::Favorites => db::photos(connection, None, true, None),
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
}
impl Preferences {
    fn load(connection: &Connection) -> Self {
        let setting = |key| db::setting(connection, key).ok().flatten().unwrap_or_default();
        Self {
            style: Style::from_saved(&setting(STYLE_KEY)),
            source: setting(SOURCE_KEY),
            seconds: setting(DURATION_KEY).parse::<u32>().unwrap_or(6).clamp(3, 30),
            shuffle: setting(SHUFFLE_KEY) != "false",
        }
    }
    fn save(&self, connection: &Connection) {
        for (key, value) in [
            (STYLE_KEY, self.style.key().to_owned()),
            (SOURCE_KEY, self.source.clone()),
            (DURATION_KEY, self.seconds.to_string()),
            (SHUFFLE_KEY, self.shuffle.to_string()),
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
pub fn open(parent: &adw::ApplicationWindow, connection: Rc<RefCell<Connection>>) {
    let current = Preferences::load(&connection.borrow());
    let mut sources = vec![
        ("All Photos".to_owned(), Source::All),
        ("Favourites".to_owned(), Source::Favorites),
    ];
    if let Ok(albums) = db::albums(&connection.borrow()) {
        sources.extend(
            albums.into_iter().map(|album| (format!("Album · {}", album.name), Source::Album(album.id))),
        );
    }
    if let Ok(folders) = db::folders(&connection.borrow()) {
        sources.extend(
            folders.into_iter().filter(|f| f.photo_count > 0)
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
        "Picasa · Mosaic & picture pile",
        "iPhoto · Cinematic Ken Burns",
        "Surprise me · Random style on start",
    ]);
    style.set_selected(current.style.dropdown_index());
    body.append(&field("Presentation", &style));

    let source_names: Vec<String> = sources.iter().map(|(name, _)| name.clone()).collect();
    let names: Vec<&str> = source_names.iter().map(String::as_str).collect();
    let source = gtk::DropDown::from_strings(&names);
    let selected = sources.iter()
        .position(|(_, kind)| kind.key() == current.source)
        .unwrap_or(0);
    source.set_selected(selected as u32);
    body.append(&field("Photos", &source));

    let duration = gtk::SpinButton::with_range(3.0, 30.0, 1.0);
    duration.set_value(current.seconds as f64);
    duration.set_tooltip_text(Some("Seconds per photo or photo-wall change"));
    body.append(&field("Seconds per change", &duration));

    let shuffle_button = gtk::CheckButton::with_label("Shuffle photographs");
    shuffle_button.set_active(current.shuffle);
    body.append(&shuffle_button);

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
    let dialog_for_cancel = dialog.clone();
    cancel.connect_clicked(move |_| dialog_for_cancel.close());

    let parent = parent.clone();
    let dialog_for_start = dialog.clone();
    start.connect_clicked(move |_| {
        let source_index = source.selected() as usize;
        let Some((_, photo_source)) = sources.get(source_index) else {
            return;
        };
        let prefs = Preferences {
            style: Style::from_dropdown(style.selected()),
            source: photo_source.key(),
            seconds: duration.value_as_int().clamp(3, 30) as u32,
            shuffle: shuffle_button.is_active(),
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
        dialog_for_start.close();
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
fn cairo_pixels(image: image::RgbaImage) -> Pixels {
    let (width, height) = image.dimensions();
    let mut data = Vec::with_capacity(image.as_raw().len());
    // Cairo ARgb32 native-endian premultiplied pixels (BGRA on little-endian).
    for rgba in image.pixels() {
        let a = u16::from(rgba[3]);
        let r = ((u16::from(rgba[0]) * a + 127) / 255) as u8;
        let g = ((u16::from(rgba[1]) * a + 127) / 255) as u8;
        let b = ((u16::from(rgba[2]) * a + 127) / 255) as u8;
        #[cfg(target_endian = "little")]
        data.extend_from_slice(&[b, g, r, rgba[3]]);
        #[cfg(target_endian = "big")]
        data.extend_from_slice(&[rgba[3], r, g, b]);
    }
    Pixels { width: width as i32, height: height as i32, data }
}

fn start_decode_worker(
    slides: Vec<Slide>,
    style: Style,
    requests: Receiver<usize>,
    results: Sender<ImageResult>,
) {
    std::thread::spawn(move || {
        while let Ok(index) = requests.recv() {
            let Some(photo) = slides.get(index) else { continue };
            let recipe = edit::EditRecipe::decode(&photo.edit_recipe);
            let mut delivered = false;
            if let Ok(Some(path)) = thumbnail::existing_cache_path(
                &photo.path, photo.mtime, photo.size_bytes,
            ) {
                if let Some(path) = path.to_str() {
                    if let Ok(image) = edit::render::render_thumbnail(path, photo.rotation, &recipe) {
                        delivered = true;
                        if results.send(ImageResult {
                            index, high_quality: false, pixels: Some(cairo_pixels(image)),
                        }).is_err() {
                            break;
                        }
                    }
                }
            }
            // A mosaic uses cache-quality images; cinematic photos are decoded
            // at display resolution, with non-destructive edits applied.
            if style == Style::Picasa && delivered {
                continue;
            }
            let dimensions = if style == Style::Picasa { (1100, 900) } else { (2560, 1600) };
            match edit::render::render_for_viewer(
                &photo.path, photo.rotation, &recipe, dimensions.0, dimensions.1,
            ) {
                Ok(image) => {
                    if results.send(ImageResult {
                        index, high_quality: true, pixels: Some(cairo_pixels(image)),
                    }).is_err() {
                        break;
                    }
                }
                Err(error) if !delivered => {
                    eprintln!("PIC screensaver: cannot decode {}: {error}", photo.path);
                    if results.send(ImageResult {
                        index, high_quality: false, pixels: None,
                    }).is_err() {
                        break;
                    }
                }
                Err(_) => {}
            }
        }
    });
}

struct Surface {
    image: gtk::cairo::ImageSurface,
    high_quality: bool,
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
    frames: HashMap<usize, Surface>,
    order: VecDeque<usize>,
    requested: HashSet<usize>,
    failed: HashSet<usize>,
}
impl Player {
    fn new(style: Style, count: usize, seconds: u32) -> Self {
        Self {
            style, count,
            interval: Duration::from_secs(seconds as u64),
            switched_at: Instant::now(),
            next: if style == Style::IPhoto { 1 } else { MOSAIC_SLOTS.min(count) },
            current: 0,
            previous: None,
            slots: (0..MOSAIC_SLOTS.min(count)).collect(),
            fading_slot: None,
            changes: 0,
            frames: HashMap::new(),
            order: VecDeque::new(),
            requested: HashSet::new(),
            failed: HashSet::new(),
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
            for index in self.slots.clone() {
                self.request(index, tx);
            }
            for i in 0..PREFETCH {
                self.request((self.next + i) % self.count, tx);
            }
        }
    }
    fn receive(&mut self, result: ImageResult) {
        let Some(pixels) = result.pixels else {
            self.failed.insert(result.index);
            return;
        };
        if self.frames.get(&result.index).is_some_and(|f| f.high_quality && !result.high_quality) {
            return;
        }
        if let Ok(image) = gtk::cairo::ImageSurface::create_for_data(
            pixels.data,
            gtk::cairo::Format::ARgb32,
            pixels.width,
            pixels.height,
            pixels.width * 4,
        ) {
            self.frames.insert(result.index, Surface {
                image, high_quality: result.high_quality,
            });
            self.order.retain(|index| *index != result.index);
            self.order.push_back(result.index);
            while self.frames.len() > MAX_FRAMES {
                let Some(old) = self.order.pop_front() else { break };
                self.frames.remove(&old);
                self.requested.remove(&old);
            }
        }
    }
    fn available(&self, index: usize) -> bool {
        self.frames.contains_key(&index)
    }
    fn update(&mut self, tx: &Sender<usize>) {
        self.prefetch(tx);
        if self.count == 0 || self.switched_at.elapsed() < self.interval {
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
                if self.available(index) {
                    let slot = self.changes % self.slots.len().max(1);
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
    fn draw(&self, cr: &gtk::cairo::Context, width: f64, height: f64) {
        cr.set_source_rgb(0.045, 0.049, 0.059);
        let _ = cr.paint();
        if self.style == Style::IPhoto {
            self.draw_cinematic(cr, width, height);
        } else if (self.changes / MOSAIC_SLOTS) % 2 == 0 {
            self.draw_mosaic(cr, width, height);
        } else {
            self.draw_pile(cr, width, height);
        }
    }
    fn draw_cinematic(&self, cr: &gtk::cairo::Context, w: f64, h: f64) {
        let fade = self.progress();
        if let Some(previous) = self.previous {
            if fade < 1.0 {
                self.draw_slide(cr, previous, (0.0, 0.0, w, h), 1.06, 0.0, 1.0, false);
            }
        }
        let fraction = (self.switched_at.elapsed().as_secs_f64() / self.interval.as_secs_f64())
            .clamp(0.0, 1.0);
        self.draw_slide(cr, self.current, (0.0, 0.0, w, h),
            1.0 + 0.08 * fraction, (fraction - 0.5) * 0.03,
            if self.previous.is_some() { fade } else { 1.0 }, false);
    }
    fn draw_slide(&self, cr: &gtk::cairo::Context, index: usize,
        rect: (f64, f64, f64, f64), zoom: f64, pan: f64, alpha: f64, cover: bool
    ) {
        let Some(image) = self.frames.get(&index).map(|f| &f.image) else { return };
        let (x, y, w, h) = rect;
        if w <= 0.0 || h <= 0.0 { return; }
        let iw = image.width() as f64;
        let ih = image.height() as f64;
        if iw < 1.0 || ih < 1.0 { return; }
        let scale = if cover { (w / iw).max(h / ih) } else { (w / iw).min(h / ih) };
        let scale = scale * zoom;
        let dw = iw * scale;
        let dh = ih * scale;
        let dx = x + (w - dw) / 2.0 + pan * w;
        let dy = y + (h - dh) / 2.0 - pan * h * 0.32;
        let _ = cr.save();
        cr.rectangle(x, y, w, h);
        cr.clip();
        cr.translate(dx, dy);
        cr.scale(scale, scale);
        if cr.set_source_surface(image, 0.0, 0.0).is_ok() {
            let _ = cr.paint_with_alpha(alpha);
        }
        let _ = cr.restore();
    }
    fn draw_mosaic(&self, cr: &gtk::cairo::Context, w: f64, h: f64) {
        let margin = (w.min(h) * 0.018).max(9.0);
        let cw = (w - margin * 4.0) / 3.0;
        let ch = (h - margin * 3.0) / 2.0;
        let progress = self.progress();
        for (slot, &index) in self.slots.iter().enumerate() {
            let x = margin + (slot % 3) as f64 * (cw + margin);
            let y = margin + (slot / 3) as f64 * (ch + margin);
            cr.set_source_rgb(0.12, 0.13, 0.15);
            cr.rectangle(x, y, cw, ch);
            let _ = cr.fill();
            if let Some((changed, old)) = self.fading_slot {
                if changed == slot && progress < 1.0 {
                    self.draw_slide(cr, old, (x, y, cw, ch), 1.03, 0.0, 1.0, true);
                }
            }
            let alpha = if self.fading_slot.is_some_and(|(changed, _)| changed == slot) {
                progress
            } else { 1.0 };
            self.draw_slide(cr, index, (x, y, cw, ch), 1.025, 0.0, alpha, true);
        }
    }
    fn draw_pile(&self, cr: &gtk::cairo::Context, w: f64, h: f64) {
        let tw = (w * 0.30).min(h * 0.50);
        let th = tw * 0.76;
        let progress = self.progress();
        for (slot, &index) in self.slots.iter().enumerate() {
            let column = slot % 3;
            let row = slot / 3;
            let cx = w * (0.18 + 0.32 * column as f64) + (slot as f64 * 13.0).sin() * w * 0.018;
            let cy = h * (0.28 + 0.44 * row as f64) + (slot as f64 * 7.0).cos() * h * 0.020;
            let angle = (slot as f64 * 2.3).sin() * 0.11;
            let border = tw * 0.027;
            let _ = cr.save();
            cr.translate(cx, cy);
            cr.rotate(angle);
            cr.set_source_rgba(0.0, 0.0, 0.0, 0.38);
            cr.rectangle(-tw / 2.0 + 7.0, -th / 2.0 + 9.0, tw + 2.0 * border, th + 2.0 * border);
            let _ = cr.fill();
            cr.set_source_rgb(0.96, 0.95, 0.92);
            cr.rectangle(-tw / 2.0 - border, -th / 2.0 - border, tw + 2.0 * border, th + 2.0 * border);
            let _ = cr.fill();
            let rect = (-tw / 2.0, -th / 2.0, tw, th);
            if let Some((changed, old)) = self.fading_slot {
                if changed == slot && progress < 1.0 {
                    self.draw_slide(cr, old, rect, 1.0, 0.0, 1.0, true);
                }
            }
            let alpha = if self.fading_slot.is_some_and(|(changed, _)| changed == slot) {
                progress
            } else { 1.0 };
            self.draw_slide(cr, index, rect, 1.0, 0.0, alpha, true);
            let _ = cr.restore();
        }
    }
}

fn start_screensaver(parent: &adw::ApplicationWindow, slides: Vec<Slide>, prefs: Preferences) {
    if slides.is_empty() { return; }
    let style = prefs.style.resolved();
    let player = Rc::new(RefCell::new(Player::new(style, slides.len(), prefs.seconds)));
    let (request_tx, request_rx) = mpsc::channel::<usize>();
    let (result_tx, result_rx) = mpsc::channel::<ImageResult>();
    start_decode_worker(slides, style, request_rx, result_tx);

    let screen = gtk::Window::builder()
        .title("PIC Screensaver")
        .transient_for(parent)
        .decorated(false)
        .build();
    let canvas = gtk::DrawingArea::new();
    canvas.set_hexpand(true);
    canvas.set_vexpand(true);
    canvas.set_focusable(true);
    let drawing = player.clone();
    canvas.set_draw_func(move |_, cr, w, h| {
        drawing.borrow().draw(cr, w as f64, h as f64);
    });
    screen.set_child(Some(&canvas));

    let keys = gtk::EventControllerKey::new();
    let screen_for_keys = screen.clone();
    keys.connect_key_pressed(move |_, _, _, _| {
        screen_for_keys.close();
        glib::Propagation::Stop
    });
    screen.add_controller(keys);

    let clicked = gtk::GestureClick::new();
    let screen_for_click = screen.clone();
    clicked.connect_pressed(move |_, _, _, _| screen_for_click.close());
    screen.add_controller(clicked);

    let motion = gtk::EventControllerMotion::new();
    let origin = Rc::new(RefCell::new(None::<(f64, f64)>));
    let first_frame = Instant::now();
    let screen_for_motion = screen.clone();
    motion.connect_motion(move |_, x, y| {
        if first_frame.elapsed() < Duration::from_millis(850) {
            *origin.borrow_mut() = Some((x, y));
            return;
        }
        let mut position = origin.borrow_mut();
        if let Some((ox, oy)) = *position {
            if (x - ox).hypot(y - oy) > 26.0 {
                screen_for_motion.close();
            }
        } else {
            *position = Some((x, y));
        }
    });
    screen.add_controller(motion);

    let weak_screen = screen.downgrade();
    glib::timeout_add_local(Duration::from_millis(33), move || {
        let Some(screen) = weak_screen.upgrade() else {
            return glib::ControlFlow::Break;
        };
        if !screen.is_visible() { return glib::ControlFlow::Break; }
        {
            let mut player = player.borrow_mut();
            for _ in 0..16 {
                match result_rx.try_recv() {
                    Ok(result) => player.receive(result),
                    Err(_) => break,
                }
            }
            player.update(&request_tx);
        }
        canvas.queue_draw();
        glib::ControlFlow::Continue
    });
    screen.present();
    screen.fullscreen();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn style_and_source_keys_are_stable() {
        assert_eq!(Style::from_saved("iphoto"), Style::IPhoto);
        assert_eq!(Style::from_saved("picasa"), Style::Picasa);
        assert_eq!(Style::Surprise.key(), "surprise");
        assert_eq!(Source::Album(42).key(), "album:42");
        assert_eq!(Source::Folder(3).key(), "folder:3");
    }
    #[test]
    fn shuffle_preserves_items() {
        let mut values: Vec<usize> = (0..100).collect();
        shuffle(&mut values);
        values.sort_unstable();
        assert_eq!(values, (0..100).collect::<Vec<_>>());
    }
    #[test]
    fn player_paces_changes() {
        let player = Player::new(Style::IPhoto, 5, 6);
        assert_eq!(player.current, 0);
        assert_eq!(player.next, 1);
        assert_eq!(player.interval, Duration::from_secs(6));
        assert_eq!(Player::new(Style::Picasa, 20, 3).slots.len(), 6);
    }
}
