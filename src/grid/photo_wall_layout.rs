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
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    aspect_ratio: f64,
}

#[derive(Clone, Debug)]
pub(super) struct PhotoWallRow {
    pub item_range: Range<usize>,
    pub y: f64,
    pub image_height: f64,
    pub block_height: f64,
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
    target_height: f64,
    caption_height: f64,
}

fn ratio(value: f64) -> f64 {
    if value.is_finite() && value > 0.0 {
        value
    } else {
        1.0
    }
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
            viewport_width.max(1.0)
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
            caption_height.max(0.0)
        } else {
            0.0
        };
        let mut layout = Self {
            lookup: vec![None; aspect_ratios.len()],
            target_height: target,
            caption_height: caption,
            ..Self::default()
        };
        let mut y = 0.0;
        for (section_index, section) in sections.iter().enumerate() {
            let header_y = y;
            let header = if section.header_height.is_finite() {
                section.header_height.max(0.0)
            } else {
                0.0
            };
            y += header;
            let first_photo_y = y;
            let mut start = section.photo_range.start.min(aspect_ratios.len());
            let section_end = section.photo_range.end.min(aspect_ratios.len());
            while start < section_end {
                let mut end = start;
                let mut sum = 0.0;
                // Stop on reaching the target width. A single wide image still
                // produces a row, and tiny viewports never gain negative space.
                while end < section_end {
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
                let height = if crossed {
                    fitted(sum, end - start)
                } else {
                    target.min(fitted(sum, end - start))
                };
                let row_index = layout.rows.len();
                let item_start = layout.items.len();
                let mut x = margin;
                for photo_index in start..end {
                    let aspect_ratio = ratio(aspect_ratios[photo_index]);
                    let width = height * aspect_ratio;
                    layout.lookup[photo_index] = Some(layout.items.len());
                    layout.items.push(PhotoWallItem {
                        photo_index,
                        section: section_index,
                        row: row_index,
                        x,
                        y,
                        width,
                        height,
                        aspect_ratio,
                    });
                    x += width + WALL_GAP;
                }
                layout.rows.push(PhotoWallRow {
                    item_range: item_start..layout.items.len(),
                    y,
                    image_height: height,
                    block_height: height + caption,
                    justified: crossed,
                });
                y += height + caption + WALL_GAP;
                start = end;
            }
            layout.sections.push(PhotoWallSectionBounds {
                header_y,
                first_photo_y,
                end_y: y,
            });
        }
        layout.total_height = y.max(1.0);
        layout
    }

    /// Fit the established row partition to a new viewport. Membership,
    /// photo order, section boundaries and the final-row policy stay fixed.
    /// Ratios are retained from partitioning, so repeated fits do not derive
    /// new ratios from rounded or previously fitted rectangles.
    pub fn refit(&mut self, viewport_width: f64) {
        let viewport = if viewport_width.is_finite() {
            viewport_width.max(1.0)
        } else {
            1.0
        };
        let margin = WALL_SIDE_MARGIN.min((viewport - 1.0) * 0.5);
        let usable = viewport - 2.0 * margin;
        let mut y = 0.0;
        let mut row_index = 0;
        for (section_index, section) in self.sections.iter_mut().enumerate() {
            let header_height = section.first_photo_y - section.header_y;
            section.header_y = y;
            y += header_height;
            section.first_photo_y = y;
            while let Some(row) = self.rows.get_mut(row_index) {
                let items = &mut self.items[row.item_range.clone()];
                if items[0].section != section_index {
                    break;
                }
                // An established row cannot shed members in a tiny viewport.
                // Reduce gaps if necessary to retain positive image space.
                let gap = WALL_GAP.min(usable / items.len() as f64);
                let sum = items.iter().map(|item| item.aspect_ratio).sum::<f64>();
                let fitted = (usable - gap * items.len().saturating_sub(1) as f64) / sum;
                let height = if row.justified {
                    fitted
                } else {
                    self.target_height.min(fitted)
                };
                let mut x = margin;
                for item in items {
                    item.x = x;
                    item.y = y;
                    item.width = height * item.aspect_ratio;
                    item.height = height;
                    x += item.width + gap;
                }
                row.y = y;
                row.image_height = height;
                row.block_height = height + self.caption_height;
                y += row.block_height + WALL_GAP;
                row_index += 1;
            }
            section.end_y = y;
        }
        self.total_height = y.max(1.0);
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
    fn completed_rows_fill_width() {
        let l = layout(&[1.5; 8], 800.0, 180.0);
        for row in &l.rows[..l.rows.len() - 1] {
            let last = &l.items[row.item_range.end - 1];
            assert!((last.x + last.width - 800.0).abs() < 0.001);
            for item in &l.items[row.item_range.clone()] {
                assert!((item.width / item.height - 1.5).abs() < 0.001);
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
    fn width_refit_keeps_row_membership_through_repeated_resizes() {
        let ratios = (0..120)
            .map(|i| [0.5, 1.0, 1.5, 2.0][i % 4])
            .collect::<Vec<_>>();
        let mut l = PhotoWallLayout::calculate(
            &ratios,
            &[
                PhotoWallSection {
                    photo_range: 0..53,
                    header_height: 70.0,
                },
                PhotoWallSection {
                    photo_range: 53..53,
                    header_height: 35.0,
                },
                PhotoWallSection {
                    photo_range: 53..120,
                    header_height: 70.0,
                },
            ],
            1400.0,
            180.0,
            24.0,
        );
        let original = l.clone();
        let membership = l
            .rows
            .iter()
            .map(|row| row.item_range.clone())
            .collect::<Vec<_>>();
        for width in [1400.0, 1395.0, 1380.0, 1300.0, 1000.0, 1200.0, 1400.0] {
            l.refit(width);
            assert_eq!(
                l.rows
                    .iter()
                    .map(|row| row.item_range.clone())
                    .collect::<Vec<_>>(),
                membership
            );
            assert_eq!(l.items.len(), ratios.len());
            for (item, before) in l.items.iter().zip(&original.items) {
                assert_eq!(
                    (item.photo_index, item.row, item.section),
                    (before.photo_index, before.row, before.section)
                );
                assert!([item.x, item.y, item.width, item.height]
                    .iter()
                    .all(|v| v.is_finite()));
                assert!(item.width > 0.0 && item.height > 0.0);
                assert!(item.x >= 0.0 && item.x + item.width <= width + 1e-8);
                assert!((item.width / item.height - ratios[item.photo_index]).abs() < 1e-10);
                assert_eq!(l.item(item.photo_index).unwrap().row, item.row);
            }
            for (row, before) in l.rows.iter().zip(&original.rows) {
                let items = &l.items[row.item_range.clone()];
                assert!(items
                    .iter()
                    .all(|item| item.section == items[0].section && item.y == row.y));
                assert_eq!(row.block_height, row.image_height + 24.0);
                for pair in items.windows(2) {
                    assert_eq!(pair[0].x + pair[0].width + WALL_GAP, pair[1].x);
                }
                let before_last = &original.items[before.item_range.end - 1];
                if (before_last.x + before_last.width - 1400.0).abs() < 1e-8 {
                    let last = items.last().unwrap();
                    assert!((last.x + last.width - width).abs() < 1e-8);
                }
            }
            let mut previous_end = 0.0;
            for (section, header) in l.sections.iter().zip([70.0, 35.0, 70.0]) {
                assert_eq!(section.header_y, previous_end);
                assert!((section.first_photo_y - section.header_y - header).abs() < 1e-8);
                assert!(section.end_y >= section.first_photo_y);
                previous_end = section.end_y;
            }
            assert_eq!(l.total_height, previous_end);
        }
        for (item, before) in l.items.iter().zip(&original.items) {
            for (actual, expected) in [
                (item.x, before.x),
                (item.y, before.y),
                (item.width, before.width),
                (item.height, before.height),
            ] {
                assert!(
                    (actual - expected).abs() < 1e-8,
                    "round trip changed geometry"
                );
            }
        }
    }

    #[test]
    fn width_refit_preserves_incomplete_and_completed_final_row_policies() {
        let mut incomplete = layout(&[1.0, 1.0], 1000.0, 100.0);
        for (width, height) in [(1200.0, 100.0), (100.0, 50.0), (1000.0, 100.0)] {
            incomplete.refit(width);
            assert_eq!(incomplete.rows[0].item_range, 0..2);
            assert_eq!(incomplete.rows[0].image_height, height);
        }
        let mut completed = layout(&[1.0; 10], 1000.0, 100.0);
        assert_eq!(completed.rows.len(), 1);
        completed.refit(1400.0);
        assert_eq!(completed.rows[0].item_range, 0..10);
        assert!((completed.rows[0].image_height - 140.0).abs() < 1e-8);
        let last = completed.items.last().unwrap();
        assert!((last.x + last.width - 1400.0).abs() < 1e-8);
    }

    #[test]
    fn width_refit_handles_narrow_invalid_widths_and_ratios() {
        let ratios = [100.0, 0.01, 0.0, -1.0, f64::NAN, f64::INFINITY];
        let expected_ratios = [100.0, 0.01, 1.0, 1.0, 1.0, 1.0];
        let mut l = layout(&ratios, 1400.0, 180.0);
        let membership = l
            .rows
            .iter()
            .map(|row| row.item_range.clone())
            .collect::<Vec<_>>();
        for width in [1.0, 43.0, 50.0, 1400.0, 0.0, -1.0, f64::NAN, f64::INFINITY] {
            l.refit(width);
            let viewport = if width.is_finite() {
                width.max(1.0)
            } else {
                1.0
            };
            assert_eq!(
                l.rows
                    .iter()
                    .map(|row| row.item_range.clone())
                    .collect::<Vec<_>>(),
                membership
            );
            for item in &l.items {
                assert!([item.x, item.y, item.width, item.height]
                    .iter()
                    .all(|v| v.is_finite()));
                assert!(item.width > 0.0 && item.height > 0.0);
                assert!(item.x + item.width <= viewport + 1e-8);
                assert!(
                    (item.width / item.height - expected_ratios[item.photo_index]).abs() < 1e-8
                );
            }
        }
        let mut empty = PhotoWallLayout::calculate(
            &[],
            &[PhotoWallSection {
                photo_range: 0..0,
                header_height: 70.0,
            }],
            800.0,
            100.0,
            0.0,
        );
        empty.refit(30.0);
        assert!(empty.rows.is_empty() && empty.items.is_empty());
        assert_eq!(empty.sections[0].first_photo_y, 70.0);
        assert_eq!(empty.sections[0].end_y, 70.0);
        assert_eq!(empty.total_height, 70.0);
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
