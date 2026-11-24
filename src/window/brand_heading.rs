//! The "PIC" header wordmark: the heading text followed by the round dot
//! from the logo artwork (logo/logo-long-SVG.svg).
//!
//! The dot is a CSS-styled widget rather than a font glyph so it stays round
//! and themes can restyle it through `.brand-dot` (base.css sets the default
//! size and the logo blue). This container places the dot itself on the text
//! baseline of "PIC", where the logo puts it. It does not use GtkBox baseline
//! alignment: header bars hand their title an outside baseline, and GtkBox
//! then squeezes the label and misplaces the dot (seen in the compact
//! sidebar header).
use gtk4::{self as gtk, prelude::*, subclass::prelude::*};
use std::cell::RefCell;

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct BrandHeading {
        pub word: RefCell<Option<gtk::Label>>,
        pub dot: RefCell<Option<gtk::Box>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for BrandHeading {
        const NAME: &'static str = "PicBrandHeading";
        type Type = super::BrandHeading;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for BrandHeading {
        fn dispose(&self) {
            if let Some(word) = self.word.take() {
                word.unparent();
            }
            if let Some(dot) = self.dot.take() {
                dot.unparent();
            }
        }
    }

    impl WidgetImpl for BrandHeading {
        fn measure(&self, orientation: gtk::Orientation, _for_size: i32) -> (i32, i32, i32, i32) {
            let (Some(word), Some(dot)) = (self.word.borrow().clone(), self.dot.borrow().clone())
            else {
                return (0, 0, -1, -1);
            };
            let (word_min, word_nat, word_min_base, word_nat_base) = word.measure(orientation, -1);
            let (dot_min, dot_nat, _, _) = dot.measure(orientation, -1);
            if orientation == gtk::Orientation::Horizontal {
                (word_min + dot_min, word_nat + dot_nat, -1, -1)
            } else {
                // The dot hangs above the baseline, inside the label's height.
                (
                    word_min.max(dot_min),
                    word_nat.max(dot_nat),
                    word_min_base,
                    word_nat_base,
                )
            }
        }

        fn size_allocate(&self, width: i32, height: i32, _baseline: i32) {
            let (Some(word), Some(dot)) = (self.word.borrow().clone(), self.dot.borrow().clone())
            else {
                return;
            };
            let (_, dot_width, _, _) = dot.measure(gtk::Orientation::Horizontal, -1);
            let (_, dot_height, _, _) = dot.measure(gtk::Orientation::Vertical, -1);
            let word_width = (width - dot_width).max(0);
            let (_, word_height, _, word_baseline) =
                word.measure(gtk::Orientation::Vertical, word_width);
            // The label keeps its natural height, centred in the slot.
            let word_y = ((height - word_height) / 2).max(0);
            word.size_allocate(
                &gtk::Allocation::new(0, word_y, word_width, word_height),
                word_baseline,
            );
            let text_baseline = word_y
                + if word_baseline >= 0 {
                    word_baseline
                } else {
                    word_height
                };
            dot.size_allocate(
                &gtk::Allocation::new(
                    word_width,
                    text_baseline - dot_height,
                    dot_width,
                    dot_height,
                ),
                -1,
            );
        }
    }
}

glib::wrapper! {
    pub struct BrandHeading(ObjectSubclass<imp::BrandHeading>) @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

/// Builds the wordmark used as the header title.
pub(super) fn brand_heading() -> BrandHeading {
    let heading: BrandHeading = glib::Object::new();
    heading.add_css_class("brand-heading");
    let word = gtk::Label::new(Some("PIC"));
    word.add_css_class("heading");
    word.set_parent(&heading);
    let dot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    dot.add_css_class("brand-dot");
    dot.set_can_target(false);
    dot.set_accessible_role(gtk::AccessibleRole::Presentation);
    dot.set_parent(&heading);
    heading.imp().word.replace(Some(word));
    heading.imp().dot.replace(Some(dot));
    heading
}

#[cfg(test)]
mod tests {
    use super::*;
    use libadwaita as adw;

    #[test]
    #[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
    fn brand_dot_sits_on_the_text_baseline_in_both_header_titles() {
        gtk::init().unwrap();
        adw::init().unwrap();
        let provider = gtk::CssProvider::new();
        provider.load_from_string(crate::css::BASE);
        gtk::style_context_add_provider_for_display(
            &gtk::gdk::Display::default().unwrap(),
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
        // The two real hosts: the regular header title and the compact
        // sidebar header (46 px tall, title expanded and centred).
        let regular = brand_heading();
        let regular_header = adw::HeaderBar::new();
        regular_header.set_title_widget(Some(&regular));
        let compact = brand_heading();
        compact.set_hexpand(true);
        compact.set_halign(gtk::Align::Center);
        let compact_header = adw::HeaderBar::new();
        compact_header.set_show_start_title_buttons(false);
        compact_header.set_show_end_title_buttons(false);
        compact_header.set_height_request(46);
        compact_header.pack_start(&gtk::Button::from_icon_name("pan-end-symbolic"));
        compact_header.set_title_widget(Some(&compact));
        let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
        column.append(&regular_header);
        column.append(&compact_header);
        let window = gtk::Window::new();
        window.set_default_size(120, 200);
        window.set_child(Some(&column));
        window.present();
        let context = glib::MainContext::default();
        while compact.width() == 0 || regular.width() == 0 {
            context.iteration(true);
        }
        for _ in 0..10 {
            context.iteration(false);
        }

        for heading in [&regular, &compact] {
            let word = heading.imp().word.borrow().clone().unwrap();
            let dot = heading.imp().dot.borrow().clone().unwrap();
            let (_, word_nat_height, _, word_nat_baseline) =
                word.measure(gtk::Orientation::Vertical, word.width());
            assert!(word_nat_baseline > 0, "label reports a baseline");
            assert_eq!(word.height(), word_nat_height, "label is never squeezed");
            let word_bounds = word.compute_bounds(heading).unwrap();
            let dot_bounds = dot.compute_bounds(heading).unwrap();
            let text_baseline = word_bounds.y() + word_nat_baseline as f32;
            let dot_bottom = dot_bounds.y() + dot_bounds.height();
            assert!(
                (dot_bottom - text_baseline).abs() <= 0.5,
                "dot bottom {dot_bottom} must sit on the text baseline {text_baseline}"
            );
            assert!(dot_bounds.width() >= 3.0 && dot_bounds.height() >= 3.0, "dot is visible");
            assert!(
                dot_bounds.x() >= word_bounds.x() + word_bounds.width(),
                "dot follows the text"
            );
            assert!(
                dot_bounds.x() <= word_bounds.x() + word_bounds.width() + 4.0,
                "dot stays next to the text"
            );
        }
        window.close();
    }
}
