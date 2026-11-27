//! Metadata panel presentation (candidate 24): whether the filter panel and
//! its header toggle are shown, decided from plain inputs.
//!
//! The GTK adapter is [`super::metadata_panel::MetadataPanel`]; its
//! `sync_view` is the only writer of the panel's `root` visibility. Immersive
//! Mode, the viewer and navigation are inputs here, so nobody keeps a copy of
//! what the panel showed before and restores it later.

use crate::sidebar::SidebarFilter;

/// What the panel's visibility depends on. `page` is the main stack page
/// name (`""` before the first navigation).
#[derive(Clone, Copy, Debug)]
pub(super) struct PanelInputs<'a> {
    pub(super) page: &'a str,
    /// The header Filters toggle: the user's intent to see the panel.
    pub(super) toggled: bool,
    /// The panel's pin: it stays toggled across navigation.
    pub(super) pinned: bool,
    pub(super) lightbox_open: bool,
    pub(super) immersive: bool,
}

/// The header Filters toggle is offered on every workspace except the editor
/// and collage.
pub(super) fn toggle_shown(page: &str) -> bool {
    !matches!(page, "edit" | "collage")
}

/// Whether the filter panel takes space above the gallery.
pub(super) fn panel_shown(inputs: &PanelInputs) -> bool {
    // A pinned panel keeps its toggle in the Library and Albums homes, but
    // those pages have no photo grid to filter.
    let pinned_home = inputs.pinned && matches!(inputs.page, "library" | "albums");
    toggle_shown(inputs.page)
        && inputs.toggled
        && !inputs.lightbox_open
        && !inputs.immersive
        && !pinned_home
}

/// Entering a view closes an unpinned panel unless the view is All Photos or
/// a folder. The user can still open it again there by hand.
pub(super) fn closes_on_entry(page: &str, filter: SidebarFilter, pinned: bool) -> bool {
    !pinned
        && (page != "photos" || !matches!(filter, SidebarFilter::All | SidebarFilter::Folder(_)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inputs(page: &str) -> PanelInputs<'_> {
        PanelInputs {
            page,
            toggled: true,
            pinned: false,
            lightbox_open: false,
            immersive: false,
        }
    }

    #[test]
    fn panel_shown_table() {
        // (case, inputs, shown)
        let cases: Vec<(&str, PanelInputs, bool)> = vec![
            ("toggled on photos", inputs("photos"), true),
            ("before the first navigation", inputs(""), true),
            (
                "toggle off",
                PanelInputs {
                    toggled: false,
                    ..inputs("photos")
                },
                false,
            ),
            ("editor", inputs("edit"), false),
            ("collage", inputs("collage"), false),
            (
                "viewer open",
                PanelInputs {
                    lightbox_open: true,
                    ..inputs("photos")
                },
                false,
            ),
            (
                "immersive",
                PanelInputs {
                    immersive: true,
                    ..inputs("photos")
                },
                false,
            ),
            (
                "immersive and viewer",
                PanelInputs {
                    immersive: true,
                    lightbox_open: true,
                    ..inputs("photos")
                },
                false,
            ),
            ("unpinned in library", inputs("library"), true),
            (
                "pinned in library",
                PanelInputs {
                    pinned: true,
                    ..inputs("library")
                },
                false,
            ),
            (
                "pinned in albums",
                PanelInputs {
                    pinned: true,
                    ..inputs("albums")
                },
                false,
            ),
            (
                "pinned on photos",
                PanelInputs {
                    pinned: true,
                    ..inputs("photos")
                },
                true,
            ),
        ];
        for (case, inputs, shown) in cases {
            assert_eq!(panel_shown(&inputs), shown, "{case}");
        }
    }

    /// The candidate's bug: the panel was open in All Photos, Immersive Mode
    /// began, navigation closed the toggle, Immersive Mode ended. The panel
    /// follows the toggle, not what it showed before Immersive Mode.
    #[test]
    fn leaving_immersive_follows_the_current_toggle() {
        let before = inputs("photos");
        assert!(panel_shown(&before));
        let during = PanelInputs {
            immersive: true,
            toggled: false,
            page: "albums",
            ..before
        };
        assert!(!panel_shown(&during));
        let after = PanelInputs {
            immersive: false,
            ..during
        };
        assert!(!panel_shown(&after));
    }

    #[test]
    fn toggle_shown_table() {
        for (page, shown) in [
            ("", true),
            ("photos", true),
            ("library", true),
            ("albums", true),
            ("edit", false),
            ("collage", false),
        ] {
            assert_eq!(toggle_shown(page), shown, "{page}");
        }
    }

    #[test]
    fn closes_on_entry_table() {
        use SidebarFilter::*;
        // (page, filter, pinned, closes)
        let cases = [
            ("photos", All, false, false),
            ("photos", Folder(7), false, false),
            ("photos", Favorites, false, true),
            ("photos", Album(3), false, true),
            ("library", All, false, true),
            ("albums", All, false, true),
            ("edit", All, false, true),
            ("photos", Favorites, true, false),
            ("library", All, true, false),
        ];
        for (page, filter, pinned, closes) in cases {
            assert_eq!(
                closes_on_entry(page, filter, pinned),
                closes,
                "{page} {filter:?} pinned={pinned}"
            );
        }
    }
}
