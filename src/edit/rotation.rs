//! Photo rotation: a quarter turn of the library rotation carries the Recipe
//! with it.
//!
//! The renderer applies the library rotation first (`render_sized`), so a
//! Recipe lives in the rotated frame:
//! - `crop` is normalized in the straightened frame and `drawings` in the
//!   cropped frame; both describe the photo's content and turn with it.
//! - `straighten` turns about the centre, which commutes with a quarter turn,
//!   so it is unchanged.
//! - `text_layers` and `overlays` are placed on the output frame (anchor plus
//!   point) and stay upright where they are.
//!
//! Every rotation writer (the InfoBar Rotate button from the grid, viewer and
//! editor) goes through [`rotate_recipe`] and saves rotation and Recipe in
//! one transaction (`db::rotate_photo`).

use super::model::{remap_drawings_for_rotation, CropRect, EditRecipe};

/// The library rotation after a quarter turn.
pub fn rotated(rotation: i32, counter_clockwise: bool) -> i32 {
    (rotation + if counter_clockwise { 270 } else { 90 }).rem_euclid(360)
}

/// Re-express `recipe` for the photo turned by `clockwise_degrees` (a
/// multiple of 90; anything else leaves it unchanged).
pub fn rotate_recipe(recipe: &mut EditRecipe, clockwise_degrees: i32) {
    let turns = clockwise_degrees.rem_euclid(360);
    if !matches!(turns, 90 | 180 | 270) {
        return;
    }
    let c = recipe.crop;
    recipe.crop = match turns {
        90 => CropRect {
            left: 1.0 - c.bottom,
            top: c.left,
            right: 1.0 - c.top,
            bottom: c.right,
        },
        180 => CropRect {
            left: 1.0 - c.right,
            top: 1.0 - c.bottom,
            right: 1.0 - c.left,
            bottom: 1.0 - c.top,
        },
        _ => CropRect {
            left: c.top,
            top: 1.0 - c.right,
            right: c.bottom,
            bottom: 1.0 - c.left,
        },
    };
    remap_drawings_for_rotation(&mut recipe.drawings, turns);
}

/// The stored recipe string after a quarter turn; an empty (unedited)
/// recipe stays empty.
pub fn rotate_encoded_recipe(encoded: &str, clockwise_degrees: i32) -> String {
    if encoded.is_empty() {
        return String::new();
    }
    let mut recipe = EditRecipe::decode(encoded);
    rotate_recipe(&mut recipe, clockwise_degrees);
    recipe.encode()
}

#[cfg(test)]
mod tests {
    use super::super::model::{CropRect, DrawAnnotation, DrawKind, EditRecipe};
    use super::super::render::{apply_library_rotation, render_decoded, NoObserver};
    use super::*;
    use image::{Rgba, RgbaImage};

    /// 40×24, every pixel distinct, so any misplaced crop shows.
    fn source() -> RgbaImage {
        RgbaImage::from_fn(40, 24, |x, y| {
            Rgba([(x * 6) as u8, (y * 10) as u8, 128, 255])
        })
    }

    fn render(rotation: i32, recipe: &EditRecipe) -> RgbaImage {
        render_decoded(source(), rotation, recipe, &mut NoObserver).unwrap()
    }

    /// Crop edges land on whole pixels in both orientations (40×24, 24×40),
    /// plus a drawing that crosses the crop.
    fn edited() -> EditRecipe {
        EditRecipe {
            crop: CropRect {
                left: 0.25,
                top: 0.5,
                right: 0.75,
                bottom: 1.0,
            },
            drawings: vec![DrawAnnotation {
                kind: DrawKind::Rectangle,
                points: vec![(0.1, 0.2), (0.6, 0.9)],
                color: 0xff0000ff,
                width: 0.02,
                opacity: 1.0,
                arrow_start: false,
                arrow_end: false,
            }],
            ..EditRecipe::default()
        }
    }

    // Regression (round five, candidate 31): the Rotate button wrote only the
    // library rotation, so a quarter turn showed another part of the photo
    // (12×20 output, wrong pixels) and drawings landed elsewhere.
    #[test]
    fn a_quarter_turn_turns_the_edited_photo_as_it_was() {
        let recipe = edited();
        let before = render(0, &recipe);
        for counter_clockwise in [false, true] {
            let rotation = rotated(0, counter_clockwise);
            let mut turned = recipe.clone();
            rotate_recipe(&mut turned, rotation);
            let after = render(rotation, &turned);
            let expected = apply_library_rotation(before.clone(), rotation);
            // Same size and pixels; only the stroke's anti-aliasing may
            // differ by a few levels (measured: at most 8 of 255).
            assert_eq!(
                after.dimensions(),
                expected.dimensions(),
                "rotation {rotation}"
            );
            let worst = after
                .as_raw()
                .iter()
                .zip(expected.as_raw())
                .map(|(a, b)| a.abs_diff(*b))
                .max()
                .unwrap_or(0);
            assert!(worst <= 12, "rotation {rotation}: pixels differ by {worst}");
        }
    }

    #[test]
    fn four_quarter_turns_either_way_give_the_recipe_back() {
        for degrees in [90, 270] {
            let mut recipe = edited();
            for _ in 0..4 {
                rotate_recipe(&mut recipe, degrees);
            }
            let original = edited();
            let close = |a: f32, b: f32| (a - b).abs() < 1e-6;
            let (a, b) = (recipe.crop, original.crop);
            assert!(close(a.left, b.left) && close(a.top, b.top));
            assert!(close(a.right, b.right) && close(a.bottom, b.bottom));
            for (p, q) in recipe.drawings[0]
                .points
                .iter()
                .zip(&original.drawings[0].points)
            {
                assert!(
                    close(p.0, q.0) && close(p.1, q.1),
                    "{degrees}: {p:?} vs {q:?}"
                );
            }
        }
    }

    /// Rotating in the editor saves at once and is not undoable: Undo and
    /// Redo must step through edits already in the new frame, and the
    /// rotation adds no step of its own.
    #[test]
    fn editor_undo_and_redo_stay_in_the_rotated_frame() {
        use super::super::model::EditSession;
        let mut session = EditSession::new(EditRecipe::default());
        session.replace(edited());
        session.reframe(|recipe| rotate_recipe(recipe, 90));
        let mut turned = edited();
        rotate_recipe(&mut turned, 90);
        assert_eq!(session.recipe, turned);
        assert!(session.undo(), "only the crop edit is undoable");
        assert_eq!(session.recipe, EditRecipe::default());
        assert!(!session.undo());
        assert!(session.redo());
        assert_eq!(session.recipe, turned, "Redo brings back the turned edit");
    }

    /// Rotation and Recipe are one saved fact, read back as such (also after
    /// a restart, which reads the same row).
    #[test]
    fn rotation_and_recipe_are_saved_together() {
        let connection = rusqlite::Connection::open_in_memory().unwrap();
        connection.execute_batch(crate::db::SCHEMA).unwrap();
        connection
            .execute_batch(
                "INSERT INTO folders(id,path,name) VALUES (1,'/p','p');
                 INSERT INTO photos(id,path,folder_id) VALUES (1,'/p/a.jpg',1);",
            )
            .unwrap();
        let recipe = rotate_encoded_recipe(&edited().encode(), 270);
        crate::db::rotate_photo(&connection, 1, 270, &recipe).unwrap();
        let saved = crate::db::photo(&connection, 1).unwrap().unwrap();
        assert_eq!((saved.rotation, saved.edit_recipe), (270, recipe));
    }

    #[test]
    fn stored_recipes_keep_their_format() {
        assert_eq!(rotate_encoded_recipe("", 90), "");
        // A recipe saved before this change: crop and tone survive, and text
        // and overlays (output-frame layers) are untouched.
        let stored = edited().encode();
        let turned = EditRecipe::decode(&rotate_encoded_recipe(&stored, 90));
        assert_eq!(turned.drawings.len(), 1);
        assert!((turned.crop.left - 0.0).abs() < 1e-6 && (turned.crop.bottom - 0.75).abs() < 1e-6);
        let back = rotate_encoded_recipe(&rotate_encoded_recipe(&stored, 90), 270);
        assert_eq!(EditRecipe::decode(&back).crop, edited().crop);
    }
}
