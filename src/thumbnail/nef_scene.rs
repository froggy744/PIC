//! PIC-owned experimental scene-linear Nikon rendering.
//!
//! Deliberately isolated from the production JPEG matching and other RAW
//! formats. `RawDevelop` provides sensor demosaicing, camera white
//! balance and the camera-to-sRGB calibration already used by PIC. This module
//! performs its own highlight-safe tone rendering *before* sRGB encoding and
//! before quantizing RGB data to 16 bits. The embedded JPEG is never read here.
//!
//! Enable only for controlled development with PIC_NEF_SCENE_RENDER=1.
//! No third-party rendering algorithm, LUT, profile, or asset is copied.

use anyhow::{Context, Result};
use image::{DynamicImage, ImageBuffer, Rgb};
use rayon::prelude::*;
use rawler::imgop::develop::{Intermediate, ProcessingStep, RawDevelop};

const SAMPLE_STRIDE: usize = 64;
const HIGHLIGHT_KNEE: f32 = 0.62;
const TARGET_MEDIAN: f32 = 0.23;
const TARGET_P85: f32 = 0.68;

/// Relative scene-referred luminance in the calibrated linear sRGB working
/// space. CIE/sRGB luminance weights are published colourimetry constants.
#[inline]
fn luminance(rgb: [f32; 3]) -> f32 {
    0.2126 * rgb[0].max(0.0)
        + 0.7152 * rgb[1].max(0.0)
        + 0.0722 * rgb[2].max(0.0)
}

/// A bounded, scene-derived exposure starting point, not a reproduction of
/// Nikon's opaque Picture Control. Percentiles make a bright jersey or a
/// patch of backlight much less influential than a histogram maximum.
///
/// Calculated from linear camera-calibrated sensor RGB only. We deliberately
/// do not fit a transform or an exposure target to the embedded JPEG.
fn scene_exposure_gain(pixels: &[[f32; 3]]) -> f32 {
    let mut measured: Vec<f32> = pixels
        .iter()
        .step_by(SAMPLE_STRIDE)
        .map(|pixel| luminance(*pixel))
        .filter(|value| value.is_finite() && *value > 0.0001 && *value < 12.0)
        .collect();
    if measured.len() < 32 {
        return 1.0;
    }
    measured.sort_unstable_by(f32::total_cmp);
    let middle = measured[measured.len() / 2].max(0.002);
    let high = measured[(measured.len() * 85) / 100].max(0.02);
    let mid_gain = TARGET_MEDIAN / middle;
    let high_gain = TARGET_P85 / high;
    // Work in photographic (logarithmic) exposure space. Bounding exposure
    // avoids exploding low-light noise or washing out deliberately dark
    // images, even if this rough initial auto exposure guesses badly.
    (mid_gain * high_gain).sqrt().clamp(0.65, 2.7)
}

/// A continuous luminance shoulder. Up to the knee the mapping is exactly
/// linear; above it, headroom is compressed without hard clipping at 1.0.
/// The derivative is 1.0 on both sides of the knee.
fn tone_luminance(x: f32) -> f32 {
    let x = x.max(0.0);
    if x <= HIGHLIGHT_KNEE {
        x
    } else {
        let range = 1.0 - HIGHLIGHT_KNEE;
        HIGHLIGHT_KNEE + range * (1.0 - (-(x - HIGHLIGHT_KNEE) / range).exp())
    }
}

/// IEC 61966-2-1 display transfer function; encoded only once after all
/// linear tone operations and before writing 16-bit RGB pixels.
#[inline]
fn encode_srgb(linear: f32) -> u16 {
    let v = linear.clamp(0.0, 1.0);
    let encoded = if v <= 0.0031308 {
        v * 12.92
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    };
    (encoded * 65535.0).round().clamp(0.0, 65535.0) as u16
}

/// Tone-map the pixel by luminance, not separately per channel. Scaling all
/// channels equally prevents a channel-specific curve from generating an
/// artificial red/green shift in unclipped skin tones. An additional shared
/// gamut scale prevents one overbright channel from clipping independently.
fn render_pixel(linear: [f32; 3], exposure: f32) -> [u16; 3] {
    if linear.iter().any(|v| !v.is_finite()) {
        return [0; 3];
    }
    let source = linear.map(|v| v.max(0.0) * exposure);
    let y = luminance(source);
    if y <= 0.0 {
        return [0; 3];
    }
    let tone_scale = tone_luminance(y) / y;
    let mapped = source.map(|v| v * tone_scale);
    let brightest = mapped.iter().copied().fold(0.0f32, f32::max);
    let gamut_scale = if brightest > 1.0 {
        1.0 / brightest
    } else {
        1.0
    };
    mapped.map(|v| encode_srgb(v * gamut_scale))
}

/// Develop sensor pixels to a true full-resolution, high-precision displayed
/// raster. The existing GTK editor still consumes RGBA8 afterwards; that
/// downstream quantization must be fixed separately before claiming a
/// complete 16-bit non-destructive editing pipeline.
pub(super) fn develop(raw: &rawler::RawImage) -> Result<DynamicImage> {
    // Preserve float values through tone rendering: never convert rawler's
    // calibrated linear pixels to RGB8 or sRGB-encoded RGB16 first.
    let mut steps = RawDevelop::default().steps;
    steps.retain(|step| *step != ProcessingStep::SRgb);
    let intermediate = RawDevelop::new_with(&steps).develop_intermediate(raw)?;
    let Intermediate::ThreeColor(linear) = intermediate else {
        anyhow::bail!("Nikon Bayer RAW development did not return RGB sensor pixels");
    };
    let width = u32::try_from(linear.width).context("RAW image width exceeds u32")?;
    let height = u32::try_from(linear.height).context("RAW image height exceeds u32")?;
    let gain = scene_exposure_gain(&linear.data);

    let elements = linear.data.len().checked_mul(3).context("RAW pixel count overflow")?;
    let mut output = vec![0u16; elements];
    output
        .par_chunks_exact_mut(3)
        .zip(linear.data.par_iter())
        .for_each(|(target, source)| {
            target.copy_from_slice(&render_pixel(*source, gain));
        });
    if std::env::var_os("PICASA_TRACE").is_some() {
        eprintln!(
            "PIC_DECODE stage=nef_scene_render source=linear_sensor exposure_stops={:.3} output={}x{} bits=16 jpeg_reference=false",
            gain.log2(),
            width,
            height,
        );
    }
    let image: ImageBuffer<Rgb<u16>, Vec<u16>> =
        ImageBuffer::from_raw(width, height, output)
            .context("RAW scene renderer returned an invalid pixel buffer")?;
    Ok(DynamicImage::ImageRgb16(image))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn highlight_shoulder_is_continuous_and_bounded() {
        assert!((tone_luminance(HIGHLIGHT_KNEE) - HIGHLIGHT_KNEE).abs() < 1e-6);
        assert!(tone_luminance(0.8) > HIGHLIGHT_KNEE);
        assert!(tone_luminance(2.0) > tone_luminance(1.0));
        assert!(tone_luminance(100.0) <= 1.0);
        assert!((tone_luminance(HIGHLIGHT_KNEE + 0.00001)
            - tone_luminance(HIGHLIGHT_KNEE - 0.00001)).abs() < 0.00003);
    }

    #[test]
    fn neutral_pixels_do_not_acquire_a_colour_cast() {
        for level in [0.0, 0.03, 0.18, 0.7, 1.0, 3.0] {
            let rgb = render_pixel([level; 3], 2.0);
            assert_eq!(rgb[0], rgb[1]);
            assert_eq!(rgb[1], rgb[2]);
        }
    }

    #[test]
    fn coloured_midtones_preserve_linear_channel_ratio() {
        let source = [0.16, 0.10, 0.06];
        let rgb16 = render_pixel(source, 1.0);
        let inverse = |v: u16| {
            let v = f32::from(v) / 65535.0;
            if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
        };
        let a = rgb16.map(inverse);
        assert!((a[0] / a[1] - source[0] / source[1]).abs() < 0.005);
        assert!((a[2] / a[1] - source[2] / source[1]).abs() < 0.005);
    }

    #[test]
    fn exposure_is_bounded_and_independent_of_camera_jpeg() {
        let dark = vec![[0.008; 3]; 256];
        let bright = vec![[0.9; 3]; 256];
        assert_eq!(scene_exposure_gain(&dark), 2.7);
        assert_eq!(scene_exposure_gain(&bright), 0.65);
        assert_eq!(scene_exposure_gain(&[]), 1.0);
    }

    #[test]
    fn highlights_remain_ordered_without_rgb_overflow() {
        let low = render_pixel([0.35; 3], 1.0)[0];
        let mid = render_pixel([0.70; 3], 1.0)[0];
        let high = render_pixel([1.6; 3], 1.0)[0];
        assert!(low < mid && mid < high, "{low} < {mid} < {high}");
        assert_eq!(render_pixel([f32::NAN, 0.2, 0.1], 1.0), [0; 3]);
        assert_eq!(render_pixel([-5.0, 0.0, 0.0], 1.0), [0; 3]);
    }
}
