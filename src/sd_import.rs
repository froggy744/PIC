use std::cell::{Cell, RefCell};
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
    Discovered(usize),
    Ready {
        source: PathBuf,
        cached: PathBuf,
    },
    Failed,
    Done {
        total: usize,
        ready: usize,
        failed: usize,
    },
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

fn preview_tile(source: &Path, cached: &Path) -> gtk::Box {
    let tile = gtk::Box::new(gtk::Orientation::Vertical, 5);
    tile.set_width_request(155);

    let picture = gtk::Picture::new();
    picture.set_filename(Some(cached));
    picture.set_content_fit(gtk::ContentFit::Cover);
    picture.set_can_shrink(true);
    picture.set_size_request(150, 112);
    picture.set_hexpand(true);
    picture.add_css_class("thumbnail");

    let filename = source
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("Photo");
    let label = gtk::Label::new(Some(filename));
    label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    label.set_max_width_chars(20);
    label.set_tooltip_text(Some(&source.display().to_string()));

    tile.append(&picture);
    tile.append(&label);
    tile
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
    let _ = sender.send((generation, PreviewMessage::Discovered(total)));

    let mut ready = 0usize;
    let mut failed = 0usize;
    for path in photos {
        let reference = path.to_string_lossy().into_owned();
        let (mtime, size_bytes) = fingerprint(&path);
        match crate::thumbnail::create(&reference, mtime, size_bytes) {
            Ok(cached) if cached.is_file() => {
                ready += 1;
                if sender
                    .send((
                        generation,
                        PreviewMessage::Ready {
                            source: path,
                            cached,
                        },
                    ))
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

/// Present the camera / SD-card import surface.
///
/// Removable media is an import source, not a permanent library folder.
/// The window auto-detects mounted camera media with a DCIM directory and
/// progressively builds preview thumbnails using PIC's normal decoder,
/// including embedded previews from supported RAW formats.
pub fn present(parent: &adw::ApplicationWindow) {
    let window = gtk::Window::builder()
        .title("Import Photos")
        .transient_for(parent)
        .modal(false)
        .default_width(920)
        .default_height(640)
        .build();

    let root = gtk::Box::new(gtk::Orientation::Vertical, 14);
    root.set_margin_top(18);
    root.set_margin_bottom(18);
    root.set_margin_start(18);
    root.set_margin_end(18);

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

    let exclude_duplicates = gtk::CheckButton::with_label("Exclude duplicates");
    exclude_duplicates.set_active(true);
    root.append(&exclude_duplicates);

    let status = gtk::Label::new(Some("Checking mounted media…"));
    status.set_xalign(0.0);
    status.add_css_class("dim-label");
    root.append(&status);

    let empty_state = gtk::Box::new(gtk::Orientation::Vertical, 10);
    empty_state.set_hexpand(true);
    empty_state.set_vexpand(true);
    empty_state.set_halign(gtk::Align::Center);
    empty_state.set_valign(gtk::Align::Center);

    let device_icon = gtk::Image::from_icon_name("media-removable-symbolic");
    device_icon.set_pixel_size(64);
    let empty = gtk::Label::new(Some("Looking for a camera or SD card…"));
    empty.add_css_class("dim-label");
    empty_state.append(&device_icon);
    empty_state.append(&empty);

    let photos = gtk::FlowBox::new();
    photos.set_selection_mode(gtk::SelectionMode::Multiple);
    photos.set_homogeneous(true);
    photos.set_min_children_per_line(2);
    photos.set_max_children_per_line(6);
    photos.set_row_spacing(10);
    photos.set_column_spacing(10);
    photos.set_margin_top(8);
    photos.set_margin_bottom(8);
    photos.set_margin_start(8);
    photos.set_margin_end(8);

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

    let destination_row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    let destination_label = gtk::Label::new(Some("Import to:"));
    let destination_value = gtk::Label::new(Some("Choose destination folder"));
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
    let cancel = gtk::Button::with_label("Cancel");
    let import_selected = gtk::Button::with_label("Import Selected");
    let import_all = gtk::Button::with_label("Import All");
    // Copying/indexing is intentionally not enabled until the source preview
    // path is proven on real camera media.
    import_selected.set_sensitive(false);
    import_all.set_sensitive(false);
    actions.append(&cancel);
    actions.append(&import_selected);
    actions.append(&import_all);
    root.append(&actions);

    let generation = Rc::new(Cell::new(0_u64));
    let current_source = Rc::new(RefCell::new(None::<PathBuf>));

    let start_scan: Rc<dyn Fn(String, PathBuf)> = {
        let source_value = source_value.clone();
        let status = status.clone();
        let empty = empty.clone();
        let photos = photos.clone();
        let preview_stack = preview_stack.clone();
        let choose_source = choose_source.clone();
        let generation = generation.clone();
        let current_source = current_source.clone();
        let window = window.downgrade();

        Rc::new(move |name: String, root_path: PathBuf| {
            let request = generation.get().wrapping_add(1);
            generation.set(request);
            current_source.replace(Some(root_path.clone()));
            clear_flow(&photos);

            source_value.set_text(&format!("{name} — {}", root_path.display()));
            choose_source.set_label("Choose other…");
            status.set_text("Scanning DCIM for photos…");
            empty.set_text("Scanning camera or SD card…");
            preview_stack.set_visible_child_name("empty");

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
            let window = window.clone();
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
                        Ok((_, PreviewMessage::Discovered(total))) => {
                            discovered = total;
                            if total == 0 {
                                status.set_text("No supported photos found");
                                empty.set_text("No supported photos found in DCIM");
                            } else {
                                status.set_text(&format!("Found {total} photos — loading previews…"));
                            }
                        }
                        Ok((_, PreviewMessage::Ready { source, cached })) => {
                            shown += 1;
                            photos.insert(&preview_tile(&source, &cached), -1);
                            preview_stack.set_visible_child_name("photos");
                            status.set_text(&format!(
                                "Loading previews… {shown}/{}",
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
                                status.set_text(&format!("{ready} photos ready to import"));
                            } else {
                                status.set_text(&format!(
                                    "{ready} photos ready — {failed} previews could not be decoded"
                                ));
                            }
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
            move |result| match result {
                Ok(folder) => {
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
                Err(error) if !error.matches(gtk::DialogError::Dismissed) => {
                    trace(format!("manual_source_error error={error}"));
                }
                Err(_) => {}
            },
        );
    });

    let destination_value_for_dialog = destination_value.clone();
    let parent_for_destination = window.clone();
    choose_destination.connect_clicked(move |_| {
        let dialog = gtk::FileDialog::builder()
            .title("Choose Import Destination")
            .accept_label("Choose")
            .modal(true)
            .build();
        let destination_value = destination_value_for_dialog.clone();
        dialog.select_folder(
            Some(&parent_for_destination),
            None::<&gio::Cancellable>,
            move |result| {
                if let Ok(folder) = result {
                    let text = folder
                        .path()
                        .map(|path| path.display().to_string())
                        .unwrap_or_else(|| folder.uri().to_string());
                    destination_value.set_text(&text);
                }
            },
        );
    });

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
            clear_flow(&photos);
            source_value.set_text("No device selected");
            status.set_text("Camera or SD card removed");
            empty.set_text("Insert a camera or SD card to import photos");
            preview_stack.set_visible_child_name("empty");
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
