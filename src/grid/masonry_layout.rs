use super::photo_wall_layout::{PhotoWallItem, PhotoWallSection, PhotoWallSectionBounds};

// Shared horizontal and vertical tile spacing, in logical pixels.
pub(super) const MASONRY_GAP: usize = 8;

#[derive(Default)]
pub(super) struct MasonryLayout {
    pub items: Vec<PhotoWallItem>,
    pub sections: Vec<PhotoWallSectionBounds>,
    pub total_height: f64,
    pub column_count: usize,
    pub column_x: Vec<f64>,
}

impl MasonryLayout {
    pub fn calculate(
        ratios: &[f64],
        sections: &[PhotoWallSection],
        viewport: i32,
        preferred_width: i32,
    ) -> Self {
        let gap = MASONRY_GAP as f64;
        let viewport = viewport.max(1);
        let side = MASONRY_GAP.min((viewport as usize - 1) / 2);
        let content_width = viewport as usize - 2 * side;
        let preferred = preferred_width.max(1).min(content_width as i32);
        // Zoom specifies the preferred width. Fit the closest column count
        // across the viewport, keeping widths fixed within each column.
        let columns = ((content_width as f64 + gap) / (f64::from(preferred) + gap))
            .round()
            .max(1.0) as usize;
        let columns = columns.min((content_width + MASONRY_GAP) / (MASONRY_GAP + 1));
        let available = content_width - (columns - 1) * MASONRY_GAP;
        // All columns share one ideal scale. Their one-pixel horizontal
        // remainder must not change hundreds of preceding image heights.
        let ideal_width = available as f64 / columns as f64;
        let base = available / columns;
        let remainder = available - base * columns;
        let widths: Vec<_> = (0..columns)
            .map(|column| base + usize::from(column < remainder))
            .collect();
        let mut x = side as f64;
        let column_x = widths
            .iter()
            .map(|&width| {
                let left = x;
                x += width as f64 + gap;
                left
            })
            .collect();
        let mut layout = Self {
            column_count: columns,
            column_x,
            ..Self::default()
        };
        let mut y = 0.0_f64;
        for (section, source) in sections.iter().enumerate() {
            let header_y = y.round();
            let header = if source.header_height.is_finite() {
                source.header_height.round().max(0.0)
            } else {
                0.0
            };
            y += header;
            let first_photo_y = y.round();
            let mut bottoms = vec![y; columns];
            let mut tops = vec![first_photo_y; columns];
            let start = layout.items.len();
            for photo_index in
                source.photo_range.start.min(ratios.len())..source.photo_range.end.min(ratios.len())
            {
                let column = (0..columns)
                    .min_by(|&a, &b| bottoms[a].total_cmp(&bottoms[b]))
                    .unwrap();
                let ratio = ratios[photo_index];
                let ratio = if ratio.is_finite() && ratio > 0.0 {
                    ratio
                } else {
                    1.0
                };
                let width = widths[column] as f64;
                let ideal_height = (ideal_width / ratio).max(1.0);
                let top = tops[column];
                let height = (bottoms[column] + ideal_height).round() - top;
                layout.items.push(PhotoWallItem {
                    photo_index,
                    section,
                    row: 0,
                    x: layout.column_x[column],
                    y: top,
                    width,
                    height,
                });
                tops[column] = top + height + gap;
                bottoms[column] += ideal_height + gap;
            }
            if layout.items.len() > start {
                y = bottoms.into_iter().fold(y, f64::max) - gap;
            }
            layout.sections.push(PhotoWallSectionBounds {
                header_y,
                first_photo_y,
                end_y: y.round(),
            });
        }
        layout.total_height = y.round().max(1.0);
        layout
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn layout(ratios: &[f64], viewport: i32) -> MasonryLayout {
        MasonryLayout::calculate(
            ratios,
            &[PhotoWallSection {
                photo_range: 0..ratios.len(),
                header_height: 0.0,
            }],
            viewport,
            100,
        )
    }
    #[test]
    fn mixed_images_use_shortest_column_and_keep_aspect() {
        let wall = layout(&[0.5, 2.0, 1.0, 1.0], 224);
        let positions = wall
            .items
            .iter()
            .map(|tile| (tile.x, tile.y, tile.width, tile.height))
            .collect::<Vec<_>>();
        assert_eq!(
            positions,
            vec![
                (8.0, 0.0, 100.0, 200.0),
                (116.0, 0.0, 100.0, 50.0),
                (116.0, 58.0, 100.0, 100.0),
                (116.0, 166.0, 100.0, 100.0)
            ]
        );
        assert_eq!(wall.total_height, 266.0);
    }
    #[test]
    fn resize_changes_columns_and_handles_narrow_width() {
        for (width, columns, tile_width) in [(332, 3, 100.0), (224, 2, 100.0), (80, 1, 64.0)] {
            let wall = layout(&[1.0; 6], width);
            assert_eq!(wall.column_count, columns);
            assert!(wall
                .items
                .iter()
                .all(|tile| tile.width == tile_width && tile.x + tile.width <= f64::from(width)));
        }
    }
    #[test]
    fn large_mixed_library_has_no_overlapping_tiles() {
        let ratios = (0..20_000)
            .map(|i| [0.5, 1.0, 1.5, 2.0][i % 4])
            .collect::<Vec<_>>();
        let wall = layout(&ratios, 997);
        assert_eq!(wall.items.len(), ratios.len());
        let mut bottoms = vec![0.0; wall.column_count];
        for tile in &wall.items {
            let column = wall.items[..wall.column_count]
                .iter()
                .position(|first| first.x == tile.x)
                .unwrap();
            assert!(tile.y >= bottoms[column]);
            assert!(
                (tile.height * ratios[tile.photo_index] - tile.width).abs()
                    <= 1.0 + ratios[tile.photo_index]
            );
            bottoms[column] = tile.y + tile.height + MASONRY_GAP as f64;
        }
    }
    #[test]
    fn viewport_query_does_not_materialize_a_library_under_one_tall_photo() {
        let mut ratios = vec![1.0; 20_000];
        ratios[0] = 0.0001;
        let sections = [PhotoWallSection {
            photo_range: 0..ratios.len(),
            header_height: 0.0,
        }];
        let wall = super::super::photo_wall_layout::PhotoWallLayout::calculate_masonry(
            &ratios, &sections, 224, 100,
        );
        let rows = wall.visible_row_indices(50_000.0, 50_600.0);
        assert!(rows.len() <= 8);
        assert!(rows.iter().any(|&row| wall.items[row].photo_index == 0));
        for row in rows {
            let tile = &wall.items[row];
            assert!(tile.y < 50_600.0 && tile.y + tile.height > 50_000.0);
        }
    }
    #[test]
    fn zoom_and_resize_fill_width_instead_of_leaving_a_missing_column() {
        for (viewport, zoom, expected_columns) in [
            (1510, 219, 7),
            (997, 219, 4),
            (997, 100, 9),
            (701, 219, 3),
            (80, 100, 1),
        ] {
            let wall = MasonryLayout::calculate(
                &[1.0; 20],
                &[PhotoWallSection {
                    photo_range: 0..20,
                    header_height: 0.0,
                }],
                viewport,
                zoom,
            );
            assert_eq!(wall.column_count, expected_columns);
            let mut first = wall
                .items
                .iter()
                .filter(|tile| tile.y == 0.0)
                .collect::<Vec<_>>();
            first.sort_by(|a, b| a.x.total_cmp(&b.x));
            assert_eq!(first.len(), expected_columns);
            assert_eq!(first[0].x, MASONRY_GAP as f64);
            for pair in first.windows(2) {
                assert_eq!(pair[0].x + pair[0].width + MASONRY_GAP as f64, pair[1].x);
                assert!((pair[0].width - pair[1].width).abs() <= 1.0);
            }
            let last = first.last().unwrap();
            assert_eq!(
                last.x + last.width,
                f64::from(viewport) - MASONRY_GAP as f64
            );
        }
    }
}
