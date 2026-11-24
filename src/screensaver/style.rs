//! Screensaver style descriptors: everything PIC knows about a style, in one
//! place.
//!
//! [`Style::spec`] gives a style's facts (saved key, label, subtitle, default
//! tile count, background, whether it uses photos or only exists in the
//! Terminal theme) and its [`Stage`]. Most numbers depend on the stage, not
//! on the style: the Settings tile limit, how many slots the player fills,
//! and the size originals are decoded at. How each style *draws* lives with
//! its renderer (`gpu/pile.rs`, `gpu/cinema.rs`, the Settings preview).

use super::{DEFAULT_MOSAIC_TILES, FLY_MAX_SLOTS, MAX_WALL_TILES, PRINT_MAX_SLOTS};

/// The family of renderer a style plays on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Stage {
    /// A wall of square tiles (Mosaic, Changing Grid).
    Wall,
    /// Prints painted on one canvas (Picture Pile, Floating Prints, Photo Drift).
    Prints,
    /// Prints flying past the viewer.
    Flying,
    /// Cover Flow cards.
    CoverFlow,
    /// One full-screen photo at a time (Ken Burns, Fade, the Matrix styles).
    Cinema,
}

/// A style's background before any photo is shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Background {
    Colour([u8; 3]),
    /// The theme's window background (Cover Flow).
    Theme,
}

/// What PIC knows about one style.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct StyleSpec {
    /// Saved in the `screensaver-style` setting.
    pub key: &'static str,
    pub label: &'static str,
    pub subtitle: &'static str,
    pub stage: Stage,
    pub default_tiles: usize,
    pub background: Background,
    /// Matrix Rain shows no photos.
    pub uses_photos: bool,
    /// Offered only in the Terminal theme (the Matrix styles).
    pub terminal_only: bool,
}

/// A screensaver style, as offered in Settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum Style {
    Mosaic,
    Grid,
    PicturePile,
    Floating,
    IPhoto,
    Fade,
    CoverFlow,
    Flying,
    PhotoDrift,
    Surprise,
    MatrixRain,
    MatrixKenBurns,
}

impl Style {
    pub(super) const CHOICES: [Self; 12] = [
        Self::Mosaic,
        Self::Grid,
        Self::PicturePile,
        Self::Floating,
        Self::IPhoto,
        Self::Fade,
        Self::CoverFlow,
        Self::Flying,
        Self::PhotoDrift,
        Self::Surprise,
        Self::MatrixRain,
        Self::MatrixKenBurns,
    ];

    pub(super) fn default_tiles(self) -> usize {
        self.spec().default_tiles
    }

    /// The style saved under `key`; an unknown key is Mosaic.
    pub(super) fn from_saved(key: &str) -> Self {
        Self::CHOICES
            .into_iter()
            .find(|style| style.spec().key == key)
            .unwrap_or(Self::Mosaic)
    }

    pub(super) fn key(self) -> &'static str {
        self.spec().key
    }

    pub(super) fn label(self) -> &'static str {
        self.spec().label
    }

    pub(super) fn subtitle(self) -> &'static str {
        self.spec().subtitle
    }

    pub(super) fn surprise_choices(terminal: bool) -> Vec<Self> {
        Self::CHOICES
            .into_iter()
            .filter(|style| *style != Self::Surprise && style.available(terminal))
            .collect()
    }

    pub(super) fn resolved(self, terminal: bool) -> Self {
        if self != Self::Surprise {
            return self;
        }
        let choices = Self::surprise_choices(terminal);
        choices[super::random_seed() as usize % choices.len()]
    }

    /// One photo at a time (Cover Flow and the cinema styles).
    pub(super) fn single_photo(self) -> bool {
        matches!(self.spec().stage, Stage::Cinema | Stage::CoverFlow)
    }

    pub(super) fn matrix(self) -> bool {
        self.spec().terminal_only
    }

    pub(super) fn uses_photos(self) -> bool {
        self.spec().uses_photos
    }

    // Easter egg: follow the white rabbit; Matrix modes only appear in Terminal.
    pub(super) fn available(self, terminal: bool) -> bool {
        terminal || !self.spec().terminal_only
    }

    pub(super) fn for_theme(self, terminal: bool) -> Self {
        if self.available(terminal) {
            self
        } else {
            Self::Mosaic
        }
    }

    /// One row per style, so a new style is one line here.
    #[rustfmt::skip]
    pub(super) fn spec(self) -> StyleSpec {
        const DARK: Background = Background::Colour([9, 11, 16]);
        const MATRIX: Background = Background::Colour([0, 4, 1]);
        let (key, label, subtitle, stage, default_tiles, background) = match self {
            Self::Mosaic => ("mosaic", "Mosaic", "Square Picasa photo wall", Stage::Wall, DEFAULT_MOSAIC_TILES, DARK),
            Self::Grid => ("grid", "Changing Grid", "Full-screen tiled gallery", Stage::Wall, DEFAULT_MOSAIC_TILES, DARK),
            Self::PicturePile => ("pile", "Picture Pile", "Layered scrapbook photographs", Stage::Prints, DEFAULT_MOSAIC_TILES, DARK),
            Self::Floating => ("floating", "Floating Prints", "Scattered framed pictures", Stage::Prints, 12, DARK),
            Self::IPhoto => ("iphoto", "Ken Burns", "Cinematic pan and zoom", Stage::Cinema, DEFAULT_MOSAIC_TILES, DARK),
            Self::Fade => ("fade", "Fullscreen Fade", "Gentle full-screen slideshow", Stage::Cinema, DEFAULT_MOSAIC_TILES, DARK),
            Self::CoverFlow => ("coverflow", "Cover Flow", "Browse photos as a reflective Cover Flow", Stage::CoverFlow, DEFAULT_MOSAIC_TILES, Background::Theme),
            Self::Flying => ("flying", "Flying Photos", "Photographs fly in from the distance and past you", Stage::Flying, 30, Background::Colour([230, 230, 224])),
            Self::PhotoDrift => ("photo-drift", "Photo Drift", "Photos gently drift across a scattered, layered field", Stage::Prints, 30, Background::Colour([235, 232, 224])),
            // Resolved to a real style before the player starts; in Settings
            // it behaves like a wall.
            Self::Surprise => ("surprise", "Surprise Me", "Random style on launch", Stage::Wall, DEFAULT_MOSAIC_TILES, DARK),
            Self::MatrixRain => ("matrix-rain", "Matrix Rain", "Animated Katakana rain on black, no photos", Stage::Cinema, DEFAULT_MOSAIC_TILES, MATRIX),
            Self::MatrixKenBurns => ("matrix-ken-burns", "Matrix Ken Burns", "Green photo with fixed Katakana streams and slow pan and zoom", Stage::Cinema, DEFAULT_MOSAIC_TILES, MATRIX),
        };
        StyleSpec {
            key,
            label,
            subtitle,
            stage,
            default_tiles,
            background,
            uses_photos: self != Self::MatrixRain,
            terminal_only: matches!(self, Self::MatrixRain | Self::MatrixKenBurns),
        }
    }
}

impl Stage {
    /// The most tiles Settings offers for a style on this stage.
    pub(super) fn tile_limit(self) -> usize {
        match self {
            Self::Prints => PRINT_MAX_SLOTS,
            Self::Flying => FLY_MAX_SLOTS,
            Self::Wall | Self::CoverFlow | Self::Cinema => MAX_WALL_TILES,
        }
    }

    /// How many photo slots the player fills for `tiles` chosen in Settings.
    pub(super) fn slots(self, tiles: usize) -> usize {
        match self {
            Self::Wall | Self::Prints | Self::Flying => tiles.clamp(1, self.tile_limit()),
            // One photo at a time; the player keeps no tile slots.
            Self::CoverFlow | Self::Cinema => 0,
        }
    }

    /// The size an original is decoded at when `tiles` are on screen.
    pub(super) fn decode_size(self, tiles: usize) -> (u32, u32) {
        match self {
            Self::Wall => {
                let edge = match tiles {
                    ..=6 => 1536,
                    ..=12 => 1152,
                    ..=30 => 896,
                    ..=100 => 640,
                    ..=400 => 384,
                    ..=1600 => 192,
                    ..=6400 => 96,
                    _ => 48,
                };
                (edge, edge)
            }
            // Prints are small when there are many of them.
            Self::Prints => match tiles {
                ..=6 => (1920, 1280),
                ..=16 => (1280, 860),
                ..=36 => (960, 640),
                _ => (640, 430),
            },
            // Flying prints grow until they pass the lens, so they are decoded
            // close to display size; the flight field is capped, which keeps
            // the number of live textures bounded.
            Self::Flying => {
                if tiles <= 6 {
                    (1920, 1280)
                } else {
                    (1440, 960)
                }
            }
            // A few large cards, or one photo: display resolution, not the
            // low-resolution preview path.
            Self::CoverFlow | Self::Cinema => (2560, 1600),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEFAULT_BACKGROUND: Background = Background::Colour([9, 11, 16]);
    const MATRIX_BACKGROUND: Background = Background::Colour([0, 4, 1]);

    /// (style, key, label, stage, default tiles, background, photos, Terminal only)
    #[rustfmt::skip]
    const SPECS: &[(Style, &str, &str, Stage, usize, Background, bool, bool)] = &[
        (Style::Mosaic, "mosaic", "Mosaic", Stage::Wall, 6, DEFAULT_BACKGROUND, true, false),
        (Style::Grid, "grid", "Changing Grid", Stage::Wall, 6, DEFAULT_BACKGROUND, true, false),
        (Style::PicturePile, "pile", "Picture Pile", Stage::Prints, 6, DEFAULT_BACKGROUND, true, false),
        (Style::Floating, "floating", "Floating Prints", Stage::Prints, 12, DEFAULT_BACKGROUND, true, false),
        (Style::IPhoto, "iphoto", "Ken Burns", Stage::Cinema, 6, DEFAULT_BACKGROUND, true, false),
        (Style::Fade, "fade", "Fullscreen Fade", Stage::Cinema, 6, DEFAULT_BACKGROUND, true, false),
        (Style::CoverFlow, "coverflow", "Cover Flow", Stage::CoverFlow, 6, Background::Theme, true, false),
        (Style::Flying, "flying", "Flying Photos", Stage::Flying, 30, Background::Colour([230, 230, 224]), true, false),
        (Style::PhotoDrift, "photo-drift", "Photo Drift", Stage::Prints, 30, Background::Colour([235, 232, 224]), true, false),
        (Style::Surprise, "surprise", "Surprise Me", Stage::Wall, 6, DEFAULT_BACKGROUND, true, false),
        (Style::MatrixRain, "matrix-rain", "Matrix Rain", Stage::Cinema, 6, MATRIX_BACKGROUND, false, true),
        (Style::MatrixKenBurns, "matrix-ken-burns", "Matrix Ken Burns", Stage::Cinema, 6, MATRIX_BACKGROUND, true, true),
    ];

    #[test]
    fn every_style_has_its_facts_in_one_spec() {
        assert_eq!(SPECS.len(), Style::CHOICES.len());
        for &(style, key, label, stage, tiles, background, photos, terminal) in SPECS {
            let spec = style.spec();
            assert_eq!(spec.key, key, "{style:?}");
            assert_eq!(spec.label, label, "{style:?}");
            assert_eq!(spec.stage, stage, "{style:?}");
            assert_eq!(spec.default_tiles, tiles, "{style:?}");
            assert_eq!(spec.background, background, "{style:?}");
            assert_eq!(spec.uses_photos, photos, "{style:?}");
            assert_eq!(spec.terminal_only, terminal, "{style:?}");
            assert!(!spec.subtitle.is_empty(), "{style:?}");
        }
        assert_eq!(DEFAULT_MOSAIC_TILES, 6);
    }

    #[test]
    fn every_saved_key_reads_back_as_its_style() {
        for style in Style::CHOICES {
            assert_eq!(Style::from_saved(style.spec().key), style);
        }
        assert_eq!(Style::from_saved("no-such-style"), Style::Mosaic);
    }

    /// (stage, tile limit, slots for 1 / 6 / 100 / 20,000 tiles)
    const LIMITS: &[(Stage, usize, [usize; 4])] = &[
        (Stage::Wall, 10_000, [1, 6, 100, 10_000]),
        (Stage::Prints, 64, [1, 6, 64, 64]),
        (Stage::Flying, 32, [1, 6, 32, 32]),
        (Stage::CoverFlow, 10_000, [0, 0, 0, 0]),
        (Stage::Cinema, 10_000, [0, 0, 0, 0]),
    ];

    #[test]
    fn the_stage_sets_the_tile_limit_and_the_players_slots() {
        assert_eq!(
            (MAX_WALL_TILES, PRINT_MAX_SLOTS, FLY_MAX_SLOTS),
            (10_000, 64, 32)
        );
        for &(stage, limit, slots) in LIMITS {
            assert_eq!(stage.tile_limit(), limit, "{stage:?}");
            for (tiles, expected) in [1, 6, 100, 20_000].into_iter().zip(slots) {
                assert_eq!(stage.slots(tiles), expected, "{stage:?} with {tiles} tiles");
            }
        }
        // Zero tiles still gives a photo stage one slot.
        assert_eq!(Stage::Wall.slots(0), 1);
    }

    /// (stage, tiles, decode size)
    const DECODE: &[(Stage, usize, (u32, u32))] = &[
        (Stage::Wall, 6, (1536, 1536)),
        (Stage::Wall, 12, (1152, 1152)),
        (Stage::Wall, 30, (896, 896)),
        (Stage::Wall, 100, (640, 640)),
        (Stage::Wall, 400, (384, 384)),
        (Stage::Wall, 1600, (192, 192)),
        (Stage::Wall, 6400, (96, 96)),
        (Stage::Wall, 6401, (48, 48)),
        (Stage::Prints, 6, (1920, 1280)),
        (Stage::Prints, 16, (1280, 860)),
        (Stage::Prints, 36, (960, 640)),
        (Stage::Prints, 37, (640, 430)),
        (Stage::Flying, 6, (1920, 1280)),
        (Stage::Flying, 7, (1440, 960)),
        (Stage::CoverFlow, 6, (2560, 1600)),
        (Stage::Cinema, 1, (2560, 1600)),
    ];

    #[test]
    fn originals_are_decoded_smaller_as_more_tiles_share_the_screen() {
        for &(stage, tiles, size) in DECODE {
            assert_eq!(
                stage.decode_size(tiles),
                size,
                "{stage:?} with {tiles} tiles"
            );
        }
    }
}
