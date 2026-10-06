//! Reproduce deep-scroll resize movement using the production layout engines.
//! This is a diagnostic, not a fix or a GTK frame-time benchmark.
//!
//! From the repository root:
//! rustc --edition=2021 -C opt-level=1 scripts/diagnostics/masonry-resize-probe.rs -o /tmp/pic-masonry-resize-probe
//! /tmp/pic-masonry-resize-probe

#![allow(dead_code)]

mod grid {
    pub mod photo_wall_layout {
        include!("../../src/grid/photo_wall_layout.rs");
    }
    pub mod masonry_layout {
        include!("../../src/grid/masonry_layout.rs");
    }

    pub fn run() {
        use photo_wall_layout::{PhotoWallLayout, PhotoWallSection};

        for count in [3_086, 20_000] {
            let ratios = (0..count)
                .map(|i| [0.5, 1.0, 1.5, 2.0][i % 4])
                .collect::<Vec<_>>();
            let sections = [PhotoWallSection {
                photo_range: 0..count,
                header_height: 0.0,
            }];
            let initial = PhotoWallLayout::calculate_masonry(&ratios, &sections, 997, 100);
            for depth in [0.0, 0.4, 0.9] {
                let top = initial.total_height * depth;
                let before = initial
                    .visible_row_indices(top, top + 600.0)
                    .into_iter()
                    .map(|r| {
                        let tile = &initial.items[r];
                        (tile.photo_index, tile.y - top)
                    })
                    .collect::<Vec<_>>();

                let mut resized = initial.clone();
                let mapped = resized.resize_masonry(998, top).unwrap();
                let worst_move = before
                    .iter()
                    .map(|&(id, y)| (resized.item(id).unwrap().y - mapped - y).abs())
                    .fold(0.0, f64::max);

                // In this exact 997 -> 998 fixture all column left edges remain
                // unchanged, so a changed x identifies a changed column assignment.
                let repacked = PhotoWallLayout::calculate_masonry(&ratios, &sections, 998, 100);
                let old_columns = resized
                    .items
                    .iter()
                    .map(|t| t.x as i32)
                    .collect::<std::collections::BTreeSet<_>>();
                let new_columns = repacked
                    .items
                    .iter()
                    .map(|t| t.x as i32)
                    .collect::<std::collections::BTreeSet<_>>();
                assert_eq!(old_columns, new_columns);
                let reassigned = resized
                    .items
                    .iter()
                    .filter(|t| repacked.item(t.photo_index).unwrap().x != t.x)
                    .count();

                let wall = PhotoWallLayout::calculate(&ratios, &sections, 997.0, 100.0, 0.0);
                let wall_top = wall.total_height * depth;
                let wall_before = wall
                    .visible_row_indices(wall_top, wall_top + 600.0)
                    .into_iter()
                    .flat_map(|r| wall.items[wall.rows[r].item_range.clone()].iter())
                    .map(|t| (t.photo_index, t.y - wall_top))
                    .collect::<Vec<_>>();
                let mut wall_after = wall.clone();
                let wall_mapped = wall_after.scale_photo_area(998.0 / 997.0, wall_top);
                let wall_worst_move = wall_before
                    .iter()
                    .map(|&(id, y)| (wall_after.item(id).unwrap().y - wall_mapped - y).abs())
                    .fold(0.0, f64::max);

                println!(
                "photos={count} width=997->998 depth={:.0}% masonry_visible_move_px={worst_move:.3} photo_wall_visible_move_px={wall_worst_move:.3} same_column_count_repack_reassigned={reassigned}", depth * 100.0
            );
            }
        }
    }
}

fn main() {
    grid::run();
}
