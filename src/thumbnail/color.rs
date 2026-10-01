//! JPEG source RGB -> sRGB, before orientation, resizing, or adding alpha.

use std::cell::RefCell;
use std::collections::VecDeque;

use anyhow::{ensure, Context, Result};
use lcms2::{ColorSpaceSignature, Flags, Intent, PixelFormat, Profile, ThreadContext, Transform};

const ICC_SIGNATURE: &[u8] = b"ICC_PROFILE\0";
const MAX_ICC_BYTES: usize = 4 * 1024 * 1024;
const MAX_METADATA_BYTES: usize = 16 * 1024 * 1024;
const MAX_ICC_TAGS: usize = 4096;
const TRANSFORM_CACHE_CAPACITY: usize = 4;

struct EmbeddedIcc {
    bytes: Vec<u8>,
    segments: u8,
}

/// Scan only JPEG headers, never entropy-coded scan data. Segment slices
/// borrow the already-read source; allocate the profile only after validation.
fn jpeg_icc_profile(bytes: &[u8]) -> Result<Option<EmbeddedIcc>> {
    ensure!(bytes.starts_with(&[0xff, 0xd8]), "missing JPEG SOI");
    let mut offset = 2;
    let mut count = None;
    let mut segments: [Option<&[u8]>; 255] = [None; 255];
    let mut total = 0usize;
    loop {
        ensure!(
            offset < MAX_METADATA_BYTES,
            "JPEG metadata scan limit exceeded"
        );
        ensure!(bytes.get(offset) == Some(&0xff), "missing JPEG marker");
        while bytes.get(offset) == Some(&0xff) {
            offset += 1;
            ensure!(
                offset < MAX_METADATA_BYTES,
                "JPEG marker fill limit exceeded"
            );
        }
        let marker = *bytes.get(offset).context("truncated JPEG marker")?;
        offset += 1;
        match marker {
            0xda | 0xd9 => break,           // SOS / EOI: do not enter compressed data
            0x01 | 0xd0..=0xd7 => continue, // TEM / restart markers have no length
            0x00 | 0xd8 => anyhow::bail!("invalid JPEG header marker"),
            _ => {}
        }
        let length_bytes = bytes
            .get(offset..offset + 2)
            .context("truncated JPEG segment length")?;
        let length = usize::from(u16::from_be_bytes([length_bytes[0], length_bytes[1]]));
        ensure!(length >= 2, "invalid JPEG segment length");
        let end = offset
            .checked_add(length)
            .context("JPEG segment overflow")?;
        ensure!(
            end <= MAX_METADATA_BYTES,
            "JPEG metadata scan limit exceeded"
        );
        let payload = bytes
            .get(offset + 2..end)
            .context("truncated JPEG segment")?;
        offset = end;
        if marker != 0xe2 || !payload.starts_with(ICC_SIGNATURE) {
            continue;
        }
        ensure!(payload.len() >= 14, "truncated ICC APP2 header");
        let sequence = payload[12];
        let segment_count = payload[13];
        ensure!(segment_count != 0, "zero ICC segment count");
        ensure!(
            sequence >= 1 && sequence <= segment_count,
            "invalid ICC sequence number"
        );
        if let Some(previous) = count {
            ensure!(previous == segment_count, "inconsistent ICC segment counts");
        }
        count = Some(segment_count);
        let slot = &mut segments[usize::from(sequence) - 1];
        ensure!(slot.is_none(), "duplicate ICC sequence number");
        let data = &payload[14..];
        ensure!(!data.is_empty(), "empty ICC segment");
        total = total.checked_add(data.len()).context("ICC size overflow")?;
        ensure!(total <= MAX_ICC_BYTES, "ICC profile exceeds 4 MiB limit");
        *slot = Some(data);
    }
    let Some(count) = count else {
        return Ok(None);
    };
    let parts = &segments[..usize::from(count)];
    ensure!(parts.iter().all(Option::is_some), "incomplete ICC sequence");
    let mut profile = Vec::new();
    profile
        .try_reserve_exact(total)
        .context("ICC allocation failed")?;
    for part in parts {
        profile.extend_from_slice(part.context("missing ICC segment")?);
    }
    Ok(Some(EmbeddedIcc {
        bytes: profile,
        segments: count,
    }))
}

/// Bound embedded ICC header/tag declarations before passing them to LCMS.
/// LCMS validates the tag contents and supported profile/transform types.
fn validate_icc(icc: &[u8]) -> Result<()> {
    ensure!(
        icc.len() >= 132 && icc.len() <= MAX_ICC_BYTES,
        "invalid ICC profile size"
    );
    let read_u32 = |offset: usize| -> usize {
        u32::from_be_bytes([
            icc[offset],
            icc[offset + 1],
            icc[offset + 2],
            icc[offset + 3],
        ]) as usize
    };
    ensure!(
        read_u32(0) == icc.len(),
        "ICC declared size does not match embedded size"
    );
    ensure!(&icc[36..40] == b"acsp", "invalid ICC signature");
    ensure!(&icc[16..20] == b"RGB ", "ICC source is not RGB");
    let tags = read_u32(128);
    ensure!(tags <= MAX_ICC_TAGS, "ICC tag count exceeds limit");
    let table_end = 132 + tags * 12;
    ensure!(table_end <= icc.len(), "truncated ICC tag table");
    for entry in (132..table_end).step_by(12) {
        let start = read_u32(entry + 4);
        let size = read_u32(entry + 8);
        ensure!(
            start >= table_end && size >= 8 && start <= icc.len() && size <= icc.len() - start,
            "ICC tag outside profile bounds"
        );
        validate_lut_tag(&icc[start..start + size])?;
    }
    Ok(())
}

/// LCMS may allocate a CLUT before discovering missing table data. Validate
/// conventional v2/v4 LUT storage sizes first; cap them by actual bounded bytes.
/// Advanced multi-process elements need a separate nested allocation validator;
/// conservatively fall back for those rather than trust their sampled curves.
fn validate_lut_tag(tag: &[u8]) -> Result<()> {
    let u32_at = |offset: usize| -> Result<usize> {
        let b = tag
            .get(offset..offset + 4)
            .context("truncated ICC LUT integer")?;
        Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as usize)
    };
    let channels = |offset: usize| -> Result<(usize, usize)> {
        let b = tag
            .get(offset..offset + 2)
            .context("truncated ICC LUT channels")?;
        let (input, output) = (usize::from(b[0]), usize::from(b[1]));
        ensure!(
            (1..=16).contains(&input) && (1..=16).contains(&output),
            "invalid ICC LUT channels"
        );
        Ok((input, output))
    };
    let table_entries = |grids: &[u8], output: usize| -> Result<usize> {
        let mut entries = output;
        for &grid in grids {
            ensure!(grid >= 2, "invalid ICC CLUT grid");
            entries = entries
                .checked_mul(usize::from(grid))
                .context("ICC CLUT size overflow")?;
            ensure!(entries <= MAX_ICC_BYTES, "ICC CLUT exceeds size limit");
        }
        Ok(entries)
    };
    match &tag[..4] {
        b"mft1" | b"mft2" => {
            let (input, output) = channels(8)?;
            let grid = *tag.get(10).context("truncated ICC CLUT grid")?;
            let entries = if grid == 0 {
                0
            } else {
                table_entries(&[grid; 16][..input], output)?
            };
            let required = if &tag[..4] == b"mft1" {
                48 + input * 256 + entries + output * 256
            } else {
                let b = tag.get(48..52).context("truncated ICC LUT16 tables")?;
                let input_entries = usize::from(u16::from_be_bytes([b[0], b[1]]));
                let output_entries = usize::from(u16::from_be_bytes([b[2], b[3]]));
                52 + 2 * (input * input_entries + entries + output * output_entries)
            };
            ensure!(required <= tag.len(), "ICC LUT tables exceed tag bytes");
        }
        b"mAB " | b"mBA " => {
            ensure!(tag.len() >= 32, "truncated ICC v4 LUT header");
            let (input, output) = channels(8)?;
            for offset in [12, 16, 20, 24, 28] {
                let start = u32_at(offset)?;
                if start == 0 {
                    continue;
                }
                let minimum = if offset == 16 {
                    48
                } else if offset == 24 {
                    20
                } else {
                    8
                };
                ensure!(
                    start >= 32 && start <= tag.len() && minimum <= tag.len() - start,
                    "ICC v4 LUT offset outside tag"
                );
            }
            let start = u32_at(24)?;
            if start != 0 {
                let entries = table_entries(&tag[start..start + input], output)?;
                let precision = usize::from(tag[start + 16]);
                ensure!(
                    precision == 1 || precision == 2,
                    "invalid ICC CLUT precision"
                );
                ensure!(
                    entries * precision <= tag.len() - start - 20,
                    "ICC CLUT tables exceed tag bytes"
                );
            }
        }
        b"mpet" => anyhow::bail!(
            "advanced ICC multi-process elements are not supported by bounded validation"
        ),
        _ => {}
    }
    Ok(())
}

type RgbTransform = Transform<u8, u8, ThreadContext>;
struct CachedTransform {
    // Compare exact bytes too, so hash collisions cannot select another profile.
    key: blake3::Hash,
    icc: Vec<u8>,
    transform: RgbTransform,
    // Keep the context alive until AFTER the transform is destroyed.
    _context: ThreadContext,
}
thread_local! {
    // No LCMS objects are shared between workers. At most 4 profiles (16 MiB)
    // plus four LCMS transforms are retained per decoding thread; no startup work.
    static TRANSFORMS: RefCell<VecDeque<CachedTransform>> = const { RefCell::new(VecDeque::new()) };
}

/// Errors occur before any pixels are touched, so callers can retain raw RGB.
/// RGB_8 is fixed, and chunks are multiples of three, satisfying LCMS's safe
/// binding requirements without casts, unsafe code, or another image buffer.
fn convert_rgb8_to_srgb(image: &mut image::RgbImage, icc: &[u8]) -> Result<()> {
    validate_icc(icc)?;
    ensure!(image.as_raw().len() % 3 == 0, "invalid RGB buffer length");
    let key = blake3::hash(icc);
    TRANSFORMS.with(|cache| -> Result<()> {
        let mut cache = cache
            .try_borrow_mut()
            .context("ICC transform cache already borrowed")?;
        let entry = if let Some(position) = cache
            .iter()
            .position(|entry| entry.key == key && entry.icc == icc)
        {
            cache.remove(position).context("ICC cache entry missing")?
        } else {
            let context = ThreadContext::new();
            let source =
                Profile::new_icc_context(&context, icc).context("LittleCMS rejected source ICC")?;
            ensure!(
                source.color_space() == ColorSpaceSignature::RgbData,
                "ICC source is not RGB"
            );
            let target = Profile::new_srgb_context(&context);
            // Fixed application intent: preserve in-gamut colours, map black
            // points. Ignore the profile header's suggested rendering intent.
            let transform = Transform::new_flags_context(
                &context,
                &source,
                PixelFormat::RGB_8,
                &target,
                PixelFormat::RGB_8,
                Intent::RelativeColorimetric,
                Flags::BLACKPOINT_COMPENSATION,
            )
            .context("LittleCMS could not create RGB -> sRGB transform")?;
            CachedTransform {
                key,
                icc: icc.to_vec(),
                transform,
                _context: context,
            }
        };
        for chunk in image.as_mut().chunks_mut(3 * 65_536) {
            entry.transform.transform_in_place(chunk);
        }
        if cache.len() >= TRANSFORM_CACHE_CAPACITY {
            cache.pop_front();
        }
        cache.push_back(entry);
        Ok(())
    })
}

pub(super) fn color_manage_jpeg_rgb(
    bytes: &[u8],
    mut image: image::RgbImage,
    reference: &str,
) -> image::RgbImage {
    let trace = std::env::var_os("PICASA_TRACE").is_some();
    // Match existing viewer trace redaction; never log URI credentials.
    let path = if trace {
        reference
            .split_once("://")
            .map(|(scheme, rest)| {
                format!(
                    "{scheme}://{}",
                    rest.rsplit_once('@').map_or(rest, |(_, tail)| tail)
                )
            })
            .unwrap_or_else(|| reference.to_owned())
    } else {
        String::new()
    };
    match jpeg_icc_profile(bytes) {
        Ok(None) => {
            if trace {
                eprintln!("COLOR TRACE jpeg_icc path={path} status=none assume=sRGB");
            }
        }
        Ok(Some(icc)) => {
            if trace {
                eprintln!(
                    "COLOR TRACE jpeg_icc path={path} status=embedded bytes={} segments={}",
                    icc.bytes.len(),
                    icc.segments
                );
            }
            let started = std::time::Instant::now();
            match convert_rgb8_to_srgb(&mut image, &icc.bytes) {
                Ok(()) => {
                    if trace {
                        eprintln!("COLOR TRACE transform path={path} source_profile_hash={} target=sRGB intent=RelativeColorimetric bpc=true transform_ms={:.3}", blake3::hash(&icc.bytes), started.elapsed().as_secs_f64()*1000.0);
                    }
                }
                Err(error) => {
                    if trace {
                        eprintln!("COLOR TRACE icc_transform_failed path={path} reason={error:#} fallback=raw-rgb");
                    }
                }
            }
        }
        Err(error) => {
            if trace {
                eprintln!(
                    "COLOR TRACE icc_parse_failed path={path} reason={error:#} fallback=raw-rgb"
                );
            }
        }
    }
    image
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jpeg(parts: &[(u8, u8, &[u8])]) -> Vec<u8> {
        let mut bytes = vec![0xff, 0xd8];
        for &(sequence, count, data) in parts {
            let mut payload = b"ICC_PROFILE\0".to_vec();
            payload.extend([sequence, count]);
            payload.extend(data);
            bytes.extend([0xff, 0xe2]);
            bytes.extend(((payload.len() + 2) as u16).to_be_bytes());
            bytes.extend(payload);
        }
        bytes.extend([0xff, 0xda]);
        bytes
    }

    fn profile(bytes: &[u8]) -> Vec<u8> {
        jpeg_icc_profile(bytes).unwrap().unwrap().bytes
    }

    #[test]
    fn no_icc_and_unrelated_app2_are_ignored() {
        assert!(jpeg_icc_profile(&jpeg(&[])).unwrap().is_none());
        assert!(
            jpeg_icc_profile(&[0xff, 0xd8, 0xff, 0xe2, 0, 5, 1, 2, 3, 0xff, 0xd9])
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn reconstructs_single_multiple_and_out_of_order_segments() {
        assert_eq!(profile(&jpeg(&[(1, 1, b"abc")])), b"abc");
        assert_eq!(profile(&jpeg(&[(1, 2, b"abc"), (2, 2, b"def")])), b"abcdef");
        assert_eq!(profile(&jpeg(&[(2, 2, b"def"), (1, 2, b"abc")])), b"abcdef");
    }

    #[test]
    fn invalid_sequences_fail_safely() {
        for parts in [
            vec![(1, 2, b"a".as_slice())],
            vec![(1, 2, b"a".as_slice()), (1, 2, b"b".as_slice())],
            vec![(1, 2, b"a".as_slice()), (2, 3, b"b".as_slice())],
            vec![(0, 1, b"a".as_slice())],
            vec![(2, 1, b"a".as_slice())],
            vec![(1, 0, b"a".as_slice())],
        ] {
            assert!(jpeg_icc_profile(&jpeg(&parts)).is_err());
        }
    }

    #[test]
    fn truncated_and_invalid_lengths_fail_safely() {
        let bytes = jpeg(&[(1, 1, b"abc")]);
        for end in 3..bytes.len() - 2 {
            assert!(jpeg_icc_profile(&bytes[..end]).is_err(), "end={end}");
        }
        assert!(jpeg_icc_profile(&[0xff, 0xd8, 0xff, 0xe2, 0xff, 0xff]).is_err());
        assert!(jpeg_icc_profile(&[0xff, 0xd8, 0xff, 0xe2, 0, 1]).is_err());
    }

    #[test]
    fn size_limit_rejects_without_reconstruction() {
        let data = vec![0; 60_000];
        let parts: Vec<_> = (1..=80).map(|i| (i, 80, data.as_slice())).collect();
        assert!(jpeg_icc_profile(&jpeg(&parts)).is_err());
    }

    #[test]
    fn fills_standalone_markers_and_scan_boundary() {
        let mut bytes = vec![0xff, 0xd8, 0xff, 0xff, 0x01, 0xff, 0xd0];
        bytes.extend_from_slice(&jpeg(&[(1, 1, b"abc")])[2..]);
        bytes.extend_from_slice(&jpeg(&[(1, 1, b"ignored scan bytes")]));
        assert_eq!(profile(&bytes), b"abc");
    }

    #[test]
    fn invalid_profile_and_absent_profile_preserve_pixels() {
        let image = image::RgbImage::from_pixel(3, 2, image::Rgb([12, 85, 230]));
        assert!(convert_rgb8_to_srgb(&mut image.clone(), b"broken").is_err());
        assert_eq!(
            color_manage_jpeg_rgb(&jpeg(&[(1, 1, b"broken")]), image.clone(), "test"),
            image
        );
        assert_eq!(
            color_manage_jpeg_rgb(&jpeg(&[]), image.clone(), "test"),
            image
        );
        assert_eq!(
            color_manage_jpeg_rgb(&jpeg(&[(1, 2, b"broken")]), image.clone(), "test"),
            image
        );
    }

    #[test]
    fn srgb_profile_preserves_dimensions_length_and_pixels() {
        let icc = lcms2::Profile::new_srgb().icc().unwrap();
        let mut image = image::RgbImage::from_pixel(3, 2, image::Rgb([12, 85, 230]));
        let original = image.clone();
        convert_rgb8_to_srgb(&mut image, &icc).unwrap();
        assert_eq!(image.dimensions(), (3, 2));
        assert_eq!(image.as_raw().len(), 18);
        assert_eq!(image, original);
    }
    fn prophoto_icc() -> Vec<u8> {
        let xy = |x, y| lcms2::CIExyY { x, y, Y: 1.0 };
        let primaries = lcms2::CIExyYTRIPLE {
            Red: xy(0.7347, 0.2653),
            Green: xy(0.1596, 0.8404),
            Blue: xy(0.0366, 0.0001),
        };
        let gamma = lcms2::ToneCurve::new(1.8);
        lcms2::Profile::new_rgb(&xy(0.3457, 0.3585), &primaries, &[&gamma, &gamma, &gamma])
            .unwrap()
            .icc()
            .unwrap()
    }

    #[test]
    fn wide_gamut_conversion_matches_independent_matrix_reference() {
        // ProPhoto gamma 1.8 -> D50 XYZ -> Bradford D65 -> sRGB, computed
        // independently; none of these input channels fall in ProPhoto's toe.
        let mut image = image::RgbImage::from_pixel(3, 2, image::Rgb([128, 64, 32]));
        convert_rgb8_to_srgb(&mut image, &prophoto_icc()).unwrap();
        assert_eq!(image.dimensions(), (3, 2));
        assert_eq!(image.as_raw().len(), 18);
        for (&actual, expected) in image.get_pixel(0, 0).0.iter().zip([191u8, 53, 29]) {
            assert!(
                actual.abs_diff(expected) <= 2,
                "actual={actual} expected={expected}"
            );
        }
    }

    #[test]
    fn hostile_icc_header_and_tag_lengths_preserve_pixels() {
        let original = image::RgbImage::from_pixel(1, 1, image::Rgb([12, 85, 230]));
        for (offset, value) in [
            (0, u32::MAX),
            (128, u32::MAX),
            (136, u32::MAX),
            (140, u32::MAX),
        ] {
            let mut icc = prophoto_icc();
            icc[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
            let mut pixels = original.clone();
            assert!(convert_rgb8_to_srgb(&mut pixels, &icc).is_err());
            assert_eq!(pixels, original);
        }
    }

    #[test]
    fn jpeg_thumbnail_fit_native_and_fallback_share_srgb_pixels() {
        use super::super::*;
        let directory = cache_layout_tests::fixture_for_wall();
        let source = directory.join("profiled.jpg");
        let cached = directory.join("cached.jpg");
        let image = image::RgbImage::from_pixel(800, 600, image::Rgb([128, 64, 32]));
        let mut bytes = Vec::new();
        let mut encoder = JpegEncoder::new_with_quality(&mut bytes, 100);
        encoder.set_icc_profile(prophoto_icc()).unwrap();
        encoder
            .write_image(image.as_raw(), 800, 600, ColorType::Rgb8.into())
            .unwrap();
        // Orientation 6 (90 degrees clockwise), to check conversion order and
        // dimensions in all existing consumer paths.
        let exif = [
            b"Exif\0\0".as_slice(),
            &[
                b'I', b'I', 42, 0, 8, 0, 0, 0, 1, 0, 0x12, 1, 3, 0, 1, 0, 0, 0, 6, 0, 0, 0, 0, 0,
                0, 0,
            ],
        ]
        .concat();
        let mut oriented = vec![0xff, 0xd8, 0xff, 0xe1];
        oriented.extend(((exif.len() + 2) as u16).to_be_bytes());
        oriented.extend(exif);
        oriented.extend_from_slice(&bytes[2..]);
        fs::write(&source, &oriented).unwrap();
        let reference = source.to_str().unwrap();
        create_uncached_with_max(reference, &cached, 200).unwrap();
        let thumbnail = image::open(&cached).unwrap().to_rgb8();
        let fit = decode_for_viewer(reference, 150, 200).unwrap();
        let native = decode_for_viewer(reference, u32::MAX, u32::MAX).unwrap();
        assert_eq!(thumbnail.dimensions(), (150, 200));
        assert_eq!(fit.dimensions(), (150, 200));
        assert_eq!(native.dimensions(), (600, 800));
        let scaled = decode_jpeg_turbo_with_target(&oriented, 200, 150).unwrap();
        assert_eq!(scaled.scale, "1/4");
        let fallback = decode_with_image(&oriented).unwrap();
        let fallback = color_manage_jpeg_rgb(&oriented, fallback.image, "test fallback");
        for pixel in [
            thumbnail.get_pixel(0, 0).0,
            fit.get_pixel(0, 0).0[..3].try_into().unwrap(),
            native.get_pixel(0, 0).0[..3].try_into().unwrap(),
            fallback.get_pixel(0, 0).0,
        ] {
            for (actual, expected) in pixel.into_iter().zip([191u8, 53, 29]) {
                assert!(actual.abs_diff(expected) <= 4, "pixel={pixel:?}");
            }
        }
        assert!(fit
            .pixels()
            .chain(native.pixels())
            .all(|pixel| pixel[3] == 255));
        assert!(jpeg_icc_profile(&fs::read(&cached).unwrap())
            .unwrap()
            .is_none());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn per_thread_transforms_are_independent_and_chunked() {
        let icc = prophoto_icc();
        let threads: Vec<_> = (0..4)
            .map(|_| {
                let icc = icc.clone();
                std::thread::spawn(move || {
                    // Longer than one transform chunk; repeated use exercises reuse.
                    for _ in 0..2 {
                        let mut image =
                            image::RgbImage::from_pixel(1000, 100, image::Rgb([128, 64, 32]));
                        convert_rgb8_to_srgb(&mut image, &icc).unwrap();
                        assert_eq!(image.get_pixel(0, 0), image.get_pixel(999, 99));
                        assert!(image.get_pixel(999, 99)[0].abs_diff(191) <= 2);
                    }
                })
            })
            .collect();
        for thread in threads {
            thread.join().unwrap();
        }
    }

    #[test]
    fn tiny_icc_cannot_declare_huge_lookup_tables() {
        for kind in [b"mft1", b"mft2", b"mAB ", b"mBA ", b"mpet"] {
            let mut tag = vec![0u8; 64];
            tag[..4].copy_from_slice(kind);
            tag[8..11].copy_from_slice(&[3, 3, 255]);
            if kind == b"mAB " || kind == b"mBA " {
                tag[24..28].copy_from_slice(&32u32.to_be_bytes());
                tag[32..35].fill(255);
                tag[48] = 2;
            }
            let mut icc = vec![0u8; 144];
            icc[16..20].copy_from_slice(b"RGB ");
            icc[36..40].copy_from_slice(b"acsp");
            icc[128..132].copy_from_slice(&1u32.to_be_bytes());
            icc[132..136].copy_from_slice(b"A2B0");
            icc[136..140].copy_from_slice(&144u32.to_be_bytes());
            icc[140..144].copy_from_slice(&(tag.len() as u32).to_be_bytes());
            icc.extend(tag);
            let len = icc.len() as u32;
            icc[..4].copy_from_slice(&len.to_be_bytes());
            assert!(validate_icc(&icc).is_err(), "kind={kind:?}");
        }
    }

    #[test]
    fn adobe_rgb_uses_embedded_primaries_and_gamma() {
        let xy = |x, y| lcms2::CIExyY { x, y, Y: 1.0 };
        let primaries = lcms2::CIExyYTRIPLE {
            Red: xy(0.64, 0.33),
            Green: xy(0.21, 0.71),
            Blue: xy(0.15, 0.06),
        };
        let gamma = lcms2::ToneCurve::new(563.0 / 256.0);
        let icc =
            lcms2::Profile::new_rgb(&xy(0.3127, 0.3290), &primaries, &[&gamma, &gamma, &gamma])
                .unwrap()
                .icc()
                .unwrap();
        let mut pixels = image::RgbImage::from_pixel(1, 1, image::Rgb([128, 64, 32]));
        convert_rgb8_to_srgb(&mut pixels, &icc).unwrap();
        // Independently computed Adobe RGB -> XYZ D65 -> sRGB.
        for (actual, expected) in pixels.get_pixel(0, 0).0.into_iter().zip([146u8, 62, 23]) {
            assert!(
                actual.abs_diff(expected) <= 2,
                "actual={actual} expected={expected}"
            );
        }
    }
}
