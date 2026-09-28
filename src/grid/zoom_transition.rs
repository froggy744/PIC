//! Presentation only: the child keeps its normal allocation throughout zoom.
use super::*;
use gtk::subclass::prelude::*;

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct ZoomSurface {
        pub phase: Cell<Option<(bool, f64, f64)>>,
        pub last: RefCell<Option<gtk::gsk::RenderNode>>,
        pub old: RefCell<Option<gtk::gsk::RenderNode>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ZoomSurface {
        const NAME: &'static str = "PicZoomSurface";
        type Type = super::ZoomSurface;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for ZoomSurface {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().set_layout_manager(Some(gtk::BinLayout::new()));
            self.obj().set_overflow(gtk::Overflow::Hidden);
        }

        fn dispose(&self) {
            self.last.take();
            self.old.take();
            while let Some(child) = self.obj().first_child() {
                child.unparent();
            }
        }
    }

    impl WidgetImpl for ZoomSurface {
        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let widget = self.obj();
            let Some(child) = widget.first_child() else {
                return;
            };
            let child_snapshot = gtk::Snapshot::new();
            widget.snapshot_child(&child, &child_snapshot);
            let node = child_snapshot.to_node();
            self.last.replace(node.clone());
            let center = gtk::graphene::Point::new(
                widget.width() as f32 / 2.0,
                widget.height() as f32 / 2.0,
            );
            let draw = |node: &gtk::gsk::RenderNode, scale: f64, opacity: f64| {
                snapshot.save();
                snapshot.translate(&center);
                snapshot.scale(scale as f32, scale as f32);
                snapshot.translate(&gtk::graphene::Point::new(-center.x(), -center.y()));
                snapshot.push_opacity(opacity);
                snapshot.append_node(node);
                snapshot.pop();
                snapshot.restore();
            };
            match self.phase.get() {
                Some((false, direction, t)) => {
                    if let Some(node) = node.as_ref() {
                        draw(node, 1.0 + direction * 0.04 * t, 1.0 - 0.25 * t);
                    }
                }
                Some((true, direction, t)) => {
                    // A GSK crossfade blends both nodes as a single image; it
                    // avoids the brightness dip of two source-over fades.
                    snapshot.push_cross_fade(t);
                    if let Some(old) = self.old.borrow().as_ref() {
                        draw(old, 1.0 + direction * 0.04, 0.75);
                    }
                    snapshot.pop();
                    if let Some(node) = node.as_ref() {
                        draw(node, 1.0 - direction * 0.04 * (1.0 - t), 0.75 + 0.25 * t);
                    }
                    snapshot.pop();
                }
                None => {
                    if let Some(node) = node.as_ref() {
                        snapshot.append_node(node);
                    }
                }
            }
        }
    }
}

glib::wrapper! {
    pub struct ZoomSurface(ObjectSubclass<imp::ZoomSurface>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl ZoomSurface {
    pub fn new(child: &impl IsA<gtk::Widget>) -> Self {
        let surface: Self = glib::Object::new();
        child.set_parent(&surface);
        surface.set_hexpand(true);
        surface.set_vexpand(true);
        surface
    }

    pub fn frame(&self, post: bool, direction: f64, progress: f64) {
        self.imp()
            .phase
            .set(Some((post, direction, ease_in_out_cubic(progress))));
        self.queue_draw();
    }

    pub fn freeze(&self) {
        self.imp().old.replace(self.imp().last.borrow().clone());
    }

    pub fn reset(&self) {
        self.imp().phase.set(None);
        self.imp().old.take();
        self.queue_draw();
    }
}

pub(super) fn ease_in_out_cubic(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    if t < 0.5 {
        4.0 * t.powi(3)
    } else {
        1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zoom_transition_curve_is_bounded_ease_in_out() {
        assert_eq!(ease_in_out_cubic(0.0), 0.0);
        assert_eq!(ease_in_out_cubic(0.25), 0.0625);
        assert_eq!(ease_in_out_cubic(0.5), 0.5);
        assert_eq!(ease_in_out_cubic(0.75), 0.9375);
        assert_eq!(ease_in_out_cubic(1.0), 1.0);
        assert_eq!(ease_in_out_cubic(2.0), 1.0);
    }

    #[test]
    #[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
    fn zoom_transition_commits_once_and_preserves_model_and_selection() {
        fn settle(ms: u64) {
            let context = glib::MainContext::default();
            let until = Instant::now() + std::time::Duration::from_millis(ms);
            while Instant::now() < until {
                while context.pending() {
                    context.iteration(false);
                }
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
        }
        gtk::init().unwrap();
        let db = rusqlite::Connection::open_in_memory().unwrap();
        db.execute_batch(crate::db::SCHEMA).unwrap();
        db.execute_batch(
            "INSERT INTO folders(id,path,name) VALUES(1,'/zoom','zoom');
            WITH RECURSIVE ids(n) AS (SELECT 1 UNION ALL SELECT n+1 FROM ids WHERE n<200)
            INSERT INTO photos(id,path,folder_id) SELECT n, '/zoom/' || n || '.jpg', 1 FROM ids;",
        )
        .unwrap();
        let photos = crate::db::photos(&db, None, false, None).unwrap();
        for folder in [false, true] {
            let commits = Rc::new(RefCell::new(Vec::new()));
            let recorded = commits.clone();
            let gallery = Rc::new(Gallery::new(
                &photos,
                120,
                |_| {},
                |_, _, _| {},
                |_, _, _, _| {},
                |_, _| {},
                move |width| recorded.borrow_mut().push(width),
            ));
            let scroll = gtk::ScrolledWindow::new();
            if folder {
                gallery.set_grouping(GroupMode::Folder, GroupDate::Taken);
                gallery.replace(&photos);
                scroll.set_child(Some(&gallery.folder_sectioned_root));
                gallery.attach_sectioned_folder_scroll(&scroll);
            } else {
                scroll.set_child(Some(&gallery.root));
            }
            let surface = gallery.wrap_zoom_surface(&scroll);
            let window = gtk::Window::builder()
                .default_width(850)
                .default_height(500)
                .child(&surface)
                .build();
            let settings = window.settings();
            let enabled = settings.is_gtk_enable_animations();
            settings.set_gtk_enable_animations(true);
            window.present();
            settle(150);
            gallery.update_width(850);
            settle(50);
            scroll.vadjustment().set_value(200.0);
            settle(50);
            assert!(scroll.vadjustment().value() > 0.0);
            gallery.selection.select_item(3, true);
            let column_changes = Rc::new(Cell::new(0));
            let observed_columns = column_changes.clone();
            gallery.root.connect_min_columns_notify(move |_| {
                observed_columns.set(observed_columns.get() + 1)
            });
            let objects = gallery.photo_objects();
            let selected = gallery.selected_photo_ids(None);
            let edits = Rc::new(Cell::new(0));
            let edits_for_signal = edits.clone();
            gallery.store.connect_items_changed(move |_, _, _, _| {
                edits_for_signal.set(edits_for_signal.get() + 1)
            });
            let columns = gallery.current_columns.get();
            gallery.zoom_in();
            settle(25);
            assert_eq!(gallery.tile_width.get(), 120);
            assert_eq!(gallery.current_columns.get(), columns);
            assert!(commits.borrow().is_empty());
            gallery.wheel_zoom_in();
            gallery.wheel_zoom_in();
            assert_eq!(gallery.current_zoom_width(), 192);
            settle(300);
            assert_eq!(&*commits.borrow(), &[192]);
            assert_eq!(gallery.tile_width.get(), 192);
            assert_eq!(
                column_changes.get(),
                1,
                "only the final column count is published"
            );
            assert!(
                scroll.vadjustment().value() > 0.0,
                "zoom must not reset scrolling to the top"
            );
            assert_eq!(gallery.photo_objects(), objects);
            assert_eq!(gallery.selected_photo_ids(None), selected);
            assert_eq!(edits.get(), 0, "zoom must not splice the model");
            assert!(surface.imp().phase.get().is_none());
            assert!(surface.imp().old.borrow().is_none());
            assert!(gallery.zoom_tick.borrow().is_none());
            // Post-commit interruption: newest request wins, with no queued animation.
            gallery.zoom_out();
            settle(120);
            gallery.zoom_in();
            settle(300);
            assert_eq!(gallery.tile_width.get(), 192);
            assert!(surface.imp().phase.get().is_none());
            assert_eq!(edits.get(), 0);
            // A navigation/model refresh cancels a pending target immediately.
            gallery.zoom_out();
            gallery.replace(&photos);
            assert!(surface.imp().phase.get().is_none());
            assert!(gallery.pending_zoom_width.get().is_none());
            settle(250);
            assert_eq!(gallery.tile_width.get(), 192);
            // Reduced motion commits synchronously and never leaves a transform.
            settings.set_gtk_enable_animations(false);
            gallery.zoom_out();
            assert_eq!(gallery.tile_width.get(), 168);
            assert!(gallery.zoom_tick.borrow().is_none());
            settings.set_gtk_enable_animations(true);
            gallery.zoom_in();
            window.hide();
            assert!(gallery.zoom_tick.borrow().is_none());
            assert!(surface.imp().phase.get().is_none());
            settings.set_gtk_enable_animations(enabled);
            window.close();
        }
    }
}
