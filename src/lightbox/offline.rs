// Offline state belongs to the current lightbox photo, never to a texture.
struct OfflinePresentation {
    badge: gtk::Button,
    corrupt_badge: gtk::Label,
    notice: gtk::Label,
    retry_requested: Rc<Cell<bool>>,
    availability_revision: Rc<Cell<u64>>,
    tracked_photo: RefCell<Option<(PhotoObject, glib::SignalHandlerId)>>,
    unavailable: Rc<RefCell<Option<Box<dyn Fn(PhotoObject, gtk::Widget)>>>>,
}

impl OfflinePresentation {
    fn new(
        root: &gtk::Overlay,
        photos: Rc<RefCell<Vec<PhotoObject>>>,
        index: Rc<Cell<usize>>,
    ) -> Rc<Self> {
        let badge = gtk::Button::with_label("!");
        badge.add_css_class("offline-badge");
        badge.set_tooltip_text(Some("Original photo unavailable"));
        badge.set_halign(gtk::Align::Start);
        badge.set_valign(gtk::Align::Start);
        badge.set_margin_top(16);
        badge.set_margin_start(16);
        badge.set_visible(false);
        root.add_overlay(&badge);

        let corrupt_badge = gtk::Label::new(Some("Corrupt"));
        corrupt_badge.add_css_class("osd");
        corrupt_badge.add_css_class("corrupt-badge");
        corrupt_badge.set_halign(gtk::Align::Start);
        corrupt_badge.set_valign(gtk::Align::Start);
        corrupt_badge.set_margin_top(16);
        corrupt_badge.set_margin_start(16);
        corrupt_badge.set_can_target(false);
        corrupt_badge.set_visible(false);
        root.add_overlay(&corrupt_badge);

        let notice = gtk::Label::new(None);
        notice.add_css_class("osd");
        notice.set_halign(gtk::Align::Center);
        notice.set_valign(gtk::Align::End);
        notice.set_margin_bottom(16);
        notice.set_margin_start(16);
        notice.set_margin_end(16);
        notice.set_wrap(true);
        notice.set_can_target(false);
        notice.set_visible(false);
        root.add_overlay(&notice);

        let unavailable: Rc<RefCell<Option<Box<dyn Fn(PhotoObject, gtk::Widget)>>>> =
            Rc::new(RefCell::new(None));
        let handler = unavailable.clone();
        badge.connect_clicked(move |badge| {
            let photo = photos.borrow().get(index.get()).cloned();
            if let (Some(photo), Some(handler)) = (photo, handler.borrow().as_ref()) {
                handler(photo, badge.clone().upcast());
            }
        });
        Rc::new(Self {
            badge,
            corrupt_badge,
            notice,
            retry_requested: Rc::new(Cell::new(false)),
            availability_revision: Rc::new(Cell::new(0)),
            tracked_photo: RefCell::new(None),
            unavailable,
        })
    }

    fn track(&self, photo: &PhotoObject) {
        if self
            .tracked_photo
            .borrow()
            .as_ref()
            .is_some_and(|(current, _)| current == photo)
        {
            return;
        }
        self.disconnect();
        let available = Cell::new(photo.original_available());
        let retry = self.retry_requested.clone();
        let revision = self.availability_revision.clone();
        let handler = photo.connect_notify_local(Some("original-available"), move |photo, _| {
            let current = photo.original_available();
            if available.replace(current) != current {
                revision.set(revision.get().wrapping_add(1));
                retry.set(true);
            }
        });
        self.tracked_photo.replace(Some((photo.clone(), handler)));
    }

    fn disconnect(&self) {
        if let Some((photo, handler)) = self.tracked_photo.borrow_mut().take() {
            photo.disconnect(handler);
        }
        self.retry_requested.set(false);
        self.availability_revision
            .set(self.availability_revision.get().wrapping_add(1));
    }

    fn online(&self) {
        self.badge.set_visible(false);
        self.corrupt_badge.set_visible(false);
        self.notice.set_visible(false);
    }

    fn cached(&self, thumbnail: bool) {
        self.badge.set_visible(true);
        self.corrupt_badge.set_visible(false);
        self.notice.set_valign(gtk::Align::End);
        self.notice.set_label(if thumbnail {
            "Cached thumbnail — original unavailable"
        } else {
            "Cached image — original unavailable"
        });
        self.notice.set_visible(true);
    }

    fn missing(&self) {
        self.badge.set_visible(true);
        self.corrupt_badge.set_visible(false);
        self.notice.set_valign(gtk::Align::Center);
        self.notice
            .set_label("Original unavailable — no cached preview");
        self.notice.set_visible(true);
    }

    fn corrupt(&self) {
        self.badge.set_visible(false);
        self.corrupt_badge.set_visible(true);
        self.notice.set_valign(gtk::Align::Center);
        self.notice.set_label("The original photo cannot be decoded");
        self.notice.set_visible(true);
    }
}

impl Drop for OfflinePresentation {
    fn drop(&mut self) {
        self.disconnect();
    }
}

fn load_cached_lightbox_preview(
    request: &crate::thumbnail_display::DisplayRequest,
) -> Option<ViewerDecodeResult> {
    let quality = crate::thumbnail_display::wall_request(request.clone());
    for request in [&quality, request] {
        if let crate::thumbnail_display::DisplayOutcome::Loaded {
            width,
            height,
            pixels,
        } = crate::thumbnail_display::load_cached_display_thumbnail(request)
        {
            return Some(ViewerDecodeResult {
                width: width as u32,
                height: height as u32,
                pixels,
            });
        }
    }
    None
}

// Called for known offline photos, or after a failed original decode. All file
// probes and cached JPEG decoding run on a worker; this never generates cache
// entries or reads an original for a photo already known to be offline.
fn show_offline_preview(
    picture: &gtk::Picture,
    photo: PhotoObject,
    root: &gtk::Overlay,
    zoom: Rc<Cell<f64>>,
    generation: Rc<Cell<u64>>,
    expected_generation: u64,
    offline: Rc<OfflinePresentation>,
    known_offline: bool,
    navigation_ready: Option<Rc<Cell<bool>>>,
    navigation_settled: Option<Rc<dyn Fn()>>,
) {
    picture.set_paintable(gtk::gdk::Paintable::NONE);
    picture.set_filename(Option::<&str>::None);
    picture.set_size_request(1, 1);
    if known_offline {
        offline.cached(true);
    }
    let expected_availability_revision = offline.availability_revision.get();
    let path = photo.path();
    let request = crate::thumbnail_display::request_for(
        photo.cached_thumbnail_path().unwrap_or_default(),
        path.clone(),
        photo.mtime(),
        photo.size_bytes(),
        photo.rotation(),
        photo.edit_recipe(),
        photo.width(),
        photo.height(),
        true,
    );
    let picture = picture.clone();
    let root = root.clone();
    glib::MainContext::default().spawn_local(async move {
        let result = gio::spawn_blocking(move || {
            if !known_offline && crate::source::file_available(&path) {
                return None;
            }
            Some(load_cached_lightbox_preview(&request))
        })
        .await;
        if generation.get() != expected_generation || !root.is_visible() {
            return;
        }
        apply_offline_preview_result(
            &picture,
            &photo,
            &root,
            zoom.get(),
            &offline,
            known_offline,
            expected_availability_revision,
            result.unwrap_or_else(|_| known_offline.then_some(None)),
        );
        if let Some(ready) = navigation_ready {
            ready.set(true);
        }
        if let Some(settled) = navigation_settled {
            settled();
        }
    });
}

fn apply_offline_preview_result(
    picture: &gtk::Picture,
    photo: &PhotoObject,
    root: &gtk::Overlay,
    zoom: f64,
    offline: &OfflinePresentation,
    known_offline: bool,
    expected_availability_revision: u64,
    result: Option<Option<ViewerDecodeResult>>,
) {
    // Availability can change before the next frame advances the viewer
    // generation. A late cache completion must not undo that reconnection.
    if photo.original_available()
        && (known_offline || offline.availability_revision.get() != expected_availability_revision)
    {
        offline.retry_requested.set(true);
        return;
    }
    match result {
        Some(preview) => {
            photo.set_original_available(false);
            offline.retry_requested.set(false);
            if let Some(preview) = preview {
                let texture = gtk::gdk::MemoryTexture::new(
                    preview.width as i32,
                    preview.height as i32,
                    gtk::gdk::MemoryFormat::R8g8b8a8,
                    &glib::Bytes::from_owned(preview.pixels),
                    preview.width as usize * 4,
                );
                picture.set_paintable(Some(&texture));
                set_fit_geometry_from_intrinsic(
                    &picture,
                    &photo,
                    &root,
                    zoom.max(0.0),
                    texture.width(),
                    texture.height(),
                );
                offline.cached(true);
            } else {
                offline.missing();
            }
        }
        _ if known_offline => offline.missing(),
        _ => offline.corrupt(),
    }
}
