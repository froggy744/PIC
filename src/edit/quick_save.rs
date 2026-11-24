//! Quick results of an edit that do not go through the export dialog: a
//! copy saved beside the original, and the edited image for the clipboard.
//!
//! Both render through the export path (`export_batch::render_job`, which
//! calls `render::render_for_export`), so they match a normal export. The
//! original file is never written.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use image::RgbaImage;

use super::export_batch::{
    default_export_quality, export_file_name, export_job_to, file_name_for_format, render_job,
    unique_export_path, ExportFormat, ExportJob,
};

/// The format a saved copy keeps: lossless sources (screenshots are PNG)
/// stay PNG, WebP stays WebP, everything else becomes JPEG.
pub fn copy_format(source: &Path) -> ExportFormat {
    let extension = source
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    match extension.as_str() {
        "png" | "gif" | "bmp" | "tif" | "tiff" => ExportFormat::Png,
        "webp" => ExportFormat::Webp,
        _ => ExportFormat::Jpeg,
    }
}

/// Where a saved copy of `source` goes: its folder, named `<stem>_edit`
/// with the copy format's extension, numbered when that name is taken.
pub fn copy_destination(source: &Path) -> PathBuf {
    let folder = source.parent().unwrap_or_else(|| Path::new("."));
    let original = source
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "photo".to_string());
    let name = file_name_for_format(&export_file_name(&original, true), copy_format(source));
    unique_export_path(folder, &name, &mut HashSet::new())
}

/// Save `job` as a new file beside its original and return the new path.
/// An existing file is never replaced.
pub fn save_copy(job: &ExportJob) -> Result<PathBuf> {
    let source = Path::new(&job.source_path);
    let destination = copy_destination(source);
    export_job_to(
        job,
        &destination,
        0,
        default_export_quality(),
        copy_format(source),
        &render_job,
    )
    .with_context(|| format!("could not save {}", destination.display()))?;
    Ok(destination)
}

/// The edited image at full resolution as PNG bytes, for the clipboard.
///
/// The clipboard offers only this PNG. Handing GTK a texture instead lets
/// it also offer uncompressed formats (TIFF, BMP), and a receiving app may
/// pick one of those: a 4K screenshot then pastes as 30+ MB, over chat
/// apps' attachment limits, although the same image saves as a few MB.
pub fn clipboard_png(job: &ExportJob) -> Result<Vec<u8>> {
    encode_png(&render_job(job)?)
}

/// Render `job` off the main thread and put it on `clipboard` as PNG
/// (see `clipboard_png`). `done` runs on the main thread with the outcome.
/// The clipboard is served by PIC, so the image stays pasteable while PIC
/// runs (or longer when the desktop keeps a clipboard history).
pub fn copy_to_clipboard(
    clipboard: gtk4::gdk::Clipboard,
    job: ExportJob,
    done: impl FnOnce(std::result::Result<(), String>) + 'static,
) {
    glib::MainContext::default().spawn_local(async move {
        let png = gio::spawn_blocking(move || {
            clipboard_png(&job).map_err(|error| format!("{error:#}"))
        })
        .await
        .unwrap_or_else(|_| Err("the renderer stopped unexpectedly".to_string()));
        let copied = png.and_then(|png| {
            let content = gtk4::gdk::ContentProvider::for_bytes(
                "image/png",
                &glib::Bytes::from_owned(png),
            );
            clipboard
                .set_content(Some(&content))
                .map_err(|error| error.to_string())
        });
        done(copied);
    });
}

fn encode_png(image: &RgbaImage) -> Result<Vec<u8>> {
    use image::ImageEncoder;
    let mut bytes = Vec::new();
    image::codecs::png::PngEncoder::new(&mut bytes)
        .write_image(
            image.as_raw(),
            image.width(),
            image.height(),
            image::ExtendedColorType::Rgba8,
        )
        .context("could not encode the copied photo as PNG")?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_folder(label: &str) -> PathBuf {
        let folder =
            std::env::temp_dir().join(format!("pic-quick-save-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).unwrap();
        folder
    }

    #[test]
    fn copies_keep_lossless_and_webp_formats_and_make_the_rest_jpeg() {
        assert_eq!(
            copy_format(Path::new("/a/Screenshot.PNG")),
            ExportFormat::Png
        );
        assert_eq!(copy_format(Path::new("/a/scan.tiff")), ExportFormat::Png);
        assert_eq!(copy_format(Path::new("/a/web.webp")), ExportFormat::Webp);
        assert_eq!(copy_format(Path::new("/a/photo.jpeg")), ExportFormat::Jpeg);
        assert_eq!(copy_format(Path::new("/a/photo.heic")), ExportFormat::Jpeg);
        assert_eq!(copy_format(Path::new("/a/IMG.CR2")), ExportFormat::Jpeg);
    }

    #[test]
    fn copies_go_beside_the_original_and_never_take_an_existing_name() {
        let folder = temp_folder("names");
        let source = folder.join("Screenshot.png");
        assert_eq!(
            copy_destination(&source),
            folder.join("Screenshot_edit.png")
        );
        std::fs::write(folder.join("Screenshot_edit.png"), b"taken").unwrap();
        assert_eq!(
            copy_destination(&source),
            folder.join("Screenshot_edit_2.png")
        );
        assert_eq!(
            copy_destination(&folder.join("IMG_1.heic")),
            folder.join("IMG_1_edit.jpg")
        );
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn the_clipboard_gets_a_compressed_png_of_the_image() {
        let image = image::RgbaImage::from_pixel(400, 300, image::Rgba([40, 90, 200, 255]));
        let bytes = encode_png(&image).unwrap();
        assert!(bytes.starts_with(b"\x89PNG"));
        assert!(
            bytes.len() < 400 * 300 * 4 / 10,
            "PNG not compressed: {}",
            bytes.len()
        );
        let decoded = image::load_from_memory(&bytes).unwrap().to_rgba8();
        assert_eq!(decoded, image);
    }

    #[test]
    fn saving_a_copy_renders_the_edit_and_leaves_the_original_untouched() {
        let folder = temp_folder("save");
        let source = folder.join("Screenshot.png");
        image::RgbaImage::from_pixel(40, 20, image::Rgba([10, 200, 30, 255]))
            .save(&source)
            .unwrap();
        let original = std::fs::read(&source).unwrap();
        let mut recipe = crate::edit::EditRecipe::default();
        recipe.crop = crate::edit::model::CropRect {
            right: 0.5,
            ..Default::default()
        };
        let job = ExportJob::from_record(
            source.to_string_lossy().into_owned(),
            0,
            recipe.encode(),
            40,
            20,
            "Screenshot.png",
        );
        let saved = save_copy(&job).unwrap();
        assert_eq!(saved, folder.join("Screenshot_edit.png"));
        assert_eq!(std::fs::read(&source).unwrap(), original);
        let copy = image::open(&saved).unwrap();
        assert_eq!((copy.width(), copy.height()), (20, 20));
        let _ = std::fs::remove_dir_all(&folder);
    }
}
