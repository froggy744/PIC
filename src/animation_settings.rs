//! User configurable durations for visible UI animations.
//!
//! The settings file is `ANIMATION_SPEEDS.toml` in the current working
//! directory. Missing or invalid values fall back to the built-in defaults.

use serde::Deserialize;
use std::sync::OnceLock;

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
struct AnimationSettings {
    grid_resize_ms: u64,
    grid_zoom_cutoff_ms: u64,
    grid_zoom_crossfade_ms: u64,
    grid_zoom_reflow_ms: u64,
    folder_column_reflow_ms: u64,
    folder_column_reflow_style: String,
    folder_scroll_ms: u64,
    gallery_wheel_scroll_ms: u64,
    lightbox_open_ms: u64,
    sidebar_slide_ms: u64,
    folder_mode_settle_ms: u64,
}

impl Default for AnimationSettings {
    fn default() -> Self {
        Self {
            grid_resize_ms: 170,
            grid_zoom_cutoff_ms: 120,
            grid_zoom_crossfade_ms: 180,
            grid_zoom_reflow_ms: 170,
            folder_column_reflow_ms: 220,
            folder_column_reflow_style: "old_fades_over_new".to_string(),
            folder_scroll_ms: 180,
            gallery_wheel_scroll_ms: 180,
            lightbox_open_ms: 250,
            sidebar_slide_ms: 250,
            folder_mode_settle_ms: 100,
        }
    }
}

static SETTINGS: OnceLock<AnimationSettings> = OnceLock::new();

fn settings() -> &'static AnimationSettings {
    SETTINGS.get_or_init(|| {
        let defaults = AnimationSettings::default();
        let path = std::path::PathBuf::from("ANIMATION_SPEEDS.toml");
        let Ok(contents) = std::fs::read_to_string(&path) else {
            eprintln!("Could not read {}. Using built-in animation defaults.", path.display());
            return defaults;
        };
        match toml::from_str::<AnimationSettings>(&contents) {
            Ok(parsed) => AnimationSettings {
                grid_resize_ms: clamp(parsed.grid_resize_ms, defaults.grid_resize_ms),
                grid_zoom_cutoff_ms: clamp(
                    parsed.grid_zoom_cutoff_ms,
                    defaults.grid_zoom_cutoff_ms,
                ),
                grid_zoom_crossfade_ms: clamp(
                    parsed.grid_zoom_crossfade_ms,
                    defaults.grid_zoom_crossfade_ms,
                ),
                grid_zoom_reflow_ms: clamp(
                    parsed.grid_zoom_reflow_ms,
                    defaults.grid_zoom_reflow_ms,
                ),
                folder_column_reflow_ms: clamp(
                    parsed.folder_column_reflow_ms,
                    defaults.folder_column_reflow_ms,
                ),
                folder_column_reflow_style: match parsed.folder_column_reflow_style.as_str() {
                    "crossfade" => "crossfade".to_string(),
                    _ => "old_fades_over_new".to_string(),
                },
                folder_scroll_ms: clamp(parsed.folder_scroll_ms, defaults.folder_scroll_ms),
                gallery_wheel_scroll_ms: clamp(
                    parsed.gallery_wheel_scroll_ms,
                    defaults.gallery_wheel_scroll_ms,
                ),
                lightbox_open_ms: clamp(parsed.lightbox_open_ms, defaults.lightbox_open_ms),
                sidebar_slide_ms: clamp(parsed.sidebar_slide_ms, defaults.sidebar_slide_ms),
                folder_mode_settle_ms: clamp(
                    parsed.folder_mode_settle_ms,
                    defaults.folder_mode_settle_ms,
                ),
            },
            Err(error) => {
                eprintln!(
                    "Could not read animation settings at {}: {error}",
                    path.display()
                );
                defaults
            }
        }
    })
}

fn clamp(value: u64, fallback: u64) -> u64 {
    if value == 0 {
        fallback
    } else {
        value.clamp(16, 10_000)
    }
}

pub(crate) fn grid_resize_ms() -> f64 {
    settings().grid_resize_ms as f64
}
pub(crate) fn grid_zoom_cutoff_ms() -> i64 {
    (settings().grid_zoom_cutoff_ms * 1_000) as i64
}
pub(crate) fn grid_zoom_crossfade_ms() -> i64 {
    (settings().grid_zoom_crossfade_ms * 1_000) as i64
}
pub(crate) fn grid_zoom_reflow_ms() -> f64 {
    settings().grid_zoom_reflow_ms as f64
}
pub(crate) fn folder_column_reflow_ms() -> f64 {
    settings().folder_column_reflow_ms as f64
}
pub(crate) fn folder_column_reflow_uses_overlap() -> bool {
    settings().folder_column_reflow_style == "old_fades_over_new"
}
pub(crate) fn folder_scroll_ms() -> f64 {
    settings().folder_scroll_ms as f64
}
pub(crate) fn gallery_wheel_scroll_ms() -> f64 {
    settings().gallery_wheel_scroll_ms as f64
}
pub(crate) fn lightbox_open_ms() -> f64 {
    settings().lightbox_open_ms as f64
}
pub(crate) fn sidebar_slide_ms() -> u32 {
    settings().sidebar_slide_ms as u32
}
pub(crate) fn folder_mode_settle_ms() -> u64 {
    settings().folder_mode_settle_ms
}

pub(crate) fn user_config_path() -> Option<std::path::PathBuf> {
    std::env::current_dir()
        .ok()
        .map(|directory| directory.join("ANIMATION_SPEEDS.toml"))
}

#[allow(dead_code)]
pub(crate) fn config_path_for_display() -> String {
    user_config_path()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "the platform config directory/pic-rs/animations.toml".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_and_out_of_range_values_are_safe() {
        assert_eq!(clamp(0, 170), 170);
        assert_eq!(clamp(1, 170), 16);
        assert_eq!(clamp(20_000, 170), 10_000);
    }

    #[test]
    fn partial_config_uses_defaults_for_omitted_durations() {
        let parsed: AnimationSettings = toml::from_str("grid_zoom_cutoff_ms = 300").unwrap();
        assert_eq!(parsed.grid_zoom_cutoff_ms, 300);
        assert_eq!(parsed.grid_resize_ms, 170);
        assert_eq!(parsed.sidebar_slide_ms, 250);
    }
}
