use anyhow::{Context, Result};
use image::{DynamicImage, ImageEncoder, RgbaImage};
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;
use std::sync::OnceLock;

use super::model::{Background, CollageProject};

#[derive(Clone, Copy, Debug)]
pub enum ExportFormat {
    Jpeg,
    Png,
}

#[derive(Clone, Copy, Debug)]
pub struct ExportOptions {
    /// Long-edge pixel cap; the collage aspect decides which side binds.
    pub max_edge: u32,
    pub format: ExportFormat,
    /// JPEG-only quality (60-100), ignored for PNG.
    pub jpeg_quality: u8,
}

pub fn export(project: &CollageProject, destination: &Path, options: &ExportOptions) -> Result<()> {
    let offline = project
        .items
        .iter()
        .filter(|item| !crate::source::file_available(&item.photo.path))
        .map(|item| item.photo.filename.as_str())
        .collect::<Vec<_>>();
    if !offline.is_empty() {
        let files = offline
            .iter()
            .map(|filename| format!("• {filename}"))
            .collect::<Vec<_>>()
            .join("\n");
        return Err(anyhow::anyhow!(
            "Cannot export collage because these original files are offline:\n{files}"
        ));
    }

    // The long edge is capped by the requested preset; the collage aspect
    // decides which dimension binds.
    let ratio = project.effective_aspect_ratio();
    let (width, height) = if ratio >= 1.0 {
        (
            options.max_edge,
            (options.max_edge as f32 / ratio).round() as u32,
        )
    } else {
        (
            (options.max_edge as f32 * ratio).round() as u32,
            options.max_edge,
        )
    };
    let width = width.max(1);
    let height = height.max(1);
    let background = match project.background {
        Background::White => image::Rgba([255, 255, 255, 255]),
        Background::Black => image::Rgba([0, 0, 0, 255]),
        Background::LightGray => image::Rgba([238, 238, 238, 255]),
    };
    let mut canvas = RgbaImage::from_pixel(width, height, background);
    // Resolution-scaled but uniform across every photo in the collage.
    // Matching the GTK preview, do not derive stroke width from each tile.
    let border_thickness =
        super::model::border_pixels(width.min(height), project.border_width);
    let mut items = project.items.iter().collect::<Vec<_>>();
    items.sort_by_key(|item| item.z);
    for item in items {
        let fraction = if project.round_corners {
            project.corner_radius.clamp(0.0, 0.5)
        } else {
            0.0
        };
        let geometry = super::model::tile_geometry(item, width, height, fraction);
        let target_width = geometry.w;
        let target_height = geometry.h;
        if collage_trace_enabled() {
            eprintln!(
                "PIC_COLLAGE_TRACE export_tile id={} canvas={}x{} slot=({},{},{}x{}) visible=({},{},{}x{}) center=({:.2},{:.2}) shape={:.3} stroke={} color={:?} rotate={:.2} layout={:?} contain={}",
                item.photo.id,
                width, height,
                geometry.slot_x, geometry.slot_y, geometry.slot_w, geometry.slot_h,
                geometry.x, geometry.y, geometry.w, geometry.h,
                geometry.x as f64 + geometry.w as f64 / 2.0,
                geometry.y as f64 + geometry.h as f64 / 2.0,
                fraction,
                if project.border.rgb().is_some() { border_thickness } else { 0 },
                project.border, item.rotation,
                project.layout,
                matches!(project.layout, super::model::LayoutKind::Mosaic | super::model::LayoutKind::SmartMosaic)
                    && project.keep_photo_aspect,
            );
        }
        let decoded = match crate::thumbnail::decode_for_viewer(
            &item.photo.path,
            target_width.max(1),
            target_height.max(1),
        ) {
            Ok(decoded) => decoded,
            Err(error) => {
                return Err(error).with_context(|| {
                    format!(
                        "could not decode original {} for collage export",
                        item.photo.filename
                    )
                });
            }
        };
        // decode_for_viewer has already applied source EXIF orientation. The
        // database rotation is PIC presentation metadata and is applied once
        // here, before the collage's own placement rotation.
        let source = apply_library_rotation(
            DynamicImage::ImageRgba8(decoded),
            item.photo.library_rotation,
        );
        let decoded_size = (source.width(), source.height());
        let recipe = crate::edit::EditRecipe::decode(&item.photo.edit_recipe);
        let source = DynamicImage::ImageRgba8(crate::edit::render::apply_recipe(
            source.to_rgba8(),
            &recipe,
        ));
        let contain = matches!(
            project.layout,
            super::model::LayoutKind::Mosaic | super::model::LayoutKind::SmartMosaic
        ) && project.keep_photo_aspect;
        if collage_trace_enabled() {
            eprintln!(
                "PIC_COLLAGE_TRACE export_content id={} decoded={}x{} edited={}x{} target={}x{} contain={} library_rotation={}",
                item.photo.id, decoded_size.0, decoded_size.1, source.width(), source.height(),
                target_width, target_height, contain, item.photo.library_rotation
            );
        }
        // Contain builds a complete background-backed tile before rounding.
        // The alpha mask therefore affects the letterbox as well as the photo.
        let mut image = if contain {
            fit_inside_tile(
                source,
                target_width,
                target_height,
                background,
                project.round_corners.then_some(project.corner_radius),
            )
        } else {
            cover(source, target_width, target_height)
        };
        // Clip BEFORE rotating so circles stay circular rather than turning
        // into an ellipse around a rotated rectangular bounding box.
        if project.round_corners {
            round_corners(&mut image, fraction);
        }
        // The outline is drawn inside the clipped edge so it never spills
        // into the neighbouring collage photo or changes tile placement.
        if let Some(color) = project.border.rgb() {
            draw_photo_border(&mut image, fraction, color, border_thickness);
        }
        if item.rotation.abs() > 0.1 {
            let rotate_bg = if project.round_corners {
                image::Rgba([0, 0, 0, 0])
            } else {
                background
            };
            image = rotate(&image, item.rotation.to_radians(), rotate_bg);
        }
        // The visible shape stays centered in its original collage slot,
        // including when its image has a rotation-expanded bounding box.
        let x = geometry.x + (geometry.w as i64 - image.width() as i64) / 2;
        let y = geometry.y + (geometry.h as i64 - image.height() as i64) / 2;
        if collage_trace_enabled() {
            eprintln!(
                "PIC_COLLAGE_TRACE export_composite id={} bitmap={}x{} dest=({},{}) expected_center=({:.2},{:.2})",
                item.photo.id, image.width(), image.height(), x, y,
                geometry.x as f64 + geometry.w as f64 / 2.0,
                geometry.y as f64 + geometry.h as f64 / 2.0,
            );
        }
        image::imageops::overlay(&mut canvas, &image, x, y);
    }
    let parent = destination
        .parent()
        .filter(|path| path.as_os_str().len() > 0)
        .unwrap_or_else(|| Path::new("."));
    if !parent.is_dir() {
        let error = anyhow::anyhow!("parent directory does not exist: {}", parent.display());
        return Err(error);
    }

    let file = match File::create(destination) {
        Ok(file) => file,
        Err(error) => {
            return Err(error).with_context(|| {
                format!("could not create collage file at {}", destination.display())
            });
        }
    };
    let mut writer = BufWriter::new(file);
    let encode_result = match options.format {
        ExportFormat::Jpeg => {
            let rgb = DynamicImage::ImageRgba8(canvas).to_rgb8();
            let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(
                &mut writer,
                options.jpeg_quality.clamp(60, 100),
            );
            encoder.write_image(&rgb, width, height, image::ExtendedColorType::Rgb8)
        }
        ExportFormat::Png => {
            let rgba = DynamicImage::ImageRgba8(canvas).into_rgba8();
            let encoder = image::codecs::png::PngEncoder::new(&mut writer);
            encoder.write_image(
                rgba.as_raw(),
                width,
                height,
                image::ExtendedColorType::Rgba8,
            )
        }
    };
    if let Err(error) = encode_result {
        return Err(error).context("could not encode collage");
    }
    if let Err(error) = writer.flush() {
        return Err(error)
            .with_context(|| format!("could not flush collage at {}", destination.display()));
    }
    Ok(())
}

/// Match the GTK percentage-based rounding after the visible tile has been
/// made square at full radius. The 2x2 samples soften the cutout edge.
fn collage_trace_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        std::env::var("PIC_COLLAGE_TRACE").ok().is_some_and(|value| value != "0")
    })
}

fn round_corners(image: &mut RgbaImage, radius_ratio: f32) {
    let fraction = radius_ratio.clamp(0.0, 0.5);
    if fraction <= 0.0 {
        return;
    }
    let width = image.width() as f32;
    let height = image.height() as f32;
    let radius_x = width * fraction;
    let radius_y = height * fraction;
    for y in 0..image.height() {
        let middle_y = y as f32 + 0.5;
        let center_y = if middle_y < radius_y {
            radius_y
        } else if middle_y > height - radius_y {
            height - radius_y
        } else {
            continue;
        };
        for x in 0..image.width() {
            let middle_x = x as f32 + 0.5;
            let center_x = if middle_x < radius_x {
                radius_x
            } else if middle_x > width - radius_x {
                width - radius_x
            } else {
                continue;
            };
            let mut inside = 0u16;
            for offset_y in [0.25f32, 0.75] {
                for offset_x in [0.25f32, 0.75] {
                    let dx = (x as f32 + offset_x - center_x) / radius_x;
                    let dy = (y as f32 + offset_y - center_y) / radius_y;
                    if dx * dx + dy * dy <= 1.0 {
                        inside += 1;
                    }
                }
            }
            if inside < 4 {
                let pixel = image.get_pixel_mut(x, y);
                pixel.0[3] = ((pixel.0[3] as u16 * inside + 2) / 4) as u8;
            }
        }
    }
}

/// Draw an inset photo outline that follows straight, rounded and circular
/// edges. Use the same normalized radius as the GTK preview. The existing
/// alpha mask (if rounded) is preserved, including anti-aliased corner pixels.
fn draw_photo_border(
    image: &mut RgbaImage,
    radius_ratio: f32,
    color: [u8; 3],
    thickness: u32,
) {
    let width = image.width();
    let height = image.height();
    if width == 0 || height == 0 {
        return;
    }
    let inset = thickness.max(1).min(width.min(height) / 2);
    if inset == 0 {
        return;
    }
    let left = inset as f32;
    let top = inset as f32;
    let right = width as f32 - left;
    let bottom = height as f32 - top;
    let fraction = radius_ratio.clamp(0.0, 0.5);
    let radius_x = (width as f32 * fraction - left).max(0.0);
    let radius_y = (height as f32 * fraction - top).max(0.0);
    for y in 0..height {
        for x in 0..width {
            if image.get_pixel(x, y).0[3] == 0 {
                continue;
            }
            let mut interior_samples = 0u16;
            for offset_y in [0.25f32, 0.75] {
                for offset_x in [0.25f32, 0.75] {
                    let px = x as f32 + offset_x;
                    let py = y as f32 + offset_y;
                    if px < left || px > right || py < top || py > bottom {
                        continue;
                    }
                    let in_inner = if radius_x <= 0.0 || radius_y <= 0.0 {
                        true
                    } else {
                        let center_x = px.clamp(left + radius_x, right - radius_x);
                        let center_y = py.clamp(top + radius_y, bottom - radius_y);
                        let dx = (px - center_x) / radius_x;
                        let dy = (py - center_y) / radius_y;
                        dx * dx + dy * dy <= 1.0
                    };
                    if in_inner {
                        interior_samples += 1;
                    }
                }
            }
            if interior_samples == 4 {
                continue;
            }
            let outline_samples = 4 - interior_samples;
            let pixel = image.get_pixel_mut(x, y);
            for (channel, outline_color) in color.into_iter().enumerate() {
                pixel.0[channel] = ((pixel.0[channel] as u16 * interior_samples
                    + outline_color as u16 * outline_samples + 2) / 4) as u8;
            }
        }
    }
}

fn apply_library_rotation(source: DynamicImage, rotation: i32) -> DynamicImage {
    match rotation.rem_euclid(360) {
        90 => DynamicImage::ImageRgba8(image::imageops::rotate90(&source.to_rgba8())),
        180 => DynamicImage::ImageRgba8(image::imageops::rotate180(&source.to_rgba8())),
        270 => DynamicImage::ImageRgba8(image::imageops::rotate270(&source.to_rgba8())),
        _ => source,
    }
}

fn rotate(source: &RgbaImage, angle: f32, background: image::Rgba<u8>) -> RgbaImage {
    let (width, height) = source.dimensions();
    let sin = angle.sin().abs();
    let cos = angle.cos().abs();
    let output_width = (width as f32 * cos + height as f32 * sin).ceil() as u32;
    let output_height = (width as f32 * sin + height as f32 * cos).ceil() as u32;
    let mut output = RgbaImage::from_pixel(output_width.max(1), output_height.max(1), background);
    let center_x = width as f32 / 2.0;
    let center_y = height as f32 / 2.0;
    let output_center_x = output.width() as f32 / 2.0;
    let output_center_y = output.height() as f32 / 2.0;
    let (sin, cos) = angle.sin_cos();
    for y in 0..output.height() {
        for x in 0..output.width() {
            let dx = x as f32 - output_center_x;
            let dy = y as f32 - output_center_y;
            let source_x = dx * cos + dy * sin + center_x;
            let source_y = -dx * sin + dy * cos + center_y;
            if source_x >= 0.0
                && source_y >= 0.0
                && source_x < width as f32
                && source_y < height as f32
            {
                output.put_pixel(x, y, *source.get_pixel(source_x as u32, source_y as u32));
            }
        }
    }
    output
}

fn cover(source: DynamicImage, width: u32, height: u32) -> RgbaImage {
    let width = width.max(1);
    let height = height.max(1);
    let source_ratio = source.width() as f32 / source.height().max(1) as f32;
    let target_ratio = width as f32 / height as f32;
    let image = if source_ratio > target_ratio {
        let crop_width = (source.height() as f32 * target_ratio) as u32;
        let left = source.width().saturating_sub(crop_width) / 2;
        source.crop_imm(left, 0, crop_width.max(1), source.height())
    } else {
        let crop_height = (source.width() as f32 / target_ratio) as u32;
        let top = source.height().saturating_sub(crop_height) / 2;
        source.crop_imm(0, top, source.width(), crop_height.max(1))
    };
    image
        .resize_to_fill(width, height, image::imageops::FilterType::Lanczos3)
        .to_rgba8()
}

fn fit_inside_tile(
    source: DynamicImage,
    width: u32,
    height: u32,
    background: image::Rgba<u8>,
    photo_corner_radius: Option<f32>,
) -> RgbaImage {
    let width = width.max(1);
    let height = height.max(1);
    let image = source.resize(width, height, image::imageops::FilterType::Lanczos3);
    let mut image = image.to_rgba8();
    // Mask the visible fitted photo before it is placed in the full tile.
    // This radius is deliberately based on the photo, not the tile.
    if let Some(corner_radius) = photo_corner_radius {
        round_corners(&mut image, corner_radius);
    }
    let mut output = RgbaImage::from_pixel(width, height, background);
    let x = (width.saturating_sub(image.width()) / 2) as i64;
    let y = (height.saturating_sub(image.height()) / 2) as i64;
    image::imageops::overlay(&mut output, &image, x, y);
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn border_none_is_unchanged_and_square_outline_stays_inside() {
        let original = RgbaImage::from_pixel(80, 60, image::Rgba([10, 20, 30, 255]));
        let mut bordered = original.clone();
        draw_photo_border(&mut bordered, 0.0, [255, 255, 255], 3);
        assert_eq!(bordered.get_pixel(0, 25).0, [255, 255, 255, 255]);
        assert_eq!(bordered.get_pixel(40, 30).0, [10, 20, 30, 255]);
        assert_eq!(bordered.get_pixel(40, 1).0, [255, 255, 255, 255]);
        assert_eq!(original.get_pixel(0, 25).0, [10, 20, 30, 255]);
    }

    #[test]
    fn black_circle_border_tracks_alpha_and_preserves_centre() {
        let mut tile = RgbaImage::from_pixel(80, 80, image::Rgba([200, 100, 50, 255]));
        round_corners(&mut tile, 0.5);
        draw_photo_border(&mut tile, 0.5, [0, 0, 0], 4);
        assert_eq!(tile.get_pixel(0, 0).0[3], 0);
        assert_eq!(tile.get_pixel(40, 1).0, [0, 0, 0, 255]);
        assert_eq!(tile.get_pixel(40, 40).0, [200, 100, 50, 255]);
    }

    #[test]
    fn grey_outline_on_rounded_tile_has_no_square_corners() {
        let mut tile = RgbaImage::from_pixel(80, 80, image::Rgba([255, 10, 10, 255]));
        round_corners(&mut tile, 0.5);
        draw_photo_border(&mut tile, 0.5, [136, 136, 136], 3);
        assert_eq!(tile.get_pixel(0, 0).0[3], 0);
        assert_eq!(tile.get_pixel(40, 1).0, [136, 136, 136, 255]);
        assert_eq!(tile.get_pixel(40, 40).0, [255, 10, 10, 255]);
    }

    #[test]
    fn full_radius_masks_square_tiles_as_circles() {
        let mut tile = RgbaImage::from_pixel(80, 80, image::Rgba([200, 100, 50, 255]));
        round_corners(&mut tile, 0.5);
        assert_eq!(tile.get_pixel(0, 0).0[3], 0);
        assert_eq!(tile.get_pixel(79, 79).0[3], 0);
        assert_eq!(tile.get_pixel(40, 40).0[3], 255);
        assert_eq!(tile.get_pixel(40, 1).0[3], 255);
        assert_eq!(tile.get_pixel(8, 8).0[3], 0);
    }

    #[test]
    fn full_rounding_rectangular_slot_produces_a_circle_not_an_oval() {
        let (offset_x, offset_y, width, height) =
            super::super::model::rounded_tile_bounds(80, 40, 0.5);
        assert_eq!((offset_x, offset_y, width, height), (20, 0, 40, 40));
        let mut tile =
            RgbaImage::from_pixel(width, height, image::Rgba([200, 100, 50, 255]));
        round_corners(&mut tile, 0.5);
        assert_eq!(tile.get_pixel(0, 0).0[3], 0);
        assert_eq!(tile.get_pixel(39, 39).0[3], 0);
        assert_eq!(tile.get_pixel(20, 20).0[3], 255);
        assert_eq!(tile.get_pixel(1, 20).0[3], 255);
        assert_eq!(tile.get_pixel(10, 0).0[3], 0);
    }

    #[test]
    fn contain_rounds_the_complete_background_backed_tile() {
        let source = DynamicImage::ImageRgba8(RgbaImage::from_pixel(
            80,
            20,
            image::Rgba([220, 20, 20, 255]),
        ));
        let background = image::Rgba([30, 40, 50, 255]);
        let mut tile = fit_inside_tile(source, 40, 40, background, Some(0.2));

        // Before masking, the letterbox is part of the opaque tile.
        assert_eq!(*tile.get_pixel(20, 0), background);
        round_corners(&mut tile, 0.2);

        // The mask removes complete-tile corners, while the letterbox remains
        // visible along the straight edge of the rounded tile.
        assert_eq!(tile.get_pixel(0, 0).0[3], 0);
        assert_eq!(*tile.get_pixel(20, 0), background);
    }

    #[test]
    fn contain_rounds_the_fitted_photo_independently() {
        let source = DynamicImage::ImageRgba8(RgbaImage::from_pixel(
            80,
            20,
            image::Rgba([220, 20, 20, 255]),
        ));
        let background = image::Rgba([30, 40, 50, 255]);
        let tile = fit_inside_tile(source, 40, 40, background, Some(0.4));

        // The fitted 40x10 photo starts at y=15. Its own corner is masked,
        // while the adjacent letterbox remains opaque tile background.
        assert_eq!(*tile.get_pixel(0, 15), background);
        assert_eq!(*tile.get_pixel(20, 0), background);
    }
}
