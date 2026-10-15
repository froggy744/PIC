use std::cell::{Cell, RefCell};

use glib::subclass::prelude::*;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk4 as gtk;

mod imp {
    use super::*;

    pub struct OpeningTransitionImage {
        pub frame_x: Cell<f64>,
        pub frame_y: Cell<f64>,
        pub frame_width: Cell<i32>,
        pub frame_height: Cell<i32>,
        pub clip_x: Cell<f64>,
        pub clip_y: Cell<f64>,
        pub clip_width: Cell<f64>,
        pub clip_height: Cell<f64>,
        pub content_fit: Cell<gtk::ContentFit>,
        pub paintable: RefCell<Option<gtk::gdk::Paintable>>,
        pub handoff_from: RefCell<Option<gtk::gdk::Paintable>>,
        pub handoff_to: RefCell<Option<gtk::gdk::Paintable>>,
        pub handoff_progress: Cell<f64>,
        pub snapshot_count: Cell<u64>,
    }

    impl Default for OpeningTransitionImage {
        fn default() -> Self {
            Self {
                frame_x: Cell::new(0.0),
                frame_y: Cell::new(0.0),
                frame_width: Cell::new(1),
                frame_height: Cell::new(1),
                clip_x: Cell::new(0.0),
                clip_y: Cell::new(0.0),
                clip_width: Cell::new(1.0),
                clip_height: Cell::new(1.0),
                content_fit: Cell::new(gtk::ContentFit::Contain),
                paintable: RefCell::new(None),
                handoff_from: RefCell::new(None),
                handoff_to: RefCell::new(None),
                handoff_progress: Cell::new(0.0),
                snapshot_count: Cell::new(0),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for OpeningTransitionImage {
        const NAME: &'static str = "PicOpeningTransitionImage";
        type Type = super::OpeningTransitionImage;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for OpeningTransitionImage {}

    impl WidgetImpl for OpeningTransitionImage {
        fn measure(
            &self,
            orientation: gtk::Orientation,
            _for_size: i32,
        ) -> (i32, i32, i32, i32) {
            let extent = match orientation {
                gtk::Orientation::Horizontal => self.frame_width.get(),
                gtk::Orientation::Vertical => self.frame_height.get(),
                _ => 1,
            }
            .max(1);
            (extent, extent, -1, -1)
        }

        #[allow(deprecated)]
        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let Some(paintable) = self.paintable.borrow().as_ref().cloned() else {
                return;
            };
            let snapshot_count = self.snapshot_count.get().wrapping_add(1);
            self.snapshot_count.set(snapshot_count);
            if std::env::var_os("PIC_LIGHTBOX_OPEN_TRACE").is_some() {
                let widget = self.obj();
                let (frame_time_us, frame_counter) = widget
                    .frame_clock()
                    .map(|clock| (clock.frame_time(), clock.frame_counter()))
                    .unwrap_or((0, 0));
                eprintln!(
                    "PIC_LIGHTBOX_OPEN_TRACE gtk_widget_snapshot name={} count={} frame={} frame_time_us={} mapped={} visible={} allocation={}x{} drawn_at={:.1},{:.1} requested_frame={}x{} clip={:.1},{:.1} {:.1}x{:.1} opacity={:.3} paintable={}x{}",
                    widget.widget_name(),
                    snapshot_count,
                    frame_counter,
                    frame_time_us,
                    widget.is_mapped(),
                    widget.is_visible(),
                    widget.width(),
                    widget.height(),
                    self.frame_x.get(),
                    self.frame_y.get(),
                    self.frame_width.get(),
                    self.frame_height.get(),
                    self.clip_x.get(),
                    self.clip_y.get(),
                    self.clip_width.get(),
                    self.clip_height.get(),
                    widget.opacity(),
                    paintable.intrinsic_width(),
                    paintable.intrinsic_height(),
                );
            }
            let width = self.frame_width.get().max(1) as f32;
            let height = self.frame_height.get().max(1) as f32;
            let x = self.frame_x.get() as f32;
            let y = self.frame_y.get() as f32;

            let clip = gtk::graphene::Rect::new(
                self.clip_x.get() as f32,
                self.clip_y.get() as f32,
                self.clip_width.get().max(0.0) as f32,
                self.clip_height.get().max(0.0) as f32,
            );
            snapshot.push_clip(&clip);
            let widget = self.obj();
            snapshot.render_background(
                &widget.style_context(),
                x as f64,
                y as f64,
                width as f64,
                height as f64,
            );
            if let (Some(from), Some(to)) = (
                self.handoff_from.borrow().as_ref().cloned(),
                self.handoff_to.borrow().as_ref().cloned(),
            ) {
                if let (Some(start), Some(end)) = (
                    transition_paintable_node(
                        &from,
                        x,
                        y,
                        width,
                        height,
                        self.content_fit.get(),
                    ),
                    transition_paintable_node(
                        &to,
                        x,
                        y,
                        width,
                        height,
                        self.content_fit.get(),
                    ),
                ) {
                    let cross_fade = gtk::gsk::CrossFadeNode::new(
                        start,
                        end,
                        self.handoff_progress.get().clamp(0.0, 1.0) as f32,
                    );
                    snapshot.append_node(&cross_fade);
                } else {
                    append_transition_paintable(
                        snapshot,
                        &paintable,
                        x,
                        y,
                        width,
                        height,
                        self.content_fit.get(),
                    );
                }
            } else {
                append_transition_paintable(
                    snapshot,
                    &paintable,
                    x,
                    y,
                    width,
                    height,
                    self.content_fit.get(),
                );
            }
            snapshot.pop();
        }
    }
}

fn append_transition_paintable(
    snapshot: &gtk::Snapshot,
    paintable: &gtk::gdk::Paintable,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    fit: gtk::ContentFit,
) {
    let intrinsic_width = paintable.intrinsic_width().max(1) as f32;
    let intrinsic_height = paintable.intrinsic_height().max(1) as f32;
    let scale = match fit {
        gtk::ContentFit::Cover => (width / intrinsic_width).max(height / intrinsic_height),
        gtk::ContentFit::ScaleDown => (width / intrinsic_width)
            .min(height / intrinsic_height)
            .min(1.0),
        gtk::ContentFit::Contain => (width / intrinsic_width).min(height / intrinsic_height),
        gtk::ContentFit::Fill => 1.0,
        _ => (width / intrinsic_width).min(height / intrinsic_height),
    };
    let drawn_width = if fit == gtk::ContentFit::Fill {
        width
    } else {
        intrinsic_width * scale
    };
    let drawn_height = if fit == gtk::ContentFit::Fill {
        height
    } else {
        intrinsic_height * scale
    };
    snapshot.save();
    snapshot.translate(&gtk::graphene::Point::new(
        x + (width - drawn_width) * 0.5,
        y + (height - drawn_height) * 0.5,
    ));
    snapshot.scale(
        drawn_width / intrinsic_width,
        drawn_height / intrinsic_height,
    );
    paintable.snapshot(snapshot, intrinsic_width as f64, intrinsic_height as f64);
    snapshot.restore();
}

fn transition_paintable_node(
    paintable: &gtk::gdk::Paintable,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    fit: gtk::ContentFit,
) -> Option<gtk::gsk::RenderNode> {
    let snapshot = gtk::Snapshot::new();
    append_transition_paintable(&snapshot, paintable, x, y, width, height, fit);
    snapshot.to_node()
}

glib::wrapper! {
    pub struct OpeningTransitionImage(ObjectSubclass<imp::OpeningTransitionImage>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl OpeningTransitionImage {
    pub fn new(
        paintable: gtk::gdk::Paintable,
        content_fit: gtk::ContentFit,
        width: i32,
        height: i32,
    ) -> Self {
        let image: Self = glib::Object::new();
        image.imp().paintable.replace(Some(paintable));
        image.imp().content_fit.set(content_fit);
        image.set_geometry(
            0.0,
            0.0,
            width,
            height,
            0.0,
            0.0,
            width as f64,
            height as f64,
        );
        image
    }

    pub fn set_geometry(
        &self,
        x: f64,
        y: f64,
        width: i32,
        height: i32,
        clip_x: f64,
        clip_y: f64,
        clip_width: f64,
        clip_height: f64,
    ) {
        let imp = self.imp();
        let width = width.max(1);
        let height = height.max(1);
        let x_changed = imp.frame_x.replace(x) != x;
        let y_changed = imp.frame_y.replace(y) != y;
        let width_changed = imp.frame_width.replace(width) != width;
        let height_changed = imp.frame_height.replace(height) != height;
        let clip_x_changed = imp.clip_x.replace(clip_x) != clip_x;
        let clip_y_changed = imp.clip_y.replace(clip_y) != clip_y;
        let clip_width_changed = imp.clip_width.replace(clip_width) != clip_width;
        let clip_height_changed = imp.clip_height.replace(clip_height) != clip_height;
        let changed = x_changed
            || y_changed
            || width_changed
            || height_changed
            || clip_x_changed
            || clip_y_changed
            || clip_width_changed
            || clip_height_changed;
        if changed {
            self.queue_draw();
        }
    }

    pub fn begin_handoff(&self, paintable: gtk::gdk::Paintable) -> bool {
        let imp = self.imp();
        let Some(current) = imp.paintable.borrow().as_ref().cloned() else {
            imp.paintable.replace(Some(paintable));
            self.queue_draw();
            return false;
        };
        if current.as_ptr() == paintable.as_ptr() {
            return false;
        }
        imp.handoff_from.replace(Some(current));
        imp.handoff_to.replace(Some(paintable));
        imp.handoff_progress.set(0.0);
        self.queue_draw();
        true
    }

    pub fn set_handoff_progress(&self, progress: f64) {
        let imp = self.imp();
        let progress = progress.clamp(0.0, 1.0);
        imp.handoff_progress.set(progress);
        if progress >= 1.0 {
            if let Some(to) = imp.handoff_to.borrow_mut().take() {
                imp.paintable.replace(Some(to));
            }
            imp.handoff_from.borrow_mut().take();
        }
        self.queue_draw();
    }

    pub fn handoff_complete(&self) -> bool {
        self.imp().handoff_to.borrow().is_none()
    }

    pub fn current_paintable(&self) -> Option<gtk::gdk::Paintable> {
        self.imp().paintable.borrow().as_ref().cloned()
    }

    pub fn snapshot_count(&self) -> u64 {
        self.imp().snapshot_count.get()
    }

    pub fn requested_frame_size(&self) -> (i32, i32) {
        (self.imp().frame_width.get(), self.imp().frame_height.get())
    }
}
