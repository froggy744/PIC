//! Workspace session (candidate 21): which workspace (Collage, the photo
//! editor) is open over the browsing pages, where closing the editor
//! returns to, and what leaving a page tears down. Pure; the GTK adapter is
//! [`super::workspace_session_widgets::Workspace`].
//!
//! The photo editor can be opened from a photo outside the library (the
//! viewer's), from a collage tile, from the Library index or from the
//! gallery, and every way of closing it (Back, Done, the InfoBar Edit and
//! Draw toggles) takes the same way back.

/// The collage editor's state while it exists.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CollagePhase {
    Open,
    /// The user is picking photos in the gallery to add; the collage editor
    /// lives on behind the Photos page.
    AddingPhotos,
}

/// Where the photo editor was opened from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum EditOrigin {
    /// A photo outside the library, opened from the viewer.
    OutsideViewer,
    /// A collage tile; the collage editor waits behind the photo editor.
    Collage,
    /// The Library index.
    Library,
    /// The gallery or the viewer over it.
    Photos,
}

/// Where closing the photo editor goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ReturnRoute {
    /// Back to the viewer, on the edited photo or its saved copy.
    Viewer,
    /// Back to the collage, its tile refreshed.
    Collage,
    /// Back to the Library index.
    Library,
    /// Back to the gallery, scrolled back to where the editor opened.
    Photos,
}

/// What leaving for another page tears down.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Teardown {
    /// The photo editor ends: its key handlers go and it is detached.
    pub(super) end_editor: bool,
    /// The collage editor ends: its draft is saved, the grid selection
    /// cleared and the editor detached.
    pub(super) drop_collage: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct WorkspaceSession {
    collage: Option<CollagePhase>,
    editor: Option<EditOrigin>,
    /// A collage tile asked for the editor; the next open is collage-owned.
    collage_edit_requested: bool,
}

impl WorkspaceSession {
    #[cfg(test)]
    pub(super) fn collage(&self) -> Option<CollagePhase> {
        self.collage
    }

    #[cfg(test)]
    pub(super) fn editor(&self) -> Option<EditOrigin> {
        self.editor
    }

    /// A collage editor was built (new, resumed or replacing one).
    pub(super) fn collage_opened(&mut self) {
        self.collage = Some(CollagePhase::Open);
    }

    /// Add Photos in the collage: the gallery picks, the collage waits.
    /// False when there is no collage.
    pub(super) fn start_adding_photos(&mut self) -> bool {
        if self.collage.is_none() {
            return false;
        }
        self.collage = Some(CollagePhase::AddingPhotos);
        true
    }

    /// Add Selected to Collage: back to the collage with the photos.
    pub(super) fn photos_added(&mut self) {
        if self.collage.is_some() {
            self.collage = Some(CollagePhase::Open);
        }
    }

    /// The collage's own Close (Exit/Done).
    pub(super) fn collage_closed(&mut self) {
        self.collage = None;
    }

    pub(super) fn adding_photos(&self) -> bool {
        self.collage == Some(CollagePhase::AddingPhotos)
    }

    /// A collage tile's Edit Photo: the next editor open belongs to the
    /// collage.
    pub(super) fn request_collage_edit(&mut self) {
        self.collage_edit_requested = true;
    }

    /// The editor opens. A photo outside the library always returns to the
    /// viewer; otherwise a collage request, then the Library index, then the
    /// gallery.
    pub(super) fn open_editor(&mut self, outside_library: bool, from_library: bool) -> EditOrigin {
        let collage = std::mem::take(&mut self.collage_edit_requested);
        let origin = if outside_library {
            EditOrigin::OutsideViewer
        } else if collage {
            EditOrigin::Collage
        } else if from_library {
            EditOrigin::Library
        } else {
            EditOrigin::Photos
        };
        self.editor = Some(origin);
        origin
    }

    /// The photo could not be loaded; nothing opened.
    pub(super) fn editor_open_failed(&mut self) {
        self.collage_edit_requested = false;
    }

    /// The editor's Back button text and tooltip, when it is not the
    /// default.
    pub(super) fn back_label(&self) -> Option<(&'static str, &'static str)> {
        match self.editor? {
            EditOrigin::Collage => Some(("Back to Collage", "Return to the collage")),
            EditOrigin::Library => Some(("Back to Library", "Return to the Library index")),
            EditOrigin::OutsideViewer | EditOrigin::Photos => None,
        }
    }

    /// Any close of the editor (Back, Done, the Edit or Draw toggle).
    /// `None` when no editor is open.
    pub(super) fn close_editor(&mut self) -> Option<ReturnRoute> {
        Some(match self.editor.take()? {
            EditOrigin::OutsideViewer => ReturnRoute::Viewer,
            EditOrigin::Collage => ReturnRoute::Collage,
            EditOrigin::Library => ReturnRoute::Library,
            EditOrigin::Photos => ReturnRoute::Photos,
        })
    }

    /// The main stack now shows `page`, by any path (sidebar, search, a
    /// close). The editor ends when its page is left; the collage ends when
    /// its page is left, unless it is picking photos or a collage-owned
    /// editor is open over it.
    pub(super) fn leave_page(&mut self, page: &str) -> Teardown {
        let end_editor = page != "edit" && self.editor.is_some();
        if end_editor {
            self.editor = None;
        }
        let collage_behind_editor =
            self.editor == Some(EditOrigin::Collage) || self.collage_edit_requested;
        let drop_collage =
            page != "collage" && self.collage == Some(CollagePhase::Open) && !collage_behind_editor;
        if drop_collage {
            self.collage = None;
        }
        Teardown {
            end_editor,
            drop_collage,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session() -> WorkspaceSession {
        WorkspaceSession::default()
    }

    #[test]
    fn open_editor_origin_table() {
        // (outside, collage request, from library, origin)
        let cases = [
            (false, false, false, EditOrigin::Photos),
            (false, false, true, EditOrigin::Library),
            (false, true, false, EditOrigin::Collage),
            (false, true, true, EditOrigin::Collage),
            (true, false, false, EditOrigin::OutsideViewer),
            (true, true, true, EditOrigin::OutsideViewer),
        ];
        for (outside, collage, library, origin) in cases {
            let mut s = session();
            if collage {
                s.request_collage_edit();
            }
            assert_eq!(
                s.open_editor(outside, library),
                origin,
                "{outside} {collage} {library}"
            );
            assert_eq!(s.editor(), Some(origin));
        }
    }

    #[test]
    fn every_origin_has_one_way_back() {
        for (origin, route, label) in [
            (EditOrigin::OutsideViewer, ReturnRoute::Viewer, None),
            (
                EditOrigin::Collage,
                ReturnRoute::Collage,
                Some("Back to Collage"),
            ),
            (
                EditOrigin::Library,
                ReturnRoute::Library,
                Some("Back to Library"),
            ),
            (EditOrigin::Photos, ReturnRoute::Photos, None),
        ] {
            let mut s = WorkspaceSession {
                editor: Some(origin),
                ..session()
            };
            assert_eq!(s.back_label().map(|(text, _)| text), label, "{origin:?}");
            assert_eq!(s.close_editor(), Some(route), "{origin:?}");
            assert_eq!(s.close_editor(), None, "a second close finds no editor");
            assert_eq!(s.back_label(), None);
        }
    }

    #[test]
    fn a_collage_request_is_used_once() {
        let mut s = session();
        s.request_collage_edit();
        assert_eq!(s.open_editor(false, false), EditOrigin::Collage);
        s.close_editor();
        assert_eq!(s.open_editor(false, false), EditOrigin::Photos);
    }

    #[test]
    fn a_failed_open_drops_the_collage_request() {
        let mut s = session();
        s.collage_opened();
        s.request_collage_edit();
        s.editor_open_failed();
        assert_eq!(s.open_editor(false, false), EditOrigin::Photos);
    }

    #[test]
    fn collage_phases() {
        let mut s = session();
        assert!(!s.start_adding_photos(), "no collage to add to");
        assert_eq!(s.collage(), None);
        s.collage_opened();
        assert!(s.start_adding_photos());
        assert!(s.adding_photos());
        s.photos_added();
        assert_eq!(s.collage(), Some(CollagePhase::Open));
        s.start_adding_photos();
        s.collage_opened();
        assert_eq!(
            s.collage(),
            Some(CollagePhase::Open),
            "a new collage starts with no detour"
        );
        s.collage_closed();
        assert_eq!(s.collage(), None);
        s.photos_added();
        assert_eq!(s.collage(), None);
    }

    #[test]
    fn leave_page_table() {
        let open = Some(CollagePhase::Open);
        let adding = Some(CollagePhase::AddingPhotos);
        let editor = |origin| Some(origin);
        // (case, collage, editor, requested, page, teardown, collage after, editor after)
        let cases: [(
            &str,
            Option<CollagePhase>,
            Option<EditOrigin>,
            bool,
            &str,
            Teardown,
            Option<CollagePhase>,
            Option<EditOrigin>,
        ); 12] = [
            (
                "nothing open",
                None,
                None,
                false,
                "photos",
                Teardown::default(),
                None,
                None,
            ),
            (
                "collage page shown",
                open,
                None,
                false,
                "collage",
                Teardown::default(),
                open,
                None,
            ),
            (
                "collage left",
                open,
                None,
                false,
                "albums",
                Teardown {
                    end_editor: false,
                    drop_collage: true,
                },
                None,
                None,
            ),
            (
                "collage picking photos",
                adding,
                None,
                false,
                "photos",
                Teardown::default(),
                adding,
                None,
            ),
            (
                "picking, then elsewhere",
                adding,
                None,
                false,
                "albums",
                Teardown::default(),
                adding,
                None,
            ),
            (
                "collage request before the edit page",
                open,
                None,
                true,
                "edit",
                Teardown::default(),
                open,
                None,
            ),
            (
                "collage-owned editor shown",
                open,
                editor(EditOrigin::Collage),
                false,
                "edit",
                Teardown::default(),
                open,
                editor(EditOrigin::Collage),
            ),
            (
                "collage-owned editor left for the collage",
                open,
                editor(EditOrigin::Collage),
                false,
                "collage",
                Teardown {
                    end_editor: true,
                    drop_collage: false,
                },
                open,
                None,
            ),
            (
                "collage-owned editor left elsewhere ends both",
                open,
                editor(EditOrigin::Collage),
                false,
                "albums",
                Teardown {
                    end_editor: true,
                    drop_collage: true,
                },
                None,
                None,
            ),
            (
                "gallery editor left",
                None,
                editor(EditOrigin::Photos),
                false,
                "photos",
                Teardown {
                    end_editor: true,
                    drop_collage: false,
                },
                None,
                None,
            ),
            (
                "gallery editor opened over a collage",
                open,
                editor(EditOrigin::Photos),
                false,
                "edit",
                Teardown {
                    end_editor: false,
                    drop_collage: true,
                },
                None,
                editor(EditOrigin::Photos),
            ),
            (
                "editor page stays",
                None,
                editor(EditOrigin::Library),
                false,
                "edit",
                Teardown::default(),
                None,
                editor(EditOrigin::Library),
            ),
        ];
        for (case, collage, editor, requested, page, teardown, collage_after, editor_after) in cases
        {
            let mut s = WorkspaceSession {
                collage,
                editor,
                collage_edit_requested: requested,
            };
            assert_eq!(s.leave_page(page), teardown, "{case}");
            assert_eq!(s.collage(), collage_after, "{case}: collage");
            assert_eq!(s.editor(), editor_after, "{case}: editor");
        }
    }
}
