// Samsung stores some reduced DNG previews as an 8-bit Y plane followed
// by interleaved Cr/Cb (NV21). The IFD describes 12 bits per pixel and claims
// three bytes per pixel even though the complete 4:2:0 preview occupies 1.5.
// Read bounded RGB strips directly too, without preloading the RAW sensor data.
fn dng_embedded_preview(path: &Path) -> Result<Option<DecodedThumbnailSource>> {
    let mut file = BufReader::new(fs::File::open(path)?);
    let file_size = file.get_ref().metadata()?.len();
    dng_embedded_preview_from_reader(&mut file, file_size)
}

fn dng_embedded_preview_from_bytes(bytes: &[u8]) -> Result<Option<DecodedThumbnailSource>> {
    let mut reader = Cursor::new(bytes);
    dng_embedded_preview_from_reader(&mut reader, bytes.len() as u64)
}

fn dng_embedded_preview_from_reader<R: Read + Seek>(
    file: &mut R,
    file_size: u64,
) -> Result<Option<DecodedThumbnailSource>> {
    dng_embedded_preview_from_reader_with_max(file, file_size, u32::MAX)
}

fn dng_embedded_preview_from_reader_with_max<R: Read + Seek>(
    file: &mut R,
    file_size: u64,
    max_edge: u32,
) -> Result<Option<DecodedThumbnailSource>> {
    use rawler::formats::tiff::{GenericTiffReader, reader::TiffReader};

    // This reads TIFF metadata and seeks over sensor strips, without loading
    // the RAW pixel buffer or asking a decoder to develop it.
    let tiff = GenericTiffReader::new(file, 0, 0, Some(16), &[])?;
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
    let mut previews = tiff.find_ifds_with_filter(|ifd| scalar(ifd, 254) == Some(1));
    // IFD order is not a quality ranking: some DNGs list a thumbnail first.
    previews.sort_by_key(|ifd| std::cmp::Reverse(
        u64::from(scalar(ifd, 256).unwrap_or(0))
            * u64::from(scalar(ifd, 257).unwrap_or(0)),
    ));
    for preview in previews {
        // A reduced DNG IFD may hold a single lossy JPEG strip, even
        // when its dimensions match the sensor (Samsung Expert RAW).
        if scalar(preview, 259) == Some(7)
            && scalar(preview, 277) == Some(3)
            && matches!(scalar(preview, 262), Some(2 | 6))
        {
            if let (Some(offset), Some(length), Some(height)) = (
                scalar(preview, 273), scalar(preview, 279), scalar(preview, 257),
            ) {
                if length > 0 && length <= 32 * 1024 * 1024
                    && scalar(preview, 278) == Some(height)
                    && u64::from(offset) + u64::from(length) <= file_size
                {
                    file.seek(SeekFrom::Start(u64::from(offset)))?;
                    let mut bytes = vec![0; length as usize];
                    file.read_exact(&mut bytes)?;
                    if let Ok(mut decoded) = decode_jpeg_turbo_with_max(&bytes, max_edge) {
                        decoded.scale = "embedded DNG JPEG preview";
                        return Ok(Some(decoded));
                    }
                }
            }
        }
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

    #[test]
    #[ignore = "requires PICASA_TEST_REMOTE_DNG on a live share"]
    fn live_remote_dng_grid_preview() {
        let reference = std::env::var("PICASA_TEST_REMOTE_DNG").unwrap();
        let size = crate::network_shares::stat(&reference).unwrap().size;
        let started = std::time::Instant::now();
        let (decoded, orientation) = decode_remote_raw_thumbnail(&reference, 256).unwrap();
        eprintln!("DNG grid: file_bytes={size} elapsed_ms={} dimensions={:?} orientation={orientation}",
            started.elapsed().as_millis(), decoded.image.dimensions());
        assert!(decoded.image.width() > 0);
        let mut reader = RemoteNefReader::open(&reference).unwrap();
        let started = std::time::Instant::now();
        reader.seek(SeekFrom::Start(0)).unwrap();
        let preview = dng_embedded_preview_from_reader(&mut reader, size).unwrap().unwrap();
        eprintln!("DNG range preview: elapsed_ms={} dimensions={:?}",
            started.elapsed().as_millis(), preview.image.dimensions());
    }

    #[test]
    #[ignore = "requires PICASA_TEST_REMOTE_DNG on a live share"]
    fn live_remote_dng_lightbox_quality() {
        let reference = std::env::var("PICASA_TEST_REMOTE_DNG").unwrap();
        for (width, height) in [(1920, 1080), (u32::MAX, u32::MAX)] {
            let image = decode_for_viewer(&reference, width, height).unwrap();
            eprintln!("Lightbox requested={width}x{height} decoded={:?}", image.dimensions());
            assert!(image.width().max(image.height()) >= 1080 && image.width().min(image.height()) >= 768,
                "lightbox returned thumbnail-sized pixels");
            if width == u32::MAX {
                assert!(image.width().max(image.height()) >= 3000, "native view did not load full preview");
            }
        }
    }

    #[test]
    fn dng_jpeg_strip_preview_skips_sensor_pixels() {
        let fixture = Fixture::new(b"samsung\0", 24, false);
        let mut bytes = fs::read(&fixture.0).unwrap();
        let preview_ifd = 58;
        let entry_value = |index: usize| preview_ifd + 2 + index * 12 + 8;
        let mut jpeg = Vec::new();
        image::codecs::jpeg::JpegEncoder::new(&mut jpeg)
            .encode(&[120; 24], 4, 2, image::ExtendedColorType::Rgb8).unwrap();
        let pixels_offset = bytes.len() - 12;
        bytes.truncate(pixels_offset);
        bytes[entry_value(3)..entry_value(3) + 4].copy_from_slice(&8u32.to_le_bytes());
        bytes[entry_value(4)..entry_value(4) + 4].copy_from_slice(&7u32.to_le_bytes());
        bytes[entry_value(9)..entry_value(9) + 4].copy_from_slice(&(jpeg.len() as u32).to_le_bytes());
        bytes.extend(jpeg);
        let preview = dng_embedded_preview_from_bytes(&bytes).unwrap().unwrap();
        assert_eq!(preview.image.dimensions(), (4, 2));
        assert!(preview.image.get_pixel(0, 0)[0].abs_diff(120) <= 2);
    }

    #[test]
    fn dng_lightbox_prefers_large_preview_over_first_thumbnail() {
        let fixture = Fixture::new(b"samsung\0", 24, false);
        let mut bytes = fs::read(&fixture.0).unwrap();
        for (index, value) in [(3, 8u32), (5, 2)] {
            let start = 58 + 2 + index * 12 + 8;
            bytes[start..start + 4].copy_from_slice(&value.to_le_bytes());
        }
        bytes.extend([128u8; 12]);
        // A second reduced IFD contains the display preview; the first is
        // deliberately a tiny thumbnail, as seen in multi-preview DNGs.
        let second = bytes.len() as u32;
        let mut ifd = bytes[58..196].to_vec();
        for (index, value) in [(1, 8u32), (2, 4), (3, 8), (5, 2),
            (6, second + 138), (8, 4), (9, 96)] {
            let start = 2 + index * 12 + 8;
            ifd[start..start + 4].copy_from_slice(&value.to_le_bytes());
        }
        bytes.extend(ifd);
        bytes.extend([180u8; 96]);
        let offsets = bytes.len() as u32;
        bytes.extend(58u32.to_le_bytes());
        bytes.extend(second.to_le_bytes());
        bytes[26..30].copy_from_slice(&2u32.to_le_bytes());
        bytes[30..34].copy_from_slice(&offsets.to_le_bytes());
        let decoded = decode_remote_dng_for_viewer(
            "nfs://example.invalid/share/multiple.dng", &bytes).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (8, 4));
    }

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
    fn remote_raw_failed_cache_keys_are_retried() {
        for scheme in ["nfs", "smb"] {
            for extension in ["dng", "cr2", "cr3", "arw", "raf", "orf", "rw2", "pef", "srw"] {
                let path = format!("{scheme}://example.invalid/share/photo.{extension}");
                let mut old = blake3::Hasher::new();
                old.update(path.as_bytes());
                old.update(if extension == "dng" {
                    DNG_THUMBNAIL_CACHE_VERSION
                } else {
                    RAW_THUMBNAIL_CACHE_VERSION
                });
                old.update(b"\0");
                old.update(b"0");
                old.update(b"\0");
                old.update(b"0");
                assert_ne!(cache_file_name(&path, None, None),
                    format!("{}.jpg", old.finalize().to_hex()), "{path}");
            }
        }
    }

    #[test]
    fn remote_dng_grid_decodes_and_scales_preview() {
        let fixture = Fixture::new(b"samsung\0", 24, false);
        let bytes = fs::read(&fixture.0).unwrap();
        for scheme in ["nfs", "smb"] {
            let (decoded, orientation) = decode_remote_raw_thumbnail_bytes(
                &format!("{scheme}://example.invalid/share/photo.dng"), &bytes, 2,
            ).unwrap();
            assert_eq!((decoded.source_width, decoded.source_height), (4, 2));
            assert_eq!(decoded.image.dimensions(), (2, 1));
            assert_eq!(orientation, 1);
        }
    }

    #[test]
    fn remote_dng_grid_preserves_orientation_for_cache_writer() {
        let fixture = Fixture::new(b"samsung\0", 24, false);
        let mut bytes = fs::read(&fixture.0).unwrap();
        let mut root = 4u16.to_le_bytes().to_vec();
        root.extend_from_slice(&bytes[10..22]); // Make
        root.extend_from_slice(&274u16.to_le_bytes()); // Orientation
        root.extend_from_slice(&3u16.to_le_bytes()); // SHORT
        root.extend_from_slice(&1u32.to_le_bytes());
        root.extend_from_slice(&6u32.to_le_bytes()); // 90 degrees clockwise
        root.extend_from_slice(&bytes[22..50]); // SubIFD, DNGVersion, next IFD
        // Keep Samsung's short NV21 strip at EOF, as in the real layout.
        let pixels = bytes.split_off(bytes.len() - 12);
        let root_offset = bytes.len() as u32;
        bytes[4..8].copy_from_slice(&root_offset.to_le_bytes());
        bytes.extend(root);
        let pixels_offset = bytes.len() as u32;
        let strip_offset_value = 58 + 2 + 6 * 12 + 8;
        bytes[strip_offset_value..strip_offset_value + 4]
            .copy_from_slice(&pixels_offset.to_le_bytes());
        bytes.extend(pixels);
        let (decoded, orientation) = decode_remote_raw_thumbnail_bytes(
            "nfs://example.invalid/share/portrait.dng", &bytes, 4,
        ).unwrap();
        assert_eq!(orientation, 6);
        let oriented = apply_orientation(DynamicImage::ImageRgb8(decoded.image), orientation);
        assert_eq!((oriented.width(), oriented.height()), (2, 4));
        assert_eq!(oriented.to_rgb8().get_pixel(0, 0).0, [255, 37, 128]);
    }

    #[test]
    fn remote_raw_grid_rejects_corrupt_data() {
        for extension in ["dng", "cr2", "cr3", "arw", "raf", "orf", "rw2", "pef", "srw", "raw"] {
            assert!(decode_remote_raw_thumbnail_bytes(
                &format!("nfs://example.invalid/share/photo.{extension}"),
                b"not a raw file", 128,
            ).is_err());
        }
    }

    #[test]
    fn remote_dng_viewer_decodes_embedded_preview_from_memory() {
        let fixture = Fixture::new(b"samsung\0", 24, false);
        let bytes = fs::read(&fixture.0).unwrap();

        let decoded = decode_remote_dng_for_viewer(
            "nfs://example.invalid/share/photo.dng",
            &bytes,
        )
        .unwrap();

        assert_eq!((decoded.width(), decoded.height()), (4, 2));
        assert_eq!(decoded.to_rgb8().get_pixel(0, 0).0, [255, 37, 128]);
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
