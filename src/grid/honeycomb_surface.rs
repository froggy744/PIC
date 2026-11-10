//! Width-constrained clipping around the native Fixed photo surface.
//! GtkFixed keeps its own layout manager and all existing child transforms.
use glib::subclass::prelude::*;
use gtk4::{self as gtk, prelude::*, subclass::prelude::*};
use std::cell::RefCell;
mod imp {
    use super::*;
    #[derive(Default)]
    pub struct Clip {
        pub child: RefCell<Option<gtk::Fixed>>,
    }
    #[glib::object_subclass]
    impl ObjectSubclass for Clip {
        const NAME: &'static str = "PicHoneycombClip";
        type Type = super::Clip;
        type ParentType = gtk::Widget;
    }
    impl ObjectImpl for Clip {
        fn dispose(&self) {
            if let Some(child) = self.child.take() {
                child.unparent();
            }
        }
    }
    impl WidgetImpl for Clip {
        fn measure(&self, orientation: gtk::Orientation, for_size: i32) -> (i32, i32, i32, i32) {
            if orientation == gtk::Orientation::Horizontal {
                (0, 0, -1, -1)
            } else {
                self.child
                    .borrow()
                    .as_ref()
                    .map(|child| child.measure(orientation, for_size))
                    .unwrap_or((0, 0, -1, -1))
            }
        }
        fn size_allocate(&self, width: i32, height: i32, baseline: i32) {
            if let Some(child) = self.child.borrow().as_ref() {
                child.allocate(width, height, baseline, None);
            }
        }
        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            if let Some(child) = self.child.borrow().as_ref() {
                self.obj().snapshot_child(child, snapshot);
            }
        }
    }
}
glib::wrapper! {
    pub struct Clip(ObjectSubclass<imp::Clip>) @extends gtk::Widget,
        @implements gtk::Accessible,gtk::Buildable,gtk::ConstraintTarget;
}
impl Clip {
    pub(super) fn new(child: &gtk::Fixed) -> Self {
        let clip: Self = glib::Object::new();
        clip.set_overflow(gtk::Overflow::Hidden);
        clip.set_hexpand(true);
        clip.set_vexpand(true);
        child.set_parent(&clip);
        clip.imp().child.replace(Some(child.clone()));
        clip
    }
    pub(super) fn release(&self) {
        if let Some(child) = self.imp().child.take() {
            child.unparent();
        }
    }
}
