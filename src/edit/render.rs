use anyhow::{Context, Result};
use image::{Rgba, RgbaImage};
use rayon::prelude::*;

use super::filters;
use super::model::{CropRect, EditRecipe};

pub fn apply_library_rotation(image: RgbaImage, rotation: i32) -> RgbaImage {
    match rotation.rem_euclid(360) {
        90 => image::imageops::rotate90(&image),
        180 => image::imageops::rotate180(&image),
        270 => image::imageops::rotate270(&image),
        _ => image,
    }
}

pub fn apply_geometry(mut image: RgbaImage, recipe: &EditRecipe) -> RgbaImage {
    if recipe.straighten.abs() >= 0.01 {
        image = rotate_autocrop(&image, recipe.straighten.to_radians());
    }
    if !recipe.crop.is_full() {
        image = crop_normalized(&image, recipe.crop);
    }
    image
}

/// Apply a recipe end-to-end: geometry (straighten, crop), tone, then image
/// overlays. This is the single chokepoint used by thumbnails, the lightbox,
/// the main viewer, export, printing, collage and filter preview tiles, so an
/// overlay appears identically everywhere the photo is rendered.
///
/// The editor's live preview intentionally skips the overlay step and draws
/// overlays on a dedicated canvas layer instead, so dragging stays smooth.
pub fn apply_recipe(image: RgbaImage, recipe: &EditRecipe) -> RgbaImage {
    super::overlays::composite_overlays(apply_tone(apply_geometry(image, recipe), recipe), recipe)
}

pub fn estimated_output_dimensions(width: u32, height: u32, recipe: &EditRecipe) -> (u32, u32) {
    let (mut width, mut height) = (width.max(1) as f32, height.max(1) as f32);
    if recipe.straighten.abs() >= 0.01 {
        let rotated = largest_rotated_rect(width, height, recipe.straighten.to_radians());
        width = rotated.0.max(1.0);
        height = rotated.1.max(1.0);
    }
    let crop = recipe.crop.normalized();
    width *= (crop.right - crop.left).max(0.01);
    height *= (crop.bottom - crop.top).max(0.01);
    (
        width.round().max(1.0) as u32,
        height.round().max(1.0) as u32,
    )
}

pub fn render_thumbnail(path: &str, rotation: i32, recipe: &EditRecipe) -> Result<RgbaImage> {
    let image = image::open(path)
        .with_context(|| format!("could not open cached thumbnail {path}"))?
        .to_rgba8();
    Ok(apply_recipe(
        apply_library_rotation(image, rotation),
        recipe,
    ))
}

pub fn decode_base_for_viewer(
    reference: &str,
    rotation: i32,
    width: u32,
    height: u32,
) -> Result<RgbaImage> {
    let image = crate::thumbnail::decode_for_viewer(reference, width.max(1), height.max(1))?;
    Ok(apply_library_rotation(image, rotation))
}

pub fn render_for_viewer(
    reference: &str,
    rotation: i32,
    recipe: &EditRecipe,
    width: u32,
    height: u32,
) -> Result<RgbaImage> {
    Ok(apply_recipe(
        decode_base_for_viewer(reference, rotation, width, height)?,
        recipe,
    ))
}

/// Render a full-quality export from plain Rust values.
///
/// Keep GTK/GLib objects out of this API so callers can safely run the heavy
/// decode/render work on a `std::thread`. `PhotoObject` is a GTK object and is
/// intentionally neither `Send` nor `Sync`.
pub fn render_for_export(
    reference: &str,
    rotation: i32,
    edit_recipe: &str,
    source_width: i64,
    source_height: i64,
) -> Result<RgbaImage> {
    let (width, height) = if source_width > 0 && source_height > 0 {
        // A square decode *bound* preserves full sensor/photo resolution
        // whether EXIF orientation and/or PIC's library rotation swaps axes.
        // The decoder still caps actual output to the source's native size.
        let long_edge = (source_width as u32).max(source_height as u32);
        (long_edge, long_edge)
    } else {
        // RAW/remote records can lack catalog dimensions. An unbounded request
        // lets the viewer decoder clamp to the largest source/embedded preview
        // it can actually decode instead of exporting a 1x1 image.
        (u32::MAX, u32::MAX)
    };
    // Never use a portrait-only viewport for a landscape source or vice
    // versa. EXIF orientation is applied by the decoder and PIC library
    // rotation by render_for_viewer only after the native decode.
    let recipe = EditRecipe::decode(edit_recipe);
    // The interactive renderer can skip missing overlay assets to remain
    // usable while offline. Export must not silently drop a visible logo or
    // watermark from a supposedly finished image.
    for overlay in &recipe.overlays {
        if overlay.visible && overlay.opacity > 0.001
            && super::overlays::load_asset(&overlay.asset).is_none()
        {
            anyhow::bail!(
                "cannot export: required image overlay asset {} is missing or unreadable",
                overlay.asset
            );
        }
    }
    render_for_viewer(reference, rotation, &recipe, width, height)
}

fn crop_normalized(image: &RgbaImage, crop: CropRect) -> RgbaImage {
    let crop = crop.normalized();
    let width = image.width().max(1);
    let height = image.height().max(1);
    let x = (crop.left * width as f32).round() as u32;
    let y = (crop.top * height as f32).round() as u32;
    let right = (crop.right * width as f32).round() as u32;
    let bottom = (crop.bottom * height as f32).round() as u32;
    let x = x.min(width - 1);
    let y = y.min(height - 1);
    let crop_width = right.saturating_sub(x).clamp(1, width - x);
    let crop_height = bottom.saturating_sub(y).clamp(1, height - y);
    image::imageops::crop_imm(image, x, y, crop_width, crop_height).to_image()
}

/// Composite a filter at a configurable intensity before manual tone tools.
fn blend_filter_amount(image: &mut RgbaImage, original: &RgbaImage, amount: f32) {
    let t = amount.clamp(0.0, 1.0);
    image.as_mut().par_chunks_mut(4)
        .zip(original.as_raw().par_chunks(4))
        .for_each(|(processed, base)| {
            for channel in 0..3 {
                let value = base[channel] as f32
                    + (processed[channel] as f32 - base[channel] as f32) * t;
                processed[channel] = value.round().clamp(0.0, 255.0) as u8;
            }
            processed[3] = base[3];
        });
}

pub(super) fn apply_tone(mut image: RgbaImage, recipe: &EditRecipe) -> RgbaImage {
    // Keep the unedited image as-is. Besides avoiding a full pixel loop for
    // the common zero-adjustment state, this preserves every decoded detail
    // and makes reset pixel-identical to the source preview.
    if recipe.filter == filters::FilterPreset::None
        && !recipe.auto_color
        && !recipe.auto_contrast
        && recipe.exposure == 0.0
        && recipe.temperature == 0.0
        && recipe.saturation == 0.0
        && recipe.fill_light == 0.0
        && recipe.shadows == 0.0
        && recipe.highlights == 0.0
        && recipe.contrast == 0.0
        && !recipe.black_white
        && !recipe.sepia
        && recipe.clarity == 0.0
        && recipe.sharpen == 0.0
        && recipe.grain == 0.0
    {
        return image;
    }
    let filter_amount = recipe.filter_amount.clamp(0.0, 1.0);
    if recipe.filter != filters::FilterPreset::None && filter_amount > 0.0 {
        if filter_amount >= 1.0 {
            // Keep the legacy 100% path identical, with no extra copy.
            filters::apply_filter(&mut image, recipe.filter);
        } else {
            let unfiltered = image.clone();
            filters::apply_filter(&mut image, recipe.filter);
            blend_filter_amount(&mut image, &unfiltered, filter_amount);
        }
    }

    if recipe.auto_color {
        apply_auto_color(&mut image);
    }
    if recipe.auto_contrast {
        apply_auto_contrast(&mut image);
    }

    let exposure = 2.0_f32.powf(recipe.exposure);
    let temperature = recipe.temperature;
    let saturation = 1.0 + recipe.saturation;
    image.as_mut().par_chunks_mut(4).for_each(|pixel| {
        let alpha = pixel[3];
        let mut r = pixel[0] as f32 / 255.0;
        let mut g = pixel[1] as f32 / 255.0;
        let mut b = pixel[2] as f32 / 255.0;

        r *= exposure;
        g *= exposure;
        b *= exposure;

        // Work on luminance, not three equal RGB offsets. Fill Light is a
        // broad lower-midrange lift; Shadows concentrates on deeper tones;
        // Highlights acts on the upper range. A shared gain retains colour
        // ratios instead of washing dark areas grey.
        if recipe.fill_light.abs() > 0.0001
            || recipe.shadows.abs() > 0.0001
            || recipe.highlights.abs() > 0.0001
        {
            let luma = rgb_luma(r, g, b);
            if luma > 0.001 && luma < 1.0 {
                let target = map_tone_regions(luma, recipe);
                let gain = (target / luma).clamp(0.12, 5.0);
                r *= gain;
                g *= gain;
                b *= gain;
            }
        }

        r *= 1.0 + temperature * 0.18;
        b *= 1.0 - temperature * 0.18;
        g *= 1.0 + temperature.abs() * 0.015;

        let gray = 0.2126 * r + 0.7152 * g + 0.0722 * b;
        r = gray + (r - gray) * saturation;
        g = gray + (g - gray) * saturation;
        b = gray + (b - gray) * saturation;

        if recipe.contrast.abs() > 0.0001 {
            r = apply_contrast_curve(r, recipe.contrast);
            g = apply_contrast_curve(g, recipe.contrast);
            b = apply_contrast_curve(b, recipe.contrast);
        }

        // B&W and Sepia tiles also respect Filter Amount; manual exposure,
        // warmth, contrast and other tools remain independent of this value.
        let pre_look = (r, g, b);
        if recipe.black_white {
            let y = rgb_luma(r, g, b).clamp(0.0, 1.0);
            r = y;
            g = y;
            b = y;
        }
        if recipe.sepia {
            // Photographic sepia is a toned monochrome print, not a full-strength
            // RGB matrix. Preserve luminance, then add a restrained warm tint.
            let y = rgb_luma(r, g, b).clamp(0.0, 1.0);
            let strength = 0.75 * (1.0 - 0.20 * (2.0 * y - 1.0).abs());
            r = y * (1.0 + 0.080 * strength);
            g = y * (1.0 - 0.006 * strength);
            b = y * (1.0 - 0.180 * strength);
        }
        if (recipe.black_white || recipe.sepia) && filter_amount < 1.0 {
            r = pre_look.0 + (r - pre_look.0) * filter_amount;
            g = pre_look.1 + (g - pre_look.1) * filter_amount;
            b = pre_look.2 + (b - pre_look.2) * filter_amount;
        }
        pixel.copy_from_slice(&[
            (r.clamp(0.0, 1.0) * 255.0).round() as u8,
            (g.clamp(0.0, 1.0) * 255.0).round() as u8,
            (b.clamp(0.0, 1.0) * 255.0).round() as u8,
            alpha,
        ]);
    });

    if recipe.clarity.abs() > 0.001 && image.width() > 2 && image.height() > 2 {
        apply_clarity(&mut image, recipe.clarity);
    }

    if recipe.sharpen > 0.001 && image.width() > 2 && image.height() > 2 {
        let blur_started = std::time::Instant::now();
        let blurred = image::imageops::blur(&image, 1.1);
        let blur_ms = blur_started.elapsed().as_secs_f64() * 1000.0;
        let apply_started = std::time::Instant::now();
        let amount = recipe.sharpen * 1.6;
        image
            .as_mut()
            .par_chunks_mut(4)
            .zip(blurred.as_raw().par_chunks(4))
            .for_each(|(pixel, blurred)| {
                for channel in 0..3 {
                    let original = pixel[channel] as f32;
                    let soft = blurred[channel] as f32;
                    pixel[channel] = (original + (original - soft) * amount)
                        .clamp(0.0, 255.0)
                        .round() as u8;
                }
            });
        if std::env::var_os("PICASA_TRACE").is_some() {
            eprintln!(
                "PIC_EDIT_EFFECT op=sharpen dimensions={}x{} blur_ms={blur_ms:.3} apply_ms={:.3} amount={:.3}",
                image.width(),
                image.height(),
                apply_started.elapsed().as_secs_f64() * 1000.0,
                recipe.sharpen,
            );
        }
    }
    if recipe.grain > 0.001 {
        filters::apply_grain(&mut image, recipe.grain * 0.24);
    }
    image
}

/// Smooth transition between tonal regions (no abrupt masks or banding).
fn smoothstep(low: f32, high: f32, y: f32) -> f32 {
    let t = ((y - low) / (high - low)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Parametric masked gamma curves preserve black/white anchors and keep
/// exposure/fill/shadows/highlights distinct; no broad additive RGB wash.
fn map_tone_regions(y: f32, recipe: &EditRecipe) -> f32 {
    let y = y.clamp(0.0, 1.0);
    let fill = y.powf(2.0_f32.powf(-recipe.fill_light * 1.15));
    let shadows = y.powf(2.0_f32.powf(-recipe.shadows * 1.35));
    let highlights = 1.0 - (1.0 - y).powf(2.0_f32.powf(recipe.highlights * 1.20));

    let fill_mask = 1.0 - smoothstep(0.48, 0.93, y);
    let shadow_mask = 1.0 - smoothstep(0.18, 0.61, y);
    let highlight_mask = smoothstep(0.48, 0.87, y);
    (y + (fill - y) * fill_mask * 0.95
        + (shadows - y) * shadow_mask * 0.95
        + (highlights - y) * highlight_mask * 0.92).clamp(0.0, 1.0)
}

/// Clarity is a broad local-contrast boost in luminance only, strongest in
/// midtones. Downsample before Gaussian blur to keep 24MP+ exports tractable
/// and to give previews and exports comparable scene-relative detail radius.
fn apply_clarity(image: &mut RgbaImage, amount: f32) {
    let (width, height) = image.dimensions();
    if width < 3 || height < 3 {
        return;
    }
    let luma = image::GrayImage::from_fn(width, height, |x, y| {
        let p = image.get_pixel(x, y);
        image::Luma([rgb_luma(
            p[0] as f32, p[1] as f32, p[2] as f32,
        ).round().clamp(0.0, 255.0) as u8])
    });
    let downsample = (width.max(height) as f32 / 1200.0).max(1.0);
    let small_width = ((width as f32 / downsample).round() as u32).max(1);
    let small_height = ((height as f32 / downsample).round() as u32).max(1);
    let reduced = image::imageops::resize(
        &luma, small_width, small_height, image::imageops::FilterType::Triangle,
    );
    let sigma = (small_width.min(small_height) as f32 * 0.008).clamp(1.5, 20.0);
    let softened = image::imageops::blur(&reduced, sigma);
    let local = image::imageops::resize(
        &softened, width, height, image::imageops::FilterType::Triangle,
    );
    image.as_mut().par_chunks_mut(4).zip(local.as_raw().par_iter()).for_each(|(p, &blurred)| {
        if p[3] == 0 {
            return;
        }
        let y = rgb_luma(p[0] as f32, p[1] as f32, p[2] as f32) / 255.0;
        let midtones = (4.0 * y * (1.0 - y)).clamp(0.0, 1.0);
        let local_y = blurred as f32 / 255.0;
        let detail = (y - local_y).clamp(-0.24, 0.24);
        let target = (y + detail * midtones * amount * 1.85).clamp(0.0, 1.0);
        let gain = (target / y.max(0.01)).clamp(0.25, 3.0);
        for channel in &mut p[0..3] {
            *channel = (*channel as f32 * gain).round().clamp(0.0, 255.0) as u8;
        }
    });
}

fn rgb_luma(r: f32, g: f32, b: f32) -> f32 {
    0.2126 * r + 0.7152 * g + 0.0722 * b
}

fn apply_contrast_curve(value: f32, contrast: f32) -> f32 {
    let x = value.clamp(0.0, 1.0);
    // Keep the visible slider range expressive without letting +1.0 collapse
    // ordinary near-black/near-white detail to pure clipping.
    let amount = contrast.clamp(-1.0, 1.0) * 0.55;
    // Smooth S-curve with fixed black, midpoint and white anchors. Positive
    // values deepen shadows and lift highlights; negative values compress them.
    let curve = 4.0 * (x - 0.5) * x * (1.0 - x);
    (x + amount * curve).clamp(0.0, 1.0)
}

fn apply_auto_color(image: &mut RgbaImage) {
    // Estimate a restrained neutral correction from low-chroma midtones only.
    // Sampling saturated scenery/skin would mistake intentional colour for a
    // cast; when the photo has no reliable neutrals, leave it untouched.
    let mut sums = [0.0_f64; 3];
    let mut count = 0.0_f64;
    let step = ((image.width() as u64 * image.height() as u64) / 200_000).max(1) as usize;
    for pixel in image.pixels().step_by(step) {
        if pixel[3] < 16 { continue; }
        let [r, g, b] = [pixel[0] as f64, pixel[1] as f64, pixel[2] as f64];
        let luma = (0.2126 * r + 0.7152 * g + 0.0722 * b) / 255.0;
        if !(0.07..=0.93).contains(&luma) { continue; }
        let chroma = (r.max(g).max(b) - r.min(g).min(b)) / r.max(g).max(b).max(1.0);
        if chroma > 0.32 { continue; }
        let weight = (1.0 - chroma / 0.32).powi(2);
        sums[0] += r * weight;
        sums[1] += g * weight;
        sums[2] += b * weight;
        count += weight;
    }
    if count < 0.5 { return; }

    let means = [sums[0] / count, sums[1] / count, sums[2] / count];
    let neutral = 0.2126 * means[0] + 0.7152 * means[1] + 0.0722 * means[2];
    let gains = [
        1.0 + ((neutral / means[0].max(1.0)).clamp(0.82, 1.22) - 1.0) * 0.65,
        1.0 + ((neutral / means[1].max(1.0)).clamp(0.82, 1.22) - 1.0) * 0.65,
        1.0 + ((neutral / means[2].max(1.0)).clamp(0.82, 1.22) - 1.0) * 0.65,
    ];
    image.as_mut().par_chunks_mut(4).for_each(|pixel| {
        if pixel[3] < 16 { return; }
        for c in 0..3 {
            pixel[c] = (pixel[c] as f64 * gains[c])
                .clamp(0.0, 255.0).round() as u8;
        }
    });
}

fn apply_auto_contrast(image: &mut RgbaImage) {
    let mut histogram = [0u64; 256];
    let step = ((image.width() as u64 * image.height() as u64) / 250_000).max(1) as usize;
    let mut samples = 0u64;
    for pixel in image.pixels().step_by(step) {
        if pixel[3] < 16 { continue; }
        let bucket = rgb_luma(pixel[0] as f32, pixel[1] as f32, pixel[2] as f32)
            .round().clamp(0.0, 255.0) as usize;
        histogram[bucket] += 1;
        samples += 1;
    }
    if samples < 4 { return; }

    // Robust percentile bounds: small specular reflections and individual
    // dark pixels must not decide the whole photo's black/white points.
    let clip = if samples >= 100 {
        ((samples as f64 * 0.01).round() as u64).max(1)
    } else { 0 };
    let mut total = 0u64;
    let mut low = 0usize;
    for (i, count) in histogram.iter().enumerate() {
        total += *count;
        if total > clip { low = i; break; }
    }
    total = 0;
    let mut high = 255usize;
    for (i, count) in histogram.iter().enumerate().rev() {
        total += *count;
        if total > clip { high = i; break; }
    }
    if high <= low + 6 { return; }
    let low = low as f32 / 255.0;
    let high = high as f32 / 255.0;
    let span = high - low;
    // Protect compressed-histogram/low-light photographs from a huge global
    // stretch. Anchor the contrast about scene luminance rather than forcing
    // arbitrary percentile samples all the way to pure black or white.
    let contrast = (0.85 / span).clamp(1.0, 3.2);
    if contrast <= 1.001 { return; }
    let pivot = (low + high) * 0.5;

    image.as_mut().par_chunks_mut(4).for_each(|pixel| {
        if pixel[3] < 16 { return; }
        let y = rgb_luma(
            pixel[0] as f32, pixel[1] as f32, pixel[2] as f32,
        ) / 255.0;
        if y <= 0.0001 { return; }
        let stretched = ((y - pivot) * contrast + pivot).clamp(0.012, 0.988);
        let corrected = y + (stretched - y) * 0.75;
        // Scale luminance uniformly to retain hue/chroma as far as possible,
        // rather than adding an RGB offset that desaturates colour.
        let gain = (corrected / y).clamp(0.2, 3.0);
        for c in 0..3 {
            pixel[c] = (pixel[c] as f32 * gain).clamp(0.0, 255.0).round() as u8;
        }
    });
}

fn rotate_autocrop(source: &RgbaImage, angle: f32) -> RgbaImage {
    if angle.abs() < 0.0001 || source.width() < 2 || source.height() < 2 {
        return source.clone();
    }
    let (output_width, output_height) =
        largest_rotated_rect(source.width() as f32, source.height() as f32, angle);
    let output_width = output_width.floor().max(1.0) as u32;
    let output_height = output_height.floor().max(1.0) as u32;
    let mut output = RgbaImage::new(output_width, output_height);
    let cos = angle.cos();
    let sin = angle.sin();
    let src_cx = (source.width() as f32 - 1.0) * 0.5;
    let src_cy = (source.height() as f32 - 1.0) * 0.5;
    let dst_cx = (output_width as f32 - 1.0) * 0.5;
    let dst_cy = (output_height as f32 - 1.0) * 0.5;

    let row_bytes = output_width as usize * 4;
    output
        .as_mut()
        .par_chunks_mut(row_bytes)
        .enumerate()
        .for_each(|(y, row)| {
            let y = y as u32;
            for x in 0..output_width {
                let dx = x as f32 - dst_cx;
                let dy = y as f32 - dst_cy;
                let sx = cos * dx + sin * dy + src_cx;
                let sy = -sin * dx + cos * dy + src_cy;
                let pixel = bilinear(source, sx, sy);
                let offset = x as usize * 4;
                row[offset..offset + 4].copy_from_slice(&pixel.0);
            }
        });
    output
}

fn bilinear(image: &RgbaImage, x: f32, y: f32) -> Rgba<u8> {
    let max_x = image.width().saturating_sub(1) as f32;
    let max_y = image.height().saturating_sub(1) as f32;
    let x = x.clamp(0.0, max_x);
    let y = y.clamp(0.0, max_y);
    let x0 = x.floor() as u32;
    let y0 = y.floor() as u32;
    let x1 = (x0 + 1).min(image.width() - 1);
    let y1 = (y0 + 1).min(image.height() - 1);
    let tx = x - x0 as f32;
    let ty = y - y0 as f32;
    let p00 = image.get_pixel(x0, y0);
    let p10 = image.get_pixel(x1, y0);
    let p01 = image.get_pixel(x0, y1);
    let p11 = image.get_pixel(x1, y1);
    let mut out = [0u8; 4];
    for channel in 0..4 {
        let top = p00[channel] as f32 * (1.0 - tx) + p10[channel] as f32 * tx;
        let bottom = p01[channel] as f32 * (1.0 - tx) + p11[channel] as f32 * tx;
        out[channel] = (top * (1.0 - ty) + bottom * ty).clamp(0.0, 255.0).round() as u8;
    }
    Rgba(out)
}

pub(super) fn largest_rotated_rect(width: f32, height: f32, angle: f32) -> (f32, f32) {
    let width = width.abs();
    let height = height.abs();
    if width <= 0.0 || height <= 0.0 {
        return (1.0, 1.0);
    }
    let sin_a = angle.sin().abs();
    let cos_a = angle.cos().abs();
    let width_is_longer = width >= height;
    let side_long = if width_is_longer { width } else { height };
    let side_short = if width_is_longer { height } else { width };
    if side_short <= 2.0 * sin_a * cos_a * side_long || (sin_a - cos_a).abs() < 0.00001 {
        let x = 0.5 * side_short;
        if width_is_longer {
            (x / sin_a.max(0.00001), x / cos_a.max(0.00001))
        } else {
            (x / cos_a.max(0.00001), x / sin_a.max(0.00001))
        }
    } else {
        let cos_2a = cos_a * cos_a - sin_a * sin_a;
        (
            (width * cos_a - height * sin_a) / cos_2a,
            (height * cos_a - width * sin_a) / cos_2a,
        )
    }
}

/// JPEG cannot store an alpha channel. Composite transparent pixels over
/// white rather than discarding their alpha (which makes PNG logos dark).
/// Opaque photo RGB samples remain bit-for-bit unchanged.
fn flatten_jpeg_alpha(image: &RgbaImage) -> image::RgbImage {
    image::RgbImage::from_fn(image.width(), image.height(), |x, y| {
        let pixel = image.get_pixel(x, y);
        let alpha = u32::from(pixel[3]);
        image::Rgb(std::array::from_fn(|channel| {
            let foreground = u32::from(pixel[channel]);
            ((foreground * alpha + 255 * (255 - alpha) + 127) / 255) as u8
        }))
    })
}

pub fn save_jpeg(image: &RgbaImage, destination: &std::path::Path, quality: u8) -> Result<()> {
    save_jpeg_with_metadata(image, destination, quality, None, None)
}

fn save_jpeg_with_metadata(
    image: &RgbaImage,
    destination: &std::path::Path,
    quality: u8,
    exif: Option<&[u8]>,
    icc: Option<&[u8]>,
) -> Result<()> {
    use image::ImageEncoder;
    let file = std::fs::File::create(destination)
        .with_context(|| format!("could not create {}", destination.display()))?;
    let writer = std::io::BufWriter::new(file);
    let rgb = flatten_jpeg_alpha(image);
    let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(writer, quality);
    if let Some(metadata) = exif {
        encoder.set_exif_metadata(metadata.to_vec())
            .map_err(|error| anyhow::anyhow!("JPEG EXIF metadata rejected: {error}"))?;
    }
    if let Some(profile) = icc {
        encoder.set_icc_profile(profile.to_vec())
            .map_err(|error| anyhow::anyhow!("JPEG ICC profile rejected: {error}"))?;
    }
    encoder.write_image(
        &rgb,
        rgb.width(),
        rgb.height(),
        image::ExtendedColorType::Rgb8,
    )?;
    Ok(())
}

/// Test helper for exports that have no original file metadata.
#[cfg(test)]
pub fn save_export(
    image: &RgbaImage,
    destination: &std::path::Path,
    format: crate::edit::export_batch::ExportFormat,
    quality: u8,
) -> Result<()> {
    save_export_with_metadata(image, destination, format, quality, None)
}

/// Export rendered pixels with rebuilt, orientation-normalized EXIF and an
/// sRGB profile. Never copy a source ICC verbatim: pixels may have been color
/// converted or edited and may no longer be in the source's color space.
pub fn save_export_with_metadata(
    image: &RgbaImage,
    destination: &std::path::Path,
    format: crate::edit::export_batch::ExportFormat,
    quality: u8,
    exif: Option<&[u8]>,
) -> Result<()> {
    use image::ImageEncoder;
    use crate::edit::export_batch::ExportFormat;
    let icc = if exif.is_some() {
        Some(lcms2::Profile::new_srgb().icc()
            .map_err(|error| anyhow::anyhow!("could not generate sRGB profile: {error}"))?)
    } else {
        None
    };
    match format {
        ExportFormat::Jpeg => save_jpeg_with_metadata(
            image, destination, quality, exif, icc.as_deref()
        ),
        ExportFormat::Png => {
            let file = std::fs::File::create(destination)
                .with_context(|| format!("could not create {}", destination.display()))?;
            let mut encoder = image::codecs::png::PngEncoder::new(std::io::BufWriter::new(file));
            if let Some(exif) = exif {
                encoder.set_exif_metadata(exif.to_vec())
                    .map_err(|error| anyhow::anyhow!("PNG EXIF rejected: {error}"))?;
            }
            if let Some(icc) = icc {
                encoder.set_icc_profile(icc)
                    .map_err(|error| anyhow::anyhow!("PNG ICC rejected: {error}"))?;
            }
            encoder.write_image(
                image.as_raw(), image.width(), image.height(),
                image::ExtendedColorType::Rgba8,
            )?;
            Ok(())
        }
        ExportFormat::Webp => {
            let file = std::fs::File::create(destination)
                .with_context(|| format!("could not create {}", destination.display()))?;
            let mut encoder = image::codecs::webp::WebPEncoder::new_lossless(
                std::io::BufWriter::new(file)
            );
            if let Some(exif) = exif {
                encoder.set_exif_metadata(exif.to_vec())
                    .map_err(|error| anyhow::anyhow!("WebP EXIF rejected: {error}"))?;
            }
            if let Some(icc) = icc {
                encoder.set_icc_profile(icc)
                    .map_err(|error| anyhow::anyhow!("WebP ICC rejected: {error}"))?;
            }
            encoder.write_image(
                image.as_raw(), image.width(), image.height(),
                image::ExtendedColorType::Rgba8,
            )?;
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::DynamicImage;
    use crate::edit::filters::FilterPreset;

    #[test]
    fn export_refuses_to_silently_omit_a_missing_visible_logo() {
        let mut recipe = EditRecipe::default();
        recipe.overlays.push(super::super::model::OverlaySpec::new_centered(
            "nonexistent-pic-overlay-unique-test-asset",
        ));
        let error = render_for_export("/unused-original.jpg", 0, &recipe.encode(), 120, 80)
            .unwrap_err().to_string();
        assert!(error.contains("required image overlay asset"), "{error}");

        // A deliberately hidden overlay is not part of the exported recipe.
        recipe.overlays[0].visible = false;
        let hidden_error = render_for_export(
            "/unused-original.jpg", 0, &recipe.encode(), 120, 80,
        ).unwrap_err().to_string();
        assert!(!hidden_error.contains("required image overlay asset"));
    }

    #[test]
    fn library_quarter_turn_export_keeps_full_native_resolution() {
        let path = std::env::temp_dir().join(format!(
            "pic-export-rotation-{}-{}.png", std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos(),
        ));
        let source = RgbaImage::from_fn(120, 80, |x, y| {
            Rgba([(x * 2) as u8, (y * 3) as u8, (x ^ y) as u8, 255])
        });
        DynamicImage::ImageRgba8(source.clone()).save(&path).unwrap();
        for rotation in [90, 180, 270] {
            let output = render_for_export(
                path.to_str().unwrap(), rotation, "", 120, 80,
            ).unwrap();
            let expected = apply_library_rotation(source.clone(), rotation);
            assert_eq!(output.dimensions(), expected.dimensions(),
                "library rotation {rotation} downsampled the original");
            assert_eq!(output, expected,
                "library rotation {rotation} changed decoded source pixels");
        }
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn jpeg_export_alpha_matte_preserves_opaque_and_flattens_transparency() {
        let pixels = RgbaImage::from_fn(3, 1, |x, _| match x {
            0 => Rgba([12, 34, 56, 255]),
            1 => Rgba([0, 40, 120, 128]),
            _ => Rgba([0, 0, 0, 0]),
        });
        let result = flatten_jpeg_alpha(&pixels);
        assert_eq!(result.get_pixel(0, 0).0, [12, 34, 56]);
        assert_eq!(result.get_pixel(1, 0).0, [127, 147, 187]);
        assert_eq!(result.get_pixel(2, 0).0, [255, 255, 255]);
    }

    #[test]
    fn jpeg_file_export_uses_white_background_for_transparent_pixels() {
        let source = RgbaImage::from_fn(64, 32, |x, _| {
            if x < 32 { Rgba([0, 0, 0, 0]) }
            else { Rgba([35, 140, 230, 255]) }
        });
        let path = std::env::temp_dir().join(format!(
            "pic-export-jpeg-alpha-{}-{}.jpg", std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos(),
        ));
        save_export(&source, &path, crate::edit::export_batch::ExportFormat::Jpeg, 100)
            .unwrap();
        let decoded = image::open(&path).unwrap().to_rgb8();
        let transparent = decoded.get_pixel(4, 16).0;
        let opaque = decoded.get_pixel(58, 16).0;
        assert!(transparent.iter().all(|&channel| channel > 248),
            "JPEG transparency lost matte: {transparent:?}");
        for (actual, expected) in opaque.iter().zip([35u8, 140, 230]) {
            assert!(actual.abs_diff(expected) <= 5,
                "JPEG altered fully opaque image: actual={actual} expected={expected}");
        }
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn png_export_retains_transparent_overlay_pixels_exactly() {
        let image = RgbaImage::from_fn(4, 4, |x, y|
            Rgba([x as u8 * 35, y as u8 * 41, 120, (x * 51 + y * 13) as u8])
        );
        let path = std::env::temp_dir().join(format!(
            "pic-export-alpha-{}-{}.png", std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos(),
        ));
        save_export(&image, &path, crate::edit::export_batch::ExportFormat::Png, 90).unwrap();
        let reread = image::open(&path).unwrap().to_rgba8();
        assert_eq!(reread, image);
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn filter_amount_blends_only_creative_look_not_other_adjustments() {
        let source = RgbaImage::from_fn(24, 24, |x, y| {
            Rgba([(x * 9) as u8, (y * 8) as u8, 118, 196])
        });
        for preset in [FilterPreset::Newspaper, FilterPreset::OldPhoto, FilterPreset::Cinematic] {
            let mut recipe = EditRecipe::default();
            recipe.filter = preset;
            recipe.exposure = 0.25;
            recipe.filter_amount = 0.0;
            let base = apply_recipe(source.clone(), &recipe);
            recipe.filter_amount = 1.0;
            let full = apply_recipe(source.clone(), &recipe);
            recipe.filter_amount = 0.5;
            let half = apply_recipe(source.clone(), &recipe);
            assert_ne!(base.as_raw(), full.as_raw());
            for ((base, full), mid) in base.pixels().zip(full.pixels()).zip(half.pixels()) {
                for c in 0..3 {
                    let lo = base[c].min(full[c]) as i16;
                    let hi = base[c].max(full[c]) as i16;
                    assert!((lo - 1..=hi + 1).contains(&(mid[c] as i16)),
                        "{preset:?} midpoint should be between base and full look");
                }
                assert_eq!(mid[3], 196);
            }
        }
    }

    #[test]
    fn bw_and_sepia_amount_zero_are_unfiltered_and_half_is_intermediate() {
        let source = RgbaImage::from_pixel(6, 6, Rgba([190, 102, 54, 255]));
        for preset in [FilterPreset::None] {
            for (bw, sepia) in [(true, false), (false, true)] {
                let mut recipe = EditRecipe::default();
                recipe.filter = preset;
                recipe.black_white = bw;
                recipe.sepia = sepia;
                recipe.filter_amount = 0.0;
                assert_eq!(apply_recipe(source.clone(), &recipe), source);
                recipe.filter_amount = 1.0;
                let full = apply_recipe(source.clone(), &recipe);
                recipe.filter_amount = 0.5;
                let half = apply_recipe(source.clone(), &recipe);
                assert_ne!(half, source);
                assert_ne!(half, full);
                assert_eq!(half.get_pixel(0, 0)[3], 255);
            }
        }
    }

    #[test]
    fn crop_reduces_dimensions() {
        let image = RgbaImage::new(100, 80);
        let mut recipe = EditRecipe::default();
        recipe.crop = CropRect {
            left: 0.25,
            top: 0.25,
            right: 0.75,
            bottom: 0.75,
        };
        let output = apply_recipe(image, &recipe);
        assert_eq!(output.dimensions(), (50, 40));
    }

    #[test]
    fn named_filter_is_followed_by_manual_tone_adjustments() {
        let image = RgbaImage::from_pixel(8, 8, Rgba([120, 130, 140, 255]));
        let mut recipe = EditRecipe::default();
        recipe.filter = FilterPreset::Clarendon;
        recipe.exposure = 0.5;

        let output = apply_recipe(image.clone(), &recipe);
        let mut filter_only = image;
        crate::edit::filters::apply_filter(&mut filter_only, FilterPreset::Clarendon);
        let exposure = 2.0_f32.powf(0.5);
        let mut expected = filter_only.clone();
        for pixel in expected.pixels_mut() {
            for channel in 0..3 {
                pixel[channel] = (pixel[channel] as f32 * exposure).clamp(0.0, 255.0).round() as u8;
            }
        }

        assert_ne!(output.as_raw(), filter_only.as_raw());
        assert_eq!(output.as_raw(), expected.as_raw());
    }

    #[test]
    fn cached_geometry_then_tone_matches_full_recipe() {
        let mut source = RgbaImage::new(120, 90);
        for (index, pixel) in source.pixels_mut().enumerate() {
            let value = (index % 251) as u8;
            *pixel = Rgba([value, 255 - value, value / 2, 255]);
        }

        let mut recipe = EditRecipe::default();
        recipe.straighten = 3.0;
        recipe.crop = CropRect {
            left: 0.1,
            top: 0.15,
            right: 0.9,
            bottom: 0.85,
        };
        recipe.exposure = 0.35;
        recipe.contrast = 0.2;
        recipe.saturation = 0.15;

        let full = apply_recipe(source.clone(), &recipe);
        let geometry = apply_geometry(source, &recipe);
        let cached = apply_tone(geometry, &recipe);

        assert_eq!(cached.dimensions(), full.dimensions());
        assert_eq!(cached.as_raw(), full.as_raw());
    }

    #[test]
    fn straightening_autocrops_black_corners() {
        let image = RgbaImage::from_pixel(100, 80, Rgba([200, 200, 200, 255]));
        let mut recipe = EditRecipe::default();
        recipe.straighten = 5.0;
        let output = apply_recipe(image, &recipe);
        assert!(output.width() < 100);
        assert!(output.height() < 80);
        assert!(output.pixels().all(|pixel| pixel[3] == 255));
    }

    #[test]
    fn sepia_is_warm_monochrome_and_bw_plus_sepia_is_equivalent() {
        let source = RgbaImage::from_pixel(2, 2, Rgba([200, 100, 50, 255]));
        let mut combined_recipe = EditRecipe::default();
        combined_recipe.black_white = true;
        combined_recipe.sepia = true;
        let combined = apply_recipe(source.clone(), &combined_recipe);

        let mut bw_recipe = EditRecipe::default();
        bw_recipe.black_white = true;
        let black_white = apply_recipe(source.clone(), &bw_recipe);

        let mut sepia_recipe = EditRecipe::default();
        sepia_recipe.sepia = true;
        let sepia = apply_recipe(source, &sepia_recipe);

        assert_ne!(sepia.as_raw(), black_white.as_raw());
        assert_eq!(combined.as_raw(), sepia.as_raw());
        let pixel = sepia.get_pixel(0, 0);
        assert!(pixel[0] > pixel[1] && pixel[1] > pixel[2]);
    }

    #[test]
    fn positive_contrast_expands_midtones() {
        let mut source = RgbaImage::new(2, 1);
        source.put_pixel(0, 0, Rgba([96, 96, 96, 255]));
        source.put_pixel(1, 0, Rgba([160, 160, 160, 255]));
        let mut recipe = EditRecipe::default();
        recipe.contrast = 0.75;
        let output = apply_recipe(source, &recipe);
        let delta = output.get_pixel(1, 0)[0] as i16 - output.get_pixel(0, 0)[0] as i16;
        assert!(delta > 64);
    }

    #[test]
    fn sepia_preserves_midtone_luminance() {
        let source = RgbaImage::from_pixel(1, 1, Rgba([128, 128, 128, 255]));
        let mut recipe = EditRecipe::default();
        recipe.sepia = true;
        let output = apply_recipe(source, &recipe);
        let pixel = output.get_pixel(0, 0);
        let luma = 0.2126 * pixel[0] as f32 + 0.7152 * pixel[1] as f32 + 0.0722 * pixel[2] as f32;
        assert!((luma - 128.0).abs() < 12.0);
    }

    #[test]
    fn auto_contrast_ignores_single_black_and_white_outliers() {
        let mut source = RgbaImage::new(100, 1);
        source.put_pixel(0, 0, Rgba([0, 0, 0, 255]));
        source.put_pixel(99, 0, Rgba([255, 255, 255, 255]));
        for x in 1..99 {
            let value = if x % 2 == 0 { 100 } else { 120 };
            source.put_pixel(x, 0, Rgba([value, value, value, 255]));
        }
        let mut recipe = EditRecipe::default();
        recipe.auto_contrast = true;
        let output = apply_recipe(source, &recipe);
        let dark = output.get_pixel(2, 0)[0] as i16;
        let light = output.get_pixel(1, 0)[0] as i16;
        assert!(light - dark > 45, "contrast should improve useful range");
        assert!(light - dark < 85, "auto contrast should not crush a narrow histogram");
    }

    #[test]
    fn fill_shadows_and_highlights_affect_different_tonal_regions() {
        let levels = [30_u8, 74, 145, 224];
        let source = RgbaImage::from_fn(4, 1, |x, _| {
            let y = levels[x as usize];
            Rgba([y, y, y, 255])
        });
        let mut fill = EditRecipe::default();
        fill.fill_light = 0.8;
        let mut shadows = EditRecipe::default();
        shadows.shadows = 0.8;
        let mut highlights = EditRecipe::default();
        highlights.highlights = -0.8;
        let a = apply_recipe(source.clone(), &fill);
        let b = apply_recipe(source.clone(), &shadows);
        let c = apply_recipe(source.clone(), &highlights);
        assert!(a.get_pixel(1, 0)[0] > source.get_pixel(1, 0)[0]);
        assert!(b.get_pixel(0, 0)[0] > source.get_pixel(0, 0)[0]);
        assert_ne!(a.as_raw(), b.as_raw(), "Fill and Shadows must be independent");
        assert!(c.get_pixel(3, 0)[0] < source.get_pixel(3, 0)[0]);
        assert!((c.get_pixel(0, 0)[0] as i16 - 30).abs() <= 2);
    }

    #[test]
    fn clarity_changes_midtone_edges_without_changing_alpha() {
        let source = RgbaImage::from_fn(96, 96, |x, _| {
            let v = if x < 48 { 80 } else { 162 };
            Rgba([v, v, v, 175])
        });
        let mut stronger = EditRecipe::default();
        stronger.clarity = 1.0;
        let output = apply_recipe(source.clone(), &stronger);
        assert_ne!(output.as_raw(), source.as_raw());
        assert_eq!(output.get_pixel(45, 42)[3], 175);
        assert_eq!(output.get_pixel(70, 42)[0], 162);
        stronger.clarity = -1.0;
        let softened = apply_recipe(source.clone(), &stronger);
        assert_ne!(output.as_raw(), softened.as_raw());
    }

    #[test]
    fn grain_is_deterministic_and_preserves_alpha() {
        let source = RgbaImage::from_pixel(40, 40, Rgba([128, 131, 136, 91]));
        let mut recipe = EditRecipe::default();
        recipe.grain = 0.75;
        let first = apply_recipe(source.clone(), &recipe);
        let second = apply_recipe(source.clone(), &recipe);
        assert_eq!(first.as_raw(), second.as_raw());
        assert_ne!(first.as_raw(), source.as_raw());
        assert!(first.pixels().all(|p| p[3] == 91));
    }

    #[test]
    fn auto_color_uses_neutrals_without_erasing_warm_colours() {
        let cast = RgbaImage::from_pixel(30, 30, Rgba([137, 125, 114, 255]));
        let mut recipe = EditRecipe::default();
        recipe.auto_color = true;
        let p = apply_recipe(cast, &recipe).get_pixel(0, 0).0;
        assert!((p[0] as i16 - p[2] as i16).abs() < 23);
    }

    #[test]
    fn auto_color_keeps_intentional_warmth() {
        let source = RgbaImage::from_pixel(20, 20, Rgba([160, 120, 90, 255]));
        let mut recipe = EditRecipe::default();
        recipe.auto_color = true;
        let output = apply_recipe(source, &recipe);
        let pixel = output.get_pixel(0, 0);
        assert!(pixel[0] > pixel[1] && pixel[1] > pixel[2]);
    }
}
