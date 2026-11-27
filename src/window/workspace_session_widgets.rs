//! Workspace session GTK adapter (candidate 21): the one owner of the
//! Collage and photo editor sessions over the browsing pages. The decisions
//! are [`super::workspace_session::WorkspaceSession`]; this adapter applies
//! them: page switches, the collage draft, the editors' attachment, and the
//! editor's Space and Escape handlers.

use std::cell::RefCell;
use std::rc::Rc;

use gtk4 as gtk;
use gtk4::prelude::*;
use rusqlite::Connection;

use super::workspace_session::{EditOrigin, ReturnRoute, WorkspaceSession};
use crate::{db, grid};

/// The window's Space handler while the editor is open: true when handled.
pub(super) type EditSpaceSlot = Rc<RefCell<Option<Rc<dyn Fn() -> bool>>>>;
/// The window's Escape handler while the editor is open.
pub(super) type EditEscapeSlot = Rc<RefCell<Option<Rc<dyn Fn()>>>>;

pub(super) struct WorkspaceParts {
    pub(super) main_stack: gtk::Stack,
    pub(super) collage_page: gtk::Box,
    pub(super) collage_editor: Rc<RefCell<Option<crate::collage::CollageEditor>>>,
    /// Detach the photo editor (after the page's fade away from it).
    pub(super) detach_editor: Rc<dyn Fn()>,
    pub(super) edit_space: EditSpaceSlot,
    pub(super) edit_escape: EditEscapeSlot,
    pub(super) gallery: Rc<grid::Gallery>,
    pub(super) connection: Rc<RefCell<Connection>>,
}

pub(super) struct Workspace {
    session: RefCell<WorkspaceSession>,
    parts: WorkspaceParts,
    /// The open editor's close, which takes its way back.
    close_editor: RefCell<Option<Rc<dyn Fn()>>>,
}

impl Workspace {
    pub(super) fn new(parts: WorkspaceParts) -> Rc<Self> {
        Rc::new(Self {
            session: RefCell::default(),
            parts,
            close_editor: RefCell::new(None),
        })
    }

    // ----- Collage -----

    /// A collage editor was built and is about to be shown.
    pub(super) fn collage_opened(&self) {
        self.session.borrow_mut().collage_opened();
    }

    /// The Library redirect must not take the Photos page away while the
    /// gallery picks photos for the collage.
    pub(super) fn adding_photos(&self) -> bool {
        self.session.borrow().adding_photos()
    }

    /// Add Photos: mark the detour before the page switch, so the switch
    /// spares the collage. False when there is no collage.
    pub(super) fn start_adding_photos(&self) -> bool {
        self.session.borrow_mut().start_adding_photos()
    }

    /// Add Selected to Collage: the detour is over.
    pub(super) fn photos_added(&self) {
        self.session.borrow_mut().photos_added();
    }

    /// The collage's own Close (Exit/Done): leave collage selection, save
    /// the draft, return to Photos and drop the collage editor.
    pub(super) fn close_collage(&self) {
        self.session.borrow_mut().collage_closed();
        self.parts.gallery.set_collage_selection_mode(false);
        self.drop_collage();
        self.parts.main_stack.set_visible_child_name("photos");
    }

    /// Save the open collage as the draft "Resume Collage?" offers.
    pub(super) fn save_collage_draft(&self) {
        let Ok(editor) = self.parts.collage_editor.try_borrow() else {
            return;
        };
        if let Some(editor) = editor.as_ref() {
            let json = editor.draft_json();
            let _ = db::set_setting(
                &self.parts.connection.borrow(),
                crate::collage::DRAFT_SETTING_KEY,
                &json,
            );
        }
    }

    /// Save the draft, clear the grid selection (so the next Collage click
    /// is a blank start and can offer Resume) and detach the editor: GtkStack
    /// asks for the largest minimum size of all its pages, hidden ones
    /// included, so an attached collage would keep pushing the bottom bar
    /// offscreen on short windows.
    fn drop_collage(&self) {
        // try_borrow: Add Photos holds the editor while switching pages.
        if !self
            .parts
            .collage_editor
            .try_borrow()
            .is_ok_and(|editor| editor.is_some())
        {
            return;
        }
        self.save_collage_draft();
        self.parts.gallery.set_selected_photo_ids(&[]);
        while let Some(child) = self.parts.collage_page.first_child() {
            self.parts.collage_page.remove(&child);
        }
        if let Ok(mut editor) = self.parts.collage_editor.try_borrow_mut() {
            *editor = None;
        }
    }

    // ----- Photo editor -----

    /// A collage tile's Edit Photo: the next editor belongs to the collage.
    pub(super) fn request_collage_edit(&self) {
        self.session.borrow_mut().request_collage_edit();
    }

    /// The editor opens on a loaded photo; any earlier editor's keys go.
    pub(super) fn open_editor(&self, outside_library: bool, from_library: bool) -> EditOrigin {
        self.clear_edit_keys();
        self.session
            .borrow_mut()
            .open_editor(outside_library, from_library)
    }

    pub(super) fn editor_open_failed(&self) {
        self.session.borrow_mut().editor_open_failed();
    }

    pub(super) fn back_label(&self) -> Option<(&'static str, &'static str)> {
        self.session.borrow().back_label()
    }

    /// The open editor's close (its Back and Done call it too) and its
    /// Space and Escape handlers.
    pub(super) fn editor_ready(
        &self,
        close: Rc<dyn Fn()>,
        space: Rc<dyn Fn() -> bool>,
        escape: Rc<dyn Fn()>,
    ) {
        self.close_editor.replace(Some(close));
        self.parts.edit_space.replace(Some(space));
        self.parts.edit_escape.replace(Some(escape));
    }

    /// The InfoBar Edit (and Draw) toggle: close the open editor exactly as
    /// Back does. False when no editor is open.
    pub(super) fn close_open_editor(&self) -> bool {
        let close = self.close_editor.borrow().clone();
        match close {
            Some(close) => {
                close();
                true
            }
            None => false,
        }
    }

    /// Inside the editor's close: where to go. The gallery when no editor
    /// was recorded (it cannot happen; the gallery is the safe default).
    pub(super) fn take_return_route(&self) -> ReturnRoute {
        self.session
            .borrow_mut()
            .close_editor()
            .unwrap_or(ReturnRoute::Photos)
    }

    /// The editor session ends: its keys go, and the editor is detached
    /// once the page has faded away from it.
    pub(super) fn end_editor_session(&self) {
        self.close_editor.replace(None);
        self.clear_edit_keys();
        (self.parts.detach_editor)();
    }

    fn clear_edit_keys(&self) {
        self.parts.edit_space.borrow_mut().take();
        self.parts.edit_escape.borrow_mut().take();
    }

    // ----- Pages -----

    /// The main stack now shows `page`, by any path. Tear down what the
    /// session says this leaves behind.
    pub(super) fn page_shown(&self, page: &str) {
        let teardown = self.session.borrow_mut().leave_page(page);
        if teardown.end_editor {
            self.end_editor_session();
        }
        if teardown.drop_collage {
            self.drop_collage();
        }
    }
}
