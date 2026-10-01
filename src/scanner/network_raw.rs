//! Geometry-only classic TIFF traversal for remote NEF/DNG and TIFF-based RAW.
//! Never follows strip/tile data or reads MakerNotes/embedded image payloads.
use std::collections::{HashMap, HashSet};

use anyhow::Result;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Geometry {
    pub width: u32,
    pub height: u32,
    pub orientation: Option<u16>,
}

pub(super) fn dimensions(
    header: &[u8],
    mut read_range: impl FnMut(u64, usize) -> Result<Vec<u8>>,
) -> Option<Geometry> {
    // Try exclusively cached bytes first. Only missing geometry permits I/O.
    parse(true, &mut |offset, length| {
        header
            .get(offset as usize..(offset as usize).checked_add(length)?)
            .map(Vec::from)
    })
    .or_else(|| {
        let (mut reads, mut bytes_read) = (0, 0);
        parse(false, &mut |offset, length| {
            if let Some(bytes) = header.get(offset as usize..(offset as usize).checked_add(length)?)
            {
                return Some(bytes.to_vec());
            }
            // Hard budgets also bound malicious directory chains/counts.
            if reads >= 64 || bytes_read + length > 128 * 1024 {
                return None;
            }
            reads += 1;
            bytes_read += length;
            let bytes = read_range(offset, length).ok()?;
            (bytes.len() == length).then_some(bytes)
        })
    })
}

fn parse(
    require_complete: bool,
    read: &mut impl FnMut(u64, usize) -> Option<Vec<u8>>,
) -> Option<Geometry> {
    let header = read(0, 8)?;
    let little = match &header[..2] {
        b"II" => true,
        b"MM" => false,
        _ => return None,
    };
    let u16_at = |b: &[u8]| {
        let bytes = [b[0], b[1]];
        if little {
            u16::from_le_bytes(bytes)
        } else {
            u16::from_be_bytes(bytes)
        }
    };
    let u32_at = |b: &[u8]| {
        let bytes = [b[0], b[1], b[2], b[3]];
        if little {
            u32::from_le_bytes(bytes)
        } else {
            u32::from_be_bytes(bytes)
        }
    };
    if u16_at(&header[2..4]) != 42 {
        return None; // BigTIFF/non-TIFF RAW needs a format-specific reader.
    }
    let root = u32_at(&header[4..8]);
    let mut pending = vec![root];
    let mut visited = HashSet::new();
    let mut orientation = None;
    let mut best = None;
    let mut incomplete = false;
    let mut best_score = (false, 0u64);
    while let Some(offset) = pending.pop() {
        if offset == 0 || !visited.insert(offset) {
            continue;
        }
        if visited.len() > 32 {
            incomplete = true;
            break;
        }
        let Some(count_bytes) = read(u64::from(offset), 2) else {
            incomplete = true;
            continue;
        };
        let count = u16_at(&count_bytes) as usize;
        if count > 1024 {
            incomplete = true;
            continue;
        }
        let Some(directory) = read(u64::from(offset) + 2, count * 12 + 4) else {
            incomplete = true;
            continue;
        };
        let mut tags = HashMap::new();
        let mut missing_geometry_values = false;
        for entry in directory[..count * 12].chunks_exact(12) {
            let tag = u16_at(entry);
            // Only geometry and IFD links. In particular, never read the values
            // referenced by StripOffsets/TileOffsets/JPEGInterchangeFormat.
            if !matches!(
                tag,
                254 | 256 | 257 | 262 | 274 | 330 | 34665 | 40962 | 40963 | 50719 | 50720 | 50829
            ) {
                continue;
            }
            let kind = u16_at(&entry[2..4]);
            let count = u32_at(&entry[4..8]) as usize;
            let item_size = match kind {
                3 => 2,
                4 | 13 => 4,
                5 => 8,
                _ => continue,
            };
            if count == 0 || count > 64 {
                continue;
            }
            let length = count * item_size;
            let values = if length <= 4 {
                entry[8..8 + length].to_vec()
            } else {
                let Some(values) = read(u64::from(u32_at(&entry[8..12])), length) else {
                    incomplete = true;
                    missing_geometry_values |= matches!(tag, 50719 | 50720 | 50829);
                    continue;
                };
                values
            };
            let parsed = values
                .chunks_exact(item_size)
                .map(|bytes| match kind {
                    3 => Some(u32::from(u16_at(bytes))),
                    4 | 13 => Some(u32_at(bytes)),
                    // Like the local DNG reader, discard the fractional part
                    // of RATIONAL crop coordinates for integer pixel axes.
                    5 => {
                        let (num, den) = (u32_at(bytes), u32_at(&bytes[4..]));
                        (den > 0).then(|| num / den)
                    }
                    _ => None,
                })
                .collect::<Option<Vec<_>>>();
            if let Some(values) = parsed {
                tags.insert(tag, values);
            }
        }
        let scalar = |tag| tags.get(&tag).and_then(|v| v.first()).copied();
        if offset == root {
            orientation = scalar(274)
                .and_then(|v| u16::try_from(v).ok())
                .filter(|v| (1..=8).contains(v));
        }
        for tag in [330, 34665] {
            if let Some(offsets) = tags.get(&tag) {
                pending.extend(offsets);
            }
        }
        pending.push(u32_at(&directory[count * 12..]));
        // Reduced-resolution preview directories must not become sensor axes.
        if missing_geometry_values || scalar(254).unwrap_or_default() & 1 != 0 {
            continue;
        }
        let axes = scalar(40962)
            .zip(scalar(40963))
            .filter(|(w, h)| *w > 0 && *h > 0)
            .or_else(|| scalar(256).zip(scalar(257)));
        let Some((width, height)) = axes.filter(|(w, h)| *w > 0 && *h > 0) else {
            continue;
        };
        // Prefer CFA/LinearRaw IFDs over full-size RGB previews; then largest
        // sensor area. Crop is applied only after selecting the sensor IFD.
        let score = (
            matches!(scalar(262), Some(32803 | 34892)),
            u64::from(width) * u64::from(height),
        );
        if score > best_score {
            best_score = score;
            best = Some(cropped_dimensions(width, height, &tags));
        }
    }
    // The cached pass needs complete sensor metadata to choose the correct
    // IFD, not just any available axes. After exhausting targeted budgets,
    // a known CFA/LinearRaw sensor is safer than an ambiguous RGB preview.
    if incomplete && (require_complete || !best_score.0) {
        return None;
    }
    let (width, height) = best?;
    Some(Geometry {
        width,
        height,
        orientation,
    })
}

fn cropped_dimensions(width: u32, height: u32, tags: &HashMap<u16, Vec<u32>>) -> (u32, u32) {
    let active = match tags.get(&50829).map(Vec::as_slice) {
        Some([top, left, bottom, right])
            if top < bottom && left < right && *bottom <= height && *right <= width =>
        {
            (right - left, bottom - top)
        }
        _ => (width, height),
    };
    match (
        tags.get(&50719).map(Vec::as_slice),
        tags.get(&50720).map(Vec::as_slice),
    ) {
        (Some([x, y]), Some([w, h]))
            if *w > 0
                && *h > 0
                && u64::from(*x) + u64::from(*w) <= u64::from(active.0)
                && u64::from(*y) + u64::from(*h) <= u64::from(active.1) =>
        {
            (*w, *h)
        }
        _ => active,
    }
}
