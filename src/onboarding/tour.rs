use adw::prelude::*;
use gtk4 as gtk;
use libadwaita as adw;

const STEPS: [(&str, &str); 4] = [
    ("Browse", "Double-click a photo to open it."),
    ("Albums", "Create an album, then add your selected photos."),
    (
        "Edit",
        "Choose Edit to adjust a photo, then export your result.",
    ),
    ("Collage", "Select several photos to create a collage."),
];

const SCREENSHOTS: [(&str, &[u8]); 4] = [
    (
        "wiz-browse.jpg",
        include_bytes!("../../screenshots/wiz-browse.jpg"),
    ),
    (
        "wiz-albums.jpg",
        include_bytes!("../../screenshots/wiz-albums.jpg"),
    ),
    (
        "wiz-edit.jpg",
        include_bytes!("../../screenshots/wiz-edit.jpg"),
    ),
    (
        "wiz-collage.jpg",
        include_bytes!("../../screenshots/wiz-collage.jpg"),
    ),
];

fn screenshot_texture(index: usize) -> gtk::gdk::Texture {
    let (filename, bundled) = SCREENSHOTS[index];
    let mut candidates = vec![std::path::PathBuf::from("screenshots").join(filename)];
    if let Ok(executable) = std::env::current_exe() {
        if let Some(directory) = executable.parent() {
            candidates.push(directory.join("screenshots").join(filename));
        }
    }
    // Read on every tour opening, so replacing a JPEG needs no rebuild.
    for path in candidates {
        if path.is_file() {
            match gtk::gdk::Texture::from_filename(&path) {
                Ok(texture) => return texture,
                Err(error) => {
                    eprintln!("Could not load tour screenshot {}: {error}", path.display())
                }
            }
        }
    }
    gtk::gdk::Texture::from_bytes(&glib::Bytes::from_static(bundled))
        .expect("bundled tour screenshot is valid")
}

pub(crate) fn present(parent: &adw::ApplicationWindow) {
    let dialog = build();
    dialog.set_content_width(parent.width().clamp(1, 900));
    dialog.set_content_height(parent.height().clamp(1, 650));
    dialog.present(Some(parent));
}

pub(super) fn build() -> adw::Dialog {
    let dialog = adw::Dialog::new();
    dialog.set_title("Discover PIC");
    let shell = gtk::Box::new(gtk::Orientation::Vertical, 0);
    shell.append(&adw::HeaderBar::new());
    let content = gtk::Box::new(gtk::Orientation::Vertical, 16);
    content.set_margin_start(24);
    content.set_margin_end(24);
    content.set_margin_bottom(24);
    content.set_margin_top(8);
    let stack = gtk::Stack::new();
    stack.set_vexpand(true);
    stack.set_hhomogeneous(false);
    stack.set_vhomogeneous(false);
    for (index, (title, description)) in STEPS.iter().enumerate() {
        let page = gtk::Box::new(gtk::Orientation::Vertical, 12);
        let heading = gtk::Label::new(Some(title));
        heading.add_css_class("tour-heading");
        heading.set_xalign(0.5);
        let introduction = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        introduction.set_halign(gtk::Align::Center);
        introduction.append(&heading);
        let copy = gtk::Label::new(Some(description));
        copy.set_wrap(true);
        copy.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        copy.set_max_width_chars(44);
        copy.set_xalign(0.0);
        introduction.append(&copy);
        page.append(&introduction);
        let texture = screenshot_texture(index);
        let picture = gtk::Picture::for_paintable(&texture);
        picture.set_can_shrink(true);
        picture.set_content_fit(gtk::ContentFit::Contain);
        picture.set_vexpand(true);
        picture.set_alternative_text(Some(&format!("PIC {title} screen")));
        page.append(&picture);
        let scroll = gtk::ScrolledWindow::new();
        scroll.set_hscrollbar_policy(gtk::PolicyType::Never);
        let viewport = gtk::Viewport::new(None::<&gtk::Adjustment>, None::<&gtk::Adjustment>);
        viewport.set_scroll_to_focus(true);
        viewport.set_child(Some(&page));
        scroll.set_child(Some(&viewport));
        stack.add_named(&scroll, Some(title));
    }
    stack.set_visible_child_name("Browse");
    content.append(&stack);
    let controls = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let previous = gtk::Button::with_label("Back");
    previous.set_sensitive(false);
    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    let next = gtk::Button::with_label("Next");
    next.add_css_class("suggested-action");
    controls.append(&previous);
    controls.append(&spacer);
    controls.append(&next);
    content.append(&controls);
    let skip = gtk::Button::with_label("Skip tour");
    skip.add_css_class("flat");
    skip.add_css_class("caption");
    skip.set_halign(gtk::Align::Center);
    let weak_dialog = dialog.downgrade();
    skip.connect_clicked(move |_| {
        if let Some(dialog) = weak_dialog.upgrade() {
            dialog.close();
        }
    });
    content.append(&skip);
    let update = {
        let previous = previous.clone();
        let next = next.clone();
        move |stack: &gtk::Stack| {
            let index = STEPS
                .iter()
                .position(|(title, _)| Some(*title) == stack.visible_child_name().as_deref())
                .unwrap_or(0);
            previous.set_sensitive(index > 0);
            next.set_label(if index == 3 { "Start browsing" } else { "Next" });
        }
    };
    stack.connect_visible_child_notify(update);
    let weak_stack = stack.downgrade();
    previous.connect_clicked(move |_| {
        if let Some(stack) = weak_stack.upgrade() {
            let index = STEPS
                .iter()
                .position(|(title, _)| Some(*title) == stack.visible_child_name().as_deref())
                .unwrap_or(0);
            stack.set_visible_child_name(STEPS[index.saturating_sub(1)].0);
        }
    });
    let weak_stack = stack.downgrade();
    let weak_dialog = dialog.downgrade();
    next.connect_clicked(move |_| {
        if let Some(stack) = weak_stack.upgrade() {
            let index = STEPS
                .iter()
                .position(|(title, _)| Some(*title) == stack.visible_child_name().as_deref())
                .unwrap_or(0);
            if index < 3 {
                stack.set_visible_child_name(STEPS[index + 1].0);
            } else if let Some(dialog) = weak_dialog.upgrade() {
                dialog.close();
            }
        }
    });
    shell.append(&content);
    dialog.set_child(Some(&shell));
    dialog
}
