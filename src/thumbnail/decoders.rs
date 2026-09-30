pub fn dimensions(reference: &str, bytes: &[u8]) -> Result<(u32, u32)> {
    if is_psd(reference) {
        psd_dimensions(bytes)
    } else if is_raw(reference) {
        let local = crate::source::materialize(reference)?;
        let rawfile = rawler::rawsource::RawSource::new(&local)?;
        let decoder = rawler::get_decoder(&rawfile)?;
        // `dummy = true` reads the RAW geometry without unpacking the sensor
        // pixels. Prefer the recommended crop shown by photo applications.
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
    decode_jpeg_turbo_with_max(bytes, THUMBNAIL_SIZE)
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



fn psd_dimensions(bytes: &[u8]) -> Result<(u32, u32)> {
    anyhow::ensure!(bytes.len() >= 26, "PSD header is truncated");
    anyhow::ensure!(&bytes[0..4] == b"8BPS", "invalid PSD signature");
    anyhow::ensure!(u16::from_be_bytes([bytes[4], bytes[5]]) == 1, "unsupported PSD version");
    let height = u32::from_be_bytes([bytes[14], bytes[15], bytes[16], bytes[17]]);
    let width = u32::from_be_bytes([bytes[18], bytes[19], bytes[20], bytes[21]]);
    anyhow::ensure!(width > 0 && height > 0, "PSD has invalid dimensions");
    Ok((width, height))
}

fn decode_psd_rgba(bytes: &[u8]) -> Result<image::RgbaImage> {
    let psd = psd::Psd::from_bytes(bytes)
        .map_err(|error| anyhow::anyhow!("PSD decode failed: {error}"))?;
    let width = psd.width();
    let height = psd.height();
    image::RgbaImage::from_raw(width, height, psd.rgba())
        .context("PSD decoder returned an invalid composite image")
}

fn is_pdf_compatible_ai(bytes: &[u8]) -> bool {
    bytes
        .get(..bytes.len().min(4096))
        .is_some_and(|prefix| prefix.windows(5).any(|window| window == b"%PDF-"))
}

fn decode_ai_rgba(bytes: &[u8], max_dimension: u32) -> Result<image::RgbaImage> {
    anyhow::ensure!(
        is_pdf_compatible_ai(bytes),
        "Illustrator file is not PDF-compatible; save it with Create PDF Compatible File enabled"
    );

    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let base = std::env::temp_dir().join(format!("picasa-ai-{}-{stamp}", std::process::id()));
    let input = base.with_extension("ai");
    let output_prefix = base.with_extension("preview");
    let output = std::path::PathBuf::from(format!("{}.png", output_prefix.display()));
    fs::write(&input, bytes)?;

    let result = std::process::Command::new("pdftoppm")
        .arg("-f")
        .arg("1")
        .arg("-singlefile")
        .arg("-png")
        .arg("-scale-to")
        .arg(max_dimension.max(1).to_string())
        .arg(&input)
        .arg(&output_prefix)
        .output();

    let decoded = match result {
        Ok(result) if result.status.success() => image::open(&output)
            .with_context(|| format!("could not read Illustrator preview {}", output.display()))?
            .into_rgba8(),
        Ok(result) => {
            let error = String::from_utf8_lossy(&result.stderr);
            let _ = fs::remove_file(&input);
            let _ = fs::remove_file(&output);
            anyhow::bail!("Illustrator PDF renderer failed: {}", error.trim());
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let _ = fs::remove_file(&input);
            anyhow::bail!("Illustrator preview requires pdftoppm (Poppler utilities)");
        }
        Err(error) => {
            let _ = fs::remove_file(&input);
            return Err(error).context("could not start Illustrator PDF renderer");
        }
    };
    let _ = fs::remove_file(&input);
    let _ = fs::remove_file(&output);
    Ok(decoded)
}

fn decode_psd_thumbnail(bytes: &[u8]) -> Result<DecodedThumbnailSource> {
    let image = decode_psd_rgba(bytes)?;
    let source_width = image.width();
    let source_height = image.height();
    Ok(DecodedThumbnailSource {
        image: resize(DynamicImage::ImageRgba8(image).to_rgb8())?,
        source_width,
        source_height,
        scale: "PSD composite",
    })
}

fn decode_ai_thumbnail(bytes: &[u8]) -> Result<DecodedThumbnailSource> {
    let image = decode_ai_rgba(bytes, THUMBNAIL_SIZE)?;
    let source_width = image.width();
    let source_height = image.height();
    Ok(DecodedThumbnailSource {
        image: image.to_rgb8(),
        source_width,
        source_height,
        scale: "AI PDF preview",
    })
}

#[cfg(test)]
mod adobe_decoder_tests {
    use super::*;

    #[test]
    fn psd_header_dimensions_are_read_without_decoding_layers() {
        let mut header = vec![0_u8; 26];
        header[0..4].copy_from_slice(b"8BPS");
        header[4..6].copy_from_slice(&1_u16.to_be_bytes());
        header[14..18].copy_from_slice(&1080_u32.to_be_bytes());
        header[18..22].copy_from_slice(&1920_u32.to_be_bytes());
        assert_eq!(psd_dimensions(&header).unwrap(), (1920, 1080));
    }

    #[test]
    fn ai_requires_a_pdf_compatible_payload() {
        assert!(is_pdf_compatible_ai(b"%PDF-1.7\n% Illustrator"));
        assert!(!is_pdf_compatible_ai(b"%!PS-Adobe-3.0 EPSF-3.0"));
    }
}
