#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ZoomAnchorKind {
    Pointer,
    ViewportCenter,
}

#[derive(Debug, Clone, Copy)]
struct ZoomAnchor {
    photo_id: i64,
    desired_x: f64,
    desired_y: f64,
    kind: ZoomAnchorKind,
}

pub struct Gallery {
    // GtkGridView must remain the direct GtkScrolledWindow child. GTK's list
    // widgets are GtkScrollable and rely on that relationship for correct
    // visible-item allocation and virtualization. Do not wrap this GridView
    // in a Box/Viewport to implement grouping.
    pub root: gtk::GridView,
    pub folder_root: gtk::ListView,
    pub folder_sectioned_root: gtk::Fixed,
    pub folder_rubberband: gtk::DrawingArea,
    pub group_header: gtk::Box,
    group_title: gtk::Label,
    group_count: gtk::Label,
    folder_store: gio::ListStore,
    // Reusable Folder stream. Populated whenever the Folder rows are rebuilt
    // and restored when re-entering Folder mode so Open in Folder is instant.
    folder_cache: Rc<RefCell<Option<FolderStreamCache>>>,
    // Presentation-only Folder section order. This must never reorder the
    // shared Library photo model. Empty means use the natural range order.
    folder_order: Rc<RefCell<Vec<i64>>>,
    // Includes parent/container folders that have recursive photos but no
    // direct photo range. Those folders still need a real virtual header so a
    // sidebar click can land on the requested folder rather than its first
    // descendant.
    folder_catalog: Rc<RefCell<Vec<FolderCatalogEntry>>>,
    folder_view_changed: Rc<RefCell<Option<Rc<dyn Fn(bool)>>>>,
    selected: Rc<dyn Fn(Option<PhotoObject>)>,
    store: gio::ListStore,
    selection: gtk::MultiSelection,
    collage_selection_mode: Rc<Cell<bool>>,
    collage_selected_ids: Rc<RefCell<HashSet<i64>>>,
    current_columns: Rc<Cell<u32>>,
    last_layout_width: Rc<Cell<i32>>,
    tile_width: Rc<Cell<i32>>,
    tile_height: Rc<Cell<i32>>,
    current_photos: Rc<RefCell<Vec<PhotoObject>>>,
    replace_generation: Rc<Cell<u64>>,
    // True while a progressive gallery replacement is still building batches.
    // Folder navigation relies on this to keep retrying until the virtualized
    // Folder rows actually exist, instead of stopping as soon as the backing
    // photo store contains the target id.
    stream_building: Rc<Cell<bool>>,
    // Folder selected from search while the continuous stream is being built.
    // Keep the target with the Gallery so clearing the search cannot drop it.
    pending_folder_target: Rc<RefCell<Option<(i64, String)>>>,
    group_mode: Rc<Cell<GroupMode>>,
    group_date: Rc<Cell<GroupDate>>,
    group_ranges: Rc<RefCell<Vec<GroupRange>>>,
    last_scroll_y: Rc<Cell<f64>>,
    // Photo at the viewport top captured before a zoom resizes the tiles, so
    // the row reshape can restore the same viewport after the relayout.
    zoom_anchor: Rc<Cell<Option<i64>>>,
    // Presentation-only stable zoom focus. Unlike the legacy Folder anchor
    // above, this stores the exact realized photo and its viewport-relative
    // position so GridView/SectionedFolder zoom can preserve what the user is
    // looking at without changing model order or selection.
    stable_zoom_anchor: Rc<Cell<Option<ZoomAnchor>>>,
    // Invalidates bounded frame-clock restoration callbacks when a newer zoom
    // or model/navigation change supersedes the old request.
    zoom_anchor_restore_generation: Rc<Cell<u64>>,
    folder_scroll_generation: Rc<Cell<u64>>,
    folder_anchor_photo: Cell<Option<i64>>,
    // True while a column change is still positioning the viewport. A second
    // change before it settles must reuse the original anchor instead of
    // reading GTK's transient (often zeroed) adjustment.
    folder_pending_reframe: Rc<Cell<bool>>,
    folder_reframe_photo: Rc<Cell<Option<i64>>>,
    // Coalesces rapid Ctrl+wheel zoom input: the visual reflow is deferred while
    // the user is still spinning so crossing several column boundaries triggers
    // one Folder row rebuild instead of one per notch.
    pending_zoom_width: Rc<Cell<Option<i32>>>,
    zoom_reflow_source: Rc<RefCell<Option<glib::SourceId>>>,
    // Invalidates an in-flight frame-clock zoom animation when a newer zoom
    // target arrives. The next animation starts from the current visual size,
    // so rapid wheel input retargets instead of queueing animations.
    zoom_animation_generation: Rc<Cell<u64>>,
    // Visible tiles participating in the temporary Main-branch zoom settle.
    zoom_scale_tiles: Rc<RefCell<Vec<(SquareTile, i64)>>>,
    // Stable content width captured at zoom-animation start. GTK can briefly
    // report two competing allocations while GridView reflows; using one
    // width for the whole animation prevents column-count ping-pong.
    zoom_animation_layout_width: Rc<Cell<Option<i32>>>,
    // User preference for presentation-only thumbnail zoom transitions.
    // Geometry and anchor restoration still happen when this is disabled.
    zoom_animations_enabled: Rc<Cell<bool>>,
    // Invalidates/retargets presentation-only FLIP animations when live
    // window resizing crosses another column boundary mid-transition.
    resize_flip_generation: Rc<Cell<u64>>,
    // Set when no user-chosen thumbnail size exists: the first real layout
    // adopts the ~4-thumbnails-per-row default instead of a fixed pixel size.
    auto_default_zoom: Cell<bool>,
    // Letterbox whole photos (Contain) instead of cropping to the tile
    // (Cover), so portrait thumbnails show portrait, not a centre strip.
    fit_whole_photo: Rc<Cell<bool>>,
    show_file_names: Rc<Cell<bool>>,
    sectioned_folder: Rc<SectionedFolderView>,
    on_zoom_changed: Rc<dyn Fn(i32)>,
}

impl Gallery {
    pub fn new(
        photos: &[Photo],
        initial_tile_width: i32,
        selected: impl Fn(Option<PhotoObject>) + 'static,
        activate: impl Fn(Vec<PhotoObject>, usize, Option<(gtk::Widget, gtk::gdk::Paintable)>) + 'static,
        context_menu: impl Fn(PhotoObject, gtk::Widget, f64, f64) + 'static,
        unavailable: impl Fn(PhotoObject, gtk::Widget) + 'static,
        on_zoom_changed: impl Fn(i32) + 'static,
    ) -> Self {
        let selected: Rc<dyn Fn(Option<PhotoObject>)> = Rc::new(selected);
        let activate: Rc<dyn Fn(Vec<PhotoObject>, usize, Option<(gtk::Widget, gtk::gdk::Paintable)>)> =
            Rc::new(activate);
        let context_menu: Rc<dyn Fn(PhotoObject, gtk::Widget, f64, f64)> = Rc::new(context_menu);
        let unavailable: Rc<dyn Fn(PhotoObject, gtk::Widget)> = Rc::new(unavailable);
        let on_zoom_changed: Rc<dyn Fn(i32)> = Rc::new(on_zoom_changed);
        let store = gio::ListStore::new::<PhotoObject>();
        let selection = gtk::MultiSelection::new(Some(store.clone()));
        let collage_selection_mode = Rc::new(Cell::new(false));
        let collage_selected_ids = Rc::new(RefCell::new(HashSet::new()));
        let current_columns = Rc::new(Cell::new(5u32));
        // Independent thumbnail width/height. Change DEFAULT_TILE_WIDTH and
        // DEFAULT_TILE_HEIGHT above to choose your preferred starting size.
        let tile_width = Rc::new(Cell::new(
            initial_tile_width.clamp(MIN_TILE_WIDTH, MAX_TILE_WIDTH),
        ));
        let tile_height = Rc::new(Cell::new(
            ((DEFAULT_TILE_HEIGHT as f64) * tile_width.get() as f64 / DEFAULT_TILE_WIDTH as f64)
                .round()
                .max(1.0) as i32,
        ));

        // Grouping is presented as a sticky heading outside the scrolled
        // GridView. This deliberately avoids nesting multiple GtkGridViews in
        // a GtkViewport, which broke row allocation and virtualization.
        let group_header = gtk::Box::new(gtk::Orientation::Horizontal, 7);
        group_header.set_hexpand(true);
        group_header.set_visible(false);
        group_header.set_margin_start(20);
        group_header.set_margin_end(20);
        group_header.set_margin_top(10);
        group_header.set_margin_bottom(4);
        group_header.add_css_class("group-heading-bar");

        let group_title = gtk::Label::new(None);
        group_title.set_xalign(0.0);
        group_title.add_css_class("section-heading");
        group_header.append(&group_title);

        let group_count = gtk::Label::new(None);
        group_count.set_xalign(0.0);
        group_count.add_css_class("dim-label");
        group_count.add_css_class("section-count");
        group_header.append(&group_count);

        let current_photos = Rc::new(RefCell::new(Vec::<PhotoObject>::new()));

        let factory = gtk::SignalListItemFactory::new();
        let tile_width_for_setup = tile_width.clone();
        let tile_height_for_setup = tile_height.clone();
        let unavailable_for_setup = unavailable.clone();
        let fit_whole_photo = Rc::new(Cell::new(false));
        let fit_whole_photo_for_setup = fit_whole_photo.clone();
        let show_file_names = Rc::new(Cell::new(false));
        let show_file_names_for_setup = show_file_names.clone();
        let zoom_animations_enabled = Rc::new(Cell::new(true));
        let raw_jpeg_pair_folders = Rc::new(RefCell::new(HashSet::<i64>::new()));
        let raw_jpeg_mode = Rc::new(Cell::new(crate::image_format::RawJpegPairMode::Both));
        let raw_jpeg_mode_changed: Rc<
            RefCell<Option<Rc<dyn Fn(crate::image_format::RawJpegPairMode)>>>,
        > = Rc::new(RefCell::new(None));

        // Folder mode keeps the same flat PhotoObject model as the main GridView,
        // but renders it through a section-aware virtualized surface so folder
        // headers can span the full width and every folder begins on a fresh row.
        // These grouping cells used to be constructed only in Self below; create
        // them here so the sectioned surface shares the exact same metadata.
        let group_mode = Rc::new(Cell::new(GroupMode::None));
        let group_date = Rc::new(Cell::new(GroupDate::Taken));
        let group_ranges = Rc::new(RefCell::new(Vec::new()));
        let sectioned_folder = SectionedFolderView::new(
            current_photos.clone(),
            group_ranges.clone(),
            selection.clone(),
            current_columns.clone(),
            tile_width.clone(),
            tile_height.clone(),
            fit_whole_photo.clone(),
            show_file_names.clone(),
            raw_jpeg_pair_folders,
            raw_jpeg_mode,
            raw_jpeg_mode_changed,
            activate.clone(),
            context_menu.clone(),
            unavailable.clone(),
            collage_selection_mode.clone(),
            collage_selected_ids.clone(),
            zoom_animations_enabled.clone(),
        );
        let folder_sectioned_root = sectioned_folder.root().clone();
        factory.connect_setup(move |_, object| {
            let Some(list_item) = object.downcast_ref::<gtk::ListItem>() else {
                return;
            };

            let frame = gtk::Overlay::new();
            frame.set_overflow(gtk::Overflow::Hidden);
            frame.add_css_class("photo-frame");
            frame.add_css_class("photo-tile");

            let picture = gtk::Picture::new();
            picture.set_content_fit(if fit_whole_photo_for_setup.get() {
                gtk::ContentFit::Contain
            } else {
                gtk::ContentFit::Cover
            });
            picture.set_can_shrink(true);
            picture.set_size_request(1, 1);
            picture.set_hexpand(true);
            picture.set_vexpand(true);
            picture.set_halign(gtk::Align::Fill);
            picture.set_valign(gtk::Align::Fill);
            picture.add_css_class("thumbnail");
            frame.set_child(Some(&picture));

            let placeholder = gtk::Image::from_icon_name("image-x-generic-symbolic");
            placeholder.set_pixel_size(32);
            placeholder.add_css_class("dim-label");
            placeholder.set_visible(false);
            frame.add_overlay(&placeholder);

            let checkmark = gtk::Image::from_icon_name("object-select-symbolic");
            checkmark.set_pixel_size(18);
            checkmark.set_halign(gtk::Align::End);
            checkmark.set_valign(gtk::Align::Start);
            checkmark.set_margin_top(8);
            checkmark.set_margin_end(8);
            checkmark.add_css_class("selection-badge");
            frame.add_overlay(&checkmark);

            let favorite_badge = gtk::Image::from_icon_name("emote-love-symbolic");
            favorite_badge.set_pixel_size(18);
            favorite_badge.set_halign(gtk::Align::End);
            favorite_badge.set_valign(gtk::Align::End);
            favorite_badge.set_margin_bottom(8);
            favorite_badge.set_margin_end(8);
            favorite_badge.add_css_class("favorite-badge");
            favorite_badge.set_visible(false);
            frame.add_overlay(&favorite_badge);

            let edited_badge = gtk::Image::from_icon_name("document-edit-symbolic");
            edited_badge.set_pixel_size(18);
            edited_badge.set_halign(gtk::Align::Start);
            edited_badge.set_valign(gtk::Align::End);
            edited_badge.set_margin_bottom(8);
            edited_badge.set_margin_start(8);
            edited_badge.add_css_class("edited-badge");
            edited_badge.set_tooltip_text(Some("Edited"));
            edited_badge.set_visible(false);
            frame.add_overlay(&edited_badge);

            let unavailable_badge = gtk::Button::with_label("!");
            unavailable_badge.set_halign(gtk::Align::Start);
            unavailable_badge.set_valign(gtk::Align::Start);
            unavailable_badge.set_margin_top(8);
            unavailable_badge.set_margin_start(8);
            unavailable_badge.add_css_class("offline-badge");
            unavailable_badge.set_tooltip_text(Some("Original photo unavailable"));
            unavailable_badge.set_visible(false);
            frame.add_overlay(&unavailable_badge);

            let list_item_for_unavailable = list_item.clone();
            let unavailable_for_click = unavailable_for_setup.clone();
            let badge_for_click = unavailable_badge.clone();
            unavailable_badge.connect_clicked(move |_| {
                if let Some(photo) = list_item_for_unavailable
                    .item()
                    .and_downcast::<PhotoObject>()
                {
                    (unavailable_for_click)(photo, badge_for_click.clone().upcast());
                }
            });

            let tile = SquareTile::new(
                tile_width_for_setup.get(),
                tile_height_for_setup.get(),
                &frame,
            );
            tile.set_filename_visible(show_file_names_for_setup.get());
            tile.set_hexpand(true);
            tile.set_vexpand(false);
            tile.set_valign(gtk::Align::Start);
            list_item.set_child(Some(&tile));
        });

        factory.connect_bind(|_, object| {
            let Some(list_item) = object.downcast_ref::<gtk::ListItem>() else {
                return;
            };
            let Some(photo) = list_item.item().and_downcast::<PhotoObject>() else {
                return;
            };
            let Some(tile) = list_item.child().and_downcast::<SquareTile>() else {
                return;
            };
            tile.imp()
                .photo_index
                .set(Some(list_item.position() as usize));
            tile.bind_photo(&photo);
        });

        let root = gtk::GridView::new(Some(selection.clone()), Some(factory));
        root.set_min_columns(5);
        root.set_max_columns(5);
        root.set_single_click_activate(false);
        root.set_enable_rubberband(true);
        root.set_hexpand(true);
        root.set_vexpand(true);
        root.set_halign(gtk::Align::Fill);
        root.set_valign(gtk::Align::Fill);
        root.add_css_class("section-grid");

        // GridView children are recycled and the pointer may land on any
        // descendant of a tile. Use one stable controller on GridView, pick
        // the tile under the pointer, and only claim the event after a bound
        // photo has been identified. Select an unselected clicked photo so
        // context-menu actions and focus restoration target that thumbnail.
        // Right-clicking within an existing multi-selection preserves it.
        let right_click = gtk::GestureClick::new();
        right_click.set_button(3);
        right_click.set_propagation_phase(gtk::PropagationPhase::Capture);
        let context_menu_for_grid = context_menu.clone();
        let root_for_context = root.clone();
        let selection_for_context = selection.clone();
        right_click.connect_pressed(move |gesture, _, x, y| {
            let Some(picked) = root_for_context.pick(x, y, gtk::PickFlags::DEFAULT) else {
                return;
            };
            let Some(tile) = picked
                .ancestor(SquareTile::static_type())
                .and_downcast::<SquareTile>()
            else {
                return;
            };
            let Some(photo) = tile.imp().photo.borrow().as_ref().cloned() else {
                return;
            };
            let Some(position) = (0..selection_for_context.n_items()).find(|position| {
                selection_for_context
                    .item(*position)
                    .and_downcast::<PhotoObject>()
                    .is_some_and(|item| item.id() == photo.id())
            }) else {
                return;
            };
            let Some(frame) = tile.first_child().and_downcast::<gtk::Overlay>() else {
                return;
            };

            let frame_widget = frame.clone().upcast::<gtk::Widget>();
            let local = root_for_context
                .compute_point(
                    &frame_widget,
                    &gtk::graphene::Point::new(x as f32, y as f32),
                )
                .unwrap_or_else(|| gtk::graphene::Point::new(0.0, 0.0));

            gesture.set_state(gtk::EventSequenceState::Claimed);
            if !selection_for_context.is_selected(position) {
                selection_for_context.select_item(position, true);
            }
            root_for_context.grab_focus();
            (context_menu_for_grid)(photo, frame_widget, local.x() as f64, local.y() as f64);
        });
        root.add_controller(right_click);

        // Handle Add Photos clicks on the GridView itself, before the
        // built-in GridView selection controller sees them. This makes the
        // mode behave like a checklist: each click toggles one item and does
        // not collapse the other selected photos.
        let collage_click = gtk::GestureClick::new();
        collage_click.set_button(1);
        collage_click.set_propagation_phase(gtk::PropagationPhase::Capture);
        let mode = collage_selection_mode.clone();
        let selection_for_click = selection.clone();
        let collage_selected_ids_for_click = collage_selected_ids.clone();
        let root_for_click = root.clone();
        collage_click.connect_pressed(move |gesture, _, x, y| {
            if !mode.get() {
                return;
            }
            let Some(picked) = root_for_click.pick(x, y, gtk::PickFlags::DEFAULT) else {
                return;
            };
            let Some(tile) = picked
                .ancestor(SquareTile::static_type())
                .and_downcast::<SquareTile>()
            else {
                return;
            };
            let Some(photo_id) = tile.imp().photo.borrow().as_ref().map(|photo| photo.id()) else {
                return;
            };
            let Some(position) = (0..selection_for_click.n_items()).find(|position| {
                selection_for_click
                    .item(*position)
                    .and_downcast::<PhotoObject>()
                    .is_some_and(|photo| photo.id() == photo_id)
            }) else {
                return;
            };
            gesture.set_state(gtk::EventSequenceState::Claimed);
            let mut selected_ids = collage_selected_ids_for_click.borrow_mut();
            if selected_ids.remove(&photo_id) {
                selection_for_click.unselect_item(position);
            } else {
                selected_ids.insert(photo_id);
                selection_for_click.select_item(position, false);
            }
        });
        root.add_controller(collage_click);

        let selected_for_signal = selected.clone();
        let sectioned_for_selection = sectioned_folder.clone();
        selection.connect_selection_changed(move |selection, _, _| {
            let selected = selection.selection();
            let photo = gtk::BitsetIter::init_first(&selected)
                .and_then(|(_, position)| selection.item(position))
                .and_downcast::<PhotoObject>();
            sectioned_for_selection.sync_selection();
            (selected_for_signal)(photo);
        });

        let current_photos_for_activate = current_photos.clone();
        let selection_for_activate = selection.clone();
        let activate_for_grid = activate.clone();
        let root_for_activate = root.clone();
        root.connect_activate(move |_, position| {
            let Some(activated) = selection_for_activate
                .model()
                .and_then(|model| model.item(position))
                .and_downcast::<PhotoObject>()
            else {
                return;
            };
            let photos = current_photos_for_activate.borrow().clone();
            let index = photos
                .iter()
                .position(|photo| photo.id() == activated.id())
                .unwrap_or(position as usize);
            let source = {
                let mut tiles = Vec::new();
                collect_tiles(root_for_activate.upcast_ref(), &mut tiles);
                tiles
                    .into_iter()
                    .find(|tile| {
                        tile.photo()
                            .as_ref()
                            .is_some_and(|photo| photo.id() == activated.id())
                    })
                    .and_then(|tile| {
                        tile.transition_paintable()
                            .map(|paintable| (tile.upcast::<gtk::Widget>(), paintable))
                    })
            };
            (activate_for_grid)(photos, index, source);
        });

        // Folder mode is a flat virtualized ListView. Each Photos model item
        // represents exactly one visual line (at most `current_columns`
        // photos). There is no nested FlowBox and no second viewport/prefetch
        // system: GtkListView owns row lifetime, while bind/unbind owns the
        // SquareTile thumbnail lifetime exactly like the working GridView.
        let folder_store = gio::ListStore::new::<FolderRowObject>();
        let folder_selection = gtk::NoSelection::new(Some(folder_store.clone()));
        let folder_factory = gtk::SignalListItemFactory::new();
        let folder_selected_ids: Rc<RefCell<HashSet<i64>>> = Rc::new(RefCell::new(HashSet::new()));

        let setup_tile_height = tile_height.clone();
        let setup_show_file_names = show_file_names.clone();
        folder_factory.connect_setup(move |_, object| {
            let Some(list_item) = object.downcast_ref::<gtk::ListItem>() else {
                return;
            };

            let row_root = gtk::Box::new(gtk::Orientation::Vertical, 0);
            row_root.set_hexpand(true);
            row_root.set_vexpand(false);
            row_root.add_css_class("folder-stream-row");
            row_root.set_height_request(folder_model_row_height(
                FolderRowKind::Header,
                setup_tile_height.get(),
                setup_show_file_names.get(),
            ));

            let header_outer = gtk::Box::new(gtk::Orientation::Vertical, 0);
            header_outer.set_widget_name("picasa-folder-section-header");
            header_outer.set_hexpand(true);
            header_outer.set_margin_top(26);
            header_outer.set_margin_bottom(8);
            header_outer.set_margin_start(6);
            header_outer.set_margin_end(6);
            header_outer.add_css_class("folder-section-header");

            let header_line = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            header_line.set_hexpand(true);
            let folder_icon = gtk::Image::from_icon_name("folder-symbolic");
            folder_icon.set_pixel_size(16);
            folder_icon.add_css_class("folder-section-icon");
            header_line.append(&folder_icon);

            let title = gtk::Label::new(None);
            title.set_widget_name("picasa-folder-section-title");
            title.set_xalign(0.0);
            title.set_ellipsize(gtk::pango::EllipsizeMode::End);
            title.add_css_class("folder-section-title");
            header_line.append(&title);

            let count = gtk::Label::new(None);
            count.set_widget_name("picasa-folder-section-count");
            count.set_xalign(0.0);
            count.add_css_class("dim-label");
            count.add_css_class("folder-section-count");
            header_line.append(&count);

            let header_spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            header_spacer.set_hexpand(true);
            header_line.append(&header_spacer);

            let actions = gtk::Box::new(gtk::Orientation::Horizontal, 4);
            actions.set_widget_name("picasa-folder-section-actions");
            actions.add_css_class("folder-section-actions");
            header_line.append(&actions);

            header_outer.append(&header_line);
            let separator = gtk::Separator::new(gtk::Orientation::Horizontal);
            separator.add_css_class("folder-section-separator");
            header_outer.append(&separator);
            row_root.append(&header_outer);

            let photo_line = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            photo_line.set_widget_name("picasa-folder-photo-line");
            photo_line.set_hexpand(true);
            photo_line.set_vexpand(false);
            photo_line.set_halign(gtk::Align::Fill);
            photo_line.set_valign(gtk::Align::Start);
            photo_line.add_css_class("folder-photo-line");
            row_root.append(&photo_line);

            list_item.set_child(Some(&row_root));
        });

        let tile_width_for_folder_bind = tile_width.clone();
        let tile_height_for_folder_bind = tile_height.clone();
        let current_columns_for_folder_bind = current_columns.clone();
        let current_photos_for_folder_bind = current_photos.clone();
        let unavailable_for_folder_bind = unavailable.clone();
        let selected_ids_for_folder_bind = folder_selected_ids.clone();
        let show_file_names_for_folder_bind = show_file_names.clone();

        folder_factory.connect_bind(move |_, object| {
            let Some(list_item) = object.downcast_ref::<gtk::ListItem>() else {
                return;
            };
            let Some(row) = list_item.item().and_downcast::<FolderRowObject>() else {
                return;
            };
            let Some(row_root) = list_item.child().and_downcast::<gtk::Box>() else {
                return;
            };
            let data = row.data();

            let row_name = format!("picasa-folder-row-{}", data.folder_id);
            if row_root.widget_name() != row_name {
                row_root.set_widget_name(&row_name);
            }
            let Some(header) = row_root.first_child().and_downcast::<gtk::Box>() else {
                return;
            };
            let Some(photo_line) = row_root.last_child().and_downcast::<gtk::Box>() else {
                return;
            };

            match data.kind {
                FolderRowKind::Header => {
                    let row_height = folder_model_row_height(
                        FolderRowKind::Header,
                        tile_height_for_folder_bind.get(),
                        show_file_names_for_folder_bind.get(),
                    );
                    if row_root.height_request() != row_height {
                        row_root.set_height_request(row_height);
                    }
                    if !header.is_visible() {
                        header.set_visible(true);
                    }
                    if photo_line.is_visible() {
                        photo_line.set_visible(false);
                    }
                    for tile in box_tiles(&photo_line) {
                        if tile.imp().photo.borrow().is_some() {
                            tile.clear_photo_folder_recycle();
                        }
                        if tile.opacity() != 1.0 {
                            tile.set_opacity(1.0);
                        }
                        if tile.can_target() {
                            tile.set_can_target(false);
                        }
                        if tile.is_visible() {
                            tile.set_visible(false);
                        }
                    }
                    if let Some(title) = find_named_label(header.upcast_ref(), "picasa-folder-section-title") {
                        if title.text().as_str() != data.label {
                            title.set_text(&data.label);
                        }
                        if title.tooltip_text().as_deref() != Some(data.folder_path.as_str()) {
                            title.set_tooltip_text(Some(&data.folder_path));
                        }
                    }
                    if let Some(count) = find_named_label(header.upcast_ref(), "picasa-folder-section-count") {
                        let count_text = format!(
                            "{} {}",
                            format_count(data.count),
                            if data.count == 1 { "photo" } else { "photos" }
                        );
                        if count.text().as_str() != count_text {
                            count.set_text(&count_text);
                        }
                    }
                    let is_first = list_item.position() == 0;
                    if header.has_css_class("first-folder-section-header") != is_first {
                        if is_first {
                            header.add_css_class("first-folder-section-header");
                        } else {
                            header.remove_css_class("first-folder-section-header");
                        }
                    }
                    let first_margin = if is_first { 10 } else { 26 };
                    if header.margin_top() != first_margin {
                        header.set_margin_top(first_margin);
                    }
                }
                FolderRowKind::Photos => {
                    if header.is_visible() {
                        header.set_visible(false);
                    }
                    if !photo_line.is_visible() {
                        photo_line.set_visible(true);
                    }
                    let row_height = folder_model_row_height(
                        FolderRowKind::Photos,
                        tile_height_for_folder_bind.get(),
                        show_file_names_for_folder_bind.get(),
                    );
                    if row_root.height_request() != row_height {
                        row_root.set_height_request(row_height);
                    }

                    let row_photos = {
                        let photos = current_photos_for_folder_bind.borrow();
                        photos
                            .get(data.start..data.end)
                            .map(|slice| slice.to_vec())
                            .unwrap_or_default()
                    };
                    let slot_count = current_columns_for_folder_bind.get().max(1) as usize;
                    let mut tiles = box_tiles(&photo_line);
                    while tiles.len() < slot_count {
                        let tile = make_folder_tile(
                            tile_width_for_folder_bind.get(),
                            tile_height_for_folder_bind.get(),
                            &unavailable_for_folder_bind,
                        );
                        photo_line.append(&tile);
                        tiles.push(tile);
                    }

                    for (slot, tile) in tiles.iter().enumerate() {
                        if slot >= slot_count {
                            if tile.imp().photo.borrow().is_some() {
                                tile.clear_photo_folder_recycle();
                            }
                            if tile.opacity() != 1.0 {
                                tile.set_opacity(1.0);
                            }
                            if tile.can_target() {
                                tile.set_can_target(false);
                            }
                            if tile.is_visible() {
                                tile.set_visible(false);
                            }
                            continue;
                        }

                        if !tile.is_visible() {
                            tile.set_visible(true);
                        }
                        tile.set_tile_size(
                            tile_width_for_folder_bind.get(),
                            tile_height_for_folder_bind.get(),
                        );
                        if let Some(photo) = row_photos.get(slot) {
                            if tile.opacity() != 1.0 {
                                tile.set_opacity(1.0);
                            }
                            if !tile.can_target() {
                                tile.set_can_target(true);
                            }
                            tile.bind_photo_folder_fast(photo, data.start + slot);
                            // Keep the recycled tile's caption flag identical to
                            // the row height this line was sized with.
                            tile.set_filename_visible(show_file_names_for_folder_bind.get());
                            tile.set_manual_selected(
                                selected_ids_for_folder_bind.borrow().contains(&photo.id()),
                            );
                        } else {
                            // Hidden widgets do not participate in GtkBox layout.
                            // Keep an allocated transparent slot so a short final
                            // line preserves the exact same column geometry.
                            if tile.imp().photo.borrow().is_some() {
                                tile.clear_photo_folder_recycle();
                            }
                            if tile.opacity() != 0.0 {
                                tile.set_opacity(0.0);
                            }
                            if tile.can_target() {
                                tile.set_can_target(false);
                            }
                        }
                    }
                }
            }
        });

        folder_factory.connect_unbind(move |_, object| {
            let Some(list_item) = object.downcast_ref::<gtk::ListItem>() else {
                return;
            };
            let Some(row_root) = list_item.child().and_downcast::<gtk::Box>() else {
                return;
            };
            let Some(photo_line) = row_root.last_child().and_downcast::<gtk::Box>() else {
                return;
            };
            for tile in box_tiles(&photo_line) {
                tile.clear_photo_folder_recycle();
                tile.set_opacity(1.0);
                tile.set_can_target(false);
                tile.set_visible(false);
            }
        });

        let folder_root = gtk::ListView::new(Some(folder_selection.clone()), Some(folder_factory));
        folder_root.set_single_click_activate(false);
        folder_root.set_show_separators(false);
        folder_root.set_hexpand(true);
        folder_root.set_vexpand(true);
        folder_root.set_halign(gtk::Align::Fill);
        folder_root.set_valign(gtk::Align::Fill);
        folder_root.add_css_class("folder-stream");
        folder_root.add_css_class("photo-grid");

        let folder_rubberband = install_folder_root_input(
            &folder_root,
            &selection,
            &current_photos,
            &activate,
            &context_menu,
            &collage_selection_mode,
            &collage_selected_ids,
        );

        let folder_root_for_selection = folder_root.clone();
        let selection_for_folder_style = selection.clone();
        let selected_ids_for_selection = folder_selected_ids.clone();
        let styles_refresh_scheduled = Rc::new(Cell::new(false));
        selection.connect_selection_changed(move |selection, _, _| {
            selected_ids_for_selection.replace(selected_photo_id_set(selection));
            if styles_refresh_scheduled.replace(true) {
                return;
            }
            let root = folder_root_for_selection.clone();
            let selection = selection_for_folder_style.clone();
            let scheduled = styles_refresh_scheduled.clone();
            glib::idle_add_local_once(move || {
                scheduled.set(false);
                refresh_folder_selection_styles(&root, &selection);
            });
        });

        Gallery::install_folder_keyboard(&folder_root, &selection, &current_photos, &activate);

        let gallery = Self {
            root,
            folder_root,
            folder_sectioned_root,
            folder_rubberband,
            group_header,
            group_title,
            group_count,
            folder_store,
            folder_cache: Rc::new(RefCell::new(None)),
            folder_order: Rc::new(RefCell::new(Vec::new())),
            folder_catalog: Rc::new(RefCell::new(Vec::new())),
            folder_view_changed: Rc::new(RefCell::new(None)),
            selected,
            store,
            selection,
            collage_selection_mode,
            collage_selected_ids,
            current_columns,
            last_layout_width: Rc::new(Cell::new(0)),
            tile_width,
            tile_height,
            current_photos,
            replace_generation: Rc::new(Cell::new(0)),
            stream_building: Rc::new(Cell::new(false)),
            pending_folder_target: Rc::new(RefCell::new(None)),
            group_mode,
            group_date,
            group_ranges,
            last_scroll_y: Rc::new(Cell::new(0.0)),
            zoom_anchor: Rc::new(Cell::new(None)),
            stable_zoom_anchor: Rc::new(Cell::new(None)),
            zoom_anchor_restore_generation: Rc::new(Cell::new(0)),
            folder_scroll_generation: Rc::new(Cell::new(0)),
            folder_anchor_photo: Cell::new(None),
            folder_pending_reframe: Rc::new(Cell::new(false)),
            folder_reframe_photo: Rc::new(Cell::new(None)),
            pending_zoom_width: Rc::new(Cell::new(None)),
            zoom_reflow_source: Rc::new(RefCell::new(None)),
            zoom_animation_generation: Rc::new(Cell::new(0)),
            zoom_scale_tiles: Rc::new(RefCell::new(Vec::new())),
            zoom_animation_layout_width: Rc::new(Cell::new(None)),
            zoom_animations_enabled,
            resize_flip_generation: Rc::new(Cell::new(0)),
            auto_default_zoom: Cell::new(false),
            fit_whole_photo,
            show_file_names,
            sectioned_folder,
            on_zoom_changed,
        };
        gallery.replace(photos);
        gallery
    }

    pub fn set_raw_jpeg_pair_folders(&self, folder_ids: HashSet<i64>) {
        self.sectioned_folder.set_raw_jpeg_pair_folders(folder_ids);
    }

    pub fn set_raw_jpeg_mode(&self, mode: crate::image_format::RawJpegPairMode) {
        self.sectioned_folder.set_raw_jpeg_mode(mode);
    }

    pub fn set_raw_jpeg_mode_changed_handler(
        &self,
        handler: impl Fn(crate::image_format::RawJpegPairMode) + 'static,
    ) {
        self.sectioned_folder
            .set_raw_jpeg_mode_changed_handler(Rc::new(handler));
    }

    pub fn set_zoom_animations_enabled(&self, enabled: bool) {
        self.zoom_animations_enabled.set(enabled);
        if !enabled {
            self.zoom_animation_generation
                .set(self.zoom_animation_generation.get().wrapping_add(1));
            for (tile, _) in self.zoom_scale_tiles.borrow_mut().drain(..) {
                tile.set_presentation_scale(1.0);
            }
            self.sectioned_folder.cancel_zoom_settle();
            set_grid_zoom_animation_active(false);
        }
    }

    /// Column count a content width produces for the current tile size.
    /// The ceiling exists so absurdly narrow tiles cannot appear, not to
    /// limit wide monitors: on 2K/4K surfaces the default 300px tiles fill
    /// 9-11 columns, and capping below that leaves dead space on the right.
    fn columns_for_width(&self, width: i32) -> u32 {
        let available = (width - 48).max(200);
        ((available as f64) / (self.tile_width.get() as f64 + 30.0))
            .floor()
            .clamp(1.0, 12.0) as u32
    }

    pub fn update_width(&self, width: i32) {
        // While a zoom animation is active, ignore transient width feedback
        // from GridView/ScrolledWindow reflow and keep using the outer gallery
        // width captured before the animation began. This prevents the column
        // calculation from chasing its own changing requisition.
        if let Some(stable_width) = self.zoom_animation_layout_width.get() {
            self.update_layout(stable_width, false);
            return;
        }
        self.update_layout(width, false);
    }

    /// Record each realized thumbnail's current on-screen width, keyed by
    /// photo id, so a later in-cell settle can start from that size after GTK
    /// has already committed the destination cell.
    pub(super) fn capture_in_cell_visual_px(&self, from_width: i32) -> std::collections::HashMap<i64, f32> {
        let from_width = from_width.max(1) as f32;
        let mut visual_px = std::collections::HashMap::new();
        for (tile, id) in self.zoom_scale_tiles.borrow().iter() {
            if tile.photo().is_some_and(|photo| photo.id() == *id) {
                visual_px.insert(*id, tile.presentation_scale() * from_width);
            }
        }
        let mut widgets = Vec::new();
        let root_widget: gtk::Widget = self.root.clone().upcast();
        collect_tiles(&root_widget, &mut widgets);
        for tile in widgets {
            if !tile.is_mapped() {
                continue;
            }
            let Some(photo) = tile.photo() else { continue };
            visual_px
                .entry(photo.id())
                .or_insert_with(|| tile.presentation_scale() * from_width);
        }
        visual_px
    }

    /// In-cell ease-in-out zoom for Library GtkGridView.
    ///
    /// GTK commits the destination tile size and column count once. Realized
    /// thumbnails then scale from (old_visual_px / new_width) → 1.0 inside
    /// their own cells. There is no position offset: interpolating allocations
    /// is what made rc7 look as if tiles were flying around.
    pub(super) fn start_grid_zoom_settle(
        self: &Rc<Self>,
        from_width: i32,
        to_width: i32,
        visual_px: std::collections::HashMap<i64, f32>,
        from_columns: u32,
        to_columns: u32,
    ) -> u64 {
        const ZOOM_IN_US: i64 = 180_000;
        const ZOOM_OUT_US: i64 = 180_000;
        // The normal in-cell thumbnail ease stays exactly as before. When a
        // column boundary is crossed, tiles that did not exist in the old
        // realized set get a much shorter presentation-only settle. This is
        // deliberately scale-only: GTK owns the destination cell positions,
        // so there is still no X/Y interpolation and therefore no flying.
        const COLUMN_SETTLE_US: i64 = 220_000;
        const COLUMN_GROW_START: f32 = 0.88;
        const COLUMN_SHRINK_START: f32 = 1.12;

        // Zoom-out starts above 1.0 and is clipped to the cell, so the same
        // duration reads faster than zoom-in; give it more time.
        let duration_us: i64 = if to_width < from_width {
            ZOOM_OUT_US
        } else {
            ZOOM_IN_US
        };
        let columns_changed = from_columns != to_columns;
        let column_start_scale = if to_columns > from_columns {
            // A newly exposed column grows into its already-final cell.
            COLUMN_GROW_START
        } else {
            // When a column disappears, destination-only tiles settle down
            // into their already-final cells instead of popping at full size.
            COLUMN_SHRINK_START
        };

        let generation = self.zoom_animation_generation.get().wrapping_add(1);
        self.zoom_animation_generation.set(generation);

        let from_width = from_width.max(1) as f32;
        let to_width = to_width.max(1) as f32;
        let identity_start = in_cell_zoom_start_scale(from_width as i32, to_width as i32);

        for (tile, _) in self.zoom_scale_tiles.borrow_mut().drain(..) {
            tile.set_presentation_scale(1.0);
        }

        if !self.zoom_animations_enabled.get() || !self.root.settings().is_gtk_enable_animations() {
            set_grid_zoom_animation_active(false);
            return generation;
        }

        let mut widgets = Vec::new();
        let root_widget: gtk::Widget = self.root.clone().upcast();
        collect_tiles(&root_widget, &mut widgets);

        let mut entries = Vec::new();
        let column_entries: Rc<RefCell<Vec<(SquareTile, i64)>>> = Rc::new(RefCell::new(Vec::new()));
        let known_ids: Rc<RefCell<HashSet<i64>>> = Rc::new(RefCell::new(HashSet::new()));
        for tile in widgets {
            if !tile.is_mapped() {
                continue;
            }
            let Some(photo) = tile.photo() else { continue };
            let photo_id = photo.id();
            known_ids.borrow_mut().insert(photo_id);

            if let Some(old_visual_px) = visual_px.get(&photo_id).copied() {
                let start = (old_visual_px / to_width).clamp(0.25, 4.0);
                if (start - 1.0).abs() < 0.001 {
                    tile.set_presentation_scale(1.0);
                    continue;
                }
                tile.set_presentation_scale(start);
                entries.push((tile, photo_id, start));
            } else if columns_changed {
                // This tile belongs to destination geometry but was not part
                // of the pre-reflow realized set. Give it the short column
                // grow/shrink settle instead of letting it pop in at 1.0.
                tile.set_presentation_scale(column_start_scale);
                column_entries.borrow_mut().push((tile, photo_id));
            } else {
                let start = identity_start.clamp(0.25, 4.0);
                if (start - 1.0).abs() < 0.001 {
                    tile.set_presentation_scale(1.0);
                    continue;
                }
                tile.set_presentation_scale(start);
                entries.push((tile, photo_id, start));
            }
        }

        self.zoom_scale_tiles.replace(
            entries
                .iter()
                .map(|(tile, id, _)| (tile.clone(), *id))
                .chain(
                    column_entries
                        .borrow()
                        .iter()
                        .map(|(tile, id)| (tile.clone(), *id)),
                )
                .collect(),
        );

        if entries.is_empty() && column_entries.borrow().is_empty() {
            set_grid_zoom_animation_active(false);
            return generation;
        }
        set_grid_zoom_animation_active(true);
        if std::env::var_os("PICASA_TRACE").is_some() {
            eprintln!(
                "PIC_ZOOM_CELL start generation={} tiles={} column_tiles={} duration_ms={} column_duration_ms={} from={} to={} columns={}=>{} scale_from={:.3} column_scale_from={:.3}",
                generation,
                entries.len(),
                column_entries.borrow().len(),
                duration_us / 1000,
                COLUMN_SETTLE_US / 1000,
                from_width,
                to_width,
                from_columns,
                to_columns,
                identity_start,
                column_start_scale,
            );
        }

        let active_tiles = self.zoom_scale_tiles.clone();
        let generation_cell = self.zoom_animation_generation.clone();
        let root_for_late_tiles = self.root.clone();
        let column_entries_for_tick = column_entries.clone();
        let known_ids_for_tick = known_ids.clone();
        let started_at = Cell::new(None);
        self.root.add_tick_callback(move |_, clock| {
            if generation_cell.get() != generation {
                return glib::ControlFlow::Break;
            }
            let start_time = match started_at.get() {
                Some(start_time) => start_time,
                None => {
                    let start_time = clock.frame_time();
                    started_at.set(Some(start_time));
                    start_time
                }
            };
            let elapsed = clock.frame_time().saturating_sub(start_time);
            let t = (elapsed as f64 / duration_us as f64).clamp(0.0, 1.0);
            let eased = ease_in_out_cubic(t);
            for (tile, id, start) in &entries {
                if tile.photo().is_some_and(|photo| photo.id() == *id) {
                    let scale = *start + (1.0 - *start) * eased as f32;
                    tile.set_presentation_scale(scale);
                }
            }

            if columns_changed {
                let column_t =
                    (elapsed as f64 / COLUMN_SETTLE_US as f64).clamp(0.0, 1.0);
                let column_eased = ease_in_out_cubic(column_t) as f32;
                let column_scale =
                    column_start_scale + (1.0 - column_start_scale) * column_eased;

                // GtkGridView may realize the new edge column one frame after
                // the destination column count is committed. Pick those tiles
                // up while the short settle is active so they never get a
                // one-frame full-size pop before joining the transition.
                if column_t < 1.0 {
                    let mut late_widgets = Vec::new();
                    let root_widget: gtk::Widget = root_for_late_tiles.clone().upcast();
                    collect_tiles(&root_widget, &mut late_widgets);
                    for tile in late_widgets {
                        if !tile.is_mapped() {
                            continue;
                        }
                        let Some(photo) = tile.photo() else { continue };
                        let photo_id = photo.id();
                        if known_ids_for_tick.borrow_mut().insert(photo_id) {
                            tile.set_presentation_scale(column_scale);
                            active_tiles.borrow_mut().push((tile.clone(), photo_id));
                            column_entries_for_tick.borrow_mut().push((tile, photo_id));
                        }
                    }
                }

                for (tile, id) in column_entries_for_tick.borrow().iter() {
                    if tile.photo().is_some_and(|photo| photo.id() == *id) {
                        tile.set_presentation_scale(column_scale);
                    }
                }
            }

            let finished = elapsed >= if columns_changed {
                duration_us.max(COLUMN_SETTLE_US)
            } else {
                duration_us
            };
            if finished {
                for (tile, id, _) in &entries {
                    if tile.photo().is_some_and(|photo| photo.id() == *id) {
                        tile.set_presentation_scale(1.0);
                    }
                }
                for (tile, id) in column_entries_for_tick.borrow().iter() {
                    if tile.photo().is_some_and(|photo| photo.id() == *id) {
                        tile.set_presentation_scale(1.0);
                    }
                }
                active_tiles.borrow_mut().clear();
                set_grid_zoom_animation_active(false);
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
        generation
    }

    /// Presentation-only FLIP reflow for live application resizing.
    ///
    /// GTK computes and owns the real destination layout immediately. We only
    /// translate snapshots of realized tiles from their previous visual
    /// positions back to their new allocations. No synthetic width, tile size,
    /// model membership or GridView column input is introduced.
    pub fn cancel_resize_flip(&self) {
        self.resize_flip_generation
            .set(self.resize_flip_generation.get().wrapping_add(1));

        let root_widget: gtk::Widget = self.root.clone().upcast();
        let mut tiles = Vec::new();
        collect_tiles(&root_widget, &mut tiles);
        for tile in tiles {
            tile.set_presentation_offset(0.0, 0.0);
        }
        set_grid_zoom_animation_active(false);
    }

    pub fn update_width_with_flip(self: &Rc<Self>, width: i32) {
        // No presentation animation on resize. GTK owns the real destination
        // layout and we publish it immediately. Smooth wheel/touchpad scrolling
        // is unrelated and remains enabled elsewhere.
        self.cancel_resize_flip();
        self.update_width(width);
    }

    fn update_layout(&self, width: i32, tile_size_changed: bool) {
        let trace_started = std::env::var_os("PICASA_TRACE").is_some().then(std::time::Instant::now);
        // First real allocation with no stored thumbnail preference: adopt
        // the ~4-thumbnails-per-row default for this surface width. Session
        // only - it becomes a preference if the user zooms manually.
        if self.auto_default_zoom.get() && width > 0 {
            self.auto_default_zoom.set(false);
            let target = zoom_level_for_four_columns(width);
            if target != self.tile_width.get() {
                self.apply_tile_size(target, false);
                return;
            }
        }
        let old_columns = self.current_columns.get();
        let old_width = self.last_layout_width.get();
        let columns = self.columns_for_width(width);
        if width == old_width && columns == old_columns && !tile_size_changed {
            return;
        }
        self.last_layout_width.set(width);
        let folder_mode = self.group_mode.get() == GroupMode::Folder;
        let sectioned_folder_mode =
            folder_mode && crate::grid::sectioned_folder_view_enabled();
        let folder_list_mode =
            folder_mode && !sectioned_folder_mode && !crate::grid::folder_gridview_experiment_enabled();
        let sectioned_resize_anchor = if sectioned_folder_mode && !tile_size_changed {
            self.sectioned_folder.capture_center_anchor()
        } else {
            None
        };
        let sectioned_reflow_snapshot = if sectioned_folder_mode && !tile_size_changed {
            Some(self.sectioned_folder.capture_reflow_snapshot())
        } else {
            None
        };
        if columns == old_columns {
            // A width-only window resize does not need an explicit GridView
            // relayout when the column count is unchanged. GTK is already
            // allocating the widget for the new parent width. Calling
            // queue_resize() on every drag frame creates unnecessary layout
            // churn and can amplify the window's minimum-width negotiation.
            if !tile_size_changed && !folder_list_mode && !sectioned_folder_mode {
                return;
            }

            // Zooming within the same column count only changes tile geometry.
            // Replacing the Folder ListStore here used to invalidate every
            // realized row and cost ~0.8-1.1s for a 4.5k-photo library.
            if sectioned_folder_mode {
                if !tile_size_changed {
                    // Same columns + width-only resize: no vertical geometry
                    // changes. The sectioned surface refreshes header width from
                    // its own allocation tick, so do not re-anchor/rebuild here.
                    if let Some(started) = trace_started {
                        eprintln!(
                            "PIC_ZOOM layout mode=folder_sectioned action=width_only columns={} width={} elapsed_us={}",
                            columns,
                            width,
                            started.elapsed().as_micros()
                        );
                    }
                }
                // Tile-size zoom is completed by apply_tile_geometry(), which
                // already owns the centre anchor and performs exactly one
                // section refresh after this layout bookkeeping.
            } else if folder_list_mode && tile_size_changed {
                // Tile size changed within the same columns: the rows keep
                // their photos but their heights change, so re-anchor the
                // viewport to the photo that was at the top.
                let anchor = self.take_reframe_anchor();
                update_folder_realized_rows(
                    self.folder_root.upcast_ref(),
                    self.tile_width.get(),
                    self.tile_height.get(),
                    self.show_file_names.get(),
                );
                self.folder_root.queue_resize();
                if let Some(anchor) = anchor {
                    self.scroll_folder_to_photo(anchor);
                }
                if let Some(started) = trace_started {
                    eprintln!("PIC_ZOOM layout mode=folder action=resize_rows columns={} width={} elapsed_us={}", columns, width, started.elapsed().as_micros());
                }
            } else if !folder_list_mode {
                self.root.queue_resize();
                self.update_group_header_for_scroll(self.last_scroll_y.get());
                if let Some(started) = trace_started {
                    eprintln!("PIC_ZOOM layout mode=grid action=resize_tiles columns={} width={} elapsed_us={}", columns, width, started.elapsed().as_micros());
                }
            }
            // Width-only changes with the same columns need no vertical layout
            // work. GTK stretches the row boxes itself. Re-anchoring here made
            // every sidebar animation frame issue competing scroll requests.
            return;
        }

        let previous_columns = self.current_columns.replace(columns);
        if std::env::var_os("PICASA_TRACE").is_some() && previous_columns != columns { eprintln!("PIC_NAV current_columns_changed old={} new={}", previous_columns, columns); }
        self.root.set_min_columns(columns);
        self.root.set_max_columns(columns);
        self.root.queue_resize();
        if sectioned_folder_mode {
            if !tile_size_changed {
                if let Some(snapshot) = sectioned_reflow_snapshot {
                    self.sectioned_folder
                        .animate_reflow(snapshot, sectioned_resize_anchor);
                } else {
                    self.sectioned_folder.invalidate_geometry();
                    self.sectioned_folder.refresh();
                }
            }
            // During Ctrl+wheel, apply_tile_geometry() owns the tile-size
            // animation. Here we publish the new column count only once.
            if let Some(started) = trace_started {
                eprintln!(
                    "PIC_ZOOM layout mode=folder_sectioned action=columns_changed old_columns={} columns={} width={} tile_size_changed={} elapsed_us={}",
                    old_columns,
                    columns,
                    width,
                    tile_size_changed,
                    started.elapsed().as_micros()
                );
            }
        } else if folder_list_mode {
            // Each model item is one visual photo line. A column change must
            // rebuild those lines to keep the layout gapless.
            let anchor = self.take_reframe_anchor();
            self.rebuild_folder_rows();
            if let Some(anchor) = anchor {
                self.scroll_folder_to_photo(anchor);
            }
            if let Some(started) = trace_started {
                eprintln!("PIC_ZOOM layout mode=folder action=rebuild_rows old_columns={} columns={} width={} elapsed_us={}", old_columns, columns, width, started.elapsed().as_micros());
            }
        } else {
            self.update_group_header_for_scroll(self.last_scroll_y.get());
            if let Some(started) = trace_started {
                eprintln!("PIC_ZOOM layout mode=grid action=columns_changed old_columns={} columns={} width={} elapsed_us={}", old_columns, columns, width, started.elapsed().as_micros());
            }
        }
    }

}
