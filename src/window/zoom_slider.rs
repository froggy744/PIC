//! Zoom slider: what the InfoBar's shared zoom slider shows, decided in one
//! place and free of GTK.
//!
//! The one physical slider serves three surfaces. Over the gallery it picks
//! a thumbnail size (one of `grid::ZOOM_LEVELS`); over the photo viewer and
//! the editor it is a zoom from Fit (left) through 100% (middle) to
//! `LIGHTBOX_MAX_ZOOM_FACTOR` (right). [`mode_for`] picks the mode from the
//! front surface, [`spec`] gives its range, steps and tooltip, and the
//! conversions map between slider values and widths or scales. The GTK
//! adapter (`zoom_slider_widgets.rs`) applies them.

use super::front_surface::Front;
use crate::grid;
use crate::lightbox::LIGHTBOX_MAX_ZOOM_FACTOR;

/// What the slider controls.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SliderMode {
    /// Gallery thumbnail size.
    Thumbnails,
    /// The photo viewer's zoom.
    Photo,
    /// The editor's zoom.
    Edit,
}

/// The mode for the surface in front.
pub(super) fn mode_for(front: Front) -> SliderMode {
    match front {
        Front::Lightbox => SliderMode::Photo,
        Front::Editor => SliderMode::Edit,
        _ => SliderMode::Thumbnails,
    }
}

/// How the slider is set up in one mode.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct SliderSpec {
    pub(super) lower: f64,
    pub(super) upper: f64,
    pub(super) step: f64,
    pub(super) page: f64,
    pub(super) round_digits: i32,
    pub(super) tooltip: &'static str,
}

pub(super) fn spec(mode: SliderMode) -> SliderSpec {
    match mode {
        SliderMode::Thumbnails => SliderSpec {
            lower: 0.0,
            upper: (grid::ZOOM_LEVELS.len() - 1) as f64,
            step: 1.0,
            page: 1.0,
            round_digits: 0,
            tooltip: "Thumbnail size (Ctrl + wheel)",
        },
        SliderMode::Photo => SliderSpec {
            lower: 0.0,
            upper: 100.0,
            step: 0.25,
            page: 5.0,
            round_digits: -1,
            tooltip: "Photo zoom — left: Fit, middle: 100%, right: 200%",
        },
        SliderMode::Edit => SliderSpec {
            tooltip: "Edit zoom — left: Fit, middle: 100%, right: 200%",
            ..spec(SliderMode::Photo)
        },
    }
}

/// How far one wheel notch over the slider moves it.
pub(super) fn wheel_step(mode: SliderMode) -> f64 {
    match mode {
        SliderMode::Thumbnails => 1.0,
        SliderMode::Photo => 20.0,
        SliderMode::Edit => 10.0,
    }
}

/// The slider value (a `ZOOM_LEVELS` index) for a thumbnail width.
pub(super) fn thumbnail_value(width: i32) -> f64 {
    let level = grid::nearest_zoom_level(width);
    grid::ZOOM_LEVELS
        .iter()
        .position(|candidate| *candidate == level)
        .unwrap_or(0) as f64
}

/// The thumbnail width for a slider value (rounded to the nearest level).
pub(super) fn thumbnail_width(value: f64) -> i32 {
    let last = grid::ZOOM_LEVELS.len().saturating_sub(1);
    let index = (value.round() as isize).clamp(0, last as isize) as usize;
    grid::ZOOM_LEVELS[index]
}

/// The slider value for a viewer or editor zoom: `scale` is the zoom (1.0 is
/// 100%), `fit_scale` the zoom at which the photo fits. Fit is 0, 100% is
/// 50, the maximum is 100. When Fit is already 100% or more (a small photo),
/// the whole slider runs from 100% to the maximum.
pub(super) fn photo_value(scale: f64, fit_scale: f64) -> f64 {
    if scale <= 0.0 {
        return 0.0;
    }
    let fit_scale = fit_scale.clamp(f64::EPSILON, 1.0);
    if fit_scale >= 0.999 {
        return 100.0 * ((scale - 1.0) / (LIGHTBOX_MAX_ZOOM_FACTOR - 1.0)).clamp(0.0, 1.0);
    }
    if scale <= 1.0 {
        50.0 * ((scale - fit_scale) / (1.0 - fit_scale)).clamp(0.0, 1.0)
    } else {
        50.0 + 50.0 * ((scale - 1.0) / (LIGHTBOX_MAX_ZOOM_FACTOR - 1.0)).clamp(0.0, 1.0)
    }
}

/// The zoom for a slider value; the inverse of [`photo_value`]. 0 returns
/// 0.0, which the viewer and editor read as Fit.
pub(super) fn photo_scale(value: f64, fit_scale: f64) -> f64 {
    let value = value.clamp(0.0, 100.0);
    if value <= 0.0 {
        return 0.0;
    }
    let fit_scale = fit_scale.clamp(f64::EPSILON, 1.0);
    if fit_scale >= 0.999 {
        // Fit and 1:1 are the same visual state. Do not waste half the
        // physical slider on a dead Fit->100% range.
        return 1.0 + (LIGHTBOX_MAX_ZOOM_FACTOR - 1.0) * (value / 100.0);
    }
    if value <= 50.0 {
        fit_scale + (1.0 - fit_scale) * (value / 50.0)
    } else {
        1.0 + (LIGHTBOX_MAX_ZOOM_FACTOR - 1.0) * ((value - 50.0) / 50.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_front_picks_the_mode() {
        let rows = [
            (Front::Lightbox, SliderMode::Photo),
            (Front::Editor, SliderMode::Edit),
            (Front::Gallery, SliderMode::Thumbnails),
            (Front::DateOverview, SliderMode::Thumbnails),
            (Front::LibraryHome, SliderMode::Thumbnails),
            (Front::Albums, SliderMode::Thumbnails),
            (Front::FoldersOverview, SliderMode::Thumbnails),
            (Front::Collage, SliderMode::Thumbnails),
            (Front::None, SliderMode::Thumbnails),
        ];
        for (front, mode) in rows {
            assert_eq!(mode_for(front), mode, "{front:?}");
        }
    }

    #[test]
    fn each_mode_has_its_range_steps_and_tooltip() {
        assert_eq!(spec(SliderMode::Thumbnails), SliderSpec {
            lower: 0.0, upper: 7.0, step: 1.0, page: 1.0, round_digits: 0,
            tooltip: "Thumbnail size (Ctrl + wheel)",
        });
        assert_eq!(spec(SliderMode::Photo), SliderSpec {
            lower: 0.0, upper: 100.0, step: 0.25, page: 5.0, round_digits: -1,
            tooltip: "Photo zoom — left: Fit, middle: 100%, right: 200%",
        });
        assert_eq!(spec(SliderMode::Edit), SliderSpec {
            lower: 0.0, upper: 100.0, step: 0.25, page: 5.0, round_digits: -1,
            tooltip: "Edit zoom — left: Fit, middle: 100%, right: 200%",
        });
        // The thumbnail range has one stop per zoom level.
        assert_eq!(spec(SliderMode::Thumbnails).upper as usize, grid::ZOOM_LEVELS.len() - 1);
        assert_eq!(wheel_step(SliderMode::Thumbnails), 1.0);
        assert_eq!(wheel_step(SliderMode::Photo), 20.0);
        assert_eq!(wheel_step(SliderMode::Edit), 10.0);
    }

    #[test]
    fn thumbnail_widths_map_to_zoom_level_stops() {
        // ZOOM_LEVELS = [100, 117, 137, 160, 187, 219, 256, 300]
        for (width, value) in [(100, 0.0), (117, 1.0), (160, 3.0), (300, 7.0)] {
            assert_eq!(thumbnail_value(width), value, "width {width}");
        }
        for (value, width) in [(0.0, 100), (3.4, 160), (3.5, 187), (7.0, 300), (12.0, 300), (-1.0, 100)] {
            assert_eq!(thumbnail_width(value), width, "value {value}");
        }
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn photo_zoom_maps_fit_100_percent_and_maximum() {
        // A photo that fits at 50%: Fit 0, 75% 25, 100% 50, 150% 75, 200% 100.
        let rows = [
            (0.0, 0.0), (0.25, 0.0), (0.5, 0.0), (0.75, 25.0), (1.0, 50.0),
            (1.5, 75.0), (2.0, 100.0), (3.0, 100.0),
        ];
        for (scale, value) in rows {
            assert!(close(photo_value(scale, 0.5), value), "scale {scale}: {}", photo_value(scale, 0.5));
        }
        // A small photo that fits at 100%: the slider runs 100%..200%.
        for (scale, value) in [(1.0, 0.0), (1.5, 50.0), (2.0, 100.0)] {
            assert!(close(photo_value(scale, 1.0), value), "small photo, scale {scale}");
        }
    }

    #[test]
    fn slider_values_map_back_to_zoom() {
        let rows = [
            (-5.0, 0.0), (0.0, 0.0), (25.0, 0.75), (50.0, 1.0), (75.0, 1.5),
            (100.0, 2.0), (150.0, 2.0),
        ];
        for (value, scale) in rows {
            assert!(close(photo_scale(value, 0.5), scale), "value {value}: {}", photo_scale(value, 0.5));
        }
        for (value, scale) in [(0.0, 0.0), (50.0, 1.5), (100.0, 2.0)] {
            assert!(close(photo_scale(value, 1.0), scale), "small photo, value {value}");
        }
        // Round trip inside the range.
        for value in [10.0, 37.5, 50.0, 62.0, 99.0] {
            assert!(close(photo_value(photo_scale(value, 0.4), 0.4), value), "round trip {value}");
        }
    }
}
