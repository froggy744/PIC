use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::Result;
use image::RgbaImage;

use super::model::EditRecipe;

pub const EDIT_SUFFIX: &str = "_edit";
const DEFAULT_EXPORT_QUALITY: u8 = 92;

/// Output format chosen in the export dialog (shared by gallery and editor).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportFormat {
    Jpeg,
    Png,
    Webp,
}

impl ExportFormat {
    pub const ALL: [ExportFormat; 3] = [ExportFormat::Jpeg, ExportFormat::Png, ExportFormat::Webp];

    pub fn extension(self) -> &'static str {
        match self {
            ExportFormat::Jpeg => "jpg",
            ExportFormat::Png => "png",
            ExportFormat::Webp => "webp",
        }
    }

    pub fn dialog_label(self) -> &'static str {
        match self {
            ExportFormat::Jpeg => "JPEG",
            ExportFormat::Png => "PNG",
            ExportFormat::Webp => "WebP",
        }
    }

    pub fn from_dialog_index(index: u32) -> Self {
        match index {
            1 => ExportFormat::Png,
            2 => ExportFormat::Webp,
            _ => ExportFormat::Jpeg,
        }
    }

    pub fn dialog_labels() -> Vec<&'static str> {
        Self::ALL.iter().map(|format| format.dialog_label()).collect()
    }
}

pub fn recipe_is_edited(recipe: &str) -> bool {
    !EditRecipe::decode(recipe).is_default()
}

/// Force `file_name` to end with `format`'s extension (stem and any
/// `_edit` suffix preserved).
pub fn file_name_for_format(file_name: &str, format: ExportFormat) -> String {
    let path = Path::new(file_name);
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("export");
    format!("{stem}.{}", format.extension())
}

/// Build the on-disk file name for an export.
///
/// Edited photos get `_edit` before the extension (extension case preserved).
/// An existing `_edit` stem is never doubled. Unedited photos keep their
/// normal name.
pub fn export_file_name(original_name: &str, edited: bool) -> String {
    let path = Path::new(original_name);
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_string);
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("export");
    let base = if edited && !stem.to_ascii_lowercase().ends_with(EDIT_SUFFIX) {
        format!("{stem}{EDIT_SUFFIX}")
    } else {
        stem.to_string()
    };
    match extension {
        Some(extension) => format!("{base}.{extension}"),
        None => base,
    }
}

/// Pick a collision-free file name inside one batch (and against the disk).
///
/// `DSC_1001_edit.jpg` → `DSC_1001_edit_2.jpg` → `DSC_1001_edit_3.jpg`.
pub fn unique_file_name<F>(
    file_name: &str,
    reserved: &mut HashSet<String>,
    mut exists: F,
) -> String
where
    F: FnMut(&str) -> bool,
{
    if !reserved.contains(file_name) && !exists(file_name) {
        reserved.insert(file_name.to_string());
        return file_name.to_string();
    }
    let path = Path::new(file_name);
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_string);
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("export")
        .to_string();
    // Large imports can legitimately exceed 10,000 photos. Never fall back
    // to an unchecked, extensionless name after suffix 9999: that could
    // collide with an existing export or produce a file of the wrong type.
    let mut counter = 2u64;
    loop {
        let candidate = match &extension {
            Some(extension) => format!("{stem}_{counter}.{extension}"),
            None => format!("{stem}_{counter}"),
        };
        if !reserved.contains(&candidate) && !exists(&candidate) {
            reserved.insert(candidate.clone());
            return candidate;
        }
        counter = counter.checked_add(1)
            .expect("all export filename suffixes exhausted");
    }
}

pub fn unique_export_path(folder: &Path, file_name: &str, reserved: &mut HashSet<String>) -> PathBuf {
    let candidate = unique_file_name(file_name, reserved, |name| folder.join(name).exists());
    folder.join(candidate)
}

/// Shrink so the long edge is at most `max_edge`, preserving aspect ratio.
/// Never upscales. `max_edge == 0` leaves the image untouched.
pub fn fit_max_edge(image: RgbaImage, max_edge: u32) -> RgbaImage {
    if max_edge == 0 {
        return image;
    }
    let (width, height) = (image.width(), image.height());
    let long_edge = width.max(height);
    if long_edge <= max_edge {
        return image;
    }
    let scale = max_edge as f32 / long_edge as f32;
    let new_width = ((width as f32 * scale).round() as u32).max(1);
    let new_height = ((height as f32 * scale).round() as u32).max(1);
    image::imageops::resize(&image, new_width, new_height, image::imageops::FilterType::Lanczos3)
}

#[derive(Clone, Debug)]
pub struct ExportJob {
    pub source_path: String,
    pub rotation: i32,
    pub edit_recipe: String,
    pub source_width: i64,
    pub source_height: i64,
    pub file_name: String,
}

impl ExportJob {
    pub fn from_record(
        path: String,
        rotation: i32,
        edit_recipe: String,
        source_width: i64,
        source_height: i64,
        original_name: &str,
    ) -> Self {
        let edited = recipe_is_edited(&edit_recipe);
        Self {
            source_path: path,
            rotation,
            edit_recipe,
            source_width,
            source_height,
            file_name: export_file_name(original_name, edited),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BatchExportOutcome {
    pub exported: usize,
    pub skipped: usize,
    pub failed: usize,
    pub errors: Vec<String>,
}

pub fn render_job(job: &ExportJob) -> Result<RgbaImage> {
    super::render::render_for_export(
        &job.source_path,
        job.rotation,
        &job.edit_recipe,
        job.source_width,
        job.source_height,
    )
}

/// Stage an export in the destination directory. A completed output
/// appears atomically; failed encodes never leave a broken .jpg/.png/.webp.
fn staging_export_path(destination: &Path) -> Result<PathBuf> {
    use std::fs::OpenOptions;
    let parent = destination.parent().unwrap_or_else(|| Path::new("."));
    let name = destination.file_name()
        .map(|value| value.to_string_lossy())
        .unwrap_or_else(|| std::borrow::Cow::Borrowed("photo"));
    for counter in 0u32..1024 {
        let candidate = parent.join(format!(
            ".{name}.pic-export-{}-{counter}.tmp", std::process::id()
        ));
        match OpenOptions::new().write(true).create_new(true).open(&candidate) {
            Ok(file) => {
                drop(file);
                return Ok(candidate);
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.into()),
        }
    }
    anyhow::bail!("could not create temporary export file for {}", destination.display())
}

/// Publish a fully encoded sibling temporary file without ever replacing an
/// existing destination. Linux's renameat2(NOREPLACE) makes the operation
/// atomic even if another PIC process races us to the same filename.
fn publish_export(staging: &Path, destination: &Path) -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;
        let source = CString::new(staging.as_os_str().as_bytes())?;
        let target = CString::new(destination.as_os_str().as_bytes())?;
        let result = unsafe {
            libc::renameat2(
                libc::AT_FDCWD, source.as_ptr(),
                libc::AT_FDCWD, target.as_ptr(),
                libc::RENAME_NOREPLACE,
            )
        };
        if result == 0 {
            return Ok(());
        }
        let error = std::io::Error::last_os_error();
        // A mounted filesystem might not support this rename extension.
        // Never fall back to a potentially overwriting plain rename.
        if matches!(error.raw_os_error(), Some(libc::EINVAL) | Some(libc::ENOSYS) | Some(libc::EOPNOTSUPP)) {
            std::fs::hard_link(staging, destination)?;
            std::fs::remove_file(staging)?;
            return Ok(());
        }
        return Err(error.into());
    }
    #[cfg(not(target_os = "linux"))]
    {
        // Preserve the previous platform behaviour for non-Linux builds.
        // Cross-process no-replace publication needs a platform-specific API.
        if destination.exists() {
            anyhow::bail!("refusing to overwrite existing export: {}", destination.display());
        }
        std::fs::rename(staging, destination)?;
        Ok(())
    }
}

/// Render and save one job to an exact destination path (caller resolves it).
/// A full output is written beside the final path first and renamed only
/// after encoding succeeds, preserving existing files on a failed export.
pub fn export_job_to<F>(
    job: &ExportJob,
    destination: &Path,
    max_edge: u32,
    quality: u8,
    format: ExportFormat,
    render: &F,
) -> Result<()>
where
    F: Fn(&ExportJob) -> Result<RgbaImage>,
{
    use anyhow::Context;
    if destination.exists() {
        anyhow::bail!("refusing to overwrite existing export: {}", destination.display());
    }
    let image = fit_max_edge(render(job)?, max_edge);
    let staging = staging_export_path(destination)?;
    let result: Result<()> = (|| {
        let exif = super::export_metadata::for_export(&job.source_path, image.width(), image.height());
        super::render::save_export_with_metadata(
            &image, &staging, format, quality, Some(&exif)
        )?;
        // The result is only made visible at its final filename once complete.
        publish_export(&staging, destination).with_context(|| format!(
            "could not finish export {}", destination.display()
        ))?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(staging);
    }
    result
}

/// Export every job into `folder`, skipping missing sources, continuing after
/// failures, and uniquifying names so nothing is silently overwritten.
///
/// `on_progress(done, total, file_name, failed)` runs after each item so the
/// UI can show a live `n / total` count. When `cancel` is set, the batch stops
/// before the next job; already-written files stay on disk.
pub fn run_batch_export<F, P>(
    jobs: &[ExportJob],
    folder: &Path,
    max_edge: u32,
    quality: u8,
    format: ExportFormat,
    render: F,
    mut on_progress: P,
    cancel: Option<&AtomicBool>,
) -> BatchExportOutcome
where
    F: Fn(&ExportJob) -> Result<RgbaImage>,
    P: FnMut(usize, usize, &str, usize),
{
    let total = jobs.len();
    let mut outcome = BatchExportOutcome::default();
    let mut reserved = HashSet::new();
    for (index, job) in jobs.iter().enumerate() {
        if cancel.is_some_and(|flag| flag.load(Ordering::Relaxed)) {
            let done = index;
            outcome
                .errors
                .push(format!("cancelled after {done} / {total}"));
            on_progress(done, total, "", outcome.failed);
            break;
        }
        let done = index + 1;
        if !crate::source::file_available(&job.source_path) {
            outcome.skipped += 1;
            outcome
                .errors
                .push(format!("skipped (missing): {}", job.source_path));
            on_progress(done, total, &job.file_name, outcome.failed);
            continue;
        }
        let destination = unique_export_path(folder, &job.file_name, &mut reserved);
        match export_job_to(job, &destination, max_edge, quality, format, &render) {
            Ok(()) => outcome.exported += 1,
            Err(error) => {
                outcome.failed += 1;
                outcome
                    .errors
                    .push(format!("{}: {error:#}", job.source_path));
            }
        }
        on_progress(done, total, &job.file_name, outcome.failed);
    }
    outcome
}

pub fn default_export_quality() -> u8 {
    DEFAULT_EXPORT_QUALITY
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edit::model::CropRect;

    #[test]
    fn edited_name_gets_edit_suffix() {
        assert_eq!(export_file_name("DSC_1001.jpg", true), "DSC_1001_edit.jpg");
        assert_eq!(export_file_name("IMG_4221.JPG", true), "IMG_4221_edit.JPG");
    }

    #[test]
    fn unedited_name_is_preserved() {
        assert_eq!(export_file_name("DSC_1001.jpg", false), "DSC_1001.jpg");
        assert_eq!(export_file_name("IMG_4221.JPG", false), "IMG_4221.JPG");
    }

    #[test]
    fn edit_suffix_is_never_doubled() {
        assert_eq!(
            export_file_name("DSC_1001_edit.jpg", true),
            "DSC_1001_edit.jpg"
        );
        assert_eq!(
            export_file_name("IMG_4221_edit.JPG", true),
            "IMG_4221_edit.JPG"
        );
        assert_eq!(
            export_file_name("photo_Edit.jpeg", true),
            "photo_Edit.jpeg"
        );
    }

    #[test]
    fn missing_extension_still_gets_suffix() {
        assert_eq!(export_file_name("noextension", true), "noextension_edit");
        assert_eq!(export_file_name("noextension", false), "noextension");
    }

    #[test]
    fn file_name_for_format_swaps_extension_and_keeps_edit_suffix() {
        assert_eq!(
            file_name_for_format("DSC_1001_edit.jpg", ExportFormat::Png),
            "DSC_1001_edit.png"
        );
        assert_eq!(
            file_name_for_format("DSC_1001_edit.jpg", ExportFormat::Webp),
            "DSC_1001_edit.webp"
        );
        assert_eq!(
            file_name_for_format("photo.PNG", ExportFormat::Jpeg),
            "photo.jpg"
        );
        assert_eq!(file_name_for_format("noextension", ExportFormat::Jpeg), "noextension.jpg");
    }

    #[test]
    fn recipe_detection_uses_edit_recipe_defaults() {
        assert!(!recipe_is_edited(""));
        assert!(!recipe_is_edited(&EditRecipe::default().encode()));
        let mut recipe = EditRecipe::default();
        recipe.exposure = 0.5;
        assert!(recipe_is_edited(&recipe.encode()));
        let mut recipe = EditRecipe::default();
        recipe.crop = CropRect {
            left: 0.1,
            top: 0.1,
            right: 0.9,
            bottom: 0.9,
        };
        assert!(recipe_is_edited(&recipe.encode()));
        let mut recipe = EditRecipe::default();
        recipe
            .text_layers
            .push(crate::edit::model::TextLayerSpec::new_default());
        assert!(recipe_is_edited(&recipe.encode()));
        let mut recipe = EditRecipe::default();
        recipe
            .overlays
            .push(crate::edit::model::OverlaySpec::new_centered("asset"));
        assert!(recipe_is_edited(&recipe.encode()));
    }

    #[test]
    fn job_uses_recipe_detection_for_file_name() {
        let job = ExportJob::from_record(
            "/photos/DSC_1001.jpg".into(),
            0,
            String::new(),
            100,
            80,
            "DSC_1001.jpg",
        );
        assert_eq!(job.file_name, "DSC_1001.jpg");
        let mut recipe = EditRecipe::default();
        recipe.contrast = 0.3;
        let job = ExportJob::from_record(
            "/photos/DSC_1001.jpg".into(),
            0,
            recipe.encode(),
            100,
            80,
            "DSC_1001.jpg",
        );
        assert_eq!(job.file_name, "DSC_1001_edit.jpg");
    }

    #[test]
    fn collisions_are_numbered() {
        let mut reserved = HashSet::new();
        let no = |_name: &str| false;
        assert_eq!(
            unique_file_name("DSC_1001_edit.jpg", &mut reserved, no),
            "DSC_1001_edit.jpg"
        );
        assert_eq!(
            unique_file_name("DSC_1001_edit.jpg", &mut reserved, no),
            "DSC_1001_edit_2.jpg"
        );
        assert_eq!(
            unique_file_name("DSC_1001_edit.jpg", &mut reserved, no),
            "DSC_1001_edit_3.jpg"
        );
        let mut reserved = HashSet::new();
        assert_eq!(
            unique_file_name("a.jpg", &mut reserved, |name| name == "a.jpg"),
            "a_2.jpg"
        );
    }

    #[test]
    fn many_duplicate_exports_never_drop_extension_or_reuse_a_filename() {
        let mut reserved = HashSet::new();
        reserved.insert("DSC_5791.png".to_string());
        // Exercise the old fallback path, which began at suffix 10,000.
        for counter in 2..=10_005 {
            reserved.insert(format!("DSC_5791_{counter}.png"));
        }
        let result = unique_file_name("DSC_5791.png", &mut reserved, |name| {
            name == "DSC_5791_10006.png"
        });
        assert_eq!(result, "DSC_5791_10007.png");
        assert!(reserved.contains(&result));
        let next = unique_file_name("DSC_5791.png", &mut reserved, |_| false);
        assert_eq!(next, "DSC_5791_10006.png");
        assert!(reserved.contains(&next));
        assert_ne!(next, result);
        assert!(!reserved.contains("DSC_5791-export"));
    }

    #[test]
    fn fit_max_edge_preserves_portrait_and_landscape() {
        let portrait = RgbaImage::new(3000, 4000);
        let fitted = fit_max_edge(portrait, 1000);
        assert_eq!((fitted.width(), fitted.height()), (750, 1000));

        let landscape = RgbaImage::new(4000, 3000);
        let fitted = fit_max_edge(landscape, 1000);
        assert_eq!((fitted.width(), fitted.height()), (1000, 750));

        let square = RgbaImage::new(2000, 2000);
        let fitted = fit_max_edge(square, 500);
        assert_eq!((fitted.width(), fitted.height()), (500, 500));
    }

    #[test]
    fn fit_max_edge_never_upscales_or_touches_disabled() {
        let small = RgbaImage::new(100, 50);
        let fitted = fit_max_edge(small, 1000);
        assert_eq!((fitted.width(), fitted.height()), (100, 50));

        let image = RgbaImage::new(640, 480);
        let fitted = fit_max_edge(image, 0);
        assert_eq!((fitted.width(), fitted.height()), (640, 480));
    }

    #[test]
    fn export_never_overwrites_an_existing_file() {
        let folder = std::env::temp_dir().join(format!(
            "pic-export-nooverwrite-{}", std::process::id(),
        ));
        std::fs::create_dir_all(&folder).unwrap();
        let destination = folder.join("export.png");
        std::fs::write(&destination, b"keep this original").unwrap();
        let job = ExportJob::from_record(
            "unused".into(), 0, "".into(), 1, 1, "export.png",
        );
        let result = export_job_to(&job, &destination, 0, 92,
            ExportFormat::Png, &|_| Ok(RgbaImage::new(1, 1)));
        assert!(result.is_err());
        assert_eq!(std::fs::read(&destination).unwrap(), b"keep this original");
        std::fs::remove_dir_all(folder).ok();
    }

    #[test]
    fn staged_export_finishes_with_no_temporary_files() {
        let folder = std::env::temp_dir().join(format!(
            "pic-export-atomic-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos(),
        ));
        std::fs::create_dir_all(&folder).unwrap();
        let destination = folder.join("export.png");
        let job = ExportJob::from_record(
            "unused".into(), 0, "".into(), 6, 4, "export.png",
        );
        let original = RgbaImage::from_fn(6, 4, |x, y| image::Rgba([
            (x * 37) as u8, (y * 43) as u8, 120, (x * 31 + y * 17) as u8,
        ]));
        export_job_to(&job, &destination, 0, 92, ExportFormat::Png,
            &|_| Ok(original.clone())).unwrap();
        assert_eq!(image::open(&destination).unwrap().to_rgba8(), original);
        let paths = std::fs::read_dir(&folder).unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert_eq!(paths, vec!["export.png"]);
        std::fs::remove_dir_all(folder).ok();
    }

    #[test]
    fn exported_jpeg_has_correct_exif_dimensions_orientation_and_icc() {
        use image::ImageDecoder;
        let dir = std::env::temp_dir().join(format!(
            "pic-export-exif-{}-{}", std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
                .unwrap().as_nanos(),
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("source.png");
        image::DynamicImage::ImageRgba8(
            RgbaImage::from_pixel(50, 30, image::Rgba([120, 80, 50, 255]))
        ).save(&source).unwrap();
        let job = ExportJob::from_record(
            source.display().to_string(), 90, String::new(), 50, 30, "source.png"
        );
        let destination = dir.join("out.jpg");
        export_job_to(&job, &destination, 0, 95, ExportFormat::Jpeg, &render_job).unwrap();
        let output = image::open(&destination).unwrap();
        assert_eq!((output.width(), output.height()), (30, 50));
        let metadata = exif::Reader::new().read_from_container(
            &mut std::io::BufReader::new(std::fs::File::open(&destination).unwrap())
        ).unwrap();
        assert_eq!(metadata.get_field(exif::Tag::Orientation, exif::In::PRIMARY)
            .unwrap().value.get_uint(0), Some(1));
        assert_eq!(metadata.get_field(exif::Tag::PixelXDimension, exif::In::PRIMARY)
            .unwrap().value.get_uint(0), Some(30));
        assert_eq!(metadata.get_field(exif::Tag::PixelYDimension, exif::In::PRIMARY)
            .unwrap().value.get_uint(0), Some(50));
        assert!(metadata.get_field(exif::Tag::GPSLatitude, exif::In::PRIMARY).is_none());
        let mut decoder = image::codecs::jpeg::JpegDecoder::new(
            std::io::BufReader::new(std::fs::File::open(&destination).unwrap())
        ).unwrap();
        assert!(decoder.icc_profile().unwrap().is_some(),
            "exported JPEG must identify its output sRGB color space");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn nikon_style_tiff_metadata_survives_export_and_original_is_unchanged() {
        // A minimal TIFF/NEF-style source checks the real metadata reader,
        // not just the in-memory EXIF builder. The pixels are supplied by the
        // render closure because this tiny TIFF carries only camera fields.
        let dir = std::env::temp_dir().join(format!(
            "pic-export-nef-metadata-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
                .unwrap().as_nanos(),
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("sample.NEF");
        let entries: &[(u16, &[u8])] = &[
            (0x010F, b"NIKON CORPORATION\0"),
            (0x0110, b"NIKON Z 5\0"),
            (0x0132, b"2026:10:18 12:34:56\0"),
        ];
        let mut tiff = b"II\x2a\x00\x08\x00\x00\x00".to_vec();
        tiff.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        let mut offset = (8 + 2 + entries.len() * 12 + 4) as u32;
        let mut values = Vec::new();
        for &(tag, bytes) in entries {
            tiff.extend_from_slice(&tag.to_le_bytes());
            tiff.extend_from_slice(&2u16.to_le_bytes()); // ASCII
            tiff.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
            tiff.extend_from_slice(&offset.to_le_bytes());
            values.extend_from_slice(bytes);
            offset += bytes.len() as u32;
        }
        tiff.extend_from_slice(&0u32.to_le_bytes()); // next IFD
        tiff.extend_from_slice(&values);
        std::fs::write(&source, &tiff).unwrap();

        let job = ExportJob::from_record(
            source.display().to_string(), 90, String::new(), 20, 12, "sample.NEF",
        );
        let pixels = RgbaImage::from_pixel(12, 20, image::Rgba([75, 120, 180, 255]));
        for (format, suffix) in [
            (ExportFormat::Jpeg, "jpg"),
            (ExportFormat::Png, "png"),
            (ExportFormat::Webp, "webp"),
        ] {
            let destination = dir.join(format!("output.{suffix}"));
            export_job_to(&job, &destination, 0, 94, format, &|_| Ok(pixels.clone()))
                .unwrap();
            let saved = image::open(&destination).unwrap();
            assert_eq!(saved.dimensions(), (12, 20), "{suffix}");
            let bytes = std::fs::read(&destination).unwrap();
            assert!(bytes.windows(b"NIKON Z 5".len()).any(|w| w == b"NIKON Z 5"),
                "{suffix} lost the camera model metadata");
            assert!(bytes.windows(b"2026:10:18 12:34:56".len())
                .any(|w| w == b"2026:10:18 12:34:56"),
                "{suffix} lost the photo date metadata");
            assert_eq!(std::fs::read(&source).unwrap(), tiff,
                "{suffix} export modified the original");
        }
        std::fs::remove_dir_all(dir).ok();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn concurrent_exports_never_replace_the_first_completed_destination() {
        let dir = std::env::temp_dir().join(format!(
            "pic-export-race-{}-{}", std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
                .unwrap().as_nanos(),
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let dest = dir.join("same.png");
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let handles = (0..2).map(|index| {
            let destination = dest.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                let job = ExportJob::from_record(
                    "unused".into(), 0, String::new(), 12, 10, "same.png"
                );
                export_job_to(&job, &destination, 0, 92, ExportFormat::Png, &|_| {
                    barrier.wait(); // both pass the initial exists check
                    Ok(RgbaImage::from_pixel(12, 10, image::Rgba([
                        50 + (index * 80) as u8, 100, 150, 255
                    ])))
                })
            })
        }).collect::<Vec<_>>();
        let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1,
            "one concurrent export must fail instead of replacing the first");
        assert!(dest.exists());
        let survivors = std::fs::read_dir(&dir).unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert_eq!(survivors, vec!["same.png"]);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn batch_continues_after_failures_and_counts_them() {
        let dir = std::env::temp_dir().join(format!(
            "pic-export-batch-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let jobs: Vec<ExportJob> = (0..5)
            .map(|index| ExportJob {
                source_path: dir.join(format!("src-{index}.jpg")).display().to_string(),
                rotation: 0,
                edit_recipe: String::new(),
                source_width: 4,
                source_height: 4,
                file_name: export_file_name(&format!("photo-{index}.jpg"), index % 2 == 0),
            })
            .collect();
        for index in [0, 1, 3, 4] {
            std::fs::write(dir.join(format!("src-{index}.jpg")), b"x").unwrap();
        }
        let mut progress = Vec::new();
        let outcome = run_batch_export(
            &jobs,
            &dir,
            0,
            92,
            ExportFormat::Jpeg,
            |job| {
                if job.source_path.ends_with("src-3.jpg") {
                    anyhow::bail!("simulated render failure");
                }
                Ok(RgbaImage::new(4, 4))
            },
            |done, total, name, failed| progress.push((done, total, name.to_string(), failed)),
            None,
        );
        assert_eq!(outcome.exported, 3);
        assert_eq!(outcome.skipped, 1);
        assert_eq!(outcome.failed, 1);
        assert_eq!(progress.len(), 5);
        assert_eq!(progress.last().unwrap().0, 5);
        assert!(dir.join("photo-0_edit.jpg").is_file());
        assert!(dir.join("photo-1.jpg").is_file());
        assert!(dir.join("photo-4_edit.jpg").is_file());
        assert!(!dir.join("photo-2_edit.jpg").exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn batch_exports_uri_and_local_sources_together() {
        let dir = std::env::temp_dir().join(format!(
            "pic-export-uri-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("source.png");
        image::DynamicImage::ImageRgba8(RgbaImage::from_pixel(
            16,
            16,
            image::Rgba([80, 100, 120, 255]),
        ))
        .save(&source)
        .unwrap();
        let original = std::fs::read(&source).unwrap();
        let uri = format!("file://{}", source.display());
        let mut recipe = EditRecipe::default();
        recipe.exposure = 0.5;
        let jobs = [
            ExportJob::from_record(uri.clone(), 0, recipe.encode(), 16, 16, "uri.png"),
            ExportJob::from_record(
                source.display().to_string(),
                0,
                String::new(),
                16,
                16,
                "local.png",
            ),
        ];
        let outcome = run_batch_export(
            &jobs,
            &dir,
            0,
            92,
            ExportFormat::Png,
            render_job,
            |_, _, _, _| {},
            None,
        );
        assert_eq!(outcome.exported, 2, "{outcome:?}");
        let edited = image::open(dir.join("uri_edit.png")).unwrap().to_rgba8();
        let unedited = image::open(dir.join("local.png")).unwrap().to_rgba8();
        assert!(edited.get_pixel(0, 0)[0] > unedited.get_pixel(0, 0)[0]);
        assert_eq!(std::fs::read(source).unwrap(), original);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn full_quality_png_export_matches_edited_viewer_and_keeps_original() {
        use crate::edit::filters::FilterPreset;
        use crate::edit::model::{CropRect, OverlaySpec, TextLayerSpec};
        let dir = std::env::temp_dir().join(format!(
            "pic-export-parity-{}-{}", std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
                .unwrap().as_nanos(),
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let source_path = dir.join("DSC_5791.png");
        let photo = RgbaImage::from_fn(128, 128, |x, y| {
            image::Rgba([
                (x * 2) as u8, (y * 2) as u8,
                ((x + y) % 230) as u8, 255,
            ])
        });
        image::DynamicImage::ImageRgba8(photo.clone()).save(&source_path).unwrap();
        let original_file = std::fs::read(&source_path).unwrap();

        // Use PIC's same managed overlay format, not an external path whose
        // disappearance would make Export silently omit an image layer.
        let logo = RgbaImage::from_pixel(16, 16, image::Rgba([225, 35, 60, 128]));
        let logo_file = dir.join("logo.png");
        image::DynamicImage::ImageRgba8(logo).save(&logo_file).unwrap();
        let prepared = super::super::overlays::prepare_overlay_asset(&logo_file).unwrap();
        let overlay_dir = super::super::overlays::overlays_dir();
        std::fs::create_dir_all(&overlay_dir).unwrap();
        let managed_logo = overlay_dir.join(format!("{}.png", prepared.hash));
        let created_logo = !managed_logo.is_file();
        if created_logo {
            std::fs::write(&managed_logo, &prepared.bytes).unwrap();
        }

        let mut recipe = EditRecipe::default();
        recipe.filter = FilterPreset::Valencia;
        recipe.filter_amount = 0.72;
        recipe.exposure = 0.18;
        recipe.saturation = 0.17;
        recipe.sharpen = 0.25;
        recipe.straighten = 3.0;
        recipe.crop = CropRect {
            left: 0.13, top: 0.09, right: 0.9, bottom: 0.86,
        };
        let mut overlay = OverlaySpec::new_centered(prepared.hash);
        overlay.opacity = 0.6;
        overlay.width = 0.25;
        recipe.overlays.push(overlay);
        recipe.text_layers.push(TextLayerSpec::new_default());

        let job = ExportJob::from_record(
            source_path.display().to_string(), 90, recipe.encode(),
            128, 128, "DSC_5791.png",
        );
        let destination = dir.join("DSC_5791_edit.png");
        export_job_to(&job, &destination, 0, 92, ExportFormat::Png, &render_job)
            .unwrap();

        let expected = super::super::render::apply_recipe(
            super::super::render::apply_library_rotation(photo, 90), &recipe,
        );
        let actual = image::open(&destination).unwrap().to_rgba8();
        assert_eq!(actual.dimensions(), expected.dimensions());
        assert_eq!(actual, expected, "lossless Export differs from Edit recipe");
        assert_eq!(std::fs::read(&source_path).unwrap(), original_file,
            "Export changed the original source file");

        if created_logo {
            std::fs::remove_file(managed_logo).ok();
        }
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn batch_stops_when_cancel_flag_is_set() {
        let dir = std::env::temp_dir().join(format!(
            "pic-export-cancel-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("src.jpg"), b"x").unwrap();
        let jobs: Vec<ExportJob> = (0..4)
            .map(|index| ExportJob {
                source_path: dir.join("src.jpg").display().to_string(),
                rotation: 0,
                edit_recipe: String::new(),
                source_width: 4,
                source_height: 4,
                file_name: format!("photo-{index}.jpg"),
            })
            .collect();
        let cancel = AtomicBool::new(true);
        let mut progress = Vec::new();
        let outcome = run_batch_export(
            &jobs,
            &dir,
            0,
            92,
            ExportFormat::Jpeg,
            |_| Ok(RgbaImage::new(4, 4)),
            |done, total, _, failed| progress.push((done, total, failed)),
            Some(&cancel),
        );
        assert_eq!(outcome.exported, 0);
        assert_eq!(progress.len(), 1);
        assert_eq!(progress[0], (0, 4, 0));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn batch_output_names_never_collide_on_disk() {
        let dir = std::env::temp_dir().join(format!(
            "pic-export-collide-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("same_edit.jpg"), b"pre-existing").unwrap();
        let jobs: Vec<ExportJob> = (0..2)
            .map(|_| ExportJob {
                source_path: dir.join("src.jpg").display().to_string(),
                rotation: 0,
                edit_recipe: String::new(),
                source_width: 4,
                source_height: 4,
                file_name: "same_edit.jpg".into(),
            })
            .collect();
        std::fs::write(dir.join("src.jpg"), b"x").unwrap();
        let outcome = run_batch_export(
            &jobs,
            &dir,
            0,
            92,
            ExportFormat::Jpeg,
            |_| Ok(RgbaImage::new(4, 4)),
            |_, _, _, _| {},
            None,
        );
        assert_eq!(outcome.exported, 2);
        assert_eq!(
            std::fs::read(dir.join("same_edit.jpg")).unwrap(),
            b"pre-existing"
        );
        assert!(dir.join("same_edit_2.jpg").is_file());
        assert!(dir.join("same_edit_3.jpg").is_file());
        std::fs::remove_dir_all(&dir).ok();
    }
}
