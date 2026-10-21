/// Nikon Z cameras commonly put a 160x120 RGB thumbnail directly in the root
/// TIFF IFD rather than in a JPEG thumbnail directory. Reading its single
/// strip avoids decoding any large preview or RAW sensor data.
fn nef_uncompressed_thumbnail(path: &std::path::Path) -> Result<Option<DecodedThumbnailSource>> {
    let mut file = fs::File::open(path)?;
    let mut header = [0; 8];
    file.read_exact(&mut header)?;
    let little_endian = match &header[..2] {
        b"II" => true,
        b"MM" => false,
        _ => return Ok(None),
    };
    if tiff_u16(&header[2..4], little_endian) != 42 {
        return Ok(None);
    }
    file.seek(SeekFrom::Start(u64::from(tiff_u32(
        &header[4..8],
        little_endian,
    ))))?;
    let count = read_tiff_u16(&mut file, little_endian)? as usize;
    if count > 1024 {
        return Ok(None);
    }
    let (mut width, mut height, mut compression, mut samples, mut offset, mut length) =
        (None, None, None, None, None, None);
    for _ in 0..count {
        let mut entry = [0; 12];
        file.read_exact(&mut entry)?;
        let tag = tiff_u16(&entry[..2], little_endian);
        let field_type = tiff_u16(&entry[2..4], little_endian);
        let item_count = tiff_u32(&entry[4..8], little_endian);
        if item_count != 1 {
            continue;
        }
        let value = match field_type {
            3 => u32::from(tiff_u16(&entry[8..10], little_endian)),
            4 => tiff_u32(&entry[8..12], little_endian),
            _ => continue,
        };
        match tag {
            0x0100 => width = Some(value),
            0x0101 => height = Some(value),
            0x0103 => compression = Some(value),
            0x0111 => offset = Some(value),
            0x0115 => samples = Some(value),
            0x0117 => length = Some(value),
            _ => {}
        }
    }
    let (Some(width), Some(height), Some(1), Some(3), Some(offset), Some(length)) =
        (width, height, compression, samples, offset, length)
    else {
        return Ok(None);
    };
    let expected = width
        .checked_mul(height)
        .and_then(|pixels| pixels.checked_mul(3));
    if width == 0
        || height == 0
        || expected != Some(length)
        || u64::from(offset) + u64::from(length) > file.metadata()?.len()
    {
        return Ok(None);
    }
    file.seek(SeekFrom::Start(u64::from(offset)))?;
    let mut pixels = vec![0; length as usize];
    file.read_exact(&mut pixels)?;
    let image = image::RgbImage::from_raw(width, height, pixels)
        .context("invalid NEF embedded RGB thumbnail")?;
    Ok(Some(DecodedThumbnailSource {
        image,
        source_width: width,
        source_height: height,
        scale: "embedded TIFF thumbnail",
    }))
}

fn nef_embedded_thumbnail(path: &std::path::Path) -> Result<Option<Vec<u8>>> {
    read_nef_jpeg(path, false)
}

fn nef_embedded_preview(path: &std::path::Path) -> Result<Option<Vec<u8>>> {
    read_nef_jpeg(path, true)
}

fn read_nef_jpeg(path: &std::path::Path, largest: bool) -> Result<Option<Vec<u8>>> {
    let mut file = fs::File::open(path)?;
    let file_size = file.metadata()?.len();
    read_nef_jpeg_from_reader(&mut file, file_size, largest)
}

fn nef_embedded_preview_from_bytes(bytes: &[u8]) -> Result<Option<Vec<u8>>> {
    let mut cursor = Cursor::new(bytes);
    read_nef_jpeg_from_reader(&mut cursor, bytes.len() as u64, true)
}

fn read_nef_jpeg_from_reader(
    reader: &mut (impl Read + Seek),
    file_size: u64,
    largest: bool,
) -> Result<Option<Vec<u8>>> {
    let Some((offset, length)) = nef_jpeg_stream(reader, file_size, largest)? else {
        return Ok(None);
    };
    // A malformed tag must never request an unbounded allocation or seek.
    if length == 0
        || length > 100 * 1024 * 1024
        || u64::from(offset) + u64::from(length) > file_size
    {
        return Ok(None);
    }
    reader.seek(SeekFrom::Start(u64::from(offset)))?;
    let mut bytes = vec![0; length as usize];
    reader.read_exact(&mut bytes)?;
    Ok((bytes.starts_with(&[0xff, 0xd8])).then_some(bytes))
}
#[cfg(target_os = "linux")]
const REMOTE_NEF_METADATA_READ_AHEAD: usize = 4096;
#[cfg(target_os = "linux")]
struct RemoteNefReader<'a> {
    reference: &'a str,
    position: u64,
    size: u64,
    cache_offset: u64,
    cache: Vec<u8>,
    metadata_reads: usize,
    metadata_bytes: usize,
    metadata_started: std::time::Instant,
}
#[cfg(target_os = "linux")]
impl<'a> RemoteNefReader<'a> {
    fn open(reference: &'a str) -> Result<Self> {
        Ok(Self {
            reference,
            position: 0,
            size: crate::network_shares::stat(reference)?.size,
            cache_offset: 0,
            cache: Vec::new(),
            metadata_reads: 0,
            metadata_bytes: 0,
            metadata_started: std::time::Instant::now(),
        })
    }
    fn cached(&self) -> bool {
        self.position >= self.cache_offset
            && self.position < self.cache_offset + self.cache.len() as u64
    }
    fn read_metadata_block(&mut self) -> std::io::Result<()> {
        let length = REMOTE_NEF_METADATA_READ_AHEAD.min((self.size - self.position) as usize);
        self.cache = crate::source::read_range(self.reference, self.position, length)
            .map_err(std::io::Error::other)?;
        self.cache_offset = self.position;
        self.metadata_reads += 1;
        self.metadata_bytes += self.cache.len();
        Ok(())
    }
}
#[cfg(target_os = "linux")]
impl std::io::Read for RemoteNefReader<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        if self.position >= self.size || buffer.is_empty() {
            return Ok(0);
        }
        if buffer.len() > REMOTE_NEF_METADATA_READ_AHEAD {
            let requested = buffer.len().min((self.size - self.position) as usize);
            let bytes = crate::source::read_range(self.reference, self.position, requested)
                .map_err(std::io::Error::other)?;
            let received = bytes.len();
            buffer[..received].copy_from_slice(&bytes);
            self.position += received as u64;
            return Ok(received);
        }
        if !self.cached() {
            self.read_metadata_block()?
        }
        let start = (self.position - self.cache_offset) as usize;
        let received = buffer.len().min(self.cache.len() - start);
        buffer[..received].copy_from_slice(&self.cache[start..start + received]);
        self.position += received as u64;
        Ok(received)
    }
}
#[cfg(target_os = "linux")]
impl std::io::Seek for RemoteNefReader<'_> {
    fn seek(&mut self, from: SeekFrom) -> std::io::Result<u64> {
        let next = match from {
            SeekFrom::Start(v) => v as i128,
            SeekFrom::Current(v) => self.position as i128 + v as i128,
            SeekFrom::End(v) => self.size as i128 + v as i128,
        };
        if next < 0 || next > self.size as i128 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "NEF seek outside file",
            ));
        }
        self.position = next as u64;
        Ok(self.position)
    }
}
#[cfg(target_os = "linux")]
fn remote_nef_embedded_jpeg(reference: &str, largest: bool) -> Result<Option<Vec<u8>>> {
    let mut file = RemoteNefReader::open(reference)?;
    let file_size = file.size;
    let Some((offset, length)) = nef_jpeg_stream(&mut file, file_size, largest)? else {
        return Ok(None);
    };
    if length == 0
        || length > 100 * 1024 * 1024
        || u64::from(offset) + u64::from(length) > file.size
    {
        return Ok(None);
    }
    file.seek(SeekFrom::Start(u64::from(offset)))?;
    let mut bytes = vec![0; length as usize];
    let preview_started = std::time::Instant::now();
    file.read_exact(&mut bytes)?;
    crate::network_shares::trace("REMOTE_RAW",format!("operation={} extension=nef metadata_reads={} metadata_bytes={} metadata_ms={} preview_offset={offset} preview_length={length} preview_ms={}",if largest{"lightbox"}else{"thumbnail"},file.metadata_reads,file.metadata_bytes,file.metadata_started.elapsed().as_millis(),preview_started.elapsed().as_millis()));
    Ok((bytes.starts_with(&[0xff, 0xd8])).then_some(bytes))
}

#[cfg(target_os = "linux")]
fn remote_nef_orientation(reference: &str) -> Option<u16> {
    let mut file = RemoteNefReader::open(reference).ok()?;
    tiff_root_orientation(&mut file)
}

// Only the root IFD is needed: do not scan or preload RAW sensor strips.
fn tiff_root_orientation(file: &mut (impl Read + Seek)) -> Option<u16> {
    file.seek(SeekFrom::Start(0)).ok()?;
    let mut header = [0; 8];
    file.read_exact(&mut header).ok()?;
    let little = match &header[..2] {
        b"II" => true,
        b"MM" => false,
        _ => return None,
    };
    if tiff_u16(&header[2..4], little) != 42 {
        return None;
    }
    file.seek(SeekFrom::Start(u64::from(tiff_u32(&header[4..8], little))))
        .ok()?;
    let count = read_tiff_u16(file, little).ok()? as usize;
    if count > 1024 {
        return None;
    }
    for _ in 0..count {
        let mut entry = [0; 12];
        file.read_exact(&mut entry).ok()?;
        if tiff_u16(&entry[..2], little) == 0x0112 && tiff_u32(&entry[4..8], little) == 1 {
            let value = match tiff_u16(&entry[2..4], little) {
                3 => tiff_u16(&entry[8..10], little),
                4 => u16::try_from(tiff_u32(&entry[8..12], little)).ok()?,
                _ => return None,
            };
            return (1..=8).contains(&value).then_some(value);
        }
    }
    None
}

/// Locate JPEGInterchangeFormat streams in classic TIFF IFDs, including the
/// Nikon SubIFDs that kamadak-exif deliberately does not expose as IFD1.
fn nef_jpeg_stream(
    file: &mut (impl Read + Seek),
    file_size: u64,
    largest: bool,
) -> Result<Option<(u32, u32)>> {
    let mut header = [0; 8];
    file.read_exact(&mut header)?;
    let little_endian = match &header[..2] {
        b"II" => true,
        b"MM" => false,
        _ => return Ok(None),
    };
    if tiff_u16(&header[2..4], little_endian) != 42 {
        return Ok(None);
    }
    let mut pending = vec![tiff_u32(&header[4..8], little_endian)];
    let mut visited = std::collections::HashSet::new();
    let mut candidates = Vec::new();

    while let Some(ifd_offset) = pending.pop() {
        if ifd_offset == 0 || !visited.insert(ifd_offset) || visited.len() > 256 {
            continue;
        }
        file.seek(SeekFrom::Start(u64::from(ifd_offset)))?;
        let count = read_tiff_u16(file, little_endian)? as usize;
        if count > 1024 {
            continue;
        }
        let (mut jpeg_offset, mut jpeg_length, mut jpeg_width, mut jpeg_height) =
            (None, None, None, None);
        for _ in 0..count {
            let mut entry = [0; 12];
            file.read_exact(&mut entry)?;
            let tag = tiff_u16(&entry[..2], little_endian);
            let field_type = tiff_u16(&entry[2..4], little_endian);
            let item_count = tiff_u32(&entry[4..8], little_endian);
            let value = tiff_u32(&entry[8..12], little_endian);
            let scalar = match (field_type, item_count) {
                (3, 1) => Some(u32::from(tiff_u16(&entry[8..10], little_endian))),
                (4, 1) => Some(value),
                _ => None,
            };
            match tag {
                0x0100 if scalar.is_some() => jpeg_width = scalar,
                0x0101 if scalar.is_some() => jpeg_height = scalar,
                0x0201 if item_count == 1 && matches!(field_type, 3 | 4) => {
                    jpeg_offset = Some(value)
                }
                0x0202 if item_count == 1 && matches!(field_type, 3 | 4) => {
                    jpeg_length = Some(value)
                }
                // SubIFDs may point to several preview/thumbnail directories.
                0x014a if field_type == 4 && item_count > 0 && item_count <= 64 => {
                    if item_count == 1 {
                        pending.push(value);
                    } else {
                        let return_position = file.stream_position()?;
                        file.seek(SeekFrom::Start(u64::from(value)))?;
                        for _ in 0..item_count {
                            pending.push(read_tiff_u32(file, little_endian)?);
                        }
                        file.seek(SeekFrom::Start(return_position))?;
                    }
                }
                // Nikon often stores a preview IFD through this private tag.
                0x8769 | 0x014a if field_type == 4 && item_count == 1 => pending.push(value),
                _ => {}
            }
        }
        if let (Some(offset), Some(length)) = (jpeg_offset, jpeg_length) {
            candidates.push((offset, length, jpeg_width, jpeg_height));
        }
        pending.push(read_tiff_u32(file, little_endian)?);
    }
    // Nikon's JpgFromRaw stream often has no dimensions in its TIFF IFD. Read
    // only its JPEG header to find the actual pixel size; compressed byte size
    // can rank a lower-resolution OtherImage above the full-size camera frame.
    if largest {
        for (offset, length, width, height) in &mut candidates {
            if width.is_none() || height.is_none() {
                let end = u64::from(*offset).checked_add(u64::from(*length));
                if end.is_some_and(|end| end <= file_size) {
                    if let Some((jpeg_width, jpeg_height)) =
                        nef_jpeg_dimensions_at(file, *offset, *length)?
                    {
                        *width = Some(jpeg_width);
                        *height = Some(jpeg_height);
                    }
                }
            }
        }
    }

    // The thumbnail path keeps its existing byte-size choice for fast reads.
    // The Lightbox path instead selects by pixel area and uses byte length as
    // a tie-breaker or fallback when dimensions cannot be recovered.
    let candidates = candidates.into_iter().filter(|(_, length, _, _)| *length > 0);
    Ok(if largest {
        candidates
            .max_by_key(|(_, length, width, height)| {
                (
                    width
                        .zip(*height)
                        .map(|(width, height)| u64::from(width) * u64::from(height))
                        .unwrap_or(0),
                    *length,
                )
            })
            .map(|(offset, length, _, _)| (offset, length))
    } else {
        candidates
            .min_by_key(|(_, length, _, _)| *length)
            .map(|(offset, length, _, _)| (offset, length))
    })
}

fn nef_jpeg_dimensions_at(
    file: &mut (impl Read + Seek),
    offset: u32,
    length: u32,
) -> Result<Option<(u32, u32)>> {
    if length < 2 {
        return Ok(None);
    }
    let end = u64::from(offset) + u64::from(length);
    file.seek(SeekFrom::Start(u64::from(offset)))?;
    let mut soi = [0; 2];
    file.read_exact(&mut soi)?;
    if soi != [0xff, 0xd8] {
        return Ok(None);
    }

    loop {
        if file.stream_position()? >= end {
            return Ok(None);
        }
        let mut byte = [0; 1];
        file.read_exact(&mut byte)?;
        if byte[0] != 0xff {
            continue;
        }
        loop {
            if file.stream_position()? >= end {
                return Ok(None);
            }
            file.read_exact(&mut byte)?;
            if byte[0] != 0xff {
                break;
            }
        }
        let marker = byte[0];
        if marker == 0xd9 || marker == 0xda {
            return Ok(None);
        }
        if marker == 0x01 || (0xd0..=0xd7).contains(&marker) {
            continue;
        }

        if file
            .stream_position()?
            .checked_add(2)
            .is_none_or(|next| next > end)
        {
            return Ok(None);
        }
        let mut segment_length = [0; 2];
        file.read_exact(&mut segment_length)?;
        let segment_length = u16::from_be_bytes(segment_length);
        if segment_length < 2 {
            return Ok(None);
        }
        let segment_end = file.stream_position()? + u64::from(segment_length - 2);
        if segment_end > end {
            return Ok(None);
        }
        if matches!(marker, 0xc0..=0xc3 | 0xc5..=0xc7 | 0xc9..=0xcb | 0xcd..=0xcf) {
            if segment_length < 7 {
                return Ok(None);
            }
            let mut frame = [0; 5];
            file.read_exact(&mut frame)?;
            let height = u32::from(u16::from_be_bytes([frame[1], frame[2]]));
            let width = u32::from(u16::from_be_bytes([frame[3], frame[4]]));
            return Ok((width > 0 && height > 0).then_some((width, height)));
        }
        file.seek(SeekFrom::Start(segment_end))?;
    }
}

#[cfg(test)]
mod nef_jpeg_selection_tests {
    use super::*;

    fn append_ifd(bytes: &mut Vec<u8>, offset: u32, length: u32, next: u32) {
        bytes.extend(2u16.to_le_bytes());
        for (tag, value) in [(0x0201u16, offset), (0x0202u16, length)] {
            bytes.extend(tag.to_le_bytes());
            bytes.extend(4u16.to_le_bytes());
            bytes.extend(1u32.to_le_bytes());
            bytes.extend(value.to_le_bytes());
        }
        bytes.extend(next.to_le_bytes());
    }

    fn append_jpeg(bytes: &mut Vec<u8>, width: u16, height: u16, length: usize) {
        bytes.extend([0xff, 0xd8, 0xff, 0xc0, 0, 11, 8]);
        bytes.extend(height.to_be_bytes());
        bytes.extend(width.to_be_bytes());
        bytes.extend([1, 1, 0x11, 0, 0xff, 0xd9]);
        bytes.resize(bytes.len() + length - 17, 0);
    }

    #[test]
    fn selects_nef_jpegs_by_pixel_area_not_compressed_bytes() {
        let first_ifd = 8u32;
        let second_ifd = first_ifd + 2 + 2 * 12 + 4;
        let first_jpeg = second_ifd + 2 + 2 * 12 + 4;
        let second_jpeg = first_jpeg + 100;
        let mut bytes = b"II\x2a\0\x08\0\0\0".to_vec();
        append_ifd(&mut bytes, first_jpeg, 100, second_ifd);
        append_ifd(&mut bytes, second_jpeg, 20, 0);
        append_jpeg(&mut bytes, 640, 424, 100);
        append_jpeg(&mut bytes, 3936, 2624, 20);

        assert_eq!(
            nef_jpeg_stream(&mut Cursor::new(&bytes), bytes.len() as u64, true).unwrap(),
            Some((second_jpeg, 20)),
            "the full-size JPEG must win even when it compresses to fewer bytes"
        );
    }
}

fn read_tiff_u16(file: &mut impl Read, little_endian: bool) -> Result<u16> {
    let mut bytes = [0; 2];
    file.read_exact(&mut bytes)?;
    Ok(tiff_u16(&bytes, little_endian))
}

fn read_tiff_u32(file: &mut impl Read, little_endian: bool) -> Result<u32> {
    let mut bytes = [0; 4];
    file.read_exact(&mut bytes)?;
    Ok(tiff_u32(&bytes, little_endian))
}

fn tiff_u16(bytes: &[u8], little_endian: bool) -> u16 {
    if little_endian {
        u16::from_le_bytes([bytes[0], bytes[1]])
    } else {
        u16::from_be_bytes([bytes[0], bytes[1]])
    }
}

fn tiff_u32(bytes: &[u8], little_endian: bool) -> u32 {
    if little_endian {
        u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
    } else {
        u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
    }
}

#[cfg(test)]
mod tiff_orientation_io_tests {
    use super::*;
    struct MetadataOnly(std::io::Cursor<Vec<u8>>);
    impl Read for MetadataOnly {
        fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
            assert!(self.0.position() + bytes.len() as u64 <= 22, "orientation read sensor pixels");
            self.0.read(bytes)
        }
    }
    impl Seek for MetadataOnly {
        fn seek(&mut self, position: SeekFrom) -> std::io::Result<u64> {
            self.0.seek(position)
        }
    }
    #[test]
    fn malformed_tiff_orientation_defaults_without_reading_sensor_data() {
        for value in [0u32, 9, 65537] {
            let mut bytes = b"II\x2a\0\x08\0\0\0\x01\0\x12\x01\x04\0\x01\0\0\0".to_vec();
            bytes.extend(value.to_le_bytes());
            let mut file = MetadataOnly(std::io::Cursor::new(bytes));
            assert_eq!(orientation_from_container(&mut file), 1);
        }
        let mut file = std::io::Cursor::new(b"II\x2a\0\xff\xff\xff\xff".to_vec());
        assert_eq!(orientation_from_container(&mut file), 1);
    }

    #[test]
    fn tiff_orientation_reads_only_root_metadata_in_both_byte_orders() {
        for little in [true, false] {
            for kind in [3u16, 4] {
                for value in 1..=8u32 {
                    let u16_bytes = |v: u16| if little { v.to_le_bytes() } else { v.to_be_bytes() };
                    let u32_bytes = |v: u32| if little { v.to_le_bytes() } else { v.to_be_bytes() };
                    let mut bytes = if little { b"II".to_vec() } else { b"MM".to_vec() };
                    bytes.extend(u16_bytes(42));
                    bytes.extend(u32_bytes(8));
                    bytes.extend(u16_bytes(1));
                    bytes.extend(u16_bytes(0x112));
                    bytes.extend(u16_bytes(kind));
                    bytes.extend(u32_bytes(1));
                    if kind == 3 {
                        bytes.extend(u16_bytes(value as u16));
                        bytes.extend([0, 0]);
                    } else {
                        bytes.extend(u32_bytes(value));
                    }
                    let mut file = MetadataOnly(std::io::Cursor::new(bytes));
                    assert_eq!(orientation_from_container(&mut file), value as u16);
                }
            }
        }
    }
}

fn orientation_from_container(file: &mut (impl Read + Seek)) -> u16 {
    let mut header = [0; 8];
    if file.read_exact(&mut header).is_err() {
        return 1;
    }
    let classic_tiff = (&header[..2] == b"II" && header[2..4] == [42, 0])
        || (&header[..2] == b"MM" && header[2..4] == [0, 42]);
    if classic_tiff {
        // Missing/invalid orientation in a TIFF defaults to upright. Falling
        // back to the EXIF library here would read the entire RAW into RAM.
        return tiff_root_orientation(file).unwrap_or(1);
    }
    if file.seek(SeekFrom::Start(0)).is_err() {
        return 1;
    }
    exif::Reader::new()
        .read_from_container(&mut BufReader::new(file))
        .ok()
        .and_then(|exif| {
            exif.get_field(exif::Tag::Orientation, exif::In::PRIMARY)
                .and_then(|field| field.value.get_uint(0))
                .and_then(|value| u16::try_from(value).ok())
                .filter(|orientation| (1..=8).contains(orientation))
        })
        .unwrap_or(1)
}
