// Samsung stores some reduced DNG previews as an 8-bit Y plane followed
// by interleaved Cr/Cb (NV21). The IFD describes 12 bits per pixel and claims
// three bytes per pixel even though the complete 4:2:0 preview occupies 1.5.
// Read bounded RGB strips directly too, without preloading the RAW sensor data.
fn dng_embedded_preview(path: &Path) -> Result<Option<DecodedThumbnailSource>> {
    use rawler::formats::tiff::{GenericTiffReader, reader::TiffReader};

    let mut file = BufReader::new(fs::File::open(path)?);
    let file_size = file.get_ref().metadata()?.len();
    // This reads TIFF metadata and seeks over sensor strips, without loading
    // the RAW pixel buffer or asking a decoder to develop it.
    let tiff = GenericTiffReader::new(&mut file, 0, 0, Some(16), &[])?;
    let root = tiff.root_ifd();
    let samsung = root
        .get_entry(271u16)
        .and_then(|entry| entry.as_string())
        .is_some_and(|make| make.eq_ignore_ascii_case("samsung"));
    if root.get_entry(50706u16).is_none() {
        return Ok(None);
    }
    let scalar = |ifd: &rawler::formats::tiff::IFD, tag: u16| {
        let entry = ifd.get_entry(tag)?;
        (entry.count() == 1)
            .then(|| entry.get_u32(0).ok().flatten())
            .flatten()
    };
    for preview in tiff.find_ifds_with_filter(|ifd| scalar(ifd, 254) == Some(1)) {
        if scalar(preview, 259) != Some(1)
            || scalar(preview, 277) != Some(3)
            || scalar(preview, 284) != Some(1)
        {
            continue;
        }
        let (Some(width), Some(height), Some(offset), Some(claimed)) = (
            scalar(preview, 256),
            scalar(preview, 257),
            scalar(preview, 273),
            scalar(preview, 279),
        ) else {
            continue;
        };
        if width == 0
            || height == 0
            || width > 2048
            || height > 2048
            || scalar(preview, 278) != Some(height)
        {
            continue;
        }
        let pixels = u64::from(width) * u64::from(height);
        let rgb_bits = preview.get_entry(258u16).is_some_and(|entry| {
            (entry.count() == 1 || entry.count() == 3)
                && (0..entry.count())
                    .all(|index| entry.get_u32(index as usize).ok().flatten() == Some(8))
        });
        if scalar(preview, 262) == Some(2)
            && rgb_bits
            && u64::from(claimed) == pixels * 3
            && u64::from(offset) + u64::from(claimed) <= file_size
        {
            file.seek(SeekFrom::Start(u64::from(offset)))?;
            let mut bytes = vec![0; claimed as usize];
            file.read_exact(&mut bytes)?;
            let image = image::RgbImage::from_raw(width, height, bytes)
                .context("invalid DNG embedded RGB preview")?;
            return Ok(Some(DecodedThumbnailSource {
                image,
                source_width: width,
                source_height: height,
                scale: "embedded DNG RGB preview",
            }));
        }
        if !samsung
            || scalar(preview, 258) != Some(12)
            || scalar(preview, 262) != Some(6)
            || width % 2 != 0
            || height % 2 != 0
        {
            continue;
        }
        let actual_bytes = pixels * 3 / 2;
        // Require the entire observed preview at EOF. Do not salvage a
        // genuinely truncated preview or reinterpret a different layout.
        if u64::from(claimed) != pixels * 3 || u64::from(offset) + actual_bytes != file_size {
            continue;
        }
        file.seek(SeekFrom::Start(u64::from(offset)))?;
        let mut bytes = vec![0; actual_bytes as usize];
        file.read_exact(&mut bytes)?;
        let mut image = image::RgbImage::new(width, height);
        for (x, y, pixel) in image.enumerate_pixels_mut() {
            let luma = f32::from(bytes[y as usize * width as usize + x as usize]);
            let chroma = pixels as usize + (y as usize / 2) * width as usize + (x as usize / 2) * 2;
            let cr = f32::from(bytes[chroma]) - 128.0;
            let cb = f32::from(bytes[chroma + 1]) - 128.0;
            let channel = |value: f32| value.round().clamp(0.0, 255.0) as u8;
            *pixel = image::Rgb([
                channel(luma + 1.402 * cr),
                channel(luma - 0.344136 * cb - 0.714136 * cr),
                channel(luma + 1.772 * cb),
            ]);
        }
        return Ok(Some(DecodedThumbnailSource {
            image,
            source_width: width,
            source_height: height,
            scale: "embedded Samsung NV21 preview",
        }));
    }
    Ok(None)
}

#[cfg(test)]
mod samsung_dng_preview_tests {
    use super::*;

    struct Fixture(PathBuf);

    impl Fixture {
        fn new(make: &[u8], claimed_bytes: u32, truncated: bool) -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let path = std::env::temp_dir().join(format!(
                "pic-samsung-preview-{}-{}.dng",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            let preview_offset = 50 + make.len() as u32;
            let pixels_offset = preview_offset + 2 + 11 * 12 + 4;
            let mut bytes = b"II\x2a\0\x08\0\0\0".to_vec();
            let entry = |bytes: &mut Vec<u8>, tag: u16, kind: u16, count: u32, value: u32| {
                bytes.extend(tag.to_le_bytes());
                bytes.extend(kind.to_le_bytes());
                bytes.extend(count.to_le_bytes());
                bytes.extend(value.to_le_bytes());
            };
            bytes.extend(3u16.to_le_bytes());
            entry(&mut bytes, 271, 2, make.len() as u32, 50);
            entry(&mut bytes, 330, 4, 1, preview_offset);
            entry(&mut bytes, 50706, 1, 4, 0x00000401);
            bytes.extend(0u32.to_le_bytes());
            bytes.extend(make);
            bytes.extend(11u16.to_le_bytes());
            for (tag, kind, value) in [
                (254, 4, 1),
                (256, 4, 4),
                (257, 4, 2),
                (258, 3, 12),
                (259, 3, 1),
                (262, 3, 6),
                (273, 4, pixels_offset),
                (277, 3, 3),
                (278, 4, 2),
                (279, 4, claimed_bytes),
                (284, 3, 1),
            ] {
                entry(&mut bytes, tag, kind, 1, value);
            }
            bytes.extend(0u32.to_le_bytes());
            bytes.extend([128; 8]);
            // NV21 stores Cr before Cb, shared by each 2x2 pixel block.
            bytes.extend([255, 128, 0, 128]);
            if truncated {
                bytes.pop();
            }
            fs::write(&path, bytes).unwrap();
            Self(path)
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.0);
        }
    }

    #[test]
    fn samsung_nv21_preview_uses_actual_bytes_and_correct_chroma_order() {
        let fixture = Fixture::new(b"samsung\0", 24, false);
        let decoded = dng_embedded_preview(&fixture.0).unwrap().unwrap();
        assert_eq!((decoded.source_width, decoded.source_height), (4, 2));
        assert_eq!(decoded.image.get_pixel(0, 0).0, [255, 37, 128]);
        assert_eq!(decoded.image.get_pixel(3, 1).0, [0, 219, 128]);
    }

    #[test]
    fn samsung_thumbnail_succeeds_without_any_sensor_pixels() {
        let fixture = Fixture::new(b"samsung\0", 24, false);
        let decoded = decode_raw_thumbnail(fixture.0.to_str().unwrap()).unwrap();
        assert_eq!(decoded.scale, "embedded Samsung NV21 preview");
        assert_eq!(decoded.image.get_pixel(0, 0).0, [255, 37, 128]);
    }

    #[test]
    #[ignore = "set PICASA_TEST_DNG_LOG to a thumbnail trace containing Samsung DNG cache writes"]
    fn decodes_requested_samsung_dng_log_previews() {
        let log = std::env::var("PICASA_TEST_DNG_LOG").unwrap();
        let mut paths = HashSet::new();
        for line in fs::read_to_string(log).unwrap().lines() {
            if let Some((_, entry)) = line.split_once("cache_write uri=") {
                if let Some((path, _)) = entry.split_once(" cache=") {
                    if is_dng(path) {
                        paths.insert(path.to_owned());
                    }
                }
            }
        }
        assert!(!paths.is_empty());
        let started = std::time::Instant::now();
        for path in &paths {
            let preview = dng_embedded_preview(Path::new(path))
                .unwrap_or_else(|error| panic!("{path}: {error:#}"))
                .unwrap_or_else(|| panic!("{path}: no Samsung embedded preview"));
            assert!(matches!(
                preview.scale,
                "embedded Samsung NV21 preview" | "embedded DNG RGB preview"
            ));
            assert!(preview.image.width() > 0 && preview.image.height() > 0);
        }
        eprintln!(
            "Samsung DNG embedded previews={} total_elapsed_ms={}",
            paths.len(),
            started.elapsed().as_millis()
        );
    }

    #[test]
    fn conformant_rgb_preview_needs_no_sensor_pixels() {
        let fixture = Fixture::new(b"another\0", 24, false);
        let mut bytes = fs::read(&fixture.0).unwrap();
        let start = 50 + b"another\0".len() + 2;
        // Replace the reduced preview's pixel format with chunky 8-bit RGB.
        bytes[start + 3 * 12 + 8..start + 3 * 12 + 10].copy_from_slice(&8u16.to_le_bytes());
        bytes[start + 5 * 12 + 8..start + 5 * 12 + 10].copy_from_slice(&2u16.to_le_bytes());
        bytes.truncate(bytes.len() - 12);
        bytes.extend([255, 0, 0, 0, 0, 255].repeat(4));
        fs::write(&fixture.0, bytes).unwrap();
        let decoded = dng_embedded_preview(&fixture.0).unwrap().unwrap();
        assert_eq!(decoded.scale, "embedded DNG RGB preview");
        assert_eq!(decoded.image.get_pixel(0, 0).0, [255, 0, 0]);
        assert_eq!(decoded.image.get_pixel(1, 0).0, [0, 0, 255]);
    }

    #[test]
    fn samsung_workaround_rejects_other_makers_and_incomplete_pixels() {
        for (make, count, truncated) in [
            (&b"another\0"[..], 24, false),
            (&b"samsung\0"[..], 12, false),
            (&b"samsung\0"[..], 24, true),
        ] {
            let fixture = Fixture::new(make, count, truncated);
            assert!(dng_embedded_preview(&fixture.0).unwrap().is_none());
        }
    }
}
