use std::cell::{Cell, RefCell};
use std::io::Read;

use glib::prelude::ObjectExt;
use glib::subclass::prelude::*;
use glib::Properties;

use crate::db::Photo;
use crate::thumbnail;

mod imp {
    use super::*;

    #[derive(Default, Properties)]
    #[properties(wrapper_type = super::PhotoObject)]
    pub struct PhotoObject {
        #[property(get, set)]
        pub id: Cell<i64>,
        #[property(get, set)]
        pub path: RefCell<String>,
        #[property(get, set)]
        pub filename: RefCell<String>,
        #[property(get, set)]
        pub history_caption: RefCell<Option<String>>,
        #[property(get, set)]
        pub edited_at: Cell<i64>,
        #[property(get, set)]
        pub taken_at: RefCell<Option<String>>,
        #[property(get, set)]
        pub camera: RefCell<Option<String>>,
        #[property(get, set)]
        pub aperture: Cell<f64>,
        #[property(get, set)]
        pub width: Cell<i64>,
        #[property(get, set)]
        pub height: Cell<i64>,
        #[property(get, set)]
        pub size_bytes: Cell<i64>,
        #[property(get, set)]
        pub mtime: Cell<i64>,
        #[property(get, set)]
        pub rotation: Cell<i32>,
        #[property(get, set)]
        pub edit_recipe: RefCell<String>,
        #[property(get, set)]
        pub favorite: Cell<bool>,
        #[property(get, set)]
        pub rating: Cell<i32>,
        #[property(get, set)]
        pub folder_id: Cell<i64>,
        #[property(get, set)]
        pub folder_path: RefCell<Option<String>>,
        #[property(get, set)]
        pub original_available: Cell<bool>,
        #[property(get, set)]
        pub corrupt: Cell<bool>,
        // When the original was last probed for availability. Not a GObject
        // property; used to re-probe on rebind after a TTL so a drive that goes
        // offline without a mount event still gets its offline badge.
        pub original_checked_at: Cell<Option<std::time::Instant>>,
        #[property(get, set)]
        pub cached_thumbnail_path: RefCell<Option<String>>,
        #[property(get, set)]
        pub thumbnail_available: Cell<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for PhotoObject {
        const NAME: &'static str = "PicasaPhotoObject";
        type Type = super::PhotoObject;
    }

    impl ObjectImpl for PhotoObject {
        fn properties() -> &'static [glib::ParamSpec] {
            Self::derived_properties()
        }

        fn set_property(&self, id: usize, value: &glib::Value, pspec: &glib::ParamSpec) {
            self.derived_set_property(id, value, pspec);
        }

        fn property(&self, id: usize, pspec: &glib::ParamSpec) -> glib::Value {
            self.derived_property(id, pspec)
        }
    }
}

glib::wrapper! {
    pub struct PhotoObject(ObjectSubclass<imp::PhotoObject>);
}

impl PhotoObject {
    /// Compare catalog state through backing fields during cooperative refresh.
    /// Availability and decoded-preview state are maintained independently.
    pub(crate) fn matches_catalog(&self, photo: &Photo) -> bool {
        let imp = self.imp();
        imp.id.get() == photo.id
            && *imp.path.borrow() == photo.path
            && *imp.history_caption.borrow() == photo.history_caption
            && imp.edited_at.get() == photo.edited_at
            && *imp.taken_at.borrow() == photo.taken_at
            && *imp.camera.borrow() == photo.camera
            && imp.aperture.get() == photo.aperture.unwrap_or_default()
            && imp.width.get() == photo.width.unwrap_or_default()
            && imp.height.get() == photo.height.unwrap_or_default()
            && imp.mtime.get() == photo.mtime.unwrap_or_default()
            && imp.size_bytes.get() == photo.size_bytes.unwrap_or_default()
            && imp.rotation.get() == photo.rotation
            && *imp.edit_recipe.borrow() == photo.edit_recipe
            && imp.favorite.get() == photo.favorite
            && imp.rating.get() == photo.rating
            && imp.folder_id.get() == photo.folder_id.unwrap_or_default()
            && *imp.folder_path.borrow() == photo.folder_path
    }

    /// Catalog axes already include EXIF orientation; only PIC rotation is applied here.
    pub fn photo_wall_aspect_ratio(&self) -> f64 {
        let imp = self.imp();
        let (width, height) = (imp.width.get(), imp.height.get());
        if width <= 0 || height <= 0 {
            return 1.0;
        }
        if matches!(imp.rotation.get().rem_euclid(360), 90 | 270) {
            height as f64 / width as f64
        } else {
            width as f64 / height as f64
        }
    }

    /// When the original was last probed, if ever.
    pub fn original_checked_at(&self) -> Option<std::time::Instant> {
        self.imp().original_checked_at.get()
    }

    pub fn set_original_checked_at(&self, value: Option<std::time::Instant>) {
        self.imp().original_checked_at.set(value);
    }

    pub fn from_photo(photo: &Photo) -> Self {
        let object: Self = glib::Object::new();
        object.set_from_photo(photo);
        object
    }

    /// Populate a fresh object directly rather than through the GObject property
    /// system. Constructing a library-sized model sets ~15 properties for each
    /// of tens of thousands of photos; going through `set_property` (ParamSpec
    /// lookup, `Value` boxing, notifications) dominated folder-open time. No
    /// consumer connects to these properties' notify signals, so writing the
    /// subclass fields directly is equivalent and several times faster.
    pub fn set_from_photo(&self, photo: &Photo) {
        let imp = self.imp();
        imp.id.set(photo.id);
        *imp.path.borrow_mut() = photo.path.clone();
        *imp.filename.borrow_mut() = crate::source::filename(&photo.path);
        *imp.history_caption.borrow_mut() = photo.history_caption.clone();
        imp.edited_at.set(photo.edited_at);
        *imp.taken_at.borrow_mut() = photo.taken_at.clone();
        *imp.camera.borrow_mut() = photo.camera.clone();
        imp.aperture.set(photo.aperture.unwrap_or_default());
        imp.width.set(photo.width.unwrap_or_default());
        imp.height.set(photo.height.unwrap_or_default());
        imp.size_bytes.set(photo.size_bytes.unwrap_or_default());
        imp.mtime.set(photo.mtime.unwrap_or_default());
        imp.rotation.set(photo.rotation);
        *imp.edit_recipe.borrow_mut() = photo.edit_recipe.clone();
        imp.favorite.set(photo.favorite);
        imp.rating.set(photo.rating);
        imp.folder_id.set(photo.folder_id.unwrap_or_default());
        *imp.folder_path.borrow_mut() = photo.folder_path.clone();
        // Offline is source-folder state, not an individual-file check. The
        // folder map is prepared once per registered imported root, so creating
        // a large library model never stats photo originals.
        imp.original_available
            .set(crate::source::folder_available(photo.folder_id));
        imp.corrupt.set(confirmed_corrupt_local_jpeg(&photo.path, photo.width, photo.height));
        *imp.cached_thumbnail_path.borrow_mut() =
            thumbnail::cache_path(&photo.path, photo.mtime, photo.size_bytes)
                .ok()
                .map(|path| path.to_string_lossy().into_owned());
        // The visible tile performs this inexpensive cache check lazily.
        imp.thumbnail_available.set(false);
        imp.original_checked_at.set(None);
    }
}

pub(crate) fn confirmed_corrupt_local_jpeg(path: &str, width: Option<i64>, height: Option<i64>) -> bool {
    if width.is_some() || height.is_some() || !path.starts_with('/') {
        return false;
    }
    let extension = std::path::Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str());
    if !extension.is_some_and(|extension| matches!(extension.to_ascii_lowercase().as_str(), "jpg" | "jpeg")) {
        return false;
    }
    let Ok(mut file) = std::fs::File::open(path) else { return false };
    let mut header = [0u8; 16];
    let Ok(length) = file.read(&mut header) else { return false };
    if header[..length].starts_with(&[0xff, 0xd8, 0xff]) {
        return false;
    }
    image::guess_format(&header[..length]).is_err()
}

#[cfg(test)]
mod corrupt_file_tests {
    use super::*;

    #[test]
    fn corrupt_badge_requires_an_existing_unrecognized_local_jpeg() {
        let path = std::env::temp_dir().join(format!("pic-corrupt-probe-{}.jpg", std::process::id()));
        std::fs::write(&path, b"not image data").unwrap();
        let path = path.to_str().unwrap();
        assert!(confirmed_corrupt_local_jpeg(path, None, None));
        assert!(!confirmed_corrupt_local_jpeg(path, Some(100), Some(100)));
        std::fs::write(path, [0xff, 0xd8, 0xff, 0xe0]).unwrap();
        assert!(!confirmed_corrupt_local_jpeg(path, None, None));
        std::fs::remove_file(path).unwrap();
        assert!(!confirmed_corrupt_local_jpeg(path, None, None));
        assert!(!confirmed_corrupt_local_jpeg("nfs://DietPi.local/photo.jpg", None, None));
    }
}

#[cfg(test)]
mod photo_wall_tests {
    use super::*;

    #[test]
    fn photo_wall_aspect_uses_catalog_axes_and_user_rotation_once() {
        for (width, height, rotation, expected) in [
            (6000_i64, 4000_i64, 0, 1.5),
            (4000, 6000, 0, 2.0 / 3.0),
            (4000, 6000, 90, 1.5),
            (4000, 6000, 270, 1.5),
            (6000, 4000, 180, 1.5),
            (3000, 3000, 90, 1.0),
            (0, 4000, 0, 1.0),
            (-1, 4000, 90, 1.0),
        ] {
            let photo: PhotoObject = glib::Object::builder()
                .property("width", width)
                .property("height", height)
                .property("rotation", rotation)
                .build();
            assert!((photo.photo_wall_aspect_ratio() - expected).abs() < 1e-9);
        }
    }
}
