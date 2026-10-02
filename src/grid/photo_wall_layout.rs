use std::ops::Range;

pub(super) const WALL_GAP: f64 = 0.0;
pub(super) const WALL_SIDE_MARGIN: f64 = 0.0;

#[derive(Clone, Debug)]
pub(super) struct PhotoWallSection {
    pub photo_range: Range<usize>,
    pub header_height: f64,
}

#[derive(Clone, Debug)]
pub(super) struct PhotoWallItem {
    pub photo_index: usize,
    pub section: usize,
    pub row: usize,
    // Shared gallery APIs use f64; all wall allocations are whole pixels.
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Clone, Debug)]
pub(super) struct PhotoWallRow {
    pub item_range: Range<usize>,
    pub y: f64,
    pub image_height: f64,
    pub block_height: f64,
    ideal_height: f64,
    ideal_block_height: f64,
    justified: bool,
}

#[derive(Clone, Debug)]
pub(super) struct PhotoWallSectionBounds {
    pub header_y: f64,
    pub first_photo_y: f64,
    pub end_y: f64,
}

#[derive(Clone, Debug, Default)]
pub(super) struct PhotoWallLayout {
    pub items: Vec<PhotoWallItem>,
    pub rows: Vec<PhotoWallRow>,
    pub sections: Vec<PhotoWallSectionBounds>,
    pub total_height: f64,
    lookup: Vec<Option<usize>>,
    aspect_ratios: Vec<f64>,
    viewport_width: f64,
}

fn ratio(value: f64) -> f64 {
    if value.is_finite() && value > 0.0 {
        value
    } else {
        1.0
    }
}

// Allocate whole pixels by largest remainder, then accumulate those widths.
// The optional one-pixel minimum only applies when the row has enough pixels.
fn integer_widths(ratios: &[f64], available: i32) -> Vec<i32> {
    let sum: f64 = ratios.iter().sum();
    let ideal: Vec<_> = ratios
        .iter()
        .map(|r| f64::from(available) * r / sum)
        .collect();
    let minimum = i32::from(available as usize >= ratios.len());
    let mut widths: Vec<_> = ideal
        .iter()
        .map(|w| (w.floor() as i32).max(minimum))
        .collect();
    let mut remainder = available - widths.iter().sum::<i32>();
    let mut order: Vec<_> = (0..widths.len()).collect();
    order.sort_by(|&a, &b| {
        ideal[b]
            .fract()
            .total_cmp(&ideal[a].fract())
            .then(a.cmp(&b))
    });
    while remainder > 0 {
        for &i in &order {
            if remainder == 0 {
                break;
            }
            widths[i] += 1;
            remainder -= 1;
        }
    }
    while remainder < 0 {
        for &i in order.iter().rev() {
            if remainder == 0 {
                break;
            }
            if widths[i] > minimum {
                widths[i] -= 1;
                remainder += 1;
            }
        }
    }
    widths
}

impl PhotoWallLayout {
    pub fn calculate(
        aspect_ratios: &[f64],
        sections: &[PhotoWallSection],
        viewport_width: f64,
        target_height: f64,
        caption_height: f64,
    ) -> Self {
        let viewport = if viewport_width.is_finite() {
            viewport_width.round().max(1.0)
        } else {
            1.0
        };
        let margin = WALL_SIDE_MARGIN.min((viewport - 1.0) * 0.5);
        let usable = viewport - 2.0 * margin;
        let target = if target_height.is_finite() {
            target_height.max(1.0)
        } else {
            100.0
        };
        let caption = if caption_height.is_finite() {
            caption_height.round().max(0.0)
        } else {
            0.0
        };
        let mut layout = Self {
            lookup: vec![None; aspect_ratios.len()],
            aspect_ratios: aspect_ratios.iter().copied().map(ratio).collect(),
            viewport_width: viewport,
            ..Self::default()
        };
        let mut y = 0.0;
        let mut ideal_y = 0.0;
        for (section_index, section) in sections.iter().enumerate() {
            let header_y = y;
            let header = if section.header_height.is_finite() {
                section.header_height.round().max(0.0)
            } else {
                0.0
            };
            y += header;
            ideal_y += header;
            let first_photo_y = y;
            let mut start = section.photo_range.start.min(aspect_ratios.len());
            let section_end = section.photo_range.end.min(aspect_ratios.len());
            while start < section_end {
                let mut end = start;
                let mut sum = 0.0;
                // Stop on reaching the target width. A single wide image still
                // produces a row, and tiny viewports never gain negative space.
                while end < section_end && end - start < usable as usize {
                    // Leave positive image space before adding another gap.
                    if end > start && WALL_GAP * (end - start) as f64 >= usable {
                        break;
                    }
                    sum += ratio(aspect_ratios[end]);
                    end += 1;
                    if sum * target + WALL_GAP * (end - start - 1) as f64 >= usable {
                        break;
                    }
                }
                let crossed = sum * target + WALL_GAP * (end - start - 1) as f64 >= usable;
                let fitted = |sum: f64, count: usize| {
                    (usable - WALL_GAP * count.saturating_sub(1) as f64).max(f64::EPSILON) / sum
                };
                if crossed && end - start > 1 {
                    let before_sum = sum - ratio(aspect_ratios[end - 1]);
                    let before_h = fitted(before_sum, end - start - 1);
                    let after_h = fitted(sum, end - start);
                    let before_ok = (target * 0.65..=target * 1.20).contains(&before_h);
                    let after_ok = (target * 0.65..=target * 1.20).contains(&after_h);
                    if before_ok
                        && (!after_ok || (before_h - target).abs() < (after_h - target).abs())
                    {
                        end -= 1;
                        sum = before_sum;
                    }
                }
                // Sparse final rows stay at target height if justification
                // would exceed the existing preferred height band by 20%.
                let fitted_height = fitted(sum, end - start);
                let justified = crossed || fitted_height <= target * 1.20;
                let ideal_height = if justified { fitted_height } else { target };
                let height = (ideal_y + ideal_height.max(1.0)).round() - y;
                let available = if justified {
                    usable
                } else {
                    (ideal_height * sum)
                        .round()
                        .max((end - start) as f64)
                        .min(usable)
                } as i32;
                let widths = integer_widths(&layout.aspect_ratios[start..end], available);
                let row_index = layout.rows.len();
                let item_start = layout.items.len();
                let mut x = margin;
                for photo_index in start..end {
                    let width = f64::from(widths[photo_index - start]);
                    layout.lookup[photo_index] = Some(layout.items.len());
                    layout.items.push(PhotoWallItem {
                        photo_index,
                        section: section_index,
                        row: row_index,
                        x,
                        y,
                        width,
                        height,
                    });
                    x += width + WALL_GAP;
                }
                layout.rows.push(PhotoWallRow {
                    item_range: item_start..layout.items.len(),
                    y,
                    image_height: height,
                    block_height: height + caption,
                    ideal_height,
                    ideal_block_height: ideal_height + caption,
                    justified,
                });
                y += height + caption + WALL_GAP;
                ideal_y += ideal_height.max(1.0) + caption + WALL_GAP;
                start = end;
            }
            layout.sections.push(PhotoWallSectionBounds {
                header_y,
                first_photo_y,
                end_y: y,
            });
        }
        layout.total_height = y.max(1.0);
        layout.trace_rows();
        layout
    }

    /// Scale the photo area horizontally and vertically by `factor`, exactly
    /// like resizing a picture: no row is re-broken, no photo changes row, and
    /// widths are redistributed to whole pixels using the original aspect
    /// ratios. Section header bands keep their fixed height and are only shifted. `anchor_y` is a y in the old layout;
    /// the returned value is the same content point in the scaled layout.
    pub fn scale_photo_area(&mut self, factor: f64, anchor_y: f64) -> f64 {
        let factor = if factor.is_finite() && factor > 0.0 {
            factor
        } else {
            1.0
        };
        let viewport = (self.viewport_width * factor).round().max(1.0);
        let old_total = self.total_height;
        let mut cursor = 0.0;
        let mut ideal_cursor = 0.0;
        let mut mapped = None;
        let mut row_index = 0;
        for (section_index, bounds) in self.sections.iter_mut().enumerate() {
            let old_header = bounds.header_y;
            let old_first = bounds.first_photo_y;
            let old_end = bounds.end_y;
            bounds.header_y = cursor;
            cursor += old_first - old_header;
            ideal_cursor += old_first - old_header;
            bounds.first_photo_y = cursor;
            if (old_header..old_first).contains(&anchor_y) {
                mapped = Some(bounds.header_y + anchor_y - old_header);
            }
            while row_index < self.rows.len()
                && self.items[self.rows[row_index].item_range.start].section == section_index
            {
                let row = &mut self.rows[row_index];
                row_index += 1;
                let old_y = row.y;
                let old_block = row.block_height;
                row.ideal_height *= factor;
                row.ideal_block_height *= factor;
                row.y = cursor;
                // Round shared vertical edges so row-height error cannot
                // accumulate through a long collection during resizing.
                let caption = (row.ideal_block_height - row.ideal_height).round().max(0.0);
                ideal_cursor += row.ideal_height.max(1.0);
                row.image_height = ideal_cursor.round() - cursor;
                row.block_height = row.image_height + caption;
                ideal_cursor += caption;
                if (old_y..old_y + old_block).contains(&anchor_y) {
                    mapped = Some(cursor + (anchor_y - old_y) / old_block * row.block_height);
                }
                let tiles = &mut self.items[row.item_range.clone()];
                let ratios: Vec<_> = tiles
                    .iter()
                    .map(|item| self.aspect_ratios[item.photo_index])
                    .collect();
                let available = if row.justified {
                    viewport
                } else {
                    (row.ideal_height * ratios.iter().sum::<f64>())
                        .round()
                        .min(viewport)
                } as i32;
                let widths = integer_widths(&ratios, available);
                let mut x = 0;
                for (item, width) in tiles.iter_mut().zip(widths) {
                    item.x = f64::from(x);
                    item.y = cursor;
                    item.width = f64::from(width);
                    item.height = row.image_height;
                    x += width;
                }
                cursor += row.block_height;
            }
            bounds.end_y = cursor;
            if anchor_y == old_end {
                mapped = Some(cursor);
            }
        }
        self.viewport_width = viewport;
        self.total_height = cursor.max(1.0);
        self.trace_rows();
        mapped.unwrap_or(cursor + (anchor_y - old_total))
    }

    fn trace_rows(&self) {
        if std::env::var_os("PICASA_TRACE").is_none() {
            return;
        }
        for row in &self.rows {
            let tiles = &self.items[row.item_range.clone()];
            let row_width_sum: f64 = tiles.iter().map(|tile| tile.width).sum();
            eprintln!("PIC_WALL_ROW viewport_width={} row_width_sum={} remainder={} tile_count={} justified={}",
                self.viewport_width, row_width_sum, self.viewport_width - row_width_sum,
                tiles.len(), row.justified);
        }
    }

    pub fn visible_rows(&self, top: f64, bottom: f64) -> Range<usize> {
        let start = self
            .rows
            .partition_point(|row| row.y + row.block_height <= top);
        if bottom <= top {
            return start..start;
        }
        let end = self.rows.partition_point(|row| row.y < bottom);
        start.min(end)..end
    }

    pub fn item(&self, photo_index: usize) -> Option<&PhotoWallItem> {
        self.lookup
            .get(photo_index)
            .copied()
            .flatten()
            .and_then(|index| self.items.get(index))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn layout(ratios: &[f64], width: f64, target: f64) -> PhotoWallLayout {
        PhotoWallLayout::calculate(
            ratios,
            &[PhotoWallSection {
                photo_range: 0..ratios.len(),
                header_height: 0.0,
            }],
            width,
            target,
            0.0,
        )
    }
    #[test]
    fn integer_rows_fill_exactly_after_repeated_resizes_and_zoom() {
        let ratios = [1.5, 0.7, 1.0, 1.8, 0.6, 1.2, 1.3, 0.8, 1.6, 1.1];
        for target in [32.0, 137.0, 240.0] {
            let mut wall = layout(&ratios, 799.0, target);
            let mut width = 799.0;
            for next in [799.0, 801.0, 997.0, 43.0, 1401.0, 799.0] {
                wall.scale_photo_area(next / width, 0.0);
                for row in &wall.rows {
                    let tiles = &wall.items[row.item_range.clone()];
                    let mut edge = 0.0;
                    for tile in tiles {
                        for value in [tile.x, tile.y, tile.width, tile.height] {
                            assert_eq!(value.fract(), 0.0, "fractional tile: {tile:?}");
                        }
                        assert_eq!(tile.x, edge);
                        edge += tile.width;
                    }
                    if row.item_range.end < wall.items.len() {
                        assert_eq!(edge, next);
                    }
                }
                width = next;
            }
        }
    }

    #[test]
    fn reasonable_final_row_is_justified() {
        let wall = layout(&[1.0; 7], 799.0, 100.0);
        assert_eq!(wall.rows.len(), 1);
        assert_eq!(wall.items.iter().map(|item| item.width).sum::<f64>(), 799.0);
        assert_eq!(wall.rows[0].image_height, 114.0);
    }

    #[test]
    fn completed_rows_fill_width() {
        let l = layout(&[1.5; 8], 800.0, 180.0);
        for row in &l.rows[..l.rows.len() - 1] {
            let last = &l.items[row.item_range.end - 1];
            assert!((last.x + last.width - 800.0).abs() < 0.001);
            for item in &l.items[row.item_range.clone()] {
                assert!((item.width - item.height * 1.5).abs() <= 1.0 + 1.5);
            }
        }
    }
    #[test]
    fn final_row_uses_target_height() {
        let l = layout(&[1.0, 1.0], 1000.0, 100.0);
        assert_eq!(l.rows.len(), 1);
        assert_eq!(l.items[0].x, 0.0);
        assert_eq!(l.items[0].height, 100.0);
        assert_eq!(l.items[1].x, 100.0);
    }
    #[test]
    fn rows_do_not_cross_sections() {
        let l = PhotoWallLayout::calculate(
            &[1.0; 6],
            &[
                PhotoWallSection {
                    photo_range: 0..2,
                    header_height: 70.0,
                },
                PhotoWallSection {
                    photo_range: 2..6,
                    header_height: 70.0,
                },
            ],
            800.0,
            100.0,
            24.0,
        );
        assert_eq!(l.items[0].y, 70.0);
        assert_eq!(l.items[2].y, 264.0);
        for row in &l.rows {
            let section = l.items[row.item_range.start].section;
            assert!(l.items[row.item_range.clone()]
                .iter()
                .all(|item| item.section == section));
        }
    }
    #[test]
    fn empty_sections_follow_header_policy() {
        let l = PhotoWallLayout::calculate(
            &[],
            &[PhotoWallSection {
                photo_range: 0..0,
                header_height: 70.0,
            }],
            800.0,
            100.0,
            0.0,
        );
        assert!(l.items.is_empty());
        assert_eq!(l.total_height, 70.0);
        assert!(l.visible_rows(0.0, 600.0).is_empty());
    }
    #[test]
    fn narrow_width_and_panorama_fit() {
        for width in [1.0, 30.0, 60.0, 800.0] {
            let l = layout(&[100.0, 0.01, 1.0], width, 100.0);
            assert_eq!(l.items.len(), 3);
            for item in &l.items {
                assert!(item.width > 0.0 && item.height > 0.0);
                assert!(item.x >= 0.0 && item.x + item.width <= width + 0.001);
            }
        }
    }
    #[test]
    fn gaps_never_overflow_a_narrow_viewport() {
        for width in [1.0, 43.0, 50.0] {
            let layout = layout(&[0.001; 20], width, 100.0);
            assert_eq!(layout.items.len(), 20);
            for item in &layout.items {
                assert!(
                    item.x + item.width <= width + 1e-9,
                    "gap placed item past edge: {item:?}"
                );
            }
        }
    }

    #[test]
    fn invalid_ratios_are_finite() {
        let l = layout(&[0.0, -1.0, f64::NAN, f64::INFINITY], 800.0, 100.0);
        for item in &l.items {
            assert!(item.width.is_finite() && item.height.is_finite());
            assert!((item.width / item.height - 1.0).abs() < 0.001);
        }
    }
    #[test]
    fn scaling_deep_section_anchor_keeps_fixed_headers_and_row_fraction() {
        let sections = (0..20).map(|section| PhotoWallSection {
            photo_range: section * 5..(section + 1) * 5,
            header_height: 70.0,
        }).collect::<Vec<_>>();
        let mut wall = PhotoWallLayout::calculate(&[1.0; 100], &sections, 500.0, 100.0, 0.0);
        assert_eq!(wall.rows.len(), 20);
        assert_eq!(wall.items[wall.rows[6].item_range.start].section, 6);
        let members = wall.rows.iter().map(|row| row.item_range.clone()).collect::<Vec<_>>();
        // Seven fixed headers (490px) + six rows (600px) + half a row (50px).
        let mut anchor = 1140.0;
        let mut width = 500.0;
        for (next_width, expected_y) in [(200.0, 750.0), (750.0, 1465.0),
            (1000.0, 1790.0), (400.0, 1010.0), (500.0, 1140.0)] {
            anchor = wall.scale_photo_area(next_width / width, anchor);
            assert!((anchor - expected_y).abs() < 1e-9, "fixed headers corrupted the anchor: {anchor} vs {expected_y}");
            let row = &wall.rows[6];
            assert!(((anchor - row.y) / row.block_height - 0.5).abs() < 1e-9);
            assert_eq!(wall.rows.iter().map(|row| row.item_range.clone()).collect::<Vec<_>>(), members);
            for section in &wall.sections {
                assert!((section.first_photo_y - section.header_y - 70.0).abs() < 1e-9);
            }
            width = next_width;
        }
    }

    #[test]
    fn scaling_keeps_rows_and_maps_anchor() {
        let ratios = [1.5, 0.7, 1.0, 1.8, 0.6, 1.2, 1.3, 0.8, 1.6, 1.1];
        let sections = [
            PhotoWallSection { photo_range: 0..5, header_height: 70.0 },
            PhotoWallSection { photo_range: 5..10, header_height: 70.0 },
        ];
        let before = PhotoWallLayout::calculate(&ratios, &sections, 1400.0, 180.0, 0.0);
        let mut after = before.clone();
        let factor = 1395.0 / 1400.0;
        let anchor = before.sections[1].first_photo_y + 40.0;
        let mapped = after.scale_photo_area(factor, anchor);
        assert_eq!(before.rows.len(), after.rows.len());
        for (a, b) in before.items.iter().zip(&after.items) {
            assert_eq!((a.row, a.section, a.photo_index), (b.row, b.section, b.photo_index));
            // Whole-pixel widths and shared rounded row edges introduce at
            // most one width pixel plus one height pixel of aspect error.
            let ratio = ratios[b.photo_index];
            assert!((b.width - b.height * ratio).abs() <= 1.0 + ratio);
            assert_eq!(b.x.fract(), 0.0);
        }
        let last = after.items.last().unwrap();
        assert!((last.x + last.width - 1395.0).abs() < 1.0 || last.x + last.width <= 1395.0 + 1e-6);
        let old_row = &before.rows[before.items[5].row];
        let new_row = &after.rows[after.items[5].row];
        assert!((mapped - (new_row.y + 40.0 / old_row.block_height * new_row.block_height)).abs() < 1e-9);
        assert!((after.sections[1].first_photo_y - after.sections[1].header_y - 70.0).abs() < 1e-9);
        assert!((after.total_height - after.sections[1].end_y).abs() < 1e-9);
    }

    #[test]
    fn visible_rows_match_intersections() {
        let l = layout(&[1.0; 100], 800.0, 100.0);
        for (top, bottom) in [
            (0.0, 100.0),
            (120.0, 450.0),
            (l.total_height + 1.0, l.total_height + 500.0),
        ] {
            let actual = l.visible_rows(top, bottom).collect::<Vec<_>>();
            let expected = l
                .rows
                .iter()
                .enumerate()
                .filter(|(_, r)| r.y < bottom && r.y + r.block_height > top)
                .map(|(i, _)| i)
                .collect::<Vec<_>>();
            assert_eq!(actual, expected);
        }
    }
    #[test]
    fn portrait_rows_respect_preferred_height_band() {
        let l = layout(&[0.5; 50], 800.0, 100.0);
        for row in &l.rows[..l.rows.len() - 1] {
            assert!((65.0..=120.0).contains(&row.image_height));
        }
    }
    #[test]
    fn extreme_ratios_keep_width_and_aspect_ratio() {
        let ratios = [100.0, 0.01, 4.0, 0.25];
        let l = layout(&ratios, 800.0, 100.0);
        for item in &l.items {
            assert!((item.width / item.height - ratios[item.photo_index]).abs() < 0.001);
            assert!(item.x + item.width <= 800.001);
        }
        assert!(l.rows.last().unwrap().image_height <= 100.0);
    }
    #[test]
    fn touching_edges_at_every_target_height_and_viewport() {
        for width in [43.0, 101.0, 799.0, 1000.0, 1401.0] {
            for target in [32.0, 100.0, 137.0, 240.0, 600.0] {
                let ratios = (0..200)
                    .map(|i| [0.5, 1.0, 1.5, 2.0][i % 4])
                    .collect::<Vec<_>>();
                let l = layout(&ratios, width, target);
                for row in &l.rows {
                    let items = &l.items[row.item_range.clone()];
                    assert_eq!(items[0].x, 0.0);
                    for pair in items.windows(2) {
                        assert_eq!(pair[0].x + pair[0].width, pair[1].x);
                        assert_eq!((pair[0].x + pair[0].width).round(), pair[1].x.round());
                    }
                    if row.item_range.end < l.items.len() {
                        let last = items.last().unwrap();
                        assert!((last.x + last.width - width).abs() < 1e-8);
                        assert_eq!((last.x + last.width).round(), width);
                    }
                }
                for pair in l.rows.windows(2) {
                    assert_eq!(pair[0].y + pair[0].image_height, pair[1].y);
                    assert_eq!(
                        (pair[0].y + pair[0].image_height).round(),
                        pair[1].y.round()
                    );
                }
                let incomplete = layout(&[0.5, 1.0], 2000.0, target);
                assert_eq!(incomplete.items[0].x, 0.0);
                assert_eq!(incomplete.rows[0].image_height, target);
            }
        }
    }

    #[test]
    fn large_collection_geometry_and_lookup() {
        for count in [14_361, 50_000] {
            let ratios = (0..count)
                .map(|i| [0.5, 1.0, 1.5, 2.0][i % 4])
                .collect::<Vec<_>>();
            let l = layout(&ratios, 1400.0, 160.0);
            assert_eq!(l.items.len(), count);
            for index in [0, count / 2, count - 1] {
                let item = l.item(index).unwrap();
                assert_eq!(item.photo_index, index);
                let rows = l.visible_rows(item.y, item.y + 600.0);
                assert!(rows.contains(&item.row));
                assert!(rows.len() < 20);
            }
        }
    }
}
