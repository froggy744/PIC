//! Front surface: which surface is in front of the window, decided in one
//! place.
//!
//! Input routing (the zoom slider, 1:1, keys, Escape, collection
//! navigation, mark targets) depends on what the user is looking at: the
//! photo viewer, the editor, the collage, the date browser's overview or
//! the gallery. That answer comes from three pieces of live widget state:
//! the main stack page, whether the viewer is shown, and the date
//! browser's state. [`front`] combines them with one written precedence,
//! free of GTK. [`FrontSurface`] reads the live widgets and asks [`front`];
//! it caches nothing, so it can never disagree with the widgets.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::prelude::*;
use gtk4 as gtk;

use crate::lightbox::Lightbox;

/// The main stack page on screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Page {
    Photos,
    Library,
    Albums,
    Collage,
    Edit,
    FoldersOverview,
    /// No page yet, or a page this module does not know.
    Unknown,
}

impl Page {
    /// The page for a `main_stack` child name.
    pub(super) fn from_stack_name(name: Option<&str>) -> Self {
        match name {
            Some("photos") => Self::Photos,
            Some("library") => Self::Library,
            Some("albums") => Self::Albums,
            Some("collage") => Self::Collage,
            Some("edit") => Self::Edit,
            Some("folders-overview") => Self::FoldersOverview,
            _ => Self::Unknown,
        }
    }
}

/// The date browser's state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DateBrowser {
    Off,
    /// Active, showing the photos of one period in the gallery.
    Days,
    /// Active, showing its year/month overview over the gallery.
    Overview,
}

/// The surface in front.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Front {
    /// The photo viewer, over Photos or Library.
    Lightbox,
    Editor,
    Collage,
    /// The date browser's overview, over the Photos gallery.
    DateOverview,
    /// The Photos gallery, including the date browser's day view.
    Gallery,
    LibraryHome,
    Albums,
    FoldersOverview,
    /// No known page (before the stack is filled).
    None,
}

/// The precedence. The page decides first: Editor, Collage, Albums and
/// Folders overview are in front whatever else is shown. The viewer counts
/// only over Photos or Library (leaving those pages closes it). Then the
/// date overview, then the gallery or the Library home.
pub(super) fn front(page: Page, lightbox_shown: bool, date_browser: DateBrowser) -> Front {
    match page {
        Page::Edit => Front::Editor,
        Page::Collage => Front::Collage,
        Page::Albums => Front::Albums,
        Page::FoldersOverview => Front::FoldersOverview,
        Page::Unknown => Front::None,
        Page::Photos | Page::Library if lightbox_shown => Front::Lightbox,
        Page::Photos if date_browser == DateBrowser::Overview => Front::DateOverview,
        Page::Photos => Front::Gallery,
        Page::Library => Front::LibraryHome,
    }
}

/// Whether the viewer or the date overview hides the Photos gallery,
/// whatever page is on top. Unlike [`front`], this ignores the page: the
/// date overview stays up under Collage when Collage opens from it, and a
/// gallery refresh there must still not replace the photo it covers.
pub(super) fn gallery_covered(lightbox_shown: bool, date_browser: DateBrowser) -> bool {
    lightbox_shown || date_browser == DateBrowser::Overview
}

/// Reads the live widgets and answers which surface is in front. It keeps no
/// state of its own: every call reads the stack page, the viewer's
/// visibility and the date browser as they are now.
pub(super) struct FrontSurface {
    main_stack: gtk::Stack,
    lightbox_root: gtk::Overlay,
    date_browser_active: Rc<Cell<bool>>,
    date_browser: Rc<RefCell<Option<super::date_browser::DateBrowser>>>,
}

impl FrontSurface {
    pub(super) fn new(
        main_stack: &gtk::Stack,
        lightbox: &Lightbox,
        date_browser_active: Rc<Cell<bool>>,
        date_browser: Rc<RefCell<Option<super::date_browser::DateBrowser>>>,
    ) -> Rc<Self> {
        Rc::new(Self {
            main_stack: main_stack.clone(),
            lightbox_root: lightbox.root.clone(),
            date_browser_active,
            date_browser,
        })
    }

    pub(super) fn current(&self) -> Front {
        let page = Page::from_stack_name(self.main_stack.visible_child_name().as_deref());
        front(page, self.lightbox_root.is_visible(), self.date_browser())
    }

    /// See [`gallery_covered`].
    pub(super) fn gallery_covered(&self) -> bool {
        gallery_covered(self.lightbox_root.is_visible(), self.date_browser())
    }

    fn date_browser(&self) -> DateBrowser {
        if !self.date_browser_active.get() {
            DateBrowser::Off
        } else if self
            .date_browser
            .borrow()
            .as_ref()
            .is_some_and(|browser| browser.is_overview_visible())
        {
            DateBrowser::Overview
        } else {
            DateBrowser::Days
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGES: [Page; 7] = [
        Page::Photos,
        Page::Library,
        Page::Albums,
        Page::Collage,
        Page::Edit,
        Page::FoldersOverview,
        Page::Unknown,
    ];
    const BROWSER: [DateBrowser; 3] = [DateBrowser::Off, DateBrowser::Days, DateBrowser::Overview];

    /// Every combination of page × viewer × date browser, written out,
    /// including the ones the window never produces today (the viewer over
    /// the editor, the date browser under Library or Albums).
    #[test]
    fn precedence_covers_every_combination() {
        use DateBrowser::{Days, Off, Overview};
        use Front as F;
        #[rustfmt::skip]
        let rows: &[(Page, bool, DateBrowser, Front)] = &[
            // Photos: viewer, then the date overview, then the gallery.
            (Page::Photos, false, Off,      F::Gallery),
            (Page::Photos, false, Days,     F::Gallery),
            (Page::Photos, false, Overview, F::DateOverview),
            (Page::Photos, true,  Off,      F::Lightbox),
            (Page::Photos, true,  Days,     F::Lightbox),
            (Page::Photos, true,  Overview, F::Lightbox),
            // Library: the viewer opens over the Library index too.
            (Page::Library, false, Off,      F::LibraryHome),
            (Page::Library, false, Days,     F::LibraryHome),
            (Page::Library, false, Overview, F::LibraryHome),
            (Page::Library, true,  Off,      F::Lightbox),
            (Page::Library, true,  Days,     F::Lightbox),
            (Page::Library, true,  Overview, F::Lightbox),
            // Pages that win whatever else is shown.
            (Page::Edit, false, Off,      F::Editor),
            (Page::Edit, false, Days,     F::Editor),
            (Page::Edit, false, Overview, F::Editor),
            (Page::Edit, true,  Off,      F::Editor),
            (Page::Edit, true,  Days,     F::Editor),
            (Page::Edit, true,  Overview, F::Editor),
            (Page::Collage, false, Off,      F::Collage),
            (Page::Collage, false, Days,     F::Collage),
            (Page::Collage, false, Overview, F::Collage),
            (Page::Collage, true,  Off,      F::Collage),
            (Page::Collage, true,  Days,     F::Collage),
            (Page::Collage, true,  Overview, F::Collage),
            (Page::Albums, false, Off,      F::Albums),
            (Page::Albums, false, Days,     F::Albums),
            (Page::Albums, false, Overview, F::Albums),
            (Page::Albums, true,  Off,      F::Albums),
            (Page::Albums, true,  Days,     F::Albums),
            (Page::Albums, true,  Overview, F::Albums),
            (Page::FoldersOverview, false, Off,      F::FoldersOverview),
            (Page::FoldersOverview, false, Days,     F::FoldersOverview),
            (Page::FoldersOverview, false, Overview, F::FoldersOverview),
            (Page::FoldersOverview, true,  Off,      F::FoldersOverview),
            (Page::FoldersOverview, true,  Days,     F::FoldersOverview),
            (Page::FoldersOverview, true,  Overview, F::FoldersOverview),
            (Page::Unknown, false, Off,      F::None),
            (Page::Unknown, false, Days,     F::None),
            (Page::Unknown, false, Overview, F::None),
            (Page::Unknown, true,  Off,      F::None),
            (Page::Unknown, true,  Days,     F::None),
            (Page::Unknown, true,  Overview, F::None),
        ];
        assert_eq!(rows.len(), PAGES.len() * 2 * BROWSER.len(), "table must be complete");
        for page in PAGES {
            for shown in [false, true] {
                for browser in BROWSER {
                    assert!(
                        rows.iter().any(|row| row.0 == page && row.1 == shown && row.2 == browser),
                        "missing row {page:?} {shown} {browser:?}",
                    );
                }
            }
        }
        for &(page, shown, browser, expected) in rows {
            assert_eq!(
                front(page, shown, browser),
                expected,
                "page={page:?} lightbox_shown={shown} date_browser={browser:?}",
            );
        }
    }

    #[test]
    fn gallery_is_covered_by_the_viewer_or_the_date_overview() {
        use DateBrowser::{Days, Off, Overview};
        let rows = [
            (false, Off, false),
            (false, Days, false),
            (false, Overview, true),
            (true, Off, true),
            (true, Days, true),
            (true, Overview, true),
        ];
        for (shown, browser, covered) in rows {
            assert_eq!(
                gallery_covered(shown, browser),
                covered,
                "lightbox_shown={shown} date_browser={browser:?}",
            );
        }
    }

    #[test]
    fn stack_names_map_to_pages() {
        let rows = [
            (Some("photos"), Page::Photos),
            (Some("library"), Page::Library),
            (Some("albums"), Page::Albums),
            (Some("collage"), Page::Collage),
            (Some("edit"), Page::Edit),
            (Some("folders-overview"), Page::FoldersOverview),
            (Some("settings"), Page::Unknown),
            (None, Page::Unknown),
        ];
        for (name, page) in rows {
            assert_eq!(Page::from_stack_name(name), page, "{name:?}");
        }
    }
}
