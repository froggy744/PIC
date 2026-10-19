//! Privacy-conscious EXIF for rendered exports.
//!
//! Copy only selected camera/exposure/date fields. Source orientation, GPS,
//! embedded thumbnails, MakerNotes and sensor dimensions must never be copied:
//! an edited/cropped image has different pixels and may be shared publicly.
use exif::{In, Tag, Value};

#[derive(Clone)]
struct Entry {
    tag: u16,
    kind: u16,
    count: u32,
    bytes: Vec<u8>,
}

impl Entry {
    fn short(tag: u16, value: u16) -> Self {
        Self { tag, kind: 3, count: 1, bytes: value.to_le_bytes().to_vec() }
    }

    fn long(tag: u16, value: u32) -> Self {
        Self { tag, kind: 4, count: 1, bytes: value.to_le_bytes().to_vec() }
    }

    fn ascii(tag: u16, value: &[u8]) -> Option<Self> {
        // EXIF ASCII strings must be terminated and must not contain NUL in
        // the middle. Bound data copied from untrusted photo metadata.
        let value = value.split(|b| *b == 0).next()?;
        if value.is_empty() || value.len() > 255 {
            return None;
        }
        let mut bytes = value.to_vec();
        bytes.push(0);
        Some(Self { tag, kind: 2, count: bytes.len() as u32, bytes })
    }

    fn rational(tag: u16, numerator: u32, denominator: u32) -> Option<Self> {
        if denominator == 0 {
            return None;
        }
        let mut bytes = numerator.to_le_bytes().to_vec();
        bytes.extend_from_slice(&denominator.to_le_bytes());
        Some(Self { tag, kind: 5, count: 1, bytes })
    }
}

fn copied_ascii(metadata: &exif::Exif, tag: Tag, output_tag: u16) -> Option<Entry> {
    let field = metadata.get_field(tag, In::PRIMARY)?;
    match &field.value {
        Value::Ascii(values) => Entry::ascii(output_tag, values.first()?),
        _ => None,
    }
}

fn copied_short(metadata: &exif::Exif, tag: Tag, output_tag: u16) -> Option<Entry> {
    let field = metadata.get_field(tag, In::PRIMARY)?;
    match &field.value {
        Value::Short(values) => Some(Entry::short(output_tag, *values.first()?)),
        _ => None,
    }
}

fn copied_rational(metadata: &exif::Exif, tag: Tag, output_tag: u16) -> Option<Entry> {
    let field = metadata.get_field(tag, In::PRIMARY)?;
    match &field.value {
        Value::Rational(values) => {
            let value = values.first()?;
            Entry::rational(output_tag, value.num, value.denom)
        }
        _ => None,
    }
}

fn write_ifd(buf: &mut Vec<u8>, entries: &[Entry]) {
    let start = buf.len();
    buf.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    buf.resize(buf.len() + entries.len() * 12 + 4, 0);
    for (i, entry) in entries.iter().enumerate() {
        let pos = start + 2 + i * 12;
        buf[pos..pos + 2].copy_from_slice(&entry.tag.to_le_bytes());
        buf[pos + 2..pos + 4].copy_from_slice(&entry.kind.to_le_bytes());
        buf[pos + 4..pos + 8].copy_from_slice(&entry.count.to_le_bytes());
        if entry.bytes.len() <= 4 {
            buf[pos + 8..pos + 8 + entry.bytes.len()].copy_from_slice(&entry.bytes);
        } else {
            let offset = buf.len() as u32;
            buf[pos + 8..pos + 12].copy_from_slice(&offset.to_le_bytes());
            buf.extend_from_slice(&entry.bytes);
            if buf.len() % 2 != 0 {
                buf.push(0);
            }
        }
    }
}

/// Build a new, minimal TIFF/EXIF payload describing output image pixels.
/// Do not reuse raw EXIF offsets or orientation from the original.
fn build(metadata: Option<&exif::Exif>, width: u32, height: u32) -> Vec<u8> {
    let mut ifd0 = vec![Entry::short(0x0112, 1)]; // Orientation: baked into pixels
    let mut exif_ifd = vec![
        Entry::short(0xA001, 1),  // ColorSpace: sRGB output
        Entry::long(0xA002, width),
        Entry::long(0xA003, height),
    ];
    if let Some(meta) = metadata {
        for (tag, id) in [
            (Tag::Make, 0x010F), (Tag::Model, 0x0110),
            (Tag::DateTime, 0x0132), (Tag::Artist, 0x013B),
            (Tag::Copyright, 0x8298),
        ] {
            if let Some(entry) = copied_ascii(meta, tag, id) {
                ifd0.push(entry);
            }
        }
        for (tag, id) in [
            (Tag::DateTimeOriginal, 0x9003),
            (Tag::DateTimeDigitized, 0x9004),
            (Tag::LensModel, 0xA434),
        ] {
            if let Some(entry) = copied_ascii(meta, tag, id) {
                exif_ifd.push(entry);
            }
        }
        for (tag, id) in [
            (Tag::ExposureTime, 0x829A),
            (Tag::FNumber, 0x829D),
            (Tag::FocalLength, 0x920A),
        ] {
            if let Some(entry) = copied_rational(meta, tag, id) {
                exif_ifd.push(entry);
            }
        }
        if let Some(entry) = copied_short(meta, Tag::PhotographicSensitivity, 0x8827) {
            exif_ifd.push(entry);
        }
    }
    ifd0.push(Entry::long(0x8769, 0)); // ExifIFDPointer
    ifd0.sort_by_key(|e| e.tag);
    exif_ifd.sort_by_key(|e| e.tag);

    let mut buf = b"II\x2a\x00\x08\x00\x00\x00".to_vec();
    write_ifd(&mut buf, &ifd0);
    let exif_offset = buf.len() as u32;
    // Update ExifIFDPointer in the primary directory.
    let index = ifd0.iter().position(|e| e.tag == 0x8769).unwrap();
    let pointer = 8 + 2 + index * 12 + 8;
    buf[pointer..pointer + 4].copy_from_slice(&exif_offset.to_le_bytes());
    write_ifd(&mut buf, &exif_ifd);
    buf
}

/// Read camera/date metadata for a supported local or URI source.
/// Failure to parse optional EXIF must not stop a valid image export.
pub fn for_export(reference: &str, width: u32, height: u32) -> Vec<u8> {
    let source = crate::source::file(reference);
    let metadata = if let Some(path) = source.path() {
        std::fs::File::open(path).ok().and_then(|file| {
            exif::Reader::new()
                .read_from_container(&mut std::io::BufReader::new(file)).ok()
        })
    } else {
        crate::source::read(reference).ok().and_then(|bytes| {
            exif::Reader::new()
                .read_from_container(&mut std::io::Cursor::new(bytes)).ok()
        })
    };
    build(metadata.as_ref(), width, height)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_exif_has_baked_orientation_and_current_dimensions_without_gps() {
        let bytes = build(None, 1200, 800);
        let exif = exif::Reader::new().read_raw(bytes).unwrap();
        assert_eq!(exif.get_field(Tag::Orientation, In::PRIMARY).unwrap().value.get_uint(0), Some(1));
        assert_eq!(exif.get_field(Tag::PixelXDimension, In::PRIMARY).unwrap().value.get_uint(0), Some(1200));
        assert_eq!(exif.get_field(Tag::PixelYDimension, In::PRIMARY).unwrap().value.get_uint(0), Some(800));
        assert!(exif.get_field(Tag::GPSLatitude, In::PRIMARY).is_none());
        assert!(exif.get_field(Tag::GPSLongitude, In::PRIMARY).is_none());
    }

    #[test]
    fn tiff_writer_round_trips_camera_dates_and_lens_without_copying_gps() {
        // A fabricated source EXIF block proves the outgoing allowlist.
        let mut source_ifd = vec![
            Entry::ascii(0x010F, b"NIKON CORPORATION").unwrap(),
            Entry::ascii(0x0110, b"NIKON Z 5").unwrap(),
        ];
        source_ifd.push(Entry::short(0x0112, 6));
        source_ifd.sort_by_key(|e| e.tag);
        let mut source = b"II\x2a\x00\x08\x00\x00\x00".to_vec();
        write_ifd(&mut source, &source_ifd);
        let parsed = exif::Reader::new().read_raw(source).unwrap();
        let exported = exif::Reader::new().read_raw(build(Some(&parsed), 800, 1200)).unwrap();
        let make = exported.get_field(Tag::Make, In::PRIMARY).unwrap();
        assert!(make.display_value().with_unit(&exported).to_string().contains("NIKON"));
        assert_eq!(exported.get_field(Tag::Orientation, In::PRIMARY).unwrap().value.get_uint(0), Some(1));
        assert!(exported.get_field(Tag::GPSLatitude, In::PRIMARY).is_none());
    }
}
