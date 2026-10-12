use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::{mpsc, Arc};
use std::time::Duration;

use anyhow::Context;
use gtk::prelude::*;
use gtk4 as gtk;

use crate::{db, photo_object::PhotoObject, sidebar::SidebarFilter, thumbnail_display};

type OpenPhoto = Rc<dyn Fn(Vec<PhotoObject>, usize, bool)>;
type Navigate = Rc<dyn Fn(SidebarFilter)>;

/// Square thumbnail edge length for every Home preview card.
const CARD_THUMB: i32 = 180;
/// Fixed gap between cards in a horizontal row (spec: 12–16 px).
const CARD_GAP: i32 = 14;

/// Home cards need exact geometry independent of theme button padding, so the
/// rules ship with the widget and load once per display (tests skip base.css).
const HOME_CSS: &str = r#"
button.home-card {
  padding: 0;
  min-width: 180px;
}
.home-thumb {
  min-width: 180px;
  min-height: 180px;
}
.home-card-caption { font-size: 12px; }
.home-card-caption-secondary { font-size: 11px; }
.home-section {
  border: 1px solid alpha(@borders, 0.55);
  border-radius: 14px;
  padding: 16px;
  background: alpha(@card_bg_color, 0.65);
}
.home-summary {
  border: 1px solid alpha(@borders, 0.55);
  border-radius: 12px;
  padding: 10px 14px;
  background: alpha(@card_bg_color, 0.7);
}
.home-summary-value { font-size: 18px; font-weight: 700; }
.home-summary-label { font-size: 11px; }
.home-subtitle { font-size: 13px; }
button.home-card.home-selected { background: alpha(@accent_bg_color, 0.14); }
button.home-section-pan {
  min-width: 28px;
  min-height: 28px;
  padding: 2px;
}
"#;

fn install_home_css(display: &gtk::gdk::Display) {
    use std::sync::OnceLock;
    static LOADED: OnceLock<()> = OnceLock::new();
    if LOADED.get().is_some() {
        return;
    }
    let provider = gtk::CssProvider::new();
    provider.load_from_string(HOME_CSS);
    gtk::style_context_add_provider_for_display(
        display,
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 5,
    );
    let _ = LOADED.set(());
}

struct PreviewImage {
    width: i32,
    height: i32,
    pixels: Vec<u8>,
}

struct Snapshot {
    data: db::LibraryHomeData,
    images: HashMap<i64, Arc<PreviewImage>>,
}

struct HomeSection {
    scroller: gtk::ScrolledWindow,
    track: gtk::Box,
}

pub struct LibraryHome {
    pub root: gtk::ScrolledWindow,
    database: Rc<std::cell::RefCell<std::path::PathBuf>>,
    requests: mpsc::SyncSender<HomeRequest>,
    pending: Rc<Cell<bool>>,
}

struct HomeRequest {
    database: std::path::PathBuf,
    force: bool,
}

#[cfg(test)]
#[path = "library_home_tests.rs"]
mod tests;

impl LibraryHome {
    pub fn new(database: std::path::PathBuf, navigate: Navigate, open: OpenPhoto) -> Self {
        if let Some(display) = gtk::gdk::Display::default() {
            install_home_css(&display);
        }
        let root = gtk::ScrolledWindow::new();
        root.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        root.set_hexpand(true);
        root.set_vexpand(true);
        let content = gtk::Box::new(gtk::Orientation::Vertical, 16);
        content.set_margin_top(24);
        content.set_margin_bottom(24);
        content.set_margin_start(28);
        content.set_margin_end(28);
        let header = gtk::Box::new(gtk::Orientation::Horizontal, 16);
        header.set_valign(gtk::Align::Center);
        let identity = gtk::Box::new(gtk::Orientation::Vertical, 3);
        identity.set_hexpand(true);
        let title = gtk::Label::new(Some("Library"));
        title.set_xalign(0.0);
        title.add_css_class("title-1");
        identity.append(&title);
        let subtitle = gtk::Label::new(Some("Your photos, organised and at your fingertips."));
        subtitle.set_xalign(0.0);
        subtitle.add_css_class("dim-label");
        subtitle.add_css_class("home-subtitle");
        identity.append(&subtitle);
        header.append(&identity);

        let summary = gtk::Box::new(gtk::Orientation::Horizontal, 16);
        summary.add_css_class("home-summary");
        summary.set_valign(gtk::Align::Center);
        let mut summary_values = Vec::new();
        for (index, (icon, label)) in [
            ("image-x-generic-symbolic", "All Photos"),
            ("emblem-favorite-symbolic", "Favourites"),
            ("folder-symbolic", "Folders"),
        ].into_iter().enumerate() {
            if index > 0 {
                summary.append(&gtk::Separator::new(gtk::Orientation::Vertical));
            }
            let stat = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            let image = gtk::Image::from_icon_name(icon);
            image.set_pixel_size(20);
            stat.append(&image);
            let labels = gtk::Box::new(gtk::Orientation::Vertical, 2);
            let value = gtk::Label::new(Some("—"));
            value.add_css_class("home-summary-value");
            value.set_xalign(0.0);
            labels.append(&value);
            let caption = gtk::Label::new(Some(label));
            caption.add_css_class("dim-label");
            caption.add_css_class("home-summary-label");
            caption.set_xalign(0.0);
            labels.append(&caption);
            stat.append(&labels);
            summary.append(&stat);
            summary_values.push(value);
        }
        header.append(&summary);
        content.append(&header);
        let mut sections = Vec::new();
        for (title, action, destination) in [
            ("Recently Added", "View All", SidebarFilter::RecentlyAdded),
            ("Recently Edited", "View History", SidebarFilter::History),
            ("Favourites", "View All", SidebarFilter::Favorites),
            ("Albums", "View All", SidebarFilter::Albums),
        ] {
            let section = gtk::Box::new(gtk::Orientation::Vertical, 12);
            section.add_css_class("home-section");
            let heading = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            heading.set_valign(gtk::Align::Center);
            let label = gtk::Label::new(Some(title));
            label.add_css_class("title-2");
            label.set_xalign(0.0);
            label.set_hexpand(true);
            label.set_valign(gtk::Align::Center);
            heading.append(&label);

            let scroller = gtk::ScrolledWindow::new();
            // NEVER makes the entire thumbnail track a minimum-size request,
            // preventing the parent window from shrinking. EXTERNAL keeps
            // horizontal scrolling but delegates the scrollbar to UI controls.
            scroller.set_policy(gtk::PolicyType::External, gtk::PolicyType::Never);
            scroller.set_propagate_natural_width(false);
            scroller.set_min_content_width(1);
            scroller.set_hexpand(true);
            scroller.set_kinetic_scrolling(true);
            scroller.add_css_class("home-section-row");
            let track = gtk::Box::new(gtk::Orientation::Horizontal, CARD_GAP);
            track.add_css_class("home-section-track");
            track.set_valign(gtk::Align::Start);
            track.set_halign(gtk::Align::Start);
            scroller.set_child(Some(&track));

            let prev = gtk::Button::from_icon_name("pan-start-symbolic");
            prev.add_css_class("flat");
            prev.add_css_class("home-section-pan");
            prev.set_valign(gtk::Align::Center);
            prev.set_tooltip_text(Some("Scroll left"));
            prev.set_visible(false);
            let next = gtk::Button::from_icon_name("pan-end-symbolic");
            next.add_css_class("flat");
            next.add_css_class("home-section-pan");
            next.set_valign(gtk::Align::Center);
            next.set_tooltip_text(Some("Scroll right"));
            next.set_visible(false);
            wire_scroll_buttons(&scroller, &prev, &next);
            heading.append(&prev);
            heading.append(&next);

            let button = gtk::Button::with_label(action);
            button.add_css_class("flat");
            let navigate = navigate.clone();
            button.connect_clicked(move |_| navigate(destination));
            heading.append(&button);
            section.append(&heading);
            section.append(&scroller);
            content.append(&section);
            sections.push(HomeSection {
                scroller,
                track,
            });
        }
        let status = gtk::Label::new(Some("Loading library…"));
        status.add_css_class("dim-label");
        content.append(&status);
        root.set_child(Some(&content));

        let database = Rc::new(std::cell::RefCell::new(database));
        let (requests, receiver) = mpsc::sync_channel::<HomeRequest>(1);
        let (sender, snapshots) = mpsc::channel();
        std::thread::spawn(move || worker(receiver, sender));
        let pending = Rc::new(Cell::new(false));
        {
            let pending = pending.clone();
            let requests = requests.clone();
            let database = database.clone();
            root.connect_map(move |_| {
                let request = HomeRequest {
                    database: database.borrow().clone(),
                    force: true,
                };
                if !pending.replace(true) && requests.try_send(request).is_err() {
                    pending.set(false);
                }
            });
        }
        let weak_root = root.downgrade();
        let database_for_tick = database.clone();
        let requests_for_tick = requests.clone();
        let pending_for_tick = pending.clone();
        let mut ticks = 0;
        glib::timeout_add_local(Duration::from_millis(100), move || {
            let Some(root) = weak_root.upgrade() else {
                return glib::ControlFlow::Break;
            };
            if let Ok((snapshot_database, result)) = snapshots.try_recv() {
                pending_for_tick.set(false);
                if snapshot_database != *database_for_tick.borrow() {
                    return glib::ControlFlow::Continue;
                }
                match result {
                    Ok(Some(snapshot)) => {
                        for (value, count) in summary_values.iter().zip([
                            snapshot.data.photo_count,
                            snapshot.data.favorite_count,
                            snapshot.data.folder_count,
                        ]) {
                            value.set_text(&count.to_string());
                        }
                        populate(&sections, &snapshot, &navigate, &open);
                        status.set_visible(false);
                    }
                    Ok(None) => {}
                    Err(error) => {
                        status.set_text(&format!("Could not load library: {error}"));
                        status.set_visible(true);
                    }
                }
            }
            ticks += 1;
            if ticks >= 20 {
                ticks = 0;
                let request = HomeRequest {
                    database: database_for_tick.borrow().clone(),
                    force: false,
                };
                if root.is_mapped()
                    && !pending_for_tick.replace(true)
                    && requests_for_tick.try_send(request).is_err()
                {
                    pending_for_tick.set(false);
                }
            }
            glib::ControlFlow::Continue
        });
        Self {
            root,
            database,
            requests,
            pending,
        }
    }

    pub fn reload(&self, database: std::path::PathBuf) {
        self.database.replace(database.clone());
        let request = HomeRequest { database, force: true };
        self.pending.set(true);
        if self.requests.try_send(request).is_err() {
            self.pending.set(false);
        }
    }
}

fn wire_scroll_buttons(scroller: &gtk::ScrolledWindow, prev: &gtk::Button, next: &gtk::Button) {
    let hadj = scroller.hadjustment();

    // Animate page movements with a smooth ease-in/out curve. The generation
    // cancels a previous animation when the next arrow is clicked.
    let generation = Rc::new(Cell::new(0u64));
    let pan = {
        let hadj = hadj.clone();
        let generation = generation.clone();
        Rc::new(move |direction: f64| {
            let lower = hadj.lower();
            let upper = (hadj.upper() - hadj.page_size()).max(lower);
            let start = hadj.value();
            let step = (hadj.page_size() * 0.75).max(160.0);
            let end = (start + direction * step).clamp(lower, upper);
            if (end - start).abs() < 0.5 { return; }
            let sequence = generation.get().wrapping_add(1);
            generation.set(sequence);
            let begun = std::time::Instant::now();
            let hadj = hadj.clone();
            let generation = generation.clone();
            glib::timeout_add_local(std::time::Duration::from_millis(16), move || {
                if generation.get() != sequence {
                    return glib::ControlFlow::Break;
                }
                let t = (begun.elapsed().as_secs_f64() / 0.28).min(1.0);
                let eased = t * t * (3.0 - 2.0 * t);
                let maximum = (hadj.upper() - hadj.page_size()).max(hadj.lower());
                hadj.set_value((start + (end - start) * eased).clamp(hadj.lower(), maximum));
                if t >= 1.0 {
                    glib::ControlFlow::Break
                } else {
                    glib::ControlFlow::Continue
                }
            });
        })
    };
    {
        let pan = pan.clone();
        prev.connect_clicked(move |_| pan(-1.0));
    }
    {
        let pan = pan.clone();
        next.connect_clicked(move |_| pan(1.0));
    }

    let prev = prev.clone();
    let next = next.clone();
    let update = {
        let hadj = hadj.clone();
        let prev = prev.clone();
        let next = next.clone();
        Rc::new(move || {
            let max = (hadj.upper() - hadj.page_size()).max(hadj.lower());
            let scrollable = max > hadj.lower() + 1.0;
            let value = hadj.value();
            prev.set_visible(scrollable);
            next.set_visible(scrollable);
            prev.set_sensitive(scrollable && value > hadj.lower() + 0.5);
            next.set_sensitive(scrollable && value < max - 0.5);
        })
    };
    update();
    hadj.connect_value_changed({
        let update = update.clone();
        move |_| update()
    });
    hadj.connect_changed({
        let update = update.clone();
        move |_| update()
    });
}

fn worker(
    requests: mpsc::Receiver<HomeRequest>,
    sender: mpsc::Sender<(std::path::PathBuf, Result<Option<Snapshot>, String>)>,
) {
    let mut connection = None;
    let mut active_database = None;
    let mut version = None;
    let mut cache = HashMap::<String, Arc<PreviewImage>>::new();
    let mut missing = false;
    while let Ok(request) = requests.recv() {
        let request_database = request.database.clone();
        let result = (|| -> anyhow::Result<Option<Snapshot>> {
            if active_database.as_ref() != Some(&request.database) {
                connection = Some(rusqlite::Connection::open_with_flags(
                    &request.database,
                    rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
                ));
                active_database = Some(request.database.clone());
                version = None;
                cache.clear();
                missing = false;
            }
            let connection = connection
                .as_ref()
                .context("library home connection is not initialized")?
                .as_ref()
                .map_err(|error| anyhow::anyhow!(error.to_string()))?;
            connection.busy_timeout(Duration::from_millis(750))?;
            let current: i64 = connection.query_row("PRAGMA data_version", [], |row| row.get(0))?;
            if !request.force && version == Some(current) && !missing {
                return Ok(None);
            }
            let data = db::library_home_data(connection)?;
            let mut images = HashMap::new();
            let mut used = std::collections::HashSet::new();
            let mut loaded = false;
            missing = false;
            for photo in data
                .added
                .iter()
                .chain(&data.edited)
                .chain(&data.favorites)
                .chain(data.albums.iter().filter_map(|(_, cover)| cover.as_ref()))
            {
                let path =
                    crate::thumbnail::cache_path(&photo.path, photo.mtime, photo.size_bytes)?;
                let request = thumbnail_display::request_for(
                    path.to_string_lossy().into_owned(),
                    photo.path.clone(),
                    photo.mtime.unwrap_or_default(),
                    photo.size_bytes.unwrap_or_default(),
                    photo.rotation,
                    photo.edit_recipe.clone(),
                    photo.width.unwrap_or_default(),
                    photo.height.unwrap_or_default(),
                    false,
                );
                used.insert(request.key.clone());
                if !cache.contains_key(&request.key) {
                    if let thumbnail_display::DisplayOutcome::Loaded {
                        width,
                        height,
                        pixels,
                    } = thumbnail_display::load_cached_display_thumbnail(&request)
                    {
                        cache.insert(
                            request.key.clone(),
                            Arc::new(PreviewImage {
                                width,
                                height,
                                pixels,
                            }),
                        );
                        loaded = true;
                    } else {
                        missing = true;
                    }
                }
                if let Some(image) = cache.get(&request.key) {
                    images.insert(photo.id, image.clone());
                }
            }
            cache.retain(|key, _| used.contains(key));
            let changed = request.force || version != Some(current) || loaded;
            version = Some(current);
            Ok(changed.then_some(Snapshot { data, images }))
        })()
        .map_err(|error| error.to_string());
        if sender.send((request_database, result)).is_err() {
            break;
        }
    }
}

fn populate(sections: &[HomeSection], snapshot: &Snapshot, navigate: &Navigate, open: &OpenPhoto) {
    let positions: Vec<f64> = sections
        .iter()
        .map(|section| section.scroller.hadjustment().value())
        .collect();
    for section in sections {
        while let Some(child) = section.track.first_child() {
            section.track.remove(&child);
        }
    }
    for (index, photos) in [
        &snapshot.data.added,
        &snapshot.data.edited,
        &snapshot.data.favorites,
    ]
    .into_iter()
    .enumerate()
    {
        let objects: Vec<_> = photos.iter().map(PhotoObject::from_photo).collect();
        for (position, photo) in photos.iter().enumerate() {
            let filename = crate::source::filename(&photo.path);
            let image = snapshot.images.get(&photo.id);
            // Recently Edited keeps the filename as the primary caption and the
            // history line ("Photo · …" / "Collage · …") as smaller secondary
            // text so saved collages stay distinguishable from photographs.
            let button = if index == 1 {
                let secondary = photo.history_caption.as_deref().map(short_edit_line);
                let collage = secondary
                    .as_deref()
                    .map(|line| line.starts_with("Collage"))
                    .unwrap_or(false);
                let button = card(&filename, secondary.as_deref(), image);
                if collage {
                    button.add_css_class("home-collage");
                }
                button
            } else {
                card(&filename, None, image)
            };
            button.set_tooltip_text(Some(&filename));
            // Single click only marks the card. Double click opens the viewer.
            // This prevents an accidental lightbox transition while browsing.
            let track = sections[index].track.clone();
            let button_for_select = button.clone();
            button.connect_clicked(move |_| {
                let mut child = track.first_child();
                while let Some(widget) = child {
                    widget.remove_css_class("home-selected");
                    child = widget.next_sibling();
                }
                button_for_select.add_css_class("home-selected");
            });
            let gesture = gtk::GestureClick::new();
            gesture.set_button(1);
            // GtkButton owns a click gesture; capture the double press before
            // its internal handler can reset the click sequence.
            gesture.set_propagation_phase(gtk::PropagationPhase::Capture);
            let open = open.clone();
            let objects = objects.clone();
            gesture.connect_pressed(move |gesture, n_press, _, _| {
                if n_press == 2 {
                    gesture.set_state(gtk::EventSequenceState::Claimed);
                    open(objects.clone(), position, index == 1);
                }
            });
            button.add_controller(gesture);
            sections[index].track.append(&button);
        }
    }
    for (album, cover) in &snapshot.data.albums {
        let button = card(
            &album.name,
            Some(&format!("{} photos", album.photo_count)),
            cover
                .as_ref()
                .and_then(|photo| snapshot.images.get(&photo.id)),
        );
        button.set_tooltip_text(Some(&album.name));
        let navigate = navigate.clone();
        let id = album.id;
        button.connect_clicked(move |_| navigate(SidebarFilter::Album(id)));
        sections[3].track.append(&button);
    }
    for (section, message) in sections.iter().zip([
        "No photos added yet",
        "No editing activity yet",
        "No favourites yet",
        "No albums yet",
    ]) {
        if section.track.first_child().is_none() {
            let label = gtk::Label::new(Some(message));
            label.add_css_class("dim-label");
            label.set_margin_top(24);
            label.set_margin_bottom(24);
            section.track.append(&label);
        }
    }
    restore_row_positions(sections, &positions);
}

/// Reapply row scroll offsets after children are rebuilt. The adjustment upper
/// is only correct once the new cards have been measured, so retry shortly
/// after the rebuild instead of assuming the value sticks immediately.
fn restore_row_positions(sections: &[HomeSection], positions: &[f64]) {
    for (section, position) in sections.iter().zip(positions) {
        let position = *position;
        if position <= 0.0 {
            continue;
        }
        let hadj = section.scroller.hadjustment();
        apply_position(&hadj, position);
        let apply = {
            let hadj = hadj.clone();
            Rc::new(move || apply_position(&hadj, position))
        };
        // One idle tick plus a later tick cover measure/allocate after rebuild.
        glib::timeout_add_local_once(Duration::from_millis(16), {
            let apply = apply.clone();
            move || apply()
        });
        glib::timeout_add_local_once(Duration::from_millis(50), move || apply());
    }
}

fn apply_position(hadj: &gtk::Adjustment, position: f64) {
    let max = (hadj.upper() - hadj.page_size()).max(hadj.lower());
    if position <= max {
        hadj.set_value(position);
    }
}

fn card(title: &str, subtitle: Option<&str>, image: Option<&Arc<PreviewImage>>) -> gtk::Button {
    let button = gtk::Button::new();
    button.add_css_class("flat");
    button.add_css_class("home-card");
    // Left-align in the track; never expand or distribute across the window.
    button.set_hexpand(false);
    button.set_vexpand(false);
    button.set_halign(gtk::Align::Start);
    button.set_valign(gtk::Align::Start);
    let content = gtk::Box::new(gtk::Orientation::Vertical, 6);
    content.set_halign(gtk::Align::Start);
    content.set_hexpand(false);
    content.set_size_request(CARD_THUMB, -1);
    let frame = gtk::Overlay::new();
    frame.add_css_class("card");
    frame.add_css_class("home-thumb");
    // Square thumbnail so every preview card shares one size, regardless of
    // the cached image's natural dimensions or the caption length.
    frame.set_size_request(CARD_THUMB, CARD_THUMB);
    frame.set_hexpand(false);
    frame.set_vexpand(false);
    frame.set_halign(gtk::Align::Fill);
    frame.set_valign(gtk::Align::Start);
    frame.set_overflow(gtk::Overflow::Hidden);
    let picture = gtk::Picture::new();
    picture.set_can_shrink(true);
    picture.set_content_fit(gtk::ContentFit::Cover);
    picture.set_size_request(1, 1);
    picture.set_hexpand(true);
    picture.set_vexpand(true);
    picture.set_halign(gtk::Align::Fill);
    picture.set_valign(gtk::Align::Fill);
    picture.add_css_class("thumbnail");
    if let Some(image) = image {
        let texture = gtk::gdk::MemoryTexture::new(
            image.width,
            image.height,
            gtk::gdk::MemoryFormat::R8g8b8a8,
            &glib::Bytes::from_owned(image.pixels.clone()),
            image.width as usize * 4,
        );
        picture.set_paintable(Some(&texture));
    }
    // Keep card dimensions independent of the cache image's natural size.
    let backing = gtk::Box::new(gtk::Orientation::Vertical, 0);
    frame.set_child(Some(&backing));
    frame.add_overlay(&picture);
    if image.is_none() {
        let placeholder = gtk::Image::from_icon_name("image-x-generic-symbolic");
        placeholder.set_pixel_size(32);
        placeholder.add_css_class("dim-label");
        frame.add_overlay(&placeholder);
    }
    content.append(&frame);
    content.append(&caption_label(title, false));
    if let Some(text) = subtitle {
        content.append(&caption_label(text, true));
    }
    button.set_child(Some(&content));
    button
}

/// Compact caption under a thumbnail. Ellipsizes only when the parent width
/// forces it; the button tooltip carries the complete text.
fn caption_label(text: &str, secondary: bool) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    label.set_xalign(0.0);
    label.set_halign(gtk::Align::Fill);
    label.set_hexpand(true);
    // Cap natural width so long filenames cannot stretch the card past 180px.
    label.set_max_width_chars(if secondary { 26 } else { 22 });
    if secondary {
        label.add_css_class("dim-label");
        label.add_css_class("home-card-caption-secondary");
    } else {
        label.add_css_class("home-card-caption");
    }
    label
}

/// History lines arrive as "Photo · YYYY-MM-DD HH:MM:SS". Drop the seconds so
/// the secondary caption fits the fixed card width while keeping the kind
/// (Photo/Collage) that distinguishes saved collages from photographs.
fn short_edit_line(line: &str) -> String {
    if line.len() >= 3 && line.as_bytes()[line.len() - 3] == b':' {
        line[..line.len() - 3].to_string()
    } else {
        line.to_string()
    }
}
