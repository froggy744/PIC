use std::cell::{Cell, RefCell};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::mpsc;
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
    },
    Done {
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

fn preview_tile(source: &Path) -> (gtk::FlowBoxChild, gtk::Picture) {
    // Make the selectable FlowBox cell itself fixed-width. Keeping only the
    // inner picture small still lets FlowBox stretch the cell across the row,
    // leaving large empty "columns" between thumbnails.
    let tile = gtk::Box::new(gtk::Orientation::Vertical, 0);
    tile.set_size_request(96, 72);
    tile.set_hexpand(false);
    tile.set_vexpand(false);
    tile.set_halign(gtk::Align::Start);
    tile.set_valign(gtk::Align::Start);
    tile.set_tooltip_text(Some(&source.display().to_string()));

    let picture = gtk::Picture::new();
    picture.set_content_fit(gtk::ContentFit::Cover);
    picture.set_can_shrink(true);
    picture.set_size_request(96, 72);
    picture.set_hexpand(false);
    picture.set_vexpand(false);
    picture.set_halign(gtk::Align::Start);
    picture.set_valign(gtk::Align::Start);
    picture.add_css_class("thumbnail");
    tile.append(&picture);

    let child = gtk::FlowBoxChild::new();
    child.set_child(Some(&tile));
    child.set_size_request(96, 72);
    child.set_hexpand(false);
    child.set_vexpand(false);
    child.set_halign(gtk::Align::Start);
    child.set_valign(gtk::Align::Start);
    child.set_tooltip_text(Some(&source.display().to_string()));

    (child, picture)
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
    import_selected: &gtk::Button,
    import_all: &gtk::Button,
    sources: &Rc<RefCell<Vec<PathBuf>>>,
    destination: &Rc<RefCell<Option<PathBuf>>>,
    busy: &Rc<Cell<bool>>,
) {
    let total = sources.borrow().len();
    let selected = photos.selected_children().len();
    selection_label.set_text(&format!("{selected} selected / {total} photos"));

    let ready = destination.borrow().is_some() && !busy.get();
    import_selected.set_sensitive(ready && selected > 0);
    import_all.set_sensitive(ready && total > 0);
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
pub fn present(parent: &adw::ApplicationWindow, on_imported: Rc<dyn Fn(String)>) {
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
    let source_value = gtk::Label::new(Some("Looking for camera or SD card…"));
    source_value.set_xalign(0.0);
    source_value.set_hexpand(true);
    source_value.set_ellipsize(gtk::pango::EllipsizeMode::Middle);

    let choose_source = gtk::Button::with_label("Choose SD card…");
    source_row.append(&source_label);
    source_row.append(&source_value);
    source_row.append(&choose_source);
    root.append(&source_row);

    let options_row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    let exclude_duplicates = gtk::CheckButton::with_label("Exclude duplicates");
    exclude_duplicates.set_active(true);
    let selection_label = gtk::Label::new(Some("0 selected / 0 photos"));
    selection_label.set_xalign(1.0);
    selection_label.set_hexpand(true);
    selection_label.add_css_class("dim-label");
    options_row.append(&exclude_duplicates);
    options_row.append(&selection_label);
    root.append(&options_row);

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
    let destination_row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    let destination_label = gtk::Label::new(Some("Import to:"));
    let destination_value = gtk::Label::new(
        destination
            .borrow()
            .as_ref()
            .and_then(|path| path.to_str())
            .or(Some("Choose destination folder")),
    );
    destination_value.set_xalign(0.0);
    destination_value.set_hexpand(true);
    destination_value.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    let choose_destination = gtk::Button::with_label("Destination…");

    destination_row.append(&destination_label);
    destination_row.append(&destination_value);
    destination_row.append(&choose_destination);
    root.append(&destination_row);

    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    actions.set_halign(gtk::Align::End);
    let deselect_all = gtk::Button::with_label("Deselect All");
    let cancel = gtk::Button::with_label("Cancel");
    let import_selected = gtk::Button::with_label("Import Selected");
    let import_all = gtk::Button::with_label("Import All");
    deselect_all.set_sensitive(false);
    import_selected.set_sensitive(false);
    import_all.set_sensitive(false);
    actions.append(&deselect_all);
    actions.append(&cancel);
    actions.append(&import_selected);
    actions.append(&import_all);
    root.append(&actions);

    let generation = Rc::new(Cell::new(0_u64));
    let current_source = Rc::new(RefCell::new(None::<PathBuf>));
    let sources = Rc::new(RefCell::new(Vec::<PathBuf>::new()));
    let busy = Rc::new(Cell::new(false));

    {
        let selection_label = selection_label.clone();
        let deselect_all = deselect_all.clone();
        let import_selected = import_selected.clone();
        let import_all = import_all.clone();
        let sources = sources.clone();
        let destination = destination.clone();
        let busy = busy.clone();
        photos.connect_selected_children_changed(move |flow| {
            deselect_all.set_sensitive(!flow.selected_children().is_empty());
            update_selection(
                flow,
                &selection_label,
                &import_selected,
                &import_all,
                &sources,
                &destination,
                &busy,
            );
        });
    }

    let start_scan: Rc<dyn Fn(String, PathBuf)> = {
        let source_value = source_value.clone();
        let status = status.clone();
        let empty = empty.clone();
        let photos = photos.clone();
        let preview_stack = preview_stack.clone();
        let choose_source = choose_source.clone();
        let generation = generation.clone();
        let current_source = current_source.clone();
        let sources = sources.clone();
        let selection_label = selection_label.clone();
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
            clear_flow(&photos);

            source_value.set_text(&format!("{name} — {}", root_path.display()));
            choose_source.set_label("Choose other…");
            status.set_text("Scanning DCIM for photos…");
            empty.set_text("Scanning camera or SD card…");
            selection_label.set_text("0 selected / 0 photos");
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
            let selection_label = selection_label.clone();
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
                            clear_flow(&photos);

                            for source in &found {
                                let (tile, picture) = preview_tile(source);
                                photos.insert(&tile, -1);
                                pictures.push(picture);
                            }

                            if discovered == 0 {
                                status.set_text("No supported photos found");
                                empty.set_text("No supported photos found in DCIM");
                                preview_stack.set_visible_child_name("empty");
                            } else {
                                preview_stack.set_visible_child_name("photos");
                                status.set_text(&format!(
                                    "Found {discovered} photos — loading thumbnails…"
                                ));
                            }
                            update_selection(
                                &photos,
                                &selection_label,
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
                                "Loading thumbnails… {shown}/{}",
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
                                status.set_text("No supported photos found");
                                empty.set_text("No supported photos found in DCIM");
                                preview_stack.set_visible_child_name("empty");
                            } else if failed == 0 {
                                status.set_text(&format!("{ready} thumbnails ready"));
                            } else {
                                status.set_text(&format!(
                                    "{ready} thumbnails ready — {failed} could not be decoded"
                                ));
                            }
                            update_selection(
                                &photos,
                                &selection_label,
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

    let source_value_for_dialog = source_value.clone();
    let parent_for_source = window.clone();
    let start_scan_for_dialog = start_scan.clone();
    choose_source.connect_clicked(move |_| {
        let dialog = gtk::FileDialog::builder()
            .title("Choose SD Card or Camera")
            .accept_label("Choose")
            .modal(true)
            .build();
        let source_value = source_value_for_dialog.clone();
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
                        source_value.set_text("Selected source is not a local mounted folder");
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
        let photos = photos.clone();
        let choose_source = choose_source.clone();
        let choose_destination = choose_destination.clone();
        let import_selected = import_selected.clone();
        let import_all = import_all.clone();
        let selection_label = selection_label.clone();
        let sources = sources.clone();
        let busy = busy.clone();
        let on_imported = on_imported.clone();
        let window = window.downgrade();

        Rc::new(move |files: Vec<PathBuf>| {
            if files.is_empty() || busy.get() {
                return;
            }
            let Some(destination_path) = destination.borrow().clone() else {
                status.set_text("Choose an import destination first");
                return;
            };

            busy.set(true);
            photos.set_sensitive(false);
            choose_source.set_sensitive(false);
            choose_destination.set_sensitive(false);
            exclude_duplicates.set_sensitive(false);
            import_selected.set_sensitive(false);
            import_all.set_sensitive(false);
            status.set_text(&format!("Importing {} photos…", files.len()));

            trace(format!(
                "copy_start count={} destination={} exclude_duplicates={}",
                files.len(),
                destination_path.display(),
                exclude_duplicates.is_active()
            ));

            let (sender, receiver) = mpsc::channel::<ImportMessage>();
            let exclude = exclude_duplicates.is_active();
            std::thread::spawn(move || copy_photos(files, destination_path, exclude, sender));

            let destination = destination.clone();
            let exclude_duplicates = exclude_duplicates.clone();
            let status = status.clone();
            let photos = photos.clone();
            let choose_source = choose_source.clone();
            let choose_destination = choose_destination.clone();
            let import_selected = import_selected.clone();
            let import_all = import_all.clone();
            let selection_label = selection_label.clone();
            let sources = sources.clone();
            let busy = busy.clone();
            let on_imported = on_imported.clone();
            let window = window.clone();

            glib::timeout_add_local(Duration::from_millis(50), move || {
                if window.upgrade().is_none() {
                    return glib::ControlFlow::Break;
                }

                loop {
                    match receiver.try_recv() {
                        Ok(ImportMessage::Progress {
                            done,
                            total,
                            copied,
                            skipped,
                        }) => {
                            status.set_text(&format!(
                                "Importing… {done}/{total} — {copied} copied, {skipped} skipped"
                            ));
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
                            busy.set(false);
                            photos.set_sensitive(true);
                            choose_source.set_sensitive(true);
                            choose_destination.set_sensitive(true);
                            exclude_duplicates.set_sensitive(true);
                            status.set_text(&format!(
                                "{copied} copied, {skipped} skipped — adding to PIC library…"
                            ));
                            on_imported(imported_root.to_string_lossy().into_owned());
                            update_selection(
                                &photos,
                                &selection_label,
                                &import_selected,
                                &import_all,
                                &sources,
                                &destination,
                                &busy,
                            );
                            return glib::ControlFlow::Break;
                        }
                        Ok(ImportMessage::Error(error)) => {
                            trace(format!("copy_failed error={error}"));
                            busy.set(false);
                            photos.set_sensitive(true);
                            choose_source.set_sensitive(true);
                            choose_destination.set_sensitive(true);
                            exclude_duplicates.set_sensitive(true);
                            status.set_text(&format!("Import failed: {error}"));
                            update_selection(
                                &photos,
                                &selection_label,
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
                            busy.set(false);
                            status.set_text("Import worker stopped unexpectedly");
                            update_selection(
                                &photos,
                                &selection_label,
                                &import_selected,
                                &import_all,
                                &sources,
                                &destination,
                                &busy,
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
        let source_value = source_value.clone();
        let status = status.clone();
        let empty = empty.clone();
        let photos = photos.clone();
        let preview_stack = preview_stack.clone();
        let sources = sources.clone();
        let selection_label = selection_label.clone();
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
            source_value.set_text("No device selected");
            status.set_text("Camera or SD card removed");
            empty.set_text("Insert a camera or SD card to import photos");
            preview_stack.set_visible_child_name("empty");
            update_selection(
                &photos,
                &selection_label,
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
        source_value.set_text("No camera or SD card detected");
        status.set_text("No camera or SD card detected");
        empty.set_text("Insert a camera or SD card, or choose one manually");
    }
}
