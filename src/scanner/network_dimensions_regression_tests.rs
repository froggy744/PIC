use super::*;

// EXIF contains only Orientation: no PixelX/YDimension. The SOF is
// followed by no scan data, so dimension extraction cannot decode pixels.
fn jpeg_header(width: u16, height: u16, orientation: u16, sof: u8) -> Vec<u8> {
    let mut bytes = vec![
        0xff, 0xd8, 0xff, 0xe1, 0, 34, b'E', b'x', b'i', b'f', 0, 0, b'I', b'I', 42, 0, 8, 0, 0, 0,
        1, 0, 0x12, 1, 3, 0, 1, 0, 0, 0,
    ];
    bytes.extend_from_slice(&orientation.to_le_bytes());
    bytes.extend_from_slice(&[0, 0, 0, 0, 0, 0]);
    bytes.extend_from_slice(&[0xff, sof, 0, 17, 8]);
    bytes.extend_from_slice(&height.to_be_bytes());
    bytes.extend_from_slice(&width.to_be_bytes());
    bytes.extend_from_slice(&[3, 1, 0x22, 0, 2, 0x11, 1, 3, 0x11, 1]);
    bytes
}

#[test]
fn network_jpeg_sof_dimensions_reach_photo_wall_without_exif_pixel_dimensions() {
    for (width, height, orientation, sof, display, ratio) in [
        (6000, 4000, 1, 0xc0, (Some(6000), Some(4000)), 1.5),
        (4000, 6000, 1, 0xc2, (Some(4000), Some(6000)), 2.0 / 3.0),
        (6000, 4000, 6, 0xc0, (Some(4000), Some(6000)), 2.0 / 3.0),
        (4000, 6000, 8, 0xc2, (Some(6000), Some(4000)), 1.5),
    ] {
        let bytes = jpeg_header(width, height, orientation, sof);
        let exif = exif_from_bytes(&bytes).unwrap();
        assert!(exif_u32(&exif, Tag::PixelXDimension).is_none());
        assert!(exif_u32(&exif, Tag::PixelYDimension).is_none());
        let (w, h, exif) = network_header_metadata(&bytes);
        assert_eq!((w, h), (Some(u32::from(width)), Some(u32::from(height))));
        let dimensions =
            display_oriented_dimensions(w, h, exif.as_ref().and_then(exif_orientation_value));
        assert_eq!(dimensions, display);
        let photo = glib::Object::builder::<crate::photo_object::PhotoObject>()
            .property("width", i64::from(dimensions.0.unwrap()))
            .property("height", i64::from(dimensions.1.unwrap()))
            .build();
        assert!((photo.photo_wall_aspect_ratio() - ratio).abs() < 1e-9);
        photo.set_rotation(90);
        assert!((photo.photo_wall_aspect_ratio() - 1.0 / ratio).abs() < 1e-9);
    }
}

// Classic TIFF root thumbnail + SubIFD sensor, like NEF/DNG. No
// PixelX/YDimension tags; only metadata, never actual sensor strips.
fn raw_tiff(little: bool, sensor_offset: usize) -> Vec<u8> {
    let u16b = |v: u16| {
        if little {
            v.to_le_bytes()
        } else {
            v.to_be_bytes()
        }
    };
    let u32b = |v: u32| {
        if little {
            v.to_le_bytes()
        } else {
            v.to_be_bytes()
        }
    };
    let mut bytes = if little {
        b"II".to_vec()
    } else {
        b"MM".to_vec()
    };
    bytes.extend(u16b(42));
    bytes.extend(u32b(8));
    let ifd = |entries: &[(u16, u32)]| {
        let mut data = u16b(entries.len() as u16).to_vec();
        for &(tag, value) in entries {
            data.extend(u16b(tag));
            data.extend(u16b(4));
            data.extend(u32b(1));
            data.extend(u32b(value));
        }
        data.extend(u32b(0));
        data
    };
    bytes.extend(ifd(&[
        (254, 1),
        (256, 160),
        (257, 120),
        (274, 6),
        (330, sensor_offset as u32),
    ]));
    bytes.resize(sensor_offset, 0);
    bytes.extend(ifd(&[
        (254, 0),
        (256, 6000),
        (257, 4000),
        (262, 32803),
        (273, 20_000_000),
        (279, 48_000_000),
    ]));
    bytes
}

#[test]
fn network_nef_and_dng_use_sensor_ifd_without_exif_pixel_dimensions() {
    for extension in ["nef", "dng"] {
        for little in [true, false] {
            let bytes = raw_tiff(little, 128);
            let path = format!("nfs://server/share/photo.{extension}");
            let (w, h, exif, orientation) = network_metadata(&path, &bytes, |_, _| {
                panic!("bounded metadata needs no extra network read")
            })
            .unwrap();
            assert_eq!((w, h), (Some(6000), Some(4000)));
            assert_eq!(
                display_oriented_dimensions(
                    w,
                    h,
                    exif.as_ref()
                        .and_then(exif_orientation_value)
                        .or(orientation)
                ),
                (Some(4000), Some(6000))
            );
        }
    }
}

#[test]
fn network_raw_sensor_ifd_beyond_header_uses_only_targeted_metadata_ranges() {
    let offset = 700_000;
    let bytes = raw_tiff(true, offset);
    let header = &bytes[..512 * 1024];
    let mut reads = Vec::new();
    let (w, h, _, _) = network_metadata("smb://server/share/photo.dng", header, |start, length| {
        reads.push((start, length));
        assert!(
            start >= offset as u64 && start + length as u64 <= bytes.len() as u64,
            "must read IFD metadata only"
        );
        Ok(bytes[start as usize..start as usize + length].to_vec())
    })
    .unwrap();
    assert_eq!((w, h), (Some(6000), Some(4000)));
    assert!(
        !reads.is_empty(),
        "missing geometry must invoke targeted fallback"
    );
    assert!(reads.iter().map(|(_, length)| length).sum::<usize>() < 1024);
}

#[test]
fn network_raw_prefers_exif_dimensions_without_extra_reads() {
    let mut bytes = raw_tiff(true, 128);
    // Root SubIFD pointer becomes an EXIF IFD pointer.
    bytes[58..60].copy_from_slice(&34665u16.to_le_bytes());
    bytes[142..144].copy_from_slice(&40962u16.to_le_bytes());
    bytes[154..156].copy_from_slice(&40963u16.to_le_bytes());
    let (w, h, exif, _) = network_metadata("smb://server/share/photo.nef", &bytes, |_, _| {
        panic!("EXIF geometry must not trigger fallback reads")
    })
    .unwrap();
    assert_eq!((w, h), (Some(6000), Some(4000)));
    let exif = exif.unwrap();
    assert_eq!(exif_u32(&exif, Tag::PixelXDimension), Some(6000));
    assert_eq!(exif_u32(&exif, Tag::PixelYDimension), Some(4000));
}

#[test]
fn network_dng_geometry_preserves_active_area_and_default_crop() {
    let mut bytes = raw_tiff(true, 128);
    bytes[128..130].copy_from_slice(&9u16.to_le_bytes());
    bytes.truncate(202); // six existing entries, no next-IFD pointer
                         // Three geometry arrays after the expanded directory.
    for (tag, count, offset) in [(50829u16, 4u32, 242u32), (50719, 2, 258), (50720, 2, 266)] {
        bytes.extend(tag.to_le_bytes());
        bytes.extend(4u16.to_le_bytes());
        bytes.extend(count.to_le_bytes());
        bytes.extend(offset.to_le_bytes());
    }
    bytes.extend(0u32.to_le_bytes());
    for value in [10u32, 20, 3990, 5980, 30, 40, 5900, 3800] {
        bytes.extend(value.to_le_bytes());
    }
    let (w, h, exif, orientation) =
        network_metadata("nfs://server/share/photo.dng", &bytes, |_, _| {
            panic!("crop metadata is in header")
        })
        .unwrap();
    assert_eq!((w, h), (Some(5900), Some(3800)));
    assert_eq!(
        display_oriented_dimensions(
            w,
            h,
            exif.as_ref()
                .and_then(exif_orientation_value)
                .or(orientation)
        ),
        (Some(3800), Some(5900))
    );
    // Rational crop coordinates are converted to pixel axes the same way
    // as the local DNG geometry reader (fractional part discarded).
    bytes[216..218].copy_from_slice(&5u16.to_le_bytes());
    bytes[228..230].copy_from_slice(&5u16.to_le_bytes());
    bytes[234..238].copy_from_slice(&274u32.to_le_bytes());
    bytes.truncate(258);
    for value in [61u32, 2, 81, 2, 11801, 2, 7601, 2] {
        bytes.extend(value.to_le_bytes());
    }
    let (w, h, _, _) = network_metadata("nfs://server/share/photo.dng", &bytes, |_, _| {
        panic!("rational geometry is in header")
    })
    .unwrap();
    assert_eq!((w, h), (Some(5900), Some(3800)));
}

#[test]
fn network_raw_does_not_accept_root_thumbnail_before_unread_sensor_ifd() {
    let bytes = raw_tiff(true, 700_000);
    let mut header = bytes[..512 * 1024].to_vec();
    // Some cameras omit the reduced-resolution marker on their root RGB
    // thumbnail. The remote sensor SubIFD must still win.
    header[18..22].copy_from_slice(&0u32.to_le_bytes());
    let (w, h, _, _) =
        network_metadata("nfs://server/share/photo.nef", &header, |start, length| {
            Ok(bytes[start as usize..start as usize + length].to_vec())
        })
        .unwrap();
    assert_eq!((w, h), (Some(6000), Some(4000)));
}

#[test]
fn malformed_network_raw_metadata_is_bounded_and_never_materialized() {
    for bytes in [b"not TIFF".to_vec(), b"II*\0\0\0\0\0".to_vec()] {
        let (w, h, _, _) = network_metadata("smb://server/share/photo.dng", &bytes, |_, _| {
            panic!("unsupported metadata cannot trigger original reads")
        })
        .unwrap();
        assert_eq!((w, h), (None, None));
    }
    let mut header = raw_tiff(true, 128);
    header.truncate(128);
    let mut reads = 0;
    let (w, h, _, _) =
        network_metadata("smb://server/share/photo.nef", &header, |start, length| {
            reads += 1;
            assert_eq!((start, length), (128, 2));
            // Reject excessive IFD count before requesting any directory.
            Ok(65535u16.to_le_bytes().to_vec())
        })
        .unwrap();
    assert_eq!(reads, 1);
    assert_eq!((w, h), (None, None));
}

#[test]
fn network_raw_targeted_root_orientation_is_applied_once() {
    let small = raw_tiff(true, 128);
    let root = 700_000usize;
    let mut bytes = small[..8].to_vec();
    bytes[4..8].copy_from_slice(&(root as u32).to_le_bytes());
    bytes.resize(root, 0);
    bytes.extend_from_slice(&small[8..]);
    bytes[root + 58..root + 62].copy_from_slice(&((root + 120) as u32).to_le_bytes());
    let (w, h, exif, orientation) = network_metadata(
        "nfs://server/share/photo.nef",
        &bytes[..512 * 1024],
        |start, length| {
            assert!(start >= root as u64);
            Ok(bytes[start as usize..start as usize + length].to_vec())
        },
    )
    .unwrap();
    assert!(exif.is_none(), "root EXIF is outside the initial header");
    assert_eq!(orientation, Some(6));
    let display = display_oriented_dimensions(w, h, orientation);
    assert_eq!(display, (Some(4000), Some(6000)));
    let photo = glib::Object::builder::<crate::photo_object::PhotoObject>()
        .property("width", i64::from(display.0.unwrap()))
        .property("height", i64::from(display.1.unwrap()))
        .build();
    assert!((photo.photo_wall_aspect_ratio() - 2.0 / 3.0).abs() < 1e-9);
}

#[test]
fn network_raw_metadata_traversal_limits_requests_and_total_bytes() {
    for count in [1u16, 1024] {
        let mut header = b"II*\0".to_vec();
        header.extend(700_000u32.to_le_bytes());
        let (mut requests, mut total) = (0, 0);
        let (w, h, _, _) =
            network_metadata("smb://server/share/photo.dng", &header, |start, length| {
                requests += 1;
                total += length;
                assert!(requests <= 64 && total <= 128 * 1024);
                if length == 2 {
                    Ok(count.to_le_bytes().to_vec())
                } else {
                    let mut directory = vec![0; length];
                    // A chain of directories with no geometry and no pixels.
                    directory[length - 4..]
                        .copy_from_slice(&((start as u32 - 2) + 20_000).to_le_bytes());
                    Ok(directory)
                }
            })
            .unwrap();
        assert_eq!((w, h), (None, None));
        assert!(requests > 0);
    }
}

#[test]
fn network_raw_incomplete_sensor_ifds_do_not_hide_larger_remote_geometry() {
    let small = raw_tiff(true, 128);
    let second = 700_000usize;
    let mut bytes = small.clone();
    bytes[62..66].copy_from_slice(&2u32.to_le_bytes());
    bytes[66..70].copy_from_slice(&96u32.to_le_bytes());
    bytes[96..100].copy_from_slice(&128u32.to_le_bytes());
    bytes[100..104].copy_from_slice(&(second as u32).to_le_bytes());
    bytes.resize(second, 0);
    bytes.extend_from_slice(&small[128..]);
    bytes[second + 22..second + 26].copy_from_slice(&8000u32.to_le_bytes());
    bytes[second + 34..second + 38].copy_from_slice(&6000u32.to_le_bytes());
    let (w, h, _, _) = network_metadata(
        "nfs://server/share/photo.dng",
        &bytes[..512 * 1024],
        |start, length| {
            assert!(start >= second as u64);
            Ok(bytes[start as usize..start as usize + length].to_vec())
        },
    )
    .unwrap();
    assert_eq!((w, h), (Some(8000), Some(6000)));
}

#[test]
fn bounded_jpeg_header_handles_missing_exif_and_truncation() {
    let header = jpeg_header(6000, 4000, 1, 0xc0);
    // Remove APP1 entirely: photos without any EXIF need the same fallback.
    let mut no_exif = vec![0xff, 0xd8];
    no_exif.extend_from_slice(&header[38..]);
    let (w, h, exif) = network_header_metadata(&no_exif);
    assert_eq!((w, h), (Some(6000), Some(4000)));
    assert!(exif.is_none());
    for end in 0..header.len() {
        let (w, h, _) = network_header_metadata(&header[..end]);
        assert_eq!((w, h), (None, None), "truncated at {end}");
    }
    for bytes in [
        vec![0xff, 0xd8, 0xff, 0xe0, 0, 1],       // invalid segment length
        vec![0xff, 0xd8, 0xff, 0xe0, 0xff, 0xff], // unavailable segment
        jpeg_header(0, 4000, 1, 0xc0),
        jpeg_header(6000, 0, 1, 0xc0),
        b"II*\0not a JPEG".to_vec(),
    ] {
        let (w, h, _) = network_header_metadata(&bytes);
        assert_eq!((w, h), (None, None));
    }
    // A fake SOF after SOS must not be interpreted as image geometry.
    let mut after_scan = vec![0xff, 0xd8, 0xff, 0xda];
    after_scan.extend_from_slice(&header[38..]);
    assert_eq!(jpeg_sof_dimensions(&after_scan), None);
    // SOF beyond the network limit does not cause an additional read.
    let mut beyond_limit = vec![0xff, 0xd8];
    for _ in 0..8 {
        beyond_limit.extend_from_slice(&[0xff, 0xe0, 0xff, 0xff]);
        beyond_limit.resize(beyond_limit.len() + 65533, 0);
    }
    beyond_limit.extend_from_slice(&header[38..]);
    assert_eq!(jpeg_sof_dimensions(&beyond_limit), Some((6000, 4000)));
    assert_eq!(jpeg_sof_dimensions(&beyond_limit[..512 * 1024]), None);
}

#[test]
fn matching_fingerprint_repairs_missing_dimensions_then_skips_healthy_record() {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "pic-dimension-repair-{}-{unique}",
        std::process::id()
    ));
    let photos = root.join("photos");
    fs::create_dir_all(&photos).unwrap();
    let database = root.join("library.db");
    let connection = db::open(&database).unwrap();
    for (index, dimensions) in [
        (None, None),
        (Some(0), Some(40)),
        (Some(60), Some(-1)),
        (Some(60), Some(40)),
    ]
    .into_iter()
    .enumerate()
    {
        let path = photos.join(format!("{index}.jpg"));
        image::RgbImage::new(60, 40).save(&path).unwrap();
        if index == 0 {
            let original = fs::read(&path).unwrap();
            let mut oriented = original[..2].to_vec();
            oriented.extend_from_slice(&jpeg_header(60, 40, 6, 0xc0)[2..38]);
            oriented.extend_from_slice(&original[2..]);
            fs::write(&path, oriented).unwrap();
        }
        let file = gio::File::for_path(&path);
        let attributes = file
            .query_info(
                "time::modified,standard::size",
                gio::FileQueryInfoFlags::NONE,
                gio::Cancellable::NONE,
            )
            .unwrap();
        let mut metadata = read_metadata(path.to_str().unwrap(), &attributes).unwrap();
        metadata.width = dimensions.0;
        metadata.height = dimensions.1;
        // A healthy row must retain this sentinel: scanning it would replace it.
        metadata.camera = Some("skip sentinel".into());
        db::upsert_photo(&connection, &path, None, &metadata).unwrap();
    }
    assert_eq!(
        scan_with_control(
            photos.to_str().unwrap(),
            &database,
            None,
            &ScanControl::default()
        )
        .unwrap(),
        3
    );
    let records = db::photos(&connection, None, false, None).unwrap();
    assert_eq!(records.len(), 4);
    for record in records {
        let (display, ratio) = if record.path.ends_with("0.jpg") {
            ((Some(40), Some(60)), 2.0 / 3.0)
        } else {
            ((Some(60), Some(40)), 1.5)
        };
        assert_eq!((record.width, record.height), display);
        if record.path.ends_with("3.jpg") {
            assert_eq!(record.camera.as_deref(), Some("skip sentinel"));
        } else {
            assert_ne!(record.camera.as_deref(), Some("skip sentinel"));
        }
        let photo = crate::photo_object::PhotoObject::from_photo(&record);
        assert!((photo.photo_wall_aspect_ratio() - ratio).abs() < 1e-9);
    }
    assert_eq!(
        scan_with_control(
            photos.to_str().unwrap(),
            &database,
            None,
            &ScanControl::default()
        )
        .unwrap(),
        0
    );
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}
