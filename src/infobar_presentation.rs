//! InfoBar presentation: what the bottom InfoBar shows. Pure (no GTK).
//!
//! The InfoBar shows one of three things: nothing, one photo's metadata, or
//! the multi-selection strip (count, thumbnails, grouped actions). Callers
//! report what happened as `InfoBarEvent`s; `InfoBarPresentation::target`
//! says what to show. The InfoBar is the adapter that writes its existing
//! widgets from that target.
//!
//! The transitions keep the InfoBar's established behaviour exactly, for
//! example a one-photo selection keeps whichever photo was last shown, and
//! closing the lightbox re-applies the gallery selection.

pub(crate) type PhotoId = i64;

/// The gallery's multi-selection: its size and a bounded sample of ids for
/// the strip's thumbnails (a selection of thousands only samples a few).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct SelectionSample {
    pub(crate) count: usize,
    pub(crate) ids: Vec<PhotoId>,
}

impl SelectionSample {
    pub(crate) fn new(count: usize, ids: Vec<PhotoId>) -> Self {
        Self { count, ids }
    }
}

/// The main stack page on screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum InfoBarPage {
    Photos,
    Collage,
    /// Any other page (library home, editor, ...).
    Other,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum InfoBarEvent {
    /// The gallery's focused photo changed (`None`: focus cleared, for
    /// example while the gallery model is replaced).
    GalleryFocus(Option<PhotoId>),
    /// The gallery's selection changed.
    GallerySelection(SelectionSample),
    /// The date browser handed the gallery selection back: its first
    /// selected photo becomes the shown (current) photo.
    DateBrowserSelection(SelectionSample),
    LightboxShown(bool),
    /// The lightbox moved to another photo.
    LightboxPhoto(PhotoId),
    /// The date browser's year/month overview covers the gallery.
    DateOverviewShown(bool),
    Page(InfoBarPage),
    /// A photo's facts changed (rating, favourite, rotation, edit, rename,
    /// rescan); it becomes the shown photo.
    PhotoUpdated(PhotoId),
    /// No photo to show (library switched, photo trashed, startup).
    PhotoCleared,
}

/// Which InfoBar page is visible.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum InfoBarMode {
    /// Photo metadata, or nothing when there is no photo.
    Metadata,
    /// The multi-selection strip.
    Selection(SelectionSample),
}

/// Everything the InfoBar writes. The photo is kept while the selection
/// strip is visible: the photo actions stay enabled for it. `photo` is also
/// the window's current photo (`InfoBar::current_photo`), the one every
/// photo action anchors on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct InfoBarTarget {
    pub(crate) mode: InfoBarMode,
    pub(crate) photo: Option<PhotoId>,
    pub(crate) collage: bool,
}

impl InfoBarTarget {
    /// Favourite, rating, album, rotate, export and print.
    pub(crate) fn photo_actions_enabled(&self) -> bool {
        self.photo.is_some()
    }

    pub(crate) fn edit_enabled(&self) -> bool {
        self.photo.is_some() && !self.collage
    }

    /// Layout toggle and the shared zoom slider; Collage has no zoom.
    pub(crate) fn gallery_controls_enabled(&self) -> bool {
        !self.collage
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct InfoBarPresentation {
    photo: Option<PhotoId>,
    strip: Option<SelectionSample>,
    selection: SelectionSample,
    lightbox: bool,
    overview: bool,
    collage: bool,
}

impl InfoBarPresentation {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn target(&self) -> InfoBarTarget {
        InfoBarTarget {
            mode: match &self.strip {
                Some(selection) => InfoBarMode::Selection(selection.clone()),
                None => InfoBarMode::Metadata,
            },
            photo: self.photo,
            collage: self.collage,
        }
    }

    pub(crate) fn handle(&mut self, event: InfoBarEvent) {
        match event {
            InfoBarEvent::GalleryFocus(photo) => {
                // A hidden gallery refresh must not replace the photo the
                // lightbox or the date overview presents.
                if self.covered() {
                    return;
                }
                // Selection notifications can be elided when the selected
                // index stays the same, so a cleared focus also leaves the
                // strip.
                if photo.is_none() {
                    self.strip = None;
                }
                self.photo = photo;
            }
            InfoBarEvent::GallerySelection(selection) => {
                self.selection = selection;
                if !self.covered() {
                    self.apply_selection();
                }
            }
            InfoBarEvent::DateBrowserSelection(selection) => {
                self.selection = selection;
                self.apply_selection();
                // The first selected photo becomes current, whatever the
                // count: the strip's actions anchor on it.
                if let Some(first) = self.selection.ids.first() {
                    self.photo = Some(*first);
                }
            }
            InfoBarEvent::LightboxShown(true) => {
                self.lightbox = true;
                self.strip = None;
            }
            InfoBarEvent::LightboxShown(false) => {
                self.lightbox = false;
                self.apply_selection();
            }
            InfoBarEvent::LightboxPhoto(photo) => {
                self.photo = Some(photo);
                self.strip = None;
            }
            InfoBarEvent::DateOverviewShown(true) => {
                self.overview = true;
                self.strip = None;
                self.photo = None;
            }
            InfoBarEvent::DateOverviewShown(false) => self.overview = false,
            InfoBarEvent::Page(page) => {
                self.collage = page == InfoBarPage::Collage;
                match page {
                    InfoBarPage::Photos => self.apply_selection(),
                    InfoBarPage::Collage => self.strip = None,
                    InfoBarPage::Other => {}
                }
            }
            InfoBarEvent::PhotoUpdated(photo) => self.photo = Some(photo),
            InfoBarEvent::PhotoCleared => self.photo = None,
        }
    }

    fn covered(&self) -> bool {
        self.lightbox || self.overview
    }

    /// Show the last reported gallery selection. Collage never shows the
    /// strip; an empty selection also clears the photo.
    fn apply_selection(&mut self) {
        match self.selection.count {
            0 => {
                self.strip = None;
                self.photo = None;
            }
            1 => self.strip = None,
            _ if self.collage => self.strip = None,
            _ => self.strip = Some(self.selection.clone()),
        }
    }
}

/// The photo facts the metadata page shows, as plain values.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct PhotoFacts {
    pub(crate) filename: String,
    pub(crate) taken_at: Option<String>,
    pub(crate) camera: Option<String>,
    pub(crate) width: i64,
    pub(crate) height: i64,
    pub(crate) size_bytes: i64,
    pub(crate) aperture: f64,
    pub(crate) iso: i64,
    pub(crate) shutter_speed: f64,
    pub(crate) focal_length: f64,
    pub(crate) rating: i32,
    pub(crate) favorite: bool,
}

/// What the metadata page writes for one photo. Comparing two summaries
/// tells the adapter whether any widget needs writing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PhotoSummary {
    pub(crate) filename: String,
    /// Taken, camera, dimensions, size, aperture, ISO, shutter, focal
    /// length. An absent optional metric is an empty string.
    pub(crate) metrics: [String; 8],
    /// Clamped to 0..=5.
    pub(crate) rating: i32,
    pub(crate) favorite: bool,
}

impl PhotoSummary {
    pub(crate) fn of(facts: &PhotoFacts) -> Self {
        // An unknown fact is an empty string, which hides its metric: the
        // InfoBar shows only what the photo actually records (issue #269).
        let dimensions = if facts.width > 0 && facts.height > 0 {
            format!("{} × {}", facts.width, facts.height)
        } else {
            String::new()
        };
        let camera = facts
            .camera
            .clone()
            .filter(|camera| !camera.trim().is_empty())
            .unwrap_or_default();
        let taken = facts
            .taken_at
            .as_deref()
            .filter(|taken| !taken.trim().is_empty())
            .map(super::format_date)
            .unwrap_or_default();
        let size = if facts.size_bytes > 0 {
            super::format_size(facts.size_bytes)
        } else {
            String::new()
        };
        Self {
            filename: facts.filename.clone(),
            metrics: [
                taken,
                camera,
                dimensions,
                size,
                super::format_aperture(facts.aperture).unwrap_or_default(),
                if facts.iso > 0 {
                    facts.iso.to_string()
                } else {
                    String::new()
                },
                crate::window::format_shutter_speed(facts.shutter_speed).unwrap_or_default(),
                crate::window::format_focal_length(facts.focal_length).unwrap_or_default(),
            ],
            rating: facts.rating.clamp(0, 5),
            favorite: facts.favorite,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use InfoBarEvent::*;

    fn sel(count: usize, ids: &[PhotoId]) -> SelectionSample {
        SelectionSample::new(count, ids.to_vec())
    }

    fn metadata(photo: Option<PhotoId>) -> InfoBarTarget {
        InfoBarTarget {
            mode: InfoBarMode::Metadata,
            photo,
            collage: false,
        }
    }

    fn strip(count: usize, ids: &[PhotoId], photo: Option<PhotoId>) -> InfoBarTarget {
        InfoBarTarget {
            mode: InfoBarMode::Selection(sel(count, ids)),
            photo,
            collage: false,
        }
    }

    fn run(events: &[InfoBarEvent]) -> InfoBarTarget {
        let mut presentation = InfoBarPresentation::new();
        for event in events {
            presentation.handle(event.clone());
        }
        presentation.target()
    }

    fn check(cases: &[(&str, Vec<InfoBarEvent>, InfoBarTarget)]) {
        for (name, events, expected) in cases {
            assert_eq!(&run(events), expected, "case: {name}");
        }
    }

    #[test]
    fn empty_states() {
        check(&[
            ("startup shows nothing", vec![], metadata(None)),
            (
                "an empty selection clears the shown photo",
                vec![GalleryFocus(Some(1)), GallerySelection(sel(0, &[]))],
                metadata(None),
            ),
            (
                "a cleared focus leaves a stale strip",
                vec![GallerySelection(sel(3, &[1, 2, 3])), GalleryFocus(None)],
                metadata(None),
            ),
            (
                "clearing the photo keeps the strip",
                vec![
                    GalleryFocus(Some(1)),
                    GallerySelection(sel(2, &[1, 2])),
                    PhotoCleared,
                ],
                strip(2, &[1, 2], None),
            ),
            (
                "the date overview shows nothing",
                vec![
                    GalleryFocus(Some(1)),
                    GallerySelection(sel(2, &[1, 2])),
                    DateOverviewShown(true),
                ],
                metadata(None),
            ),
            (
                "leaving the overview without a sync stays empty",
                vec![
                    GalleryFocus(Some(1)),
                    DateOverviewShown(true),
                    DateOverviewShown(false),
                ],
                metadata(None),
            ),
            (
                "an empty date browser selection shows nothing",
                vec![GalleryFocus(Some(4)), DateBrowserSelection(sel(0, &[]))],
                metadata(None),
            ),
        ]);
    }

    #[test]
    fn selection_changes() {
        check(&[
            (
                "one selected photo shows its metadata",
                vec![GalleryFocus(Some(7)), GallerySelection(sel(1, &[7]))],
                metadata(Some(7)),
            ),
            (
                "two or more show the strip and keep the photo",
                vec![GalleryFocus(Some(7)), GallerySelection(sel(2, &[7, 8]))],
                strip(2, &[7, 8], Some(7)),
            ),
            (
                "a growing selection updates the strip",
                vec![
                    GalleryFocus(Some(7)),
                    GallerySelection(sel(2, &[7, 8])),
                    GallerySelection(sel(500, &[7, 8, 9])),
                ],
                strip(500, &[7, 8, 9], Some(7)),
            ),
            (
                "shrinking to one returns to metadata",
                vec![
                    GalleryFocus(Some(7)),
                    GallerySelection(sel(2, &[7, 8])),
                    GallerySelection(sel(1, &[7])),
                ],
                metadata(Some(7)),
            ),
            (
                "focus moving inside a multi-selection keeps the strip",
                vec![GallerySelection(sel(2, &[7, 8])), GalleryFocus(Some(8))],
                strip(2, &[7, 8], Some(8)),
            ),
            (
                "a single date browser selection becomes the shown photo",
                vec![GalleryFocus(Some(1)), DateBrowserSelection(sel(1, &[9]))],
                metadata(Some(9)),
            ),
            (
                "a date browser multi-selection shows the strip",
                vec![
                    GalleryFocus(Some(1)),
                    DateBrowserSelection(sel(3, &[1, 2, 3])),
                ],
                strip(3, &[1, 2, 3], Some(1)),
            ),
            (
                // The current photo the actions anchor on (candidate 10):
                // the date browser hands back its first selected photo.
                "a date browser multi-selection makes its first photo current",
                vec![
                    GalleryFocus(Some(1)),
                    DateBrowserSelection(sel(3, &[4, 5, 6])),
                ],
                strip(3, &[4, 5, 6], Some(4)),
            ),
            (
                "a multi-selection after the date overview has a current photo",
                vec![
                    GalleryFocus(Some(1)),
                    DateOverviewShown(true),
                    DateOverviewShown(false),
                    DateBrowserSelection(sel(2, &[4, 5])),
                ],
                strip(2, &[4, 5], Some(4)),
            ),
        ]);
    }

    #[test]
    fn photo_changes() {
        check(&[
            (
                "gallery focus shows the photo",
                vec![GalleryFocus(Some(3))],
                metadata(Some(3)),
            ),
            (
                "an updated photo becomes the shown photo",
                vec![GalleryFocus(Some(3)), PhotoUpdated(3)],
                metadata(Some(3)),
            ),
            (
                "an update keeps the strip",
                vec![GallerySelection(sel(2, &[3, 4])), PhotoUpdated(3)],
                strip(2, &[3, 4], Some(3)),
            ),
        ]);
    }

    #[test]
    fn lightbox_integration() {
        check(&[
            (
                "opening the lightbox leaves the strip",
                vec![
                    GalleryFocus(Some(1)),
                    GallerySelection(sel(2, &[1, 2])),
                    LightboxShown(true),
                ],
                metadata(Some(1)),
            ),
            (
                "lightbox navigation shows its photo",
                vec![GalleryFocus(Some(1)), LightboxShown(true), LightboxPhoto(5)],
                metadata(Some(5)),
            ),
            (
                "hidden gallery focus does not replace the lightbox photo",
                vec![LightboxShown(true), LightboxPhoto(5), GalleryFocus(Some(1))],
                metadata(Some(5)),
            ),
            (
                "hidden gallery selection is not shown while the lightbox is open",
                vec![
                    LightboxShown(true),
                    LightboxPhoto(5),
                    GallerySelection(sel(2, &[1, 2])),
                ],
                metadata(Some(5)),
            ),
            (
                "closing with one selected keeps the lightbox's photo",
                vec![
                    GallerySelection(sel(1, &[1])),
                    LightboxShown(true),
                    LightboxPhoto(5),
                    LightboxShown(false),
                ],
                metadata(Some(5)),
            ),
            (
                "closing restores the multi-selection strip",
                vec![
                    GallerySelection(sel(2, &[1, 2])),
                    LightboxShown(true),
                    LightboxPhoto(5),
                    LightboxShown(false),
                ],
                strip(2, &[1, 2], Some(5)),
            ),
            (
                "a selection that changed while hidden is used on close",
                vec![
                    GallerySelection(sel(2, &[1, 2])),
                    LightboxShown(true),
                    GallerySelection(sel(0, &[])),
                    LightboxShown(false),
                ],
                metadata(None),
            ),
            (
                "hidden gallery focus is ignored under the date overview",
                vec![DateOverviewShown(true), GalleryFocus(Some(2))],
                metadata(None),
            ),
        ]);
    }

    #[test]
    fn collage_and_pages() {
        let collage = |mode, photo| InfoBarTarget {
            mode,
            photo,
            collage: true,
        };
        check(&[
            (
                "collage leaves the strip and disables edit",
                vec![
                    GalleryFocus(Some(1)),
                    GallerySelection(sel(2, &[1, 2])),
                    Page(InfoBarPage::Collage),
                ],
                collage(InfoBarMode::Metadata, Some(1)),
            ),
            (
                "collage never shows the strip",
                vec![
                    Page(InfoBarPage::Collage),
                    GalleryFocus(Some(1)),
                    GallerySelection(sel(3, &[1, 2, 3])),
                ],
                collage(InfoBarMode::Metadata, Some(1)),
            ),
            (
                "returning to Photos restores the strip",
                vec![
                    GalleryFocus(Some(1)),
                    GallerySelection(sel(2, &[1, 2])),
                    Page(InfoBarPage::Collage),
                    Page(InfoBarPage::Photos),
                ],
                strip(2, &[1, 2], Some(1)),
            ),
            (
                "another page keeps what was shown",
                vec![
                    GalleryFocus(Some(1)),
                    Page(InfoBarPage::Collage),
                    Page(InfoBarPage::Other),
                ],
                metadata(Some(1)),
            ),
        ]);
        let edit = run(&[Page(InfoBarPage::Collage), GalleryFocus(Some(1))]);
        assert!(edit.photo_actions_enabled());
        assert!(!edit.edit_enabled());
        assert!(!edit.gallery_controls_enabled());
        let none = run(&[]);
        assert!(!none.photo_actions_enabled() && !none.edit_enabled());
        assert!(none.gallery_controls_enabled());
    }

    #[test]
    fn photo_summary_handles_missing_metadata() {
        let full = PhotoFacts {
            filename: "DSC_0001.NEF".into(),
            taken_at: Some("2024-05-03 14-22-10".into()),
            camera: Some("NIKON Z 6".into()),
            width: 6048,
            height: 4024,
            size_bytes: 24_500_000,
            aperture: 2.8,
            iso: 400,
            shutter_speed: 1.0 / 250.0,
            focal_length: 50.0,
            rating: 4,
            favorite: true,
        };
        let cases: Vec<(&str, PhotoFacts, [&str; 8], i32)> = vec![
            (
                "complete metadata",
                full.clone(),
                [
                    "03 May 2024",
                    "NIKON Z 6",
                    "6048 × 4024",
                    "24.5 MB",
                    "f/2.8",
                    "400",
                    "1/250 s",
                    "50 mm",
                ],
                4,
            ),
            (
                "no metadata at all",
                PhotoFacts {
                    filename: "scan.png".into(),
                    ..PhotoFacts::default()
                },
                [
                    "",
                    "",
                    "",
                    "",
                    "",
                    "",
                    "",
                    "",
                ],
                0,
            ),
            (
                "one missing dimension",
                PhotoFacts {
                    height: 0,
                    ..full.clone()
                },
                [
                    "03 May 2024",
                    "NIKON Z 6",
                    "",
                    "24.5 MB",
                    "f/2.8",
                    "400",
                    "1/250 s",
                    "50 mm",
                ],
                4,
            ),
            (
                "unreadable exposure values",
                PhotoFacts {
                    aperture: f64::NAN,
                    iso: -1,
                    shutter_speed: -1.0,
                    focal_length: f64::INFINITY,
                    ..full.clone()
                },
                [
                    "03 May 2024",
                    "NIKON Z 6",
                    "6048 × 4024",
                    "24.5 MB",
                    "",
                    "",
                    "",
                    "",
                ],
                4,
            ),
            (
                "an unparsed date is shown as stored",
                PhotoFacts {
                    taken_at: Some("sometime in 1999".into()),
                    rating: 9,
                    ..full.clone()
                },
                [
                    "sometime in 1999",
                    "NIKON Z 6",
                    "6048 × 4024",
                    "24.5 MB",
                    "f/2.8",
                    "400",
                    "1/250 s",
                    "50 mm",
                ],
                5,
            ),
        ];
        for (name, facts, metrics, rating) in cases {
            let summary = PhotoSummary::of(&facts);
            assert_eq!(summary.metrics, metrics.map(String::from), "case: {name}");
            assert_eq!(summary.rating, rating, "case: {name}");
            assert_eq!(summary.filename, facts.filename, "case: {name}");
            assert_eq!(summary.favorite, facts.favorite, "case: {name}");
        }
        assert_eq!(
            PhotoSummary::of(&full),
            PhotoSummary::of(&full.clone()),
            "equal facts give equal summaries, so the adapter can skip writes"
        );
    }
}
