//! Tone module: the rules and constants that the CPU tone kernel
//! (`render::apply_tone_scaled`) and the GLSL tone kernel (`live_gpu`) share.
//! Both kernels are adapters of this module: the GLSL constants are generated
//! from the Rust ones below, and "is this recipe's tone neutral" and "can the
//! GPU draw it" are each decided once, here.

use super::filters::FilterPreset;
use super::model::EditRecipe;

/// A tone slider closer to zero than this is off. Recipes are stored with
/// four decimals, so every stored non-zero value is at least this large.
pub const NEUTRAL_EPSILON: f32 = 0.0001;

/// Fill light, shadows and highlights: exponent gain per slider unit, the
/// luminance mask's smoothstep edges, and the mask's weight.
pub const FILL_GAIN: f32 = 1.15;
pub const SHADOWS_GAIN: f32 = 1.35;
pub const HIGHLIGHTS_GAIN: f32 = 1.20;
pub const FILL_MASK: (f32, f32) = (0.48, 0.93);
pub const SHADOWS_MASK: (f32, f32) = (0.18, 0.61);
pub const HIGHLIGHTS_MASK: (f32, f32) = (0.48, 0.87);
pub const FILL_WEIGHT: f32 = 0.95;
pub const SHADOWS_WEIGHT: f32 = 0.95;
pub const HIGHLIGHTS_WEIGHT: f32 = 0.92;
/// Contrast S-curve strength at slider ±1.
pub const CONTRAST_STRENGTH: f32 = 0.55;
/// Auto Colour's per-channel gain limits (the CPU computes them in f64).
pub const AUTO_COLOR_GAIN: (f64, f64) = (0.75, 1.35);
/// Auto Contrast keeps the stretched luminance inside this range.
pub const AUTO_CONTRAST_RANGE: (f32, f32) = (0.012, 0.988);

fn off(value: f32) -> bool {
    value.abs() < NEUTRAL_EPSILON
}

/// The recipe changes no pixel in the tone stage: no look, no automatic
/// correction and every tone slider off. Geometry and layers are not tone.
pub fn is_neutral(recipe: &EditRecipe) -> bool {
    recipe.filter == FilterPreset::None
        && !recipe.auto_color
        && !recipe.auto_contrast
        && !recipe.auto_brightness
        && !recipe.auto_enhance
        && !recipe.black_white
        && !recipe.sepia
        && [
            recipe.exposure,
            recipe.contrast,
            recipe.fill_light,
            recipe.highlights,
            recipe.shadows,
            recipe.whites,
            recipe.blacks,
            recipe.temperature,
            recipe.tint,
            recipe.saturation,
            recipe.vibrance,
            recipe.sharpen,
            recipe.clarity,
            recipe.smooth,
            recipe.grain,
            recipe.vignette,
        ]
        .into_iter()
        .all(off)
}

/// Why the GPU tone kernel cannot draw `recipe` yet, or `None` when it can.
/// The reason is the trace label of the editor's CPU fallback.
pub fn gpu_unsupported_reason(recipe: &EditRecipe) -> Option<&'static str> {
    let any_auto = recipe.auto_color || recipe.auto_contrast;
    // A filtered/cropped GPU histogram prepass exists, but does not yet match
    // the CPU Export reference consistently. Keep automatic corrections on
    // CPU for creative looks and non-identity geometry until strict parity is
    // proven. Standalone/both autos on plain, uncropped photos are verified.
    let geometry_changes_pixels = !recipe.crop.is_full() || recipe.straighten.abs() >= 0.01;
    if recipe.auto_brightness {
        Some("auto_brightness")
    } else if recipe.auto_enhance {
        Some("auto_enhance")
    } else if any_auto && recipe.filter_amount < 1.0 {
        // Shader Auto Colour/Contrast assumes 100% strength.
        Some("auto_with_partial_strength")
    } else if recipe.smooth > 0.001 {
        Some("smooth")
    } else if recipe.vignette.abs() > 0.001 {
        Some("vignette")
    } else if any_auto && recipe.filter != FilterPreset::None {
        Some("auto_with_filter")
    } else if any_auto && geometry_changes_pixels {
        Some("auto_with_geometry")
    } else {
        None
    }
}

/// `source` (a GLSL shader starting with `#version` and optional
/// `#extension` lines) with the tone constants declared after those lines.
pub fn with_glsl_constants(source: &str) -> String {
    let constants = [
        ("TONE_FILL_GAIN", FILL_GAIN),
        ("TONE_SHADOWS_GAIN", SHADOWS_GAIN),
        ("TONE_HIGHLIGHTS_GAIN", HIGHLIGHTS_GAIN),
        ("TONE_FILL_MASK_LOW", FILL_MASK.0),
        ("TONE_FILL_MASK_HIGH", FILL_MASK.1),
        ("TONE_SHADOWS_MASK_LOW", SHADOWS_MASK.0),
        ("TONE_SHADOWS_MASK_HIGH", SHADOWS_MASK.1),
        ("TONE_HIGHLIGHTS_MASK_LOW", HIGHLIGHTS_MASK.0),
        ("TONE_HIGHLIGHTS_MASK_HIGH", HIGHLIGHTS_MASK.1),
        ("TONE_FILL_WEIGHT", FILL_WEIGHT),
        ("TONE_SHADOWS_WEIGHT", SHADOWS_WEIGHT),
        ("TONE_HIGHLIGHTS_WEIGHT", HIGHLIGHTS_WEIGHT),
        ("TONE_CONTRAST_STRENGTH", CONTRAST_STRENGTH),
        ("TONE_AUTO_COLOR_GAIN_LOW", AUTO_COLOR_GAIN.0 as f32),
        ("TONE_AUTO_COLOR_GAIN_HIGH", AUTO_COLOR_GAIN.1 as f32),
        ("TONE_AUTO_CONTRAST_LOW", AUTO_CONTRAST_RANGE.0),
        ("TONE_AUTO_CONTRAST_HIGH", AUTO_CONTRAST_RANGE.1),
    ];
    let mut header_end = 0;
    for line in source.split_inclusive('\n') {
        if !line.trim_start().starts_with('#') {
            break;
        }
        header_end += line.len();
    }
    let mut shader = String::with_capacity(source.len() + constants.len() * 48);
    shader.push_str(&source[..header_end]);
    for (name, value) in constants {
        // `{:?}` always prints a decimal point, which GLSL needs for a float.
        shader.push_str(&format!("const float {name} = {value:?};\n"));
    }
    shader.push_str(&source[header_end..]);
    shader
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neutral_means_no_tone_setting_is_on() {
        let mut recipe = EditRecipe::default();
        assert!(is_neutral(&recipe));
        recipe.crop.left = 0.2;
        assert!(is_neutral(&recipe), "geometry is not tone");
        recipe.vignette = 0.0001;
        assert!(!is_neutral(&recipe), "a stored four-decimal value is on");
        recipe.vignette = 0.00004;
        assert!(is_neutral(&recipe));
    }

    #[test]
    fn gpu_reasons_name_the_first_unsupported_setting() {
        let mut recipe = EditRecipe::default();
        assert_eq!(gpu_unsupported_reason(&recipe), None);
        recipe.clarity = 0.3;
        recipe.grain = 0.2;
        recipe.filter = FilterPreset::Noir;
        assert_eq!(
            gpu_unsupported_reason(&recipe),
            None,
            "the shader draws these"
        );
        recipe.auto_color = true;
        assert_eq!(gpu_unsupported_reason(&recipe), Some("auto_with_filter"));
        recipe.smooth = 0.5;
        assert_eq!(gpu_unsupported_reason(&recipe), Some("smooth"));
    }

    #[test]
    fn glsl_constants_follow_the_version_and_extension_lines() {
        let shader = with_glsl_constants("#version 150\n#extension X : enable\nvoid main() {}\n");
        let lines: Vec<&str> = shader.lines().collect();
        assert_eq!(lines[0], "#version 150");
        assert_eq!(lines[1], "#extension X : enable");
        assert_eq!(lines[2], "const float TONE_FILL_GAIN = 1.15;");
        assert!(shader.contains("const float TONE_HIGHLIGHTS_GAIN = 1.2;"));
        assert!(shader.ends_with("void main() {}\n"));
    }
}
