use gdk_pixbuf::prelude::*;

pub fn dimensions(reference: &str, bytes: &[u8]) -> Result<(u32, u32)> {
    if is_svg(reference) {
        let (_, width, height) = load_svg_pixbuf(bytes, 1, 1)?;
        Ok((width, height))
    } else if is_raw(reference) {
        let local = crate::source::materialize(reference)?;
        rawler_decode(reference, "dimensions", || {
            let rawfile = rawler::rawsource::RawSource::new(&local)?;
            let decoder = rawler::get_decoder(&rawfile)?;
            if is_dng(reference) {
                // rawler 0.8's DNG raw_image(dummy=true) still unpacks and
                // linearizes sensor data. Read geometry from the RAW IFD
                // instead, preserving the crop/active-area preference.
                let raw = decoder
                    .ifd(rawler::decoders::WellKnownIFD::Raw)?
                    .context("DNG decoder did not provide a RAW IFD")?;
                return dng_ifd_dimensions(&raw);
            }
            let raw = decoder.raw_image(
                &rawfile,
                &rawler::decoders::RawDecodeParams::default(),
                true,
            )?;
            let (width, height) = raw
                .crop_area
                .or(raw.active_area)
                .map(|area| (area.d.w, area.d.h))
                .unwrap_or((raw.width, raw.height));
            Ok((u32::try_from(width)?, u32::try_from(height)?))
        })
    } else if is_jpeg(reference) {
        // Do not use image-rs' zune-jpeg dimension reader here. Some corrupt
        // JPEG APP segments can make that parser attempt an unchecked huge
        // allocation and abort the process instead of returning an error.
        let mut decompressor = Decompressor::new()
            .map_err(|error| anyhow::anyhow!("TurboJPEG initialization failed: {error}"))?;
        let header = decompressor
            .read_header(bytes)
            .map_err(|error| anyhow::anyhow!("invalid JPEG header: {error}"))?;
        Ok((header.width as u32, header.height as u32))
    } else if is_heif(reference) {
        let decoded = decode_heif(bytes)?;
        Ok((decoded.source_width, decoded.source_height))
    } else {
        Ok(ImageReader::new(Cursor::new(bytes))
            .with_guessed_format()?
            .into_dimensions()?)
    }
}

fn decode_with_image(bytes: &[u8]) -> Result<DecodedThumbnailSource> {
    let source = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()?
        .decode()?;
    let source_width = source.width();
    let source_height = source.height();
    let image = match source {
        DynamicImage::ImageRgb8(image) => image,
        image => image.to_rgb8(),
    };
    Ok(DecodedThumbnailSource {
        image,
        source_width,
        source_height,
        scale: "1/1",
    })
}

fn decode_jpeg_turbo(bytes: &[u8]) -> Result<DecodedThumbnailSource> {
    decode_jpeg_turbo_with_max(bytes, thumbnail_size())
}

fn jpeg_dimensions(bytes: &[u8]) -> Result<(u32, u32)> {
    let mut decompressor = Decompressor::new()
        .map_err(|error| anyhow::anyhow!("TurboJPEG initialization failed: {error}"))?;
    let header = decompressor
        .read_header(bytes)
        .map_err(|error| anyhow::anyhow!("TurboJPEG header decode failed: {error}"))?;
    Ok((header.width as u32, header.height as u32))
}

fn decode_jpeg_turbo_with_target(
    bytes: &[u8],
    target_width: u32,
    target_height: u32,
) -> Result<DecodedThumbnailSource> {
    decode_jpeg_turbo_with_scale(bytes, |header| {
        native_scale_for_dimensions(header.width, header.height, target_width, target_height)
    })
}

fn decode_jpeg_turbo_with_max(bytes: &[u8], max_size: u32) -> Result<DecodedThumbnailSource> {
    decode_jpeg_turbo_with_scale(bytes, |header| {
        native_scale_for(header.width.max(header.height), max_size)
    })
}

fn decode_jpeg_turbo_with_scale(
    bytes: &[u8],
    select_scale: impl FnOnce(&turbojpeg::DecompressHeader) -> (ScalingFactor, &'static str),
) -> Result<DecodedThumbnailSource> {
    let mut decompressor = Decompressor::new()
        .map_err(|error| anyhow::anyhow!("TurboJPEG initialization failed: {error}"))?;
    let header = decompressor
        .read_header(bytes)
        .map_err(|error| anyhow::anyhow!("TurboJPEG header decode failed: {error}"))?;
    if header.is_lossless {
        return Err(anyhow::anyhow!(
            "lossless JPEG is not supported by scaled TurboJPEG decode"
        ));
    }

    let scale = select_scale(&header);
    decompressor
        .set_scaling_factor(scale.0)
        .map_err(|error| anyhow::anyhow!("TurboJPEG scale selection failed: {error}"))?;
    let scaled = header.scaled(scale.0);
    let mut pixels = vec![0u8; scaled.width * scaled.height * 3];
    let output = TurboImage {
        pixels: pixels.as_mut_slice(),
        width: scaled.width,
        pitch: scaled.width * 3,
        height: scaled.height,
        format: PixelFormat::RGB,
    };
    decompressor
        .decompress(bytes, output)
        .map_err(|error| anyhow::anyhow!("TurboJPEG pixel decode failed: {error}"))?;
    let image = image::RgbImage::from_raw(scaled.width as u32, scaled.height as u32, pixels)
        .context("TurboJPEG returned an invalid RGB buffer")?;
    Ok(DecodedThumbnailSource {
        image,
        source_width: header.width as u32,
        source_height: header.height as u32,
        scale: scale.1,
    })
}

fn native_scale_for_dimensions(
    source_width: usize,
    source_height: usize,
    target_width: u32,
    target_height: u32,
) -> (ScalingFactor, &'static str) {
    [
        (ScalingFactor::ONE_EIGHTH, "1/8"),
        (ScalingFactor::ONE_QUARTER, "1/4"),
        (ScalingFactor::ONE_HALF, "1/2"),
        (ScalingFactor::ONE, "1/1"),
    ]
    .into_iter()
    .find(|(factor, _)| {
        let scaled_width = factor.scale(source_width);
        let scaled_height = factor.scale(source_height);
        scaled_width >= target_width as usize && scaled_height >= target_height as usize
    })
    .unwrap_or((ScalingFactor::ONE, "1/1"))
}

fn native_scale_for(longest_dimension: usize, target: u32) -> (ScalingFactor, &'static str) {
    if longest_dimension / 8 >= target as usize {
        (ScalingFactor::ONE_EIGHTH, "1/8")
    } else if longest_dimension / 4 >= target as usize {
        (ScalingFactor::ONE_QUARTER, "1/4")
    } else if longest_dimension / 2 >= target as usize {
        (ScalingFactor::ONE_HALF, "1/2")
    } else {
        (ScalingFactor::ONE, "1/1")
    }
}



fn svg_fit_dimensions(
    source_width: u32,
    source_height: u32,
    max_width: u32,
    max_height: u32,
) -> (u32, u32) {
    let scale = (max_width.max(1) as f64 / source_width.max(1) as f64)
        .min(max_height.max(1) as f64 / source_height.max(1) as f64);
    (
        ((source_width as f64 * scale).round() as u32).max(1),
        ((source_height as f64 * scale).round() as u32).max(1),
    )
}

fn load_svg_pixbuf(
    bytes: &[u8],
    max_width: u32,
    max_height: u32,
) -> Result<(gdk_pixbuf::Pixbuf, u32, u32)> {
    let loader =
        gdk_pixbuf::PixbufLoader::with_type("svg").context("SVG loader is unavailable")?;
    let source_size = std::rc::Rc::new(std::cell::Cell::new((0_u32, 0_u32)));
    let prepared_size = source_size.clone();
    loader.connect_size_prepared(move |loader, width, height| {
        let width = width.max(1) as u32;
        let height = height.max(1) as u32;
        prepared_size.set((width, height));
        let (target_width, target_height) =
            svg_fit_dimensions(width, height, max_width, max_height);
        loader.set_size(target_width as i32, target_height as i32);
    });
    loader.write(bytes).context("could not parse SVG")?;
    loader.close().context("could not finish SVG decode")?;
    let pixbuf = loader.pixbuf().context("SVG decoder produced no pixels")?;
    let (source_width, source_height) = source_size.get();
    anyhow::ensure!(
        source_width > 0 && source_height > 0,
        "SVG decoder did not report intrinsic dimensions"
    );
    Ok((pixbuf, source_width, source_height))
}

fn pixbuf_to_rgba(pixbuf: &gdk_pixbuf::Pixbuf) -> Result<image::RgbaImage> {
    let width = u32::try_from(pixbuf.width())?;
    let height = u32::try_from(pixbuf.height())?;
    let rowstride = usize::try_from(pixbuf.rowstride())?;
    let channels = usize::try_from(pixbuf.n_channels())?;
    anyhow::ensure!(channels == 3 || channels == 4, "unsupported SVG pixel layout");
    let data = pixbuf.read_pixel_bytes();
    let data = data.as_ref();
    let mut output = vec![0_u8; width as usize * height as usize * 4];
    for y in 0..height as usize {
        let row = y * rowstride;
        for x in 0..width as usize {
            let source = row + x * channels;
            let destination = (y * width as usize + x) * 4;
            output[destination] = data[source];
            output[destination + 1] = data[source + 1];
            output[destination + 2] = data[source + 2];
            output[destination + 3] = if channels == 4 { data[source + 3] } else { 255 };
        }
    }
    image::RgbaImage::from_raw(width, height, output)
        .context("SVG decoder returned an invalid pixel buffer")
}

fn decode_svg_rgba(
    bytes: &[u8],
    max_width: u32,
    max_height: u32,
) -> Result<(image::RgbaImage, u32, u32)> {
    let (pixbuf, source_width, source_height) =
        load_svg_pixbuf(bytes, max_width, max_height)?;
    Ok((pixbuf_to_rgba(&pixbuf)?, source_width, source_height))
}

#[cfg(test)]
fn decode_svg_thumbnail(bytes: &[u8]) -> Result<DecodedThumbnailSource> {
    decode_svg_thumbnail_with_max(bytes, thumbnail_size())
}

fn decode_svg_thumbnail_with_max(bytes: &[u8], max_edge: u32) -> Result<DecodedThumbnailSource> {
    let (image, source_width, source_height) = decode_svg_rgba(bytes, max_edge, max_edge)?;
    Ok(DecodedThumbnailSource {
        image: DynamicImage::ImageRgba8(image).to_rgb8(),
        source_width,
        source_height,
        scale: "vector",
    })
}

#[cfg(test)]
mod svg_decoder_tests {
    use super::*;

    const SVG: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" width="120" height="80" viewBox="0 0 120 80"><rect width="120" height="80" fill="#336699"/></svg>"##;
    const RESOURCE_SVG: &[u8] =
        include_bytes!("../../resources/custom-icons/collage-grid-symbolic.svg");

    #[test]
    fn svg_dimensions_and_thumbnail_render() {
        assert_eq!(dimensions("fixture.svg", SVG).unwrap(), (120, 80));
        let decoded = decode_svg_thumbnail(SVG).unwrap();
        assert_eq!((decoded.source_width, decoded.source_height), (120, 80));
        assert_eq!((decoded.image.width(), decoded.image.height()), (320, 213));
    }

    #[test]
    fn bundled_resource_svg_renders() {
        let decoded = decode_svg_thumbnail(RESOURCE_SVG).unwrap();
        assert!(decoded.source_width > 0);
        assert!(decoded.source_height > 0);
        assert!(decoded.image.width() > 0);
        assert!(decoded.image.height() > 0);
    }
}

fn rawler_decode<T>(
    reference: &str,
    operation: &str,
    decode: impl FnOnce() -> Result<T> + std::panic::UnwindSafe,
) -> Result<T> {
    let started = std::time::Instant::now();
    let trace = std::env::var_os("PICASA_TRACE").is_some();
    if trace {
        eprintln!("RAW TRACE decode_start operation={operation} path={reference}");
    }
    let result = std::panic::catch_unwind(decode).unwrap_or_else(|_| {
        if trace {
            eprintln!("RAW TRACE panic_caught operation={operation} path={reference}");
        }
        Err(RawlerPanic(reference.to_owned()).into())
    });
    if trace {
        eprintln!("RAW TRACE decode_end operation={operation} elapsed_ms={} success={} path={reference}",
            started.elapsed().as_millis(), result.is_ok());
    }
    result
}

#[cfg(test)]
mod rawler_panic_tests {
    use super::*;

    #[test]
    fn rawler_panic_becomes_error_without_poisoning_scan_guard() {
        let lock = Mutex::new(());
        let _guard = lock.lock().unwrap();
        let result = rawler_decode::<()>("/photos/broken.nef", "thumbnail", || {
            panic!("simulated rawler panic")
        });
        let error = result.unwrap_err().to_string();
        assert!(error.contains("RAW decoder panicked"), "{error}");
        assert!(error.contains("/photos/broken.nef"), "{error}");
        drop(_guard);
        assert!(!lock.is_poisoned());
        let _next_scan = lock.lock().unwrap();
        assert_eq!(
            rawler_decode("/photos/next.nef", "thumbnail", || Ok(7)).unwrap(),
            7
        );
    }

    #[test]
    fn rawler_wrapper_preserves_success() {
        assert_eq!(
            rawler_decode("/photos/good.nef", "preview", || Ok(42)).unwrap(),
            42
        );
    }

    #[test]
    fn rawler_wrapper_preserves_decode_error() {
        let error = rawler_decode::<()>("/photos/bad.nef", "preview", || {
            Err(anyhow::anyhow!("unsupported RAW"))
        })
        .unwrap_err();
        assert_eq!(error.to_string(), "unsupported RAW");
    }
}

// A distinct error lets metadata probing retain its ordinary optional-dimension
// behavior while routing decoder panics through the scanner's per-file failure.
#[derive(Debug)]
pub(crate) struct RawlerPanic(String);

impl std::fmt::Display for RawlerPanic {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "RAW decoder panicked while reading {}", self.0)
    }
}

impl std::error::Error for RawlerPanic {}

fn dng_ifd_dimensions(raw: &rawler::formats::tiff::IFD) -> Result<(u32, u32)> {
    use rawler::tags::{DngTag, TiffCommonTag};

    let value = |entry: &rawler::formats::tiff::Entry, index| -> Result<u32> {
        entry
            .get_u32(index)
            .map_err(|_| anyhow::anyhow!("invalid DNG geometry tag {}", entry.tag))?
            .with_context(|| format!("missing DNG geometry value in tag {}", entry.tag))
    };
    let width = value(
        raw.get_entry(TiffCommonTag::ImageWidth)
            .context("missing DNG width")?,
        0,
    )?;
    let height = value(
        raw.get_entry(TiffCommonTag::ImageLength)
            .context("missing DNG height")?,
        0,
    )?;
    anyhow::ensure!(width > 0 && height > 0, "invalid DNG sensor dimensions");

    let (active_width, active_height) = if let Some(area) = raw.get_entry(DngTag::ActiveArea) {
        let (top, left, bottom, right) = (
            value(area, 0)?,
            value(area, 1)?,
            value(area, 2)?,
            value(area, 3)?,
        );
        anyhow::ensure!(
            top < bottom && left < right && bottom <= height && right <= width,
            "invalid DNG active area"
        );
        (right - left, bottom - top)
    } else {
        (width, height)
    };
    if let (Some(origin), Some(size)) = (
        raw.get_entry(DngTag::DefaultCropOrigin),
        raw.get_entry(DngTag::DefaultCropSize),
    ) {
        let (x, y) = (value(origin, 0)?, value(origin, 1)?);
        let (crop_width, crop_height) = (value(size, 0)?, value(size, 1)?);
        anyhow::ensure!(
            crop_width > 0
                && crop_height > 0
                && u64::from(x) + u64::from(crop_width) <= u64::from(active_width)
                && u64::from(y) + u64::from(crop_height) <= u64::from(active_height),
            "invalid DNG default crop"
        );
        return Ok((crop_width, crop_height));
    }
    Ok((active_width, active_height))
}

#[cfg(test)]
mod dng_dimension_tests {
    use super::*;
    use rawler::formats::tiff::{Entry, Value, IFD};

    fn geometry(extra: &[(u16, Vec<u32>)]) -> IFD {
        let mut raw = IFD::default();
        for (tag, values) in [(256, vec![6000]), (257, vec![4000])].iter().chain(extra) {
            raw.entries.insert(
                *tag,
                Entry {
                    tag: *tag,
                    value: Value::Long(values.clone()),
                    embedded: None,
                },
            );
        }
        raw
    }

    #[test]
    fn dng_dimensions_prefer_default_crop_without_sensor_pixels() {
        let raw = geometry(&[
            (50719, vec![10, 20]),
            (50720, vec![5900, 3800]),
            (50829, vec![5, 5, 3995, 5995]),
        ]);
        assert_eq!(dng_ifd_dimensions(&raw).unwrap(), (5900, 3800));
    }

    #[test]
    fn dng_dimensions_fall_back_to_active_area_then_sensor_geometry() {
        assert_eq!(
            dng_ifd_dimensions(&geometry(&[(50829, vec![10, 20, 3990, 5980])])).unwrap(),
            (5960, 3980)
        );
        assert_eq!(dng_ifd_dimensions(&geometry(&[])).unwrap(), (6000, 4000));
    }

    #[test]
    fn invalid_dng_geometry_returns_error() {
        assert!(dng_ifd_dimensions(&geometry(&[(256, vec![])])).is_err());
        assert!(dng_ifd_dimensions(&geometry(&[(50829, vec![4000, 0, 10, 6000])])).is_err());
        assert!(
            dng_ifd_dimensions(&geometry(&[(50719, vec![0, 0]), (50720, vec![7000, 4000])]))
                .is_err()
        );
    }

    #[test]
    #[ignore = "set PICASA_TEST_DNG to a real DNG for metadata-only verification"]
    fn requested_dng_dimensions_do_not_decode_pixels() {
        let reference = std::env::var("PICASA_TEST_DNG").unwrap();
        let (width, height) = dimensions(&reference, &[]).unwrap();
        eprintln!("DNG geometry: {width}x{height}");
        assert!(width > 0 && height > 0);
    }
}
