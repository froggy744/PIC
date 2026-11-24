fn connect_export_action(
    export: &gtk::Button,
    photo: crate::photo_object::PhotoObject,
    session: Rc<RefCell<EditSession>>,
    active_rotation: Rc<Cell<i32>>,
    apply_crop: Rc<dyn Fn()>,
    start_export: Rc<dyn Fn(Vec<super::export_batch::ExportJob>)>,
) {
    export.connect_clicked(move |_| {
        apply_crop();
        let recipe = session.borrow().recipe.encode();
        let job = super::export_batch::ExportJob::from_record(
            photo.path(),
            active_rotation.get(),
            recipe,
            photo.width(),
            photo.height(),
            &photo.filename(),
        );
        start_export(vec![job]);
    });
}

/// The current edit as a plain export job (the editor's pending crop applied).
fn current_edit_job(
    photo: &crate::photo_object::PhotoObject,
    session: &Rc<RefCell<EditSession>>,
    active_rotation: &Rc<Cell<i32>>,
) -> super::export_batch::ExportJob {
    super::export_batch::ExportJob::from_record(
        photo.path(),
        active_rotation.get(),
        session.borrow().recipe.encode(),
        photo.width(),
        photo.height(),
        &photo.filename(),
    )
}

fn present_quick_save_error(parent: &gtk::Window, heading: &str, error: &str) {
    let dialog = libadwaita::AlertDialog::builder()
        .heading(heading)
        .body(error)
        .close_response("close")
        .build();
    dialog.add_response("close", "Close");
    dialog.present(Some(parent));
}

/// Copy: put the edited image on the clipboard as PNG
/// (`quick_save::copy_to_clipboard`), so it can be pasted into a chat or
/// another app. The button shows a check mark for a moment once copied.
fn connect_copy_action(
    copy: &gtk::Button,
    photo: crate::photo_object::PhotoObject,
    session: Rc<RefCell<EditSession>>,
    active_rotation: Rc<Cell<i32>>,
    apply_crop: Rc<dyn Fn()>,
    parent: gtk::Window,
) {
    let tooltip = copy.tooltip_text();
    copy.connect_clicked(move |copy| {
        apply_crop();
        let job = current_edit_job(&photo, &session, &active_rotation);
        copy.set_sensitive(false);
        let button = copy.clone();
        let parent = parent.clone();
        let tooltip = tooltip.clone();
        super::quick_save::copy_to_clipboard(copy.clipboard(), job, move |copied| {
            let copy = button;
            copy.set_sensitive(true);
            if let Err(error) = copied {
                present_quick_save_error(&parent, "Could not copy the photo", &error);
                return;
            }
            copy.set_icon_name("object-select-symbolic");
            copy.set_tooltip_text(Some("Copied. Paste it into another app"));
            let copy = copy.downgrade();
            glib::timeout_add_local_once(Duration::from_millis(1600), move || {
                if let Some(copy) = copy.upgrade() {
                    copy.set_icon_name("edit-copy-symbolic");
                    copy.set_tooltip_text(tooltip.as_deref());
                }
            });
        });
    });
}

/// Save Copy (photos outside the library): write the edit as a new file
/// beside the original, then close the editor. Nothing is written to the
/// library, the original is never touched, and the original's photo keeps
/// no recipe (the viewer would otherwise show the original as edited).
fn connect_save_copy_action(
    done: &gtk::Button,
    photo: crate::photo_object::PhotoObject,
    session: Rc<RefCell<EditSession>>,
    active_rotation: Rc<Cell<i32>>,
    apply_crop: Rc<dyn Fn()>,
    parent: gtk::Window,
    on_close: Rc<dyn Fn()>,
    copy_saved: Rc<RefCell<Option<Box<dyn Fn(std::path::PathBuf)>>>>,
) {
    done.connect_clicked(move |done| {
        apply_crop();
        let job = current_edit_job(&photo, &session, &active_rotation);
        done.set_sensitive(false);
        let done = done.clone();
        let parent = parent.clone();
        let on_close = on_close.clone();
        let copy_saved = copy_saved.clone();
        glib::MainContext::default().spawn_local(async move {
            let saved = gio::spawn_blocking(move || {
                super::quick_save::save_copy(&job).map_err(|error| format!("{error:#}"))
            })
            .await
            .unwrap_or_else(|_| Err("the renderer stopped unexpectedly".to_string()));
            done.set_sensitive(true);
            match saved {
                Ok(path) => {
                    if let Some(handler) = copy_saved.borrow().as_ref() {
                        handler(path);
                    }
                    on_close();
                }
                Err(error) => {
                    present_quick_save_error(&parent, "Could not save a copy", &error);
                }
            }
        });
    });
}
