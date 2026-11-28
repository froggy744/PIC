//! Photo capabilities: what a photo of each kind may do, in one table.
//!
//! A photo opened from outside the library has no catalogue row, so every
//! action that writes one (marks, albums, rotation, edit recipes, rename,
//! delete) is off for it; actions that read the file (Edit with Save Copy,
//! Export, Print, Copy, Open With) stay on. The context menu and the InfoBar
//! both read [`PhotoKind::capabilities`], so the two surfaces cannot disagree.

use crate::photo_object::PhotoObject;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PhotoKind {
    /// Catalogued in the library.
    Library,
    /// Opened from outside the library (no catalogue row, a negative id).
    Outside,
}

/// One flag per action group; true when a photo of this kind may do it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PhotoCapabilities {
    /// Favourite and rating.
    pub marks: bool,
    /// Add to Album (and the album rows' own actions).
    pub albums: bool,
    pub rotate: bool,
    /// Open the editor (an outside photo's Done saves a copy).
    pub edit: bool,
    /// Copy, Paste and Reset Edits (the stored recipe).
    pub recipe: bool,
    pub collage: bool,
    pub export: bool,
    pub print: bool,
    /// Rename and Delete (the library's file operations).
    pub file_operations: bool,
    /// Open in Folder (the library's folder view).
    pub open_in_folder: bool,
}

impl PhotoKind {
    pub(crate) fn of(photo: &PhotoObject) -> Self {
        Self::of_id(photo.id())
    }

    pub(crate) fn of_id(id: i64) -> Self {
        if id < 0 {
            Self::Outside
        } else {
            Self::Library
        }
    }

    pub(crate) fn capabilities(self) -> PhotoCapabilities {
        let library = self == Self::Library;
        PhotoCapabilities {
            marks: library,
            albums: library,
            rotate: library,
            edit: true,
            recipe: library,
            collage: library,
            export: true,
            print: true,
            file_operations: library,
            open_in_folder: library,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_outside_photo_reads_its_file_but_writes_no_catalogue_row() {
        let outside = PhotoKind::Outside.capabilities();
        assert_eq!(
            outside,
            PhotoCapabilities {
                marks: false,
                albums: false,
                rotate: false,
                edit: true,
                recipe: false,
                collage: false,
                export: true,
                print: true,
                file_operations: false,
                open_in_folder: false,
            }
        );
    }

    #[test]
    fn a_library_photo_may_do_everything() {
        let library = PhotoKind::Library.capabilities();
        assert_eq!(
            library,
            PhotoCapabilities {
                marks: true,
                albums: true,
                rotate: true,
                edit: true,
                recipe: true,
                collage: true,
                export: true,
                print: true,
                file_operations: true,
                open_in_folder: true,
            }
        );
    }

    #[test]
    fn the_kind_follows_the_catalogue_id() {
        assert_eq!(PhotoKind::of_id(-1), PhotoKind::Outside);
        assert_eq!(PhotoKind::of_id(0), PhotoKind::Library);
        assert_eq!(PhotoKind::of_id(42), PhotoKind::Library);
    }
}
