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

    /// The `main_stack` child name of this page (`None` when unknown).
    pub(super) fn stack_name(self) -> Option<&'static str> {
        match self {
            Self::Photos => Some("photos"),
            Self::Library => Some("library"),
            Self::Albums => Some("albums"),
            Self::Collage => Some("collage"),
            Self::Edit => Some("edit"),
            Self::FoldersOverview => Some("folders-overview"),
            Self::Unknown => None,
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

/// The three inputs of the front, as last seen. Listeners of
/// [`FrontSurface::connect_changed`] receive a before/after pair of these.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct FrontState {
    pub(super) page: Page,
    pub(super) lightbox_shown: bool,
    pub(super) date_browser: DateBrowser,
}

impl FrontState {
    pub(super) fn front(&self) -> Front {
        front(self.page, self.lightbox_shown, self.date_browser)
    }

    pub(super) fn gallery_covered(&self) -> bool {
        gallery_covered(self.lightbox_shown, self.date_browser)
    }
}

/// One change of the front's inputs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct FrontChange {
    pub(super) before: FrontState,
    pub(super) after: FrontState,
}

impl FrontChange {
    pub(super) fn viewer_changed(&self) -> bool {
        self.before.lightbox_shown != self.after.lightbox_shown
    }

    pub(super) fn page_changed(&self) -> bool {
        self.before.page != self.after.page
    }

    pub(super) fn date_overview_changed(&self) -> bool {
        (self.before.date_browser == DateBrowser::Overview)
            != (self.after.date_browser == DateBrowser::Overview)
    }
}

/// The change from `before` to `after`, or `None` when nothing changed.
pub(super) fn change(before: FrontState, after: FrontState) -> Option<FrontChange> {
    (before != after).then_some(FrontChange { before, after })
}

/// Delivers front changes to listeners, synchronously and in registration
/// order. A change made while delivering (a listener, or a GTK handler it
/// triggers, changes the page or the viewer) is delivered at once, nested;
/// the outer delivery then stops, because the listeners it had not reached
/// yet already received the newer state. No GTK: tested directly.
pub(super) struct FrontEvents {
    last: Cell<FrontState>,
    generation: Cell<u64>,
    listeners: RefCell<Vec<Rc<dyn Fn(&FrontChange)>>>,
}

impl FrontEvents {
    pub(super) fn new(initial: FrontState) -> Self {
        Self {
            last: Cell::new(initial),
            generation: Cell::new(0),
            listeners: RefCell::new(Vec::new()),
        }
    }

    pub(super) fn connect(&self, listener: impl Fn(&FrontChange) + 'static) {
        self.listeners.borrow_mut().push(Rc::new(listener));
    }

    /// Report the inputs as they are now.
    pub(super) fn dispatch(&self, now: FrontState) {
        let Some(change) = change(self.last.get(), now) else {
            return;
        };
        self.last.set(now);
        let generation = self.generation.get().wrapping_add(1);
        self.generation.set(generation);
        // Clone the list: a listener may connect another, or dispatch.
        let listeners = self.listeners.borrow().clone();
        for listener in listeners {
            listener(&change);
            if self.generation.get() != generation {
                // A nested change was delivered to every listener already.
                return;
            }
        }
    }
}

/// Reads the live widgets and answers which surface is in front. It keeps no
/// state of its own: every call reads the stack page, the viewer's
/// visibility and the date browser as they are now.
pub(super) struct FrontSurface {
    main_stack: gtk::Stack,
    lightbox_root: gtk::Overlay,
    date_browser_active: Rc<Cell<bool>>,
    date_browser: Rc<RefCell<Option<super::date_browser::DateBrowser>>>,
    events: FrontEvents,
}

impl FrontSurface {
    pub(super) fn new(
        main_stack: &gtk::Stack,
        lightbox: &Lightbox,
        date_browser_active: Rc<Cell<bool>>,
        date_browser: Rc<RefCell<Option<super::date_browser::DateBrowser>>>,
    ) -> Rc<Self> {
        let surface = Rc::new_cyclic(|weak: &std::rc::Weak<Self>| {
            // Connected before any other handler, so listeners run first.
            let on_viewer = weak.clone();
            lightbox.root.connect_visible_notify(move |_| {
                if let Some(surface) = on_viewer.upgrade() {
                    surface.inputs_changed();
                }
            });
            let on_page = weak.clone();
            main_stack.connect_visible_child_notify(move |_| {
                if let Some(surface) = on_page.upgrade() {
                    surface.inputs_changed();
                }
            });
            Self {
                main_stack: main_stack.clone(),
                lightbox_root: lightbox.root.clone(),
                date_browser_active,
                date_browser,
                events: FrontEvents::new(FrontState {
                    page: Page::Unknown,
                    lightbox_shown: false,
                    date_browser: DateBrowser::Off,
                }),
            }
        });
        // The starting state is not a change.
        surface.events.last.set(surface.state());
        surface
    }

    pub(super) fn current(&self) -> Front {
        self.state().front()
    }

    /// The inputs as they are now, read from the widgets.
    pub(super) fn state(&self) -> FrontState {
        FrontState {
            page: Page::from_stack_name(self.main_stack.visible_child_name().as_deref()),
            lightbox_shown: self.lightbox_root.is_visible(),
            date_browser: self.date_browser(),
        }
    }

    /// Listen for changes of the front's inputs (see [`FrontEvents`]).
    /// Not called for the starting state.
    pub(super) fn connect_changed(&self, listener: impl Fn(&FrontChange) + 'static) {
        self.events.connect(listener);
    }

    /// Report a change of the date browser: it was activated or left, or its
    /// overview was shown or hidden. GTK reports the viewer and page itself.
    pub(super) fn date_browser_changed(&self) {
        self.inputs_changed();
    }

    fn inputs_changed(&self) {
        self.events.dispatch(self.state());
    }

    /// See [`gallery_covered`].
    pub(super) fn gallery_covered(&self) -> bool {
        self.state().gallery_covered()
    }

    /// The date browser's level, whatever is in front (the sort controls
    /// follow it even while the viewer covers the overview).
    pub(super) fn date_browser(&self) -> DateBrowser {
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

    fn state(page: Page, lightbox_shown: bool, date_browser: DateBrowser) -> FrontState {
        FrontState { page, lightbox_shown, date_browser }
    }

    #[test]
    fn a_change_is_reported_only_when_an_input_changes() {
        use DateBrowser::{Days, Off, Overview};
        let gallery = state(Page::Photos, false, Off);
        // (before, after, changed?, viewer, page, date overview)
        let rows = [
            (gallery, gallery, false, false, false, false),
            (gallery, state(Page::Photos, true, Off), true, true, false, false),
            (gallery, state(Page::Edit, false, Off), true, false, true, false),
            (gallery, state(Page::Photos, false, Days), true, false, false, false),
            (gallery, state(Page::Photos, false, Overview), true, false, false, true),
            (state(Page::Photos, false, Overview), state(Page::Photos, false, Off), true, false, false, true),
            (state(Page::Photos, false, Overview), state(Page::Collage, false, Overview), true, false, true, false),
            (state(Page::Photos, true, Days), state(Page::Library, false, Off), true, true, true, false),
        ];
        for (before, after, changed, viewer, page, overview) in rows {
            let result = change(before, after);
            assert_eq!(result.is_some(), changed, "{before:?} -> {after:?}");
            if let Some(result) = result {
                assert_eq!(result, FrontChange { before, after });
                assert_eq!(
                    (result.viewer_changed(), result.page_changed(), result.date_overview_changed()),
                    (viewer, page, overview),
                    "{before:?} -> {after:?}",
                );
            }
        }
    }

    type Log = Rc<RefCell<Vec<(&'static str, Page, Page)>>>;

    fn logging(events: &FrontEvents, log: &Log, name: &'static str) {
        let log = log.clone();
        events.connect(move |change| log.borrow_mut().push((name, change.before.page, change.after.page)));
    }

    #[test]
    fn listeners_hear_each_change_once_in_registration_order() {
        let events = FrontEvents::new(state(Page::Photos, false, DateBrowser::Off));
        let log: Log = Rc::default();
        logging(&events, &log, "a");
        logging(&events, &log, "b");
        events.dispatch(state(Page::Photos, false, DateBrowser::Off)); // no change
        events.dispatch(state(Page::Edit, false, DateBrowser::Off));
        events.dispatch(state(Page::Edit, false, DateBrowser::Off)); // repeated, no change
        events.dispatch(state(Page::Photos, false, DateBrowser::Off));
        assert_eq!(
            *log.borrow(),
            vec![
                ("a", Page::Photos, Page::Edit),
                ("b", Page::Photos, Page::Edit),
                ("a", Page::Edit, Page::Photos),
                ("b", Page::Edit, Page::Photos),
            ],
        );
    }

    /// A listener changes the inputs again during a delivery (in the window:
    /// a page change closes the viewer). The newer state reaches every
    /// listener at once; none is left with the older state after it. (GTK
    /// itself queues a page change made inside a page notification, so the
    /// Library redirect arrives as a second, separate change.)
    #[test]
    fn a_nested_change_is_delivered_at_once_and_supersedes_the_outer_one() {
        let events = Rc::new(FrontEvents::new(state(Page::Albums, false, DateBrowser::Off)));
        let log: Log = Rc::default();
        logging(&events, &log, "first");
        {
            let weak = Rc::downgrade(&events);
            let log = log.clone();
            events.connect(move |change| {
                log.borrow_mut().push(("redirect", change.before.page, change.after.page));
                if change.after.page == Page::Photos {
                    if let Some(events) = weak.upgrade() {
                        events.dispatch(state(Page::Library, false, DateBrowser::Off));
                    }
                }
            });
        }
        logging(&events, &log, "last");
        events.dispatch(state(Page::Photos, false, DateBrowser::Off));
        assert_eq!(
            *log.borrow(),
            vec![
                ("first", Page::Albums, Page::Photos),
                ("redirect", Page::Albums, Page::Photos),
                ("first", Page::Photos, Page::Library),
                ("redirect", Page::Photos, Page::Library),
                ("last", Page::Photos, Page::Library),
            ],
        );
    }

    /// FrontSurface against real widgets: the viewer, the stack page and the
    /// date browser each report a change, and a redirect made inside a
    /// listener reaches every listener as the latest state.
    #[test]
    #[ignore = "requires a GTK display; run individually"]
    fn front_surface_reports_viewer_page_and_date_browser_changes() {
        use super::super::date_browser::DateBrowser as Browser;
        gtk::init().unwrap();
        let stack = gtk::Stack::new();
        for name in ["photos", "library", "edit"] {
            stack.add_named(&gtk::Box::new(gtk::Orientation::Vertical, 0), Some(name));
        }
        stack.set_visible_child_name("photos");
        let lightbox = Lightbox::new();
        lightbox.root.set_visible(false);
        let active = Rc::new(Cell::new(false));
        let browser = Browser::new(Rc::new(|_| {}), Rc::new(|| {}), Rc::new(|| {}));
        browser.show_month_photo(None); // start with the overview hidden
        let slot = Rc::new(RefCell::new(Some(browser.clone())));
        let front = FrontSurface::new(&stack, &lightbox, active.clone(), slot);
        {
            let front = Rc::downgrade(&front);
            browser.connect_overview_visible(move || {
                if let Some(front) = front.upgrade() {
                    front.date_browser_changed();
                }
            });
        }
        let log: Rc<RefCell<Vec<(&'static str, FrontState)>>> = Rc::default();
        let listen = |name: &'static str| {
            let log = log.clone();
            move |change: &FrontChange| log.borrow_mut().push((name, change.after))
        };
        front.connect_changed(listen("a"));
        let s = |page, lightbox_shown, date_browser| FrontState { page, lightbox_shown, date_browser };
        let take = || std::mem::take(&mut *log.borrow_mut());

        lightbox.root.set_visible(true);
        lightbox.root.set_visible(false);
        assert_eq!(take(), vec![
            ("a", s(Page::Photos, true, DateBrowser::Off)),
            ("a", s(Page::Photos, false, DateBrowser::Off)),
        ]);
        assert_eq!(front.current(), Front::Gallery);

        active.set(true);
        front.date_browser_changed();
        browser.show_years();
        active.set(false);
        front.date_browser_changed();
        assert_eq!(take(), vec![
            ("a", s(Page::Photos, false, DateBrowser::Days)),
            ("a", s(Page::Photos, false, DateBrowser::Overview)),
            ("a", s(Page::Photos, false, DateBrowser::Off)),
        ]);

        // Leaving for Edit closes the viewer from inside the page change (the
        // close-on-page-change handler). The viewer is another widget, so
        // its change is delivered nested and supersedes the outer delivery:
        // "c" never sees the viewer still shown over Edit.
        lightbox.root.set_visible(true);
        take();
        {
            let root = lightbox.root.clone();
            front.connect_changed(move |change| {
                if change.after.page == Page::Edit && change.after.lightbox_shown {
                    root.set_visible(false);
                }
            });
        }
        front.connect_changed(listen("c"));
        stack.set_visible_child_name("edit");
        assert_eq!(take(), vec![
            ("a", s(Page::Edit, true, DateBrowser::Off)),
            ("a", s(Page::Edit, false, DateBrowser::Off)),
            ("c", s(Page::Edit, false, DateBrowser::Off)),
        ]);
        assert_eq!(front.current(), Front::Editor);

        // The Library redirect: GTK queues a page change made inside a page
        // notification until that notification ends, so every listener sees
        // Photos, then every listener sees Library. None is left on Photos.
        {
            let stack = stack.clone();
            front.connect_changed(move |change| {
                if change.after.page == Page::Photos {
                    stack.set_visible_child_name("library");
                }
            });
        }
        stack.set_visible_child_name("photos");
        assert_eq!(take(), vec![
            ("a", s(Page::Photos, false, DateBrowser::Off)),
            ("c", s(Page::Photos, false, DateBrowser::Off)),
            ("a", s(Page::Library, false, DateBrowser::Off)),
            ("c", s(Page::Library, false, DateBrowser::Off)),
        ]);
        assert_eq!(front.current(), Front::LibraryHome);
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
        for page in PAGES {
            assert_eq!(Page::from_stack_name(page.stack_name()), page, "{page:?} round trip");
        }
    }
}
