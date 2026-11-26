//! Album list: which album list feeds each album surface, and when.
//!
//! Two surfaces show albums: the sidebar's album rows (whole albums,
//! `db::albums`) and the Albums home (counts narrowed by the browse scope,
//! `db::browse_albums`). The Add to Album list builds itself each time it
//! opens (`db::manual_albums`) and needs no refresh. Callers only say what
//! changed ([`AlbumChange`]); [`plan`] decides which surfaces refresh and
//! whether now, on a worker or batched, and [`AlbumListState`] decides
//! which results are still current. The GTK adapter is
//! `window::album_list_widgets` (`AlbumList`).

/// What happened to the albums or to how they are browsed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum AlbumChange {
    /// An album was created, renamed, deleted, filled, emptied, given a
    /// cover or new rules, or a folder holding album photos was removed.
    Edited,
    /// Photos changed in a way Smart Album rules may see (marks, tags,
    /// edits, trash, imports, metadata indexing).
    PhotosChanged,
    /// The browse scope changed (toolbar rating, metadata filters).
    BrowseChanged,
    /// The album appearance (theme, layout, covers) changed.
    Appearance,
    /// The Albums home was opened.
    Shown,
    /// Another library was opened.
    LibraryOpened,
}

/// When a refresh runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Timing {
    /// On the GTK thread, before the caller continues.
    Now,
    /// On a worker; the result is applied while still current.
    Worker,
    /// On a worker after a short delay that one burst of changes shares.
    Batched,
}

/// Which surfaces a change refreshes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct AlbumRefresh {
    pub(super) sidebar: bool,
    pub(super) home: bool,
    pub(super) timing: Timing,
}

/// The refresh `change` needs. `home_shown`: the Albums home is the current
/// destination (a hidden home is reloaded when it is shown). `has_smart`:
/// the library has Smart Albums (photo changes cannot move other albums'
/// counts). `None`: nothing to refresh.
pub(super) fn plan(change: AlbumChange, home_shown: bool, has_smart: bool) -> Option<AlbumRefresh> {
    let refresh = |sidebar, home, timing| {
        (sidebar || home).then_some(AlbumRefresh { sidebar, home, timing })
    };
    match change {
        AlbumChange::Edited | AlbumChange::Appearance | AlbumChange::LibraryOpened => {
            refresh(true, home_shown, Timing::Now)
        }
        AlbumChange::PhotosChanged if has_smart => refresh(true, home_shown, Timing::Batched),
        AlbumChange::PhotosChanged => None,
        AlbumChange::BrowseChanged => refresh(false, home_shown, Timing::Worker),
        AlbumChange::Shown => refresh(false, true, Timing::Now),
    }
}

/// Which refresh a result belongs to: one generation per surface, so a home
/// refresh never cancels a sidebar recount (and the other way round), plus
/// the library it was started in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct AlbumTicket {
    sidebar: Option<u64>,
    home: Option<u64>,
    library: u64,
}

/// Bookkeeping for current results and batched recounts.
#[derive(Debug, Default)]
pub(super) struct AlbumListState {
    sidebar: u64,
    home: u64,
    library: u64,
    batch_pending: bool,
}

impl AlbumListState {
    /// Start `refresh`: every surface it writes gets a new generation, so
    /// any older result for that surface is no longer current.
    pub(super) fn begin(&mut self, refresh: AlbumRefresh) -> AlbumTicket {
        if refresh.sidebar {
            self.sidebar += 1;
        }
        if refresh.home {
            self.home += 1;
        }
        AlbumTicket {
            sidebar: refresh.sidebar.then_some(self.sidebar),
            home: refresh.home.then_some(self.home),
            library: self.library,
        }
    }

    /// Whether the ticket's sidebar list may still be shown.
    pub(super) fn sidebar_current(&self, ticket: AlbumTicket) -> bool {
        ticket.library == self.library && ticket.sidebar == Some(self.sidebar)
    }

    /// Whether the ticket's home list may still be shown (the browse scope
    /// is checked separately, against the scope it was counted under).
    pub(super) fn home_current(&self, ticket: AlbumTicket) -> bool {
        ticket.library == self.library && ticket.home == Some(self.home)
    }

    /// Another library is open: nothing started before may be shown, and a
    /// batch waiting to run is dropped.
    pub(super) fn library_opened(&mut self) {
        self.library += 1;
        self.sidebar += 1;
        self.home += 1;
        self.batch_pending = false;
    }

    /// Ask for a batched recount. `Some(library)` when the caller must
    /// schedule one (pass the value back to [`Self::take_batch`]); `None` when
    /// one is already waiting and covers this change too.
    pub(super) fn request_batch(&mut self) -> Option<u64> {
        if self.batch_pending {
            return None;
        }
        self.batch_pending = true;
        Some(self.library)
    }

    /// The scheduled batch is due: whether to run it. A batch scheduled for
    /// another library never runs.
    pub(super) fn take_batch(&mut self, library: u64) -> bool {
        if library != self.library || !self.batch_pending {
            return false;
        }
        self.batch_pending = false;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use AlbumChange::*;

    fn r(sidebar: bool, home: bool, timing: Timing) -> Option<AlbumRefresh> {
        Some(AlbumRefresh { sidebar, home, timing })
    }

    #[test]
    fn each_change_refreshes_its_surfaces() {
        use Timing::*;
        for (change, home_shown, has_smart, expected) in [
            (Edited, false, false, r(true, false, Now)),
            (Edited, true, false, r(true, true, Now)),
            (PhotosChanged, true, true, r(true, true, Batched)),
            (PhotosChanged, false, true, r(true, false, Batched)),
            // Without Smart Albums photo changes move no album counts.
            (PhotosChanged, true, false, None),
            (BrowseChanged, true, true, r(false, true, Worker)),
            // The sidebar counts whole albums; a hidden home reloads when shown.
            (BrowseChanged, false, true, None),
            (Appearance, false, false, r(true, false, Now)),
            (Appearance, true, false, r(true, true, Now)),
            (Shown, false, false, r(false, true, Now)),
            (Shown, true, true, r(false, true, Now)),
            (LibraryOpened, false, false, r(true, false, Now)),
            (LibraryOpened, true, true, r(true, true, Now)),
        ] {
            assert_eq!(plan(change, home_shown, has_smart), expected, "{change:?} shown={home_shown} smart={has_smart}");
        }
    }

    fn both(timing: Timing) -> AlbumRefresh {
        AlbumRefresh { sidebar: true, home: true, timing }
    }

    #[test]
    fn a_newer_refresh_makes_older_results_stale_per_surface() {
        let mut state = AlbumListState::default();
        let recount = state.begin(both(Timing::Batched));
        assert!(state.sidebar_current(recount) && state.home_current(recount));
        // A browse change while the recount counts: its home list is stale,
        // its sidebar list is not.
        let browse = state.begin(AlbumRefresh { sidebar: false, home: true, timing: Timing::Worker });
        assert!(state.sidebar_current(recount));
        assert!(!state.home_current(recount));
        assert!(state.home_current(browse));
        // An album edit applied now: every older result is stale.
        let edit = state.begin(both(Timing::Now));
        assert!(!state.sidebar_current(recount));
        assert!(!state.home_current(browse));
        assert!(state.sidebar_current(edit) && state.home_current(edit));
    }

    #[test]
    fn another_library_drops_running_results_and_waiting_batches() {
        let mut state = AlbumListState::default();
        let recount = state.begin(both(Timing::Batched));
        let waiting = state.request_batch().expect("schedule a batch");
        state.library_opened();
        assert!(!state.sidebar_current(recount) && !state.home_current(recount));
        assert!(!state.take_batch(waiting), "a batch for the old library never runs");
        // The new library batches again; the old timer firing late changes nothing.
        let fresh = state.request_batch().expect("schedule a batch");
        assert!(!state.take_batch(waiting));
        assert!(state.take_batch(fresh));
    }

    #[test]
    fn one_batch_covers_a_burst_of_changes() {
        let mut state = AlbumListState::default();
        let first = state.request_batch().expect("schedule a batch");
        assert_eq!(state.request_batch(), None);
        assert_eq!(state.request_batch(), None);
        assert!(state.take_batch(first));
        assert!(!state.take_batch(first), "it runs once");
        assert!(state.request_batch().is_some(), "the next change batches again");
    }
}
