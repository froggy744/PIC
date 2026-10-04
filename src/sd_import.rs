use std::cell::{Cell, RefCell};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, UNIX_EPOCH};

use gio::prelude::*;
use gtk::prelude::*;
use gtk4 as gtk;
use libadwaita as adw;
use walkdir::WalkDir;

#[derive(Debug)]
enum PreviewMessage {
    Discovered(Vec<PathBuf>),
    Ready { index: usize, cached: PathBuf },
    Failed,
    Done { total: usize, ready: usize, failed: usize },
}

#[derive(Debug)]
enum ImportMessage {
    Progress {
        done: usize,
        total: usize,
        copied: usize,
        skipped: usize,
        filename: String,
    },
    Done {
        copied: usize,
        skipped: usize,
        destination: PathBuf,
    },
    Cancelled {
        copied: usize,
        skipped: usize,
        destination: PathBuf,
    },
    Error(String),
}

fn trace(message: impl AsRef<str>) {
    if std::env::var_os("PICASA_TRACE").is_some() {
        eprintln!("PIC_IMPORT {}", message.as_ref());
    }
}

fn dcim_directory(root: &Path) -> Option<PathBuf> {
    if root
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case("DCIM"))
        && root.is_dir()
    {
        return Some(root.to_path_buf());
    }

    for name in ["DCIM", "dcim"] {
        let candidate = root.join(name);
        if candidate.is_dir() {
            return Some(candidate);
        }
    }
    None
}

fn camera_mount(mount: &gio::Mount) -> Option<(String, PathBuf)> {
    let root = mount.root().path()?;
    dcim_directory(&root)?;
    Some((mount.name().to_string(), root))
}

fn mounted_camera() -> Option<(String, PathBuf)> {
    gio::VolumeMonitor::get()
        .mounts()
        .into_iter()
        .find_map(|mount| camera_mount(&mount))
}

fn clear_flow(flow: &gtk::FlowBox) {
    while let Some(child) = flow.first_child() {
        flow.remove(&child);
    }
}

fn fingerprint(path: &Path) -> (Option<i64>, Option<i64>) {
    let Ok(metadata) = std::fs::metadata(path) else {
        return (None, None);
    };
    let mtime = metadata
        .modified()
        .ok()
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .map(|elapsed| elapsed.as_secs() as i64);
    (mtime, Some(metadata.len() as i64))
}

fn preview_tile(source: &Path) -> (gtk::FlowBoxChild, gtk::Picture, gtk::Button) {
    // Nikon stills are 3:2, so use a matching 144x96 thumbnail box.
    // Cover only trims minimally when source aspect ratios differ.
    let tile = gtk::Box::new(gtk::Orientation::Vertical, 0);
    tile.set_size_request(144, 96);
    tile.set_hexpand(false);
    tile.set_vexpand(false);
    tile.set_halign(gtk::Align::Start);
    tile.set_valign(gtk::Align::Start);
    tile.set_tooltip_text(Some(&source.display().to_string()));

    let picture = gtk::Picture::new();
    picture.set_content_fit(gtk::ContentFit::Cover);
    picture.set_can_shrink(true);
    picture.set_size_request(144, 96);
    picture.set_hexpand(false);
    picture.set_vexpand(false);
    picture.set_halign(gtk::Align::Start);
    picture.set_valign(gtk::Align::Start);
    picture.add_css_class("thumbnail");

    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&picture));
    overlay.set_size_request(144, 96);

    let selected_badge = gtk::Button::from_icon_name("object-select-symbolic");
    selected_badge.add_css_class("suggested-action");
    selected_badge.add_css_class("circular");
    selected_badge.set_size_request(26, 26);
    selected_badge.set_halign(gtk::Align::End);
    selected_badge.set_valign(gtk::Align::Start);
    selected_badge.set_margin_top(4);
    selected_badge.set_margin_end(4);
    selected_badge.set_can_target(false);
    selected_badge.set_focusable(false);
    selected_badge.set_visible(false);
    overlay.add_overlay(&selected_badge);

    tile.append(&overlay);

    let child = gtk::FlowBoxChild::new();
    child.set_child(Some(&tile));
    child.set_size_request(144, 96);
    child.set_hexpand(false);
    child.set_vexpand(false);
    child.set_halign(gtk::Align::Start);
    child.set_valign(gtk::Align::Start);
    child.set_tooltip_text(Some(&source.display().to_string()));

    (child, picture, selected_badge)
}

fn scan_source(
    generation: u64,
    root: PathBuf,
    sender: mpsc::Sender<(u64, PreviewMessage)>,
) {
    let scan_root = dcim_directory(&root).unwrap_or_else(|| root.clone());
    trace(format!(
        "scan_start generation={generation} root={} scan_root={}",
        root.display(),
        scan_root.display()
    ));

    let mut photos = WalkDir::new(&scan_root)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .map(|entry| entry.into_path())
        .filter(|path| crate::image_format::supported(path))
        .collect::<Vec<_>>();

    photos.sort_by(|left, right| {
        left.file_name()
            .cmp(&right.file_name())
            .then_with(|| left.cmp(right))
    });

    let total = photos.len();
    if sender
        .send((generation, PreviewMessage::Discovered(photos.clone())))
        .is_err()
    {
        return;
    }

    let mut ready = 0usize;
    let mut failed = 0usize;
    for (index, path) in photos.into_iter().enumerate() {
        let reference = path.to_string_lossy().into_owned();
        let (mtime, size_bytes) = fingerprint(&path);
        match crate::thumbnail::create(&reference, mtime, size_bytes) {
            Ok(cached) if cached.is_file() => {
                ready += 1;
                if sender
                    .send((generation, PreviewMessage::Ready { index, cached }))
                    .is_err()
                {
                    return;
                }
            }
            Ok(_) | Err(_) => {
                failed += 1;
                if sender
                    .send((generation, PreviewMessage::Failed))
                    .is_err()
                {
                    return;
                }
            }
        }
    }

    trace(format!(
        "scan_done generation={generation} total={total} ready={ready} failed={failed}"
    ));
    let _ = sender.send((
        generation,
        PreviewMessage::Done {
            total,
            ready,
            failed,
        },
    ));
}

fn file_digest(path: &Path) -> std::io::Result<blake3::Hash> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = blake3::Hasher::new();
    let mut buffer = [0_u8; 128 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher.finalize())
}

fn same_file(source: &Path, destination: &Path) -> bool {
    let Ok(source_meta) = std::fs::metadata(source) else {
        return false;
    };
    let Ok(destination_meta) = std::fs::metadata(destination) else {
        return false;
    };
    source_meta.len() == destination_meta.len()
        && file_digest(source)
            .ok()
            .zip(file_digest(destination).ok())
            .is_some_and(|(left, right)| left == right)
}

fn unique_destination(destination: &Path, filename: &std::ffi::OsStr) -> PathBuf {
    let first = destination.join(filename);
    if !first.exists() {
        return first;
    }

    let file = Path::new(filename);
    let stem = file
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("photo");
    let extension = file.extension().and_then(|value| value.to_str());

    for suffix in 1..10_000usize {
        let name = match extension {
            Some(extension) => format!("{stem}_{suffix}.{extension}"),
            None => format!("{stem}_{suffix}"),
        };
        let candidate = destination.join(name);
        if !candidate.exists() {
            return candidate;
        }
    }

    destination.join(format!("{stem}_{}", std::process::id()))
}

fn copy_photos(
    files: Vec<PathBuf>,
    destination: PathBuf,
    exclude_duplicates: bool,
    cancelled: Arc<AtomicBool>,
    sender: mpsc::Sender<ImportMessage>,
) {
    if let Err(error) = std::fs::create_dir_all(&destination) {
        let _ = sender.send(ImportMessage::Error(format!(
            "Could not create {}: {error}",
            destination.display()
        )));
        return;
    }

    let total = files.len();
    let mut copied = 0usize;
    let mut skipped = 0usize;

    for (index, source) in files.into_iter().enumerate() {
        if cancelled.load(Ordering::Relaxed) {
            let _ = sender.send(ImportMessage::Cancelled {
                copied,
                skipped,
                destination,
            });
            return;
        }

        let Some(filename) = source.file_name() else {
            skipped += 1;
            continue;
        };

        let direct = destination.join(filename);
        let target = if direct.exists() {
            if exclude_duplicates && same_file(&source, &direct) {
                skipped += 1;
                let _ = sender.send(ImportMessage::Progress {
                    done: index + 1,
                    total,
                    copied,
                    skipped,
                    filename: filename.to_string_lossy().into_owned(),
                });
                continue;
            }
            unique_destination(&destination, filename)
        } else {
            direct
        };

        match std::fs::copy(&source, &target) {
            Ok(_) => copied += 1,
            Err(error) => {
                let _ = sender.send(ImportMessage::Error(format!(
                    "Could not copy {}: {error}",
                    source.display()
                )));
                return;
            }
        }

        let _ = sender.send(ImportMessage::Progress {
            done: index + 1,
            total,
            copied,
            skipped,
            filename: filename.to_string_lossy().into_owned(),
        });
    }

    let _ = sender.send(ImportMessage::Done {
        copied,
        skipped,
        destination,
    });
}

fn default_destination() -> PathBuf {
    dirs::picture_dir()
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("PIC Imports")
}

fn update_selection(
    photos: &gtk::FlowBox,
    selection_label: &gtk::Label,
    select_all: &gtk::Button,
    deselect_all: &gtk::Button,
    invert_selection: &gtk::Button,
    import_selected: &gtk::Button,
    import_all: &gtk::Button,
    sources: &Rc<RefCell<Vec<PathBuf>>>,
    destination: &Rc<RefCell<Option<PathBuf>>>,
    busy: &Rc<Cell<bool>>,
) {
    let total = sources.borrow().len();
    let selected = photos.selected_children().len();
    selection_label.set_text(&format!("{selected} of {total} selected"));

    select_all.set_sensitive(total > 0 && selected < total && !busy.get());
    deselect_all.set_sensitive(selected > 0 && !busy.get());
    invert_selection.set_sensitive(total > 0 && !busy.get());

    import_all.set_label(&format!("Import all {total}"));
    if selected == 0 {
        import_selected.set_label("Select photos to import");
    } else {
        import_selected.set_label(&format!("Import {selected} selected"));
    }

    let ready = destination.borrow().is_some() && total > 0 && !busy.get();
    // Keep the primary action visually present even at zero selection; its
    // click handler simply has no files to start until something is selected.
    import_selected.set_sensitive(ready);
    import_all.set_sensitive(ready);
}

fn selected_sources(photos: &gtk::FlowBox, sources: &[PathBuf]) -> Vec<PathBuf> {
    let mut indices = photos
        .selected_children()
        .into_iter()
        .filter_map(|child| usize::try_from(child.index()).ok())
        .collect::<Vec<_>>();
    indices.sort_unstable();
    indices
        .into_iter()
        .filter_map(|index| sources.get(index).cloned())
        .collect()
}

/// Present the camera / SD-card import surface.
///
/// The source card is preview-only. Import copies originals to the chosen
/// local destination, then hands that destination to the normal PIC folder
/// import/scan pipeline so the card itself never becomes a library root.
pub fn present(
    parent: &adw::ApplicationWindow,
    on_imported: Rc<dyn Fn(String)>,
    progress: Rc<crate::window::OperationProgressUi>,
) {
    let window = gtk::Window::builder()
        .title("Import Photos")
        .transient_for(parent)
        .modal(false)
        .default_width(920)
        .default_height(640)
        .build();

    let root = gtk::Box::new(gtk::Orientation::Vertical, 10);
    root.set_margin_top(14);
    root.set_margin_bottom(14);
    root.set_margin_start(14);
    root.set_margin_end(14);

    let source_row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    let source_label = gtk::Label::new(Some("Import from:"));
    source_label.set_xalign(0.0);

    let source_info = gtk::Box::new(gtk::Orientation::Vertical, 1);
    source_info.set_hexpand(true);

    let source_name = gtk::Label::new(Some("Looking for camera or SD card…"));
    source_name.set_xalign(0.0);
    source_name.set_ellipsize(gtk::pango::EllipsizeMode::End);
    source_name.add_css_class("heading");

    let source_path = gtk::Label::new(None);
    source_path.set_xalign(0.0);
    source_path.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    source_path.add_css_class("dim-label");
    source_path.add_css_class("caption");
    source_path.set_visible(false);

    source_info.append(&source_name);
    source_info.append(&source_path);

    let choose_source = gtk::Button::with_label("Change source");
    source_row.append(&source_label);
    source_row.append(&source_info);
    source_row.append(&choose_source);
    root.append(&source_row);

    let selection_toolbar = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let select_all = gtk::Button::with_label("Select all");
    let deselect_all = gtk::Button::with_label("Deselect all");
    let invert_selection = gtk::Button::with_label("Invert");
    let toolbar_spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    toolbar_spacer.set_hexpand(true);
    let exclude_duplicates = gtk::CheckButton::with_label("Skip duplicates");
    exclude_duplicates.set_active(true);
    let selection_label = gtk::Label::new(Some("0 of 0 selected"));
    selection_label.set_xalign(1.0);

    select_all.set_sensitive(false);
    deselect_all.set_sensitive(false);
    invert_selection.set_sensitive(false);

    selection_toolbar.append(&select_all);
    selection_toolbar.append(&deselect_all);
    selection_toolbar.append(&invert_selection);
    selection_toolbar.append(&toolbar_spacer);
    selection_toolbar.append(&exclude_duplicates);
    selection_toolbar.append(&selection_label);
    root.append(&selection_toolbar);

    let status = gtk::Label::new(Some("Checking mounted media…"));
    status.set_xalign(0.0);
    status.add_css_class("dim-label");
    root.append(&status);

    let empty_state = gtk::Box::new(gtk::Orientation::Vertical, 10);
    empty_state.set_hexpand(true);
    empty_state.set_vexpand(true);
    empty_state.set_halign(gtk::Align::Center);
    empty_state.set_valign(gtk::Align::Center);

    let device_icon = gtk::Image::from_icon_name("media-flash-symbolic");
    device_icon.set_pixel_size(64);
    let empty = gtk::Label::new(Some("Looking for a camera or SD card…"));
    empty.add_css_class("dim-label");
    empty_state.append(&device_icon);
    empty_state.append(&empty);

    let photos = gtk::FlowBox::new();
    photos.set_selection_mode(gtk::SelectionMode::Multiple);
    photos.set_homogeneous(false);
    photos.set_min_children_per_line(4);
    photos.set_max_children_per_line(20);
    photos.set_row_spacing(4);
    photos.set_column_spacing(4);
    photos.set_margin_top(6);
    photos.set_margin_bottom(6);
    photos.set_margin_start(6);
    photos.set_margin_end(6);

    let preview_stack = gtk::Stack::new();
    preview_stack.set_hexpand(true);
    preview_stack.set_vexpand(true);
    preview_stack.add_named(&empty_state, Some("empty"));
    preview_stack.add_named(&photos, Some("photos"));
    preview_stack.set_visible_child_name("empty");

    let scrolled = gtk::ScrolledWindow::builder()
        .hexpand(true)
        .vexpand(true)
        .child(&preview_stack)
        .build();
    root.append(&scrolled);

    let separator = gtk::Separator::new(gtk::Orientation::Horizontal);
    root.append(&separator);

    let destination = Rc::new(RefCell::new(Some(default_destination())));
    let footer = gtk::Box::new(gtk::Orientation::Horizontal, 8);

    let destination_group = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    destination_group.set_hexpand(true);
    let destination_label = gtk::Label::new(Some("Import to:"));
    let destination_value = gtk::Label::new(
        destination
            .borrow()
            .as_ref()
            .and_then(|path| path.to_str())
            .or(Some("Choose destination folder")),
    );
    destination_value.set_xalign(0.0);
    destination_value.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    let choose_destination = gtk::Button::with_label("Change");

    destination_group.append(&destination_label);
    destination_group.append(&destination_value);
    destination_group.append(&choose_destination);

    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    actions.set_halign(gtk::Align::End);
    let cancel = gtk::Button::with_label("Cancel");
    let import_all = gtk::Button::with_label("Import all 0");
    let import_selected = gtk::Button::with_label("Select photos to import");
    import_selected.add_css_class("suggested-action");
    import_selected.set_sensitive(false);
    import_all.set_sensitive(false);

    actions.append(&cancel);
    actions.append(&import_all);
    actions.append(&import_selected);
    footer.append(&destination_group);
    footer.append(&actions);
    root.append(&footer);

    let generation = Rc::new(Cell::new(0_u64));
    let current_source = Rc::new(RefCell::new(None::<PathBuf>));
    let sources = Rc::new(RefCell::new(Vec::<PathBuf>::new()));
    let selection_badges = Rc::new(RefCell::new(Vec::<gtk::Button>::new()));
    let busy = Rc::new(Cell::new(false));

    {
        let selection_label = selection_label.clone();
        let select_all = select_all.clone();
        let deselect_all = deselect_all.clone();
        let invert_selection = invert_selection.clone();
        let selection_badges = selection_badges.clone();
        let import_selected = import_selected.clone();
        let import_all = import_all.clone();
        let sources = sources.clone();
        let destination = destination.clone();
        let busy = busy.clone();
        photos.connect_selected_children_changed(move |flow| {
            let selected = flow.selected_children();

            for badge in selection_badges.borrow().iter() {
                badge.set_visible(false);
            }
            for child in &selected {
                let index = child.index();
                if index >= 0 {
                    if let Some(badge) = selection_badges.borrow().get(index as usize) {
                        badge.set_visible(true);
                    }
                }
            }

            update_selection(
                flow,
                &selection_label,
                &select_all,
                &deselect_all,
                &invert_selection,
                &import_selected,
                &import_all,
                &sources,
                &destination,
                &busy,
            );
        });
    }

    let start_scan: Rc<dyn Fn(String, PathBuf)> = {
        let source_name = source_name.clone();
        let source_path = source_path.clone();
        let status = status.clone();
        let empty = empty.clone();
        let photos = photos.clone();
        let preview_stack = preview_stack.clone();
        let choose_source = choose_source.clone();
        let generation = generation.clone();
        let current_source = current_source.clone();
        let sources = sources.clone();
        let selection_badges = selection_badges.clone();
        let selection_label = selection_label.clone();
        let select_all = select_all.clone();
        let deselect_all = deselect_all.clone();
        let invert_selection = invert_selection.clone();
        let import_selected = import_selected.clone();
        let import_all = import_all.clone();
        let destination = destination.clone();
        let busy = busy.clone();
        let window = window.downgrade();

        Rc::new(move |name: String, root_path: PathBuf| {
            let request = generation.get().wrapping_add(1);
            generation.set(request);
            current_source.replace(Some(root_path.clone()));
            sources.borrow_mut().clear();
            selection_badges.borrow_mut().clear();
            clear_flow(&photos);

            source_name.set_text(&name);
            source_name.set_tooltip_text(Some(&root_path.display().to_string()));
            source_path.set_text(&root_path.display().to_string());
            source_path.set_visible(true);
            choose_source.set_label("Change source");
            status.set_text("Loading thumbnails…");
            status.set_visible(true);
            empty.set_text("Scanning camera or SD card…");
            selection_label.set_text("0 of 0 selected");
            preview_stack.set_visible_child_name("empty");
            import_selected.set_sensitive(false);
            import_all.set_sensitive(false);

            trace(format!(
                "source_selected generation={request} name={name:?} root={}",
                root_path.display()
            ));

            let (sender, receiver) = mpsc::channel::<(u64, PreviewMessage)>();
            std::thread::spawn(move || scan_source(request, root_path, sender));

            let status = status.clone();
            let empty = empty.clone();
            let photos = photos.clone();
            let preview_stack = preview_stack.clone();
            let generation = generation.clone();
            let sources = sources.clone();
            let selection_badges = selection_badges.clone();
            let selection_label = selection_label.clone();
            let select_all = select_all.clone();
            let deselect_all = deselect_all.clone();
            let invert_selection = invert_selection.clone();
            let import_selected = import_selected.clone();
            let import_all = import_all.clone();
            let destination = destination.clone();
            let busy = busy.clone();
            let window = window.clone();
            let mut pictures = Vec::<gtk::Picture>::new();
            let mut discovered = 0usize;
            let mut shown = 0usize;

            glib::timeout_add_local(Duration::from_millis(30), move || {
                if window.upgrade().is_none() {
                    return glib::ControlFlow::Break;
                }

                loop {
                    match receiver.try_recv() {
                        Ok((message_generation, _)) if message_generation != generation.get() => {
                            continue;
                        }
                        Ok((_, PreviewMessage::Discovered(found))) => {
                            discovered = found.len();
                            *sources.borrow_mut() = found.clone();
                            pictures.clear();
                            selection_badges.borrow_mut().clear();
                            clear_flow(&photos);

                            for source in &found {
                                let (tile, picture, badge) = preview_tile(source);
                                photos.insert(&tile, -1);
                                pictures.push(picture);
                                selection_badges.borrow_mut().push(badge);
                            }

                            if discovered == 0 {
                                status.set_visible(false);
                                empty.set_text("No supported photos found in DCIM");
                                preview_stack.set_visible_child_name("empty");
                            } else {
                                preview_stack.set_visible_child_name("photos");
                                status.set_text(&format!(
                                    "Loading thumbnails… 0 of {discovered}"
                                ));
                            }
                            update_selection(
                                &photos,
                                &selection_label,
                                &select_all,
                                &deselect_all,
                                &invert_selection,
                                &import_selected,
                                &import_all,
                                &sources,
                                &destination,
                                &busy,
                            );
                        }
                        Ok((_, PreviewMessage::Ready { index, cached })) => {
                            shown += 1;
                            if let Some(picture) = pictures.get(index) {
                                picture.set_filename(Some(&cached));
                            }
                            status.set_text(&format!(
                                "Loading thumbnails… {shown} of {}",
                                discovered.max(shown)
                            ));
                        }
                        Ok((_, PreviewMessage::Failed)) => {}
                        Ok((
                            _,
                            PreviewMessage::Done {
                                total,
                                ready,
                                failed,
                            },
                        )) => {
                            if total == 0 {
                                status.set_visible(false);
                                empty.set_text("No supported photos found in DCIM");
                                preview_stack.set_visible_child_name("empty");
                            } else {
                                // Once preview generation is complete the photo count
                                // already lives in the selection toolbar. Reclaim this row.
                                let _ = (ready, failed);
                                status.set_visible(false);
                            }
                            update_selection(
                                &photos,
                                &selection_label,
                                &select_all,
                                &deselect_all,
                                &invert_selection,
                                &import_selected,
                                &import_all,
                                &sources,
                                &destination,
                                &busy,
                            );
                            return glib::ControlFlow::Break;
                        }
                        Err(mpsc::TryRecvError::Empty) => break,
                        Err(mpsc::TryRecvError::Disconnected) => {
                            return glib::ControlFlow::Break;
                        }
                    }
                }
                glib::ControlFlow::Continue
            });
        })
    };

    let source_name_for_dialog = source_name.clone();
    let source_path_for_dialog = source_path.clone();
    let parent_for_source = window.clone();
    let start_scan_for_dialog = start_scan.clone();
    choose_source.connect_clicked(move |_| {
        let dialog = gtk::FileDialog::builder()
            .title("Choose SD Card or Camera")
            .accept_label("Choose")
            .modal(true)
            .build();
        let source_name = source_name_for_dialog.clone();
        let source_path = source_path_for_dialog.clone();
        let start_scan = start_scan_for_dialog.clone();
        dialog.select_folder(
            Some(&parent_for_source),
            None::<&gio::Cancellable>,
            move |result| {
                if let Ok(folder) = result {
                    if let Some(path) = folder.path() {
                        let name = path
                            .file_name()
                            .and_then(|name| name.to_str())
                            .unwrap_or("Selected media")
                            .to_string();
                        start_scan(name, path);
                    } else {
                        source_name.set_text("Selected source is not a local mounted folder");
                        source_path.set_visible(false);
                    }
                }
            },
        );
    });

    {
        let destination_value = destination_value.clone();
        let parent_for_destination = window.clone();
        let destination = destination.clone();
        let photos = photos.clone();
        let selection_label = selection_label.clone();
        let select_all = select_all.clone();
        let deselect_all = deselect_all.clone();
        let invert_selection = invert_selection.clone();
        let import_selected = import_selected.clone();
        let import_all = import_all.clone();
        let sources = sources.clone();
        let busy = busy.clone();
        choose_destination.connect_clicked(move |_| {
            let dialog = gtk::FileDialog::builder()
                .title("Choose Import Destination")
                .accept_label("Choose")
                .modal(true)
                .build();
            let destination_value = destination_value.clone();
            let destination = destination.clone();
            let photos = photos.clone();
            let selection_label = selection_label.clone();
            let select_all = select_all.clone();
            let deselect_all = deselect_all.clone();
            let invert_selection = invert_selection.clone();
            let import_selected = import_selected.clone();
            let import_all = import_all.clone();
            let sources = sources.clone();
            let busy = busy.clone();
            dialog.select_folder(
                Some(&parent_for_destination),
                None::<&gio::Cancellable>,
                move |result| {
                    if let Ok(folder) = result {
                        if let Some(path) = folder.path() {
                            destination_value.set_text(&path.display().to_string());
                            destination.replace(Some(path));
                            update_selection(
                                &photos,
                                &selection_label,
                                &select_all,
                                &deselect_all,
                                &invert_selection,
                                &import_selected,
                                &import_all,
                                &sources,
                                &destination,
                                &busy,
                            );
                        }
                    }
                },
            );
        });
    }

    let start_import: Rc<dyn Fn(Vec<PathBuf>)> = {
        let destination = destination.clone();
        let exclude_duplicates = exclude_duplicates.clone();
        let status = status.clone();
        let busy = busy.clone();
        let on_imported = on_imported.clone();
        let progress = progress.clone();
        let window = window.clone();

        Rc::new(move |files: Vec<PathBuf>| {
            if files.is_empty() || busy.get() {
                return;
            }
            let Some(destination_path) = destination.borrow().clone() else {
                status.set_text("Choose an import destination first");
                return;
            };

            let total = files.len();
            let exclude = exclude_duplicates.is_active();
            busy.set(true);

            trace(format!(
                "copy_start count={} destination={} exclude_duplicates={}",
                total,
                destination_path.display(),
                exclude
            ));

            // From this point the import belongs to PIC, not to this dialog.
            // Close immediately and continue copy + indexing in the normal
            // header progress surface.
            progress.begin("Importing photos", total);
            let cancelled = progress.cancel_flag();
            window.close();

            let (sender, receiver) = mpsc::channel::<ImportMessage>();
            std::thread::spawn({
                let destination_path = destination_path.clone();
                move || copy_photos(files, destination_path, exclude, cancelled, sender)
            });

            let progress = progress.clone();
            let on_imported = on_imported.clone();

            glib::timeout_add_local(Duration::from_millis(50), move || {
                loop {
                    match receiver.try_recv() {
                        Ok(ImportMessage::Progress {
                            done,
                            total,
                            copied: _,
                            skipped: _,
                            filename,
                        }) => {
                            progress.update("Importing photos", done, total, &filename, 0);
                        }
                        Ok(ImportMessage::Done {
                            copied,
                            skipped,
                            destination: imported_root,
                        }) => {
                            trace(format!(
                                "copy_done copied={copied} skipped={skipped} destination={}",
                                imported_root.display()
                            ));
                            progress.handoff(&format!(
                                "Import copy complete — {copied} copied, {skipped} skipped · adding to library…"
                            ));
                            on_imported(imported_root.to_string_lossy().into_owned());
                            return glib::ControlFlow::Break;
                        }
                        Ok(ImportMessage::Cancelled {
                            copied,
                            skipped,
                            destination: imported_root,
                        }) => {
                            trace(format!(
                                "copy_cancelled copied={copied} skipped={skipped} destination={}",
                                imported_root.display()
                            ));
                            if copied > 0 {
                                progress.handoff(&format!(
                                    "Import stopped — {copied} copied · adding copied photos to library…"
                                ));
                                on_imported(imported_root.to_string_lossy().into_owned());
                            } else {
                                progress.finish("Importing photos", "Import cancelled");
                            }
                            return glib::ControlFlow::Break;
                        }
                        Ok(ImportMessage::Error(error)) => {
                            trace(format!("copy_failed error={error}"));
                            progress.finish(
                                "Importing photos",
                                &format!("Import failed — {error}"),
                            );
                            return glib::ControlFlow::Break;
                        }
                        Err(mpsc::TryRecvError::Empty) => break,
                        Err(mpsc::TryRecvError::Disconnected) => {
                            progress.finish(
                                "Importing photos",
                                "Import failed — worker stopped unexpectedly",
                            );
                            return glib::ControlFlow::Break;
                        }
                    }
                }
                glib::ControlFlow::Continue
            });
        })
    };

    {
        let start_import = start_import.clone();
        let photos = photos.clone();
        let sources = sources.clone();
        import_selected.connect_clicked(move |_| {
            let selected = selected_sources(&photos, &sources.borrow());
            start_import(selected);
        });
    }

    {
        let start_import = start_import.clone();
        let sources = sources.clone();
        import_all.connect_clicked(move |_| {
            start_import(sources.borrow().clone());
        });
    }

    {
        let photos = photos.clone();
        select_all.connect_clicked(move |_| photos.select_all());
    }

    {
        let photos = photos.clone();
        let sources = sources.clone();
        invert_selection.connect_clicked(move |_| {
            let selected = photos
                .selected_children()
                .into_iter()
                .filter_map(|child| usize::try_from(child.index()).ok())
                .collect::<std::collections::HashSet<_>>();

            for index in 0..sources.borrow().len() {
                if let Some(child) = photos.child_at_index(index as i32) {
                    if selected.contains(&index) {
                        photos.unselect_child(&child);
                    } else {
                        photos.select_child(&child);
                    }
                }
            }
        });
    }

    {
        let photos = photos.clone();
        deselect_all.connect_clicked(move |_| photos.unselect_all());
    }

    let window_for_cancel = window.clone();
    cancel.connect_clicked(move |_| window_for_cancel.close());

    let volume_monitor = gio::VolumeMonitor::get();
    let added_handler = Rc::new(RefCell::new(None::<glib::SignalHandlerId>));
    let removed_handler = Rc::new(RefCell::new(None::<glib::SignalHandlerId>));

    {
        let start_scan = start_scan.clone();
        let handler = volume_monitor.connect_mount_added(move |_, mount| {
            if let Some((name, root)) = camera_mount(mount) {
                trace(format!(
                    "mount_added camera=true name={name:?} root={}",
                    root.display()
                ));
                start_scan(name, root);
            }
        });
        added_handler.replace(Some(handler));
    }

    {
        let current_source = current_source.clone();
        let generation = generation.clone();
        let source_name = source_name.clone();
        let source_path = source_path.clone();
        let status = status.clone();
        let empty = empty.clone();
        let photos = photos.clone();
        let preview_stack = preview_stack.clone();
        let sources = sources.clone();
        let selection_label = selection_label.clone();
        let select_all = select_all.clone();
        let deselect_all = deselect_all.clone();
        let invert_selection = invert_selection.clone();
        let import_selected = import_selected.clone();
        let import_all = import_all.clone();
        let destination = destination.clone();
        let busy = busy.clone();
        let handler = volume_monitor.connect_mount_removed(move |_, mount| {
            let Some(removed_root) = mount.root().path() else {
                return;
            };
            let was_current = current_source
                .borrow()
                .as_ref()
                .is_some_and(|current| current == &removed_root);
            if !was_current {
                return;
            }

            trace(format!("mount_removed root={}", removed_root.display()));
            generation.set(generation.get().wrapping_add(1));
            current_source.replace(None);
            sources.borrow_mut().clear();
            clear_flow(&photos);
            source_name.set_text("No device selected");
            source_name.set_tooltip_text(None);
            source_path.set_text("");
            source_path.set_visible(false);
            status.set_visible(false);
            empty.set_text("Insert a camera or SD card to import photos");
            preview_stack.set_visible_child_name("empty");
            update_selection(
                &photos,
                &selection_label,
                &select_all,
                &deselect_all,
                &invert_selection,
                &import_selected,
                &import_all,
                &sources,
                &destination,
                &busy,
            );
        });
        removed_handler.replace(Some(handler));
    }

    {
        let monitor = volume_monitor.clone();
        let added_handler = added_handler.clone();
        let removed_handler = removed_handler.clone();
        window.connect_close_request(move |_| {
            if let Some(handler) = added_handler.borrow_mut().take() {
                monitor.disconnect(handler);
            }
            if let Some(handler) = removed_handler.borrow_mut().take() {
                monitor.disconnect(handler);
            }
            glib::Propagation::Proceed
        });
    }

    window.set_child(Some(&root));
    window.present();

    if let Some((name, root)) = mounted_camera() {
        trace(format!(
            "startup_detect camera=true name={name:?} root={}",
            root.display()
        ));
        start_scan(name, root);
    } else {
        trace("startup_detect camera=false");
        source_name.set_text("No camera or SD card detected");
        source_name.set_tooltip_text(None);
        source_path.set_visible(false);
        status.set_visible(false);
        empty.set_text("Insert a camera or SD card, or choose one manually");
    }
}
