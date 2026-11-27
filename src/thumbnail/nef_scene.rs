//! PIC-owned experimental scene-linear Nikon rendering.
//!
//! Deliberately isolated from the production JPEG matching and other RAW
//! formats. `RawDevelop` provides only sensor demosaicing and crops for
//! this opt-in path. PIC independently applies the file's camera colour
//! matrix, WB, scene tone and sRGB encoding in float before quantizing
//! to 16 bits. The embedded JPEG is never read here.
//!
//! Enable only for controlled development with PIC_NEF_SCENE_RENDER=1.
//! No third-party rendering algorithm, LUT, profile, or asset is copied.

use anyhow::{Context, Result};
use image::{DynamicImage, ImageBuffer, Rgb};
use rawler::imgop::develop::{Intermediate, ProcessingStep, RawDevelop};
use rayon::prelude::*;

use rawler::imgop::chromatic_adaption::adapt_bradford;
use rawler::imgop::xyz::{Illuminant, CIE_1931_TRISTIMULUS_A, CIE_1931_TRISTIMULUS_D65};

type CameraMatrix = [[f32; 3]; 3];

fn matrix_from_flat(values: &[f32]) -> Option<CameraMatrix> {
    let values: &[f32; 9] = values.try_into().ok()?;
    let result = std::array::from_fn(|row| std::array::from_fn(|column| values[row * 3 + column]));
    values.iter().all(|v| v.is_finite()).then_some(result)
}

fn sensor_white_ratio(matrix: &CameraMatrix, white_xyz: [f32; 3]) -> Option<[f32; 2]> {
    let sensor_response = std::array::from_fn::<f32, 3, _>(|row| {
        (0..3)
            .map(|column| matrix[row][column] * white_xyz[column])
            .sum()
    });
    let [red, green, blue] = sensor_response;
    (red > 0.0 && green > 0.0 && blue > 0.0).then_some([red / green, blue / green])
}

/// Use recorded *as-shot* camera-neutral response as a rough estimator of
/// where the scene lies between the two camera calibration illuminants.
/// Endpoints come from the NEF's own colour matrices and published CIE white
/// tristimulus values. No known photo pixels or camera JPEG are involved.
///
/// This interpolation is a guarded engineering candidate, not a claim to
/// exactly reproduce Adobe DNG dual-illuminant colour calibration.
fn illuminant_a_fraction(wb: [f32; 4], a: &CameraMatrix, d65: &CameraMatrix) -> Option<f32> {
    if wb[..3].iter().any(|v| !v.is_finite() || *v <= 0.0) {
        return None;
    }
    let a_response = sensor_white_ratio(a, CIE_1931_TRISTIMULUS_A)?;
    let d_response = sensor_white_ratio(d65, CIE_1931_TRISTIMULUS_D65)?;
    let target = [wb[1] / wb[0], wb[1] / wb[2]];
    if target.iter().any(|v| !v.is_finite() || *v <= 0.0) {
        return None;
    }
    let x = a_response.map(f32::ln);
    let y = d_response.map(f32::ln);
    let t = target.map(f32::ln);
    let direction = [x[0] - y[0], x[1] - y[1]];
    let denom = direction[0] * direction[0] + direction[1] * direction[1];
    if denom < 1e-5 || !denom.is_finite() {
        return None;
    }
    let projection = ((t[0] - y[0]) * direction[0] + (t[1] - y[1]) * direction[1]) / denom;
    projection.is_finite().then_some(projection.clamp(0.0, 1.0))
}

/// Inject one estimated daylight-adapted matrix into an *ephemeral decoded*
/// rawler object, then use rawler's existing colour conversion unchanged.
/// Illuminant-A calibration is adapted to D65 through rawler's already-linked
/// API. Nothing is saved into a user's file, recipe or global profile.
fn select_as_shot_calibration(raw: &mut rawler::RawImage) -> Option<f32> {
    let a = matrix_from_flat(raw.color_matrix.get(&Illuminant::A)?)?;
    let d65 = matrix_from_flat(raw.color_matrix.get(&Illuminant::D65)?)?;
    let fraction = illuminant_a_fraction(raw.wb_coeffs, &a, &d65)?;
    let adapted_a = adapt_bradford(&Illuminant::A, &Illuminant::D65, &a);
    let blended: CameraMatrix = std::array::from_fn(|i| {
        std::array::from_fn(|j| d65[i][j] * (1.0 - fraction) + adapted_a[i][j] * fraction)
    });
    if blended.iter().flatten().any(|v| !v.is_finite()) {
        return None;
    }
    raw.color_matrix
        .insert(Illuminant::D65, blended.into_iter().flatten().collect());
    Some(fraction)
}

// Published IEC sRGB primary transformation into CIE XYZ with D65 white.
// The inverse of the camera's XYZ->sensor profile is the physical basis for
// transforming neutralized camera RGB back into a display colour space.
const SRGB_TO_XYZ: CameraMatrix = [
    [0.4124564, 0.3575761, 0.1804375],
    [0.2126729, 0.7151522, 0.0721750],
    [0.0193339, 0.1191920, 0.9503041],
];

fn multiply_matrix(left: &CameraMatrix, right: &CameraMatrix) -> CameraMatrix {
    std::array::from_fn(|row| {
        std::array::from_fn(|column| (0..3).map(|k| left[row][k] * right[k][column]).sum())
    })
}

fn invert_matrix(m: &CameraMatrix) -> Option<CameraMatrix> {
    // Cofactors for a real 3x3 linear transform. All terms are evaluated
    // directly here, independently of any external RAW renderer.
    let a = m[0][0];
    let b = m[0][1];
    let c = m[0][2];
    let d = m[1][0];
    let e = m[1][1];
    let f = m[1][2];
    let g = m[2][0];
    let h = m[2][1];
    let i = m[2][2];
    let a11 = e * i - f * h;
    let a12 = c * h - b * i;
    let a13 = b * f - c * e;
    let a21 = f * g - d * i;
    let a22 = a * i - c * g;
    let a23 = c * d - a * f;
    let a31 = d * h - e * g;
    let a32 = b * g - a * h;
    let a33 = a * e - b * d;
    let det = a * a11 + b * a21 + c * a31;
    if !det.is_finite() || det.abs() < 1e-7 {
        return None;
    }
    let adj = [[a11, a12, a13], [a21, a22, a23], [a31, a32, a33]];
    let inverse = adj.map(|row| row.map(|value| value / det));
    inverse
        .iter()
        .flatten()
        .all(|v| v.is_finite())
        .then_some(inverse)
}

/// Return a matrix that maps white-balanced sensor channels to scene-linear
/// sRGB. Normalize each camera response to a unit D65 neutral first, so
/// equal neutralized camera signals yield equal linear display channels.
///
/// Unlike the existing renderer this matrix never clips the sensor values;
/// highlight rolloff happens later and all steps use floating-point buffers.
fn sensor_to_scene_matrix(xyz_to_camera: &CameraMatrix) -> Option<CameraMatrix> {
    let mut rgb_to_camera = multiply_matrix(xyz_to_camera, &SRGB_TO_XYZ);
    for row in &mut rgb_to_camera {
        let white = row.iter().sum::<f32>();
        if !white.is_finite() || white <= 0.00001 {
            return None;
        }
        for element in row {
            *element /= white;
        }
    }
    invert_matrix(&rgb_to_camera)
}

fn apply_camera_calibration(camera: [f32; 3], wb: [f32; 4], matrix: &CameraMatrix) -> [f32; 3] {
    let corrected = std::array::from_fn::<f32, 3, _>(|channel| camera[channel] * wb[channel]);
    std::array::from_fn(|row| (0..3).map(|col| matrix[row][col] * corrected[col]).sum())
}

fn develop_scene_linear(
    raw: &rawler::RawImage,
    half: bool,
) -> Result<rawler::pixarray::Color2D<f32, 3>> {
    // Ask the existing RAW decoder only for its standardized, scaled sensor
    // demosaic and geometric cropping. PIC owns this candidate's camera
    // calibration, keeping >1.0 highlights and signed gamut components until
    // output rolloff.
    let mut steps = RawDevelop::default().steps;
    steps.retain(|step| {
        !matches!(
            step,
            ProcessingStep::WhiteBalance | ProcessingStep::Calibrate | ProcessingStep::SRgb
        )
    });
    let mut camera = if half {
        superpixel_camera(raw)?
    } else {
        let intermediate = RawDevelop::new_with(&steps)
            .develop_intermediate_with_owned_rgb_crop(raw)?;
        let Intermediate::ThreeColor(camera) = intermediate else {
            anyhow::bail!("Nikon Bayer demosaic did not return three camera channels");
        };
        camera
    };
    let raw_matrix = raw
        .color_matrix
        .get(&Illuminant::D65)
        .context("Nikon NEF has no D65 camera colour matrix")?;
    let matrix = matrix_from_flat(raw_matrix)
        .context("Nikon NEF contains an invalid D65 camera colour matrix")?;
    let scene_matrix = sensor_to_scene_matrix(&matrix)
        .context("Nikon camera colour matrix is singular or non-neutral")?;
    let wb = if raw.wb_coeffs[..3].iter().all(|v| v.is_finite() && *v > 0.0) {
        raw.wb_coeffs
    } else {
        if std::env::var_os("PICASA_TRACE").is_some() {
            eprintln!("PIC_DECODE stage=nef_scene_wb source=unavailable fallback=unit");
        }
        [1.0; 4]
    };
    camera.data.par_iter_mut().for_each(|pixel| {
        *pixel = develop_pixel(*pixel, wb, &scene_matrix);
    });
    Ok(camera)
}

/// Sensor level (rawler scales the white level to 1.0) where clipping starts
/// to be trusted, and where a photosite is certainly saturated.
const SENSOR_CLIP_START: f32 = 0.90;
const SENSOR_CLIP_FULL: f32 = 0.98;

fn develop_pixel(camera: [f32; 3], wb: [f32; 4], matrix: &CameraMatrix) -> [f32; 3] {
    apply_camera_calibration(clip_saturated_highlights(camera, wb), [1.0; 4], matrix)
}

/// White-balance a sensor pixel without inventing colour in saturated areas.
/// A saturated photosite only says "at least white"; multiplying it by the
/// red and blue WB gains turns a blown sky or wall magenta. Where the
/// brightest channel nears the sensor white level, each balanced channel is
/// capped at the lowest balanced white level, so a fully clipped pixel is
/// neutral white. Pixels below the start level keep their full headroom.
fn clip_saturated_highlights(camera: [f32; 3], wb: [f32; 4]) -> [f32; 3] {
    let balanced = std::array::from_fn::<f32, 3, _>(|channel| camera[channel] * wb[channel]);
    let peak = camera.iter().copied().fold(0.0f32, f32::max);
    if !(peak > SENSOR_CLIP_START) {
        return balanced;
    }
    let ramp =
        ((peak - SENSOR_CLIP_START) / (SENSOR_CLIP_FULL - SENSOR_CLIP_START)).clamp(0.0, 1.0);
    let weight = ramp * ramp * (3.0 - 2.0 * ramp);
    let white = wb[..3].iter().copied().fold(f32::INFINITY, f32::min);
    balanced.map(|value| value + weight * (value.min(white) - value))
}

/// Half-size camera RGB: rawler's scaled sensor values, its superpixel
/// demosaic over the active area, then the default crop at half scale, as
/// rawler's own develop does for a superpixel image.
fn superpixel_camera(raw: &rawler::RawImage) -> Result<rawler::pixarray::Color2D<f32, 3>> {
    use rawler::imgop::sensor::{bayer::superpixel::Superpixel3Channel, Demosaic};
    let config = bayer_config(raw).context("half-size develop needs an RGB Bayer sensor")?;
    let intermediate =
        RawDevelop::new_with(&[ProcessingStep::Rescale]).develop_intermediate(raw)?;
    let Intermediate::Monochrome(pixels) = intermediate else {
        anyhow::bail!("Nikon Bayer sensor did not return one sample per photosite");
    };
    let roi = raw.active_area.unwrap_or(pixels.rect());
    let mut camera = Superpixel3Channel::new().demosaic(&pixels, &config.cfa, &config.colors, roi);
    if let Some(mut crop) = raw.crop_area.or(raw.active_area) {
        if let Some(active_area) = &raw.active_area {
            crop = crop.adapt(active_area);
        }
        crop.scale(0.5);
        if crop.d != camera.dim() {
            camera = camera.crop(crop);
        }
    }
    Ok(camera)
}

const SAMPLE_STRIDE: usize = 64;
const HIGHLIGHT_KNEE: f32 = 0.62;
// A restrained, global photographic look. No scene-specific colour fitting,
// copied profile, imported LUT or embedded-JPEG-derived transformation.
const LUMINANCE_CONTRAST: f32 = 0.38;
const LINEAR_CHROMA_GAIN: f32 = 1.12;
const TARGET_MEDIAN: f32 = 0.23;
const TARGET_P85: f32 = 0.68;

/// Relative scene-referred luminance in the calibrated linear sRGB working
/// space. CIE/sRGB luminance weights are published colourimetry constants.
#[inline]
fn luminance(rgb: [f32; 3]) -> f32 {
    0.2126 * rgb[0].max(0.0) + 0.7152 * rgb[1].max(0.0) + 0.0722 * rgb[2].max(0.0)
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

/// A continuous luminance shoulder. The highlight compression is applied
/// before midtone contrast so input luminances remain ordered and the two
/// transformations affect all colour channels by one shared scalar.
fn tone_luminance(x: f32) -> f32 {
    let x = x.max(0.0);
    let shoulder = if x <= HIGHLIGHT_KNEE {
        x
    } else {
        let range = 1.0 - HIGHLIGHT_KNEE;
        HIGHLIGHT_KNEE + range * (1.0 - (-(x - HIGHLIGHT_KNEE) / range).exp())
    };
    // Symmetric S curve with fixed endpoints. Its derivative is strictly
    // positive throughout [0,1] when contrast is below 1.0.
    shoulder + LUMINANCE_CONTRAST * shoulder * (1.0 - shoulder) * (2.0 * shoulder - 1.0)
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
    // Restore a restrained amount of chroma lost in basic RAW development,
    // using the same scene-linear luminance anchor for all three channels.
    // Neutral grays are unchanged. This is not a per-channel colour cast
    // correction: a later camera-profile evaluation must validate its hues.
    let neutral = luminance(mapped);
    let vivid = mapped.map(|v| (neutral + LINEAR_CHROMA_GAIN * (v - neutral)).max(0.0));
    let brightest = vivid.iter().copied().fold(0.0f32, f32::max);
    let gamut_scale = if brightest > 1.0 {
        1.0 / brightest
    } else {
        1.0
    };
    vivid.map(|v| encode_srgb(v * gamut_scale))
}

/// Develop sensor pixels to a true full-resolution, high-precision displayed
/// raster. The existing GTK editor still consumes RGBA8 afterwards; that
/// downstream quantization must be fixed separately before claiming a
/// complete 16-bit non-destructive editing pipeline.
pub(super) fn develop(raw: &mut rawler::RawImage) -> Result<DynamicImage> {
    develop_sized(raw, false)
}

/// Long edge of the half-size develop ([`develop_half`]), or `None` when the
/// sensor is not an RGB Bayer sensor and there is no half-size develop.
pub(super) fn half_size_long_edge(raw: &rawler::RawImage) -> Option<usize> {
    bayer_config(raw)?;
    let area = raw.crop_area.or(raw.active_area);
    let (width, height) = area.map_or((raw.width, raw.height), |area| (area.d.w, area.d.h));
    Some(width.max(height) / 2)
}

/// [`develop`] at half size: each 2×2 Bayer block becomes one pixel (no
/// interpolation), for exports that need no more than half the sensor.
pub(super) fn develop_half(raw: &mut rawler::RawImage) -> Result<DynamicImage> {
    develop_sized(raw, true)
}

fn bayer_config(raw: &rawler::RawImage) -> Option<&rawler::rawimage::CFAConfig> {
    match &raw.photometric {
        rawler::rawimage::RawPhotometricInterpretation::Cfa(config)
            if config.cfa.is_rgb() && config.sensor == rawler::imgop::sensor::SensorType::Bayer =>
        {
            Some(config)
        }
        _ => None,
    }
}

fn develop_sized(raw: &mut rawler::RawImage, half: bool) -> Result<DynamicImage> {
    let a_fraction = select_as_shot_calibration(raw);
    if std::env::var_os("PICASA_TRACE").is_some() {
        eprintln!(
            "PIC_DECODE stage=nef_dual_illuminant_calibration selection={} A_weight={:?} source=NEF_metadata",
            if a_fraction.is_some() { "estimated_as_shot" } else { "default_D65" },
            a_fraction,
        );
    }
    // Preserve float values through tone rendering: never convert rawler's
    // calibrated linear pixels to RGB8 or sRGB-encoded RGB16 first.
    let linear = develop_scene_linear(raw, half)?;
    let width = u32::try_from(linear.width).context("RAW image width exceeds u32")?;
    let height = u32::try_from(linear.height).context("RAW image height exceeds u32")?;
    let gain = scene_exposure_gain(&linear.data);

    let elements = linear
        .data
        .len()
        .checked_mul(3)
        .context("RAW pixel count overflow")?;
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
    let image: ImageBuffer<Rgb<u16>, Vec<u16>> = ImageBuffer::from_raw(width, height, output)
        .context("RAW scene renderer returned an invalid pixel buffer")?;
    Ok(DynamicImage::ImageRgb16(image))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owned_rgb_crop_reuses_storage_and_matches_allocating_crop() {
        use rawler::{
            imgop::{Dim2, Point, Rect},
            pixarray::Color2D,
        };

        for area in [
            Rect::new(Point::new(0, 0), Dim2::new(5, 4)),
            Rect::new(Point::new(0, 0), Dim2::new(3, 4)),
            Rect::new(Point::new(1, 1), Dim2::new(3, 2)),
            Rect::new(Point::new(0, 1), Dim2::new(5, 3)),
        ] {
            let pixels = (0..20).map(|value| [value as f32, -0.0, 0.5]).collect();
            let image = Color2D::new_with(pixels, 5, 4);
            let expected = image.crop(area);
            let original_buffer = image.data.as_ptr();

            let actual = image.crop_owned(area);

            assert_eq!(actual.data.as_ptr(), original_buffer);
            assert_eq!((actual.width, actual.height), (expected.width, expected.height));
            assert!(actual.data.iter().zip(&expected.data).all(|(actual, expected)| {
                actual.map(f32::to_bits) == expected.map(f32::to_bits)
            }));
            assert_eq!(actual.data.len(), expected.data.len());
        }
    }

    #[test]
    #[ignore = "requires PIC_TEST_RAW pointing to a local Nikon NEF"]
    fn owned_nef_crop_matches_allocating_crop_pixel_for_pixel() {
        let path = std::env::var_os("PIC_TEST_RAW").expect("set PIC_TEST_RAW to a local Nikon NEF");
        let source = rawler::rawsource::RawSource::new(std::path::Path::new(&path)).unwrap();
        let raw = rawler::get_decoder(&source)
            .unwrap()
            .raw_image(&source, &rawler::decoders::RawDecodeParams::default(), false)
            .unwrap();
        let mut steps = RawDevelop::default().steps;
        steps.retain(|step| {
            !matches!(
                step,
                ProcessingStep::WhiteBalance | ProcessingStep::Calibrate | ProcessingStep::SRgb
            )
        });
        let developer = RawDevelop::new_with(&steps);
        let Intermediate::ThreeColor(expected) = developer.develop_intermediate(&raw).unwrap()
        else {
            panic!("expected RGB pixels from the reference path");
        };
        let Intermediate::ThreeColor(actual) = developer
            .develop_intermediate_with_owned_rgb_crop(&raw)
            .unwrap()
        else {
            panic!("expected RGB pixels from the owned crop path");
        };
        assert_eq!((actual.width, actual.height), (expected.width, expected.height));
        assert_eq!(actual.data.len(), expected.data.len());
        assert!(actual.data.iter().zip(&expected.data).all(|(actual, expected)| {
            actual.map(f32::to_bits) == expected.map(f32::to_bits)
        }));
    }

    #[test]
    fn camera_matrix_roundtrip_preserves_scene_values_above_white() {
        let profile: CameraMatrix = [
            [0.95, -0.20, 0.05],
            [-0.28, 1.21, 0.11],
            [0.03, -0.08, 0.88],
        ];
        let m = sensor_to_scene_matrix(&profile).expect("invertible camera profile");
        let rgb_to_camera = {
            let mut x = multiply_matrix(&profile, &SRGB_TO_XYZ);
            for row in &mut x {
                let sum: f32 = row.iter().sum();
                row.iter_mut().for_each(|v| *v /= sum);
            }
            x
        };
        for scene in [[1.0, 1.0, 1.0], [2.0, 1.0, 0.4], [0.3, 0.2, 0.8]] {
            let camera = std::array::from_fn::<f32, 3, _>(|i| {
                (0..3).map(|j| rgb_to_camera[i][j] * scene[j]).sum()
            });
            let restored = apply_camera_calibration(camera, [1.0; 4], &m);
            for channel in 0..3 {
                assert!((restored[channel] - scene[channel]).abs() < 0.0001);
            }
        }
        let white = apply_camera_calibration([1.0; 3], [1.0; 4], &m);
        assert!(white.iter().all(|v| (*v - 1.0).abs() < 0.0001));
    }

    #[test]
    fn sensor_clipped_highlights_stay_neutral_after_white_balance() {
        // Typical daylight Nikon multipliers: red and blue are boosted, so a
        // pixel clipped on all three photosites would turn magenta.
        let identity: CameraMatrix = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        let wb = [2.1, 1.0, 1.6, f32::NAN];
        let clipped = develop_pixel([1.0, 1.0, 1.0], wb, &identity);
        assert!(
            (clipped[0] - clipped[1]).abs() < 1e-4 && (clipped[2] - clipped[1]).abs() < 1e-4,
            "a fully clipped sensor pixel must render neutral, got {clipped:?}"
        );
        let unclipped = develop_pixel([0.6, 0.2, 0.1], wb, &identity);
        assert!(
            (unclipped[0] - 1.26).abs() < 1e-4,
            "unclipped colours keep their highlight headroom, got {unclipped:?}"
        );
    }

    #[test]
    fn camera_calibration_does_not_clip_hdr_values() {
        let identity: CameraMatrix = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        let out = apply_camera_calibration([1.5, 0.25, 0.1], [2.0, 1.0, 1.0, f32::NAN], &identity);
        assert!(
            out[0] > 1.0,
            "highlight headroom must remain until tone mapping"
        );
        assert!((out[0] - 3.0).abs() < 1e-6);
        assert!((out[1] - 0.25).abs() < 1e-6);
        assert!(invert_matrix(&[[0.0; 3]; 3]).is_none());
    }

    #[test]
    fn calibration_weights_reach_measured_illuminant_endpoints() {
        let d65: CameraMatrix = [[0.9, -0.2, 0.0], [-0.4, 1.2, 0.2], [-0.1, 0.2, 0.8]];
        let a: CameraMatrix = [[1.05, -0.15, 0.0], [-0.3, 1.15, 0.15], [-0.1, 0.16, 1.10]];
        let camera_neutral_for = |matrix: &CameraMatrix, white: [f32; 3]| {
            let ratio = sensor_white_ratio(matrix, white).expect("positive white response");
            [1.0 / ratio[0], 1.0, 1.0 / ratio[1], f32::NAN]
        };
        let d_wb = camera_neutral_for(&d65, CIE_1931_TRISTIMULUS_D65);
        let a_wb = camera_neutral_for(&a, CIE_1931_TRISTIMULUS_A);
        assert!(illuminant_a_fraction(d_wb, &a, &d65).unwrap() < 0.001);
        assert!(illuminant_a_fraction(a_wb, &a, &d65).unwrap() > 0.999);
        assert!(illuminant_a_fraction([f32::NAN; 4], &a, &d65).is_none());
    }

    #[test]
    fn highlight_shoulder_is_continuous_and_bounded() {
        assert!(tone_luminance(HIGHLIGHT_KNEE) > 0.0);
        assert!(tone_luminance(0.8) > tone_luminance(HIGHLIGHT_KNEE));
        assert!(tone_luminance(2.0) > tone_luminance(1.0));
        assert!(tone_luminance(100.0) <= 1.0);
        assert!(
            (tone_luminance(HIGHLIGHT_KNEE + 0.00001) - tone_luminance(HIGHLIGHT_KNEE - 0.00001))
                .abs()
                < 0.00003
        );
        let levels: Vec<_> = (0..=200)
            .map(|i| tone_luminance(i as f32 / 100.0))
            .collect();
        assert!(
            levels.windows(2).all(|w| w[1] > w[0]),
            "tone contrast must never reverse highlight or shadow order"
        );
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
    fn coloured_midtones_preserve_channel_order_and_increase_chroma() {
        let source = [0.16, 0.10, 0.06];
        let rgb16 = render_pixel(source, 1.0);
        assert!(rgb16[0] > rgb16[1] && rgb16[1] > rgb16[2]);
        let inverse = |v: u16| {
            let v = f32::from(v) / 65535.0;
            if v <= 0.04045 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        let actual = rgb16.map(inverse);
        let actual_luma = luminance(actual);
        let before = source[0] / luminance(source);
        let after = actual[0] / actual_luma;
        assert!(
            after > before,
            "fixed global chroma boost should not desaturate warm RGB"
        );
        assert!(actual.iter().all(|value| *value >= 0.0 && *value <= 1.0));
    }

    #[test]
    fn exposure_is_bounded_and_independent_of_camera_jpeg() {
        // With a 64-pixel sampling stride we need at least 2048 pixels
        // to hit the 32-sample robustness threshold.
        let dark = vec![[0.008; 3]; 4096];
        let bright = vec![[0.9; 3]; 4096];
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
