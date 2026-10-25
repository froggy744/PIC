//! Shared layout and GSK drawing for album and photo Cover Flow scenes.
use gtk::prelude::*;
use gtk::{gdk, graphene, gsk};
use gtk4 as gtk;

pub(crate) const VISIBLE_RADIUS: usize = 4;

#[derive(Clone, Copy, Debug)]
pub(crate) struct CoverflowPose {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub depth: f64,
    pub opacity: f64,
}

/// Returns only the small set of covers that can appear in the viewport.
pub(crate) fn coverflow_pose(
    index: usize,
    focus: f64,
    count: usize,
    viewport: (f64, f64),
) -> Option<CoverflowPose> {
    let (vw, vh) = viewport;
    if count == 0 || index >= count || vw <= 0.0 || vh <= 0.0 {
        return None;
    }
    let offset = index as f64 - focus.clamp(0.0, (count - 1) as f64);
    let distance = offset.abs();
    if distance > VISIBLE_RADIUS as f64 + 0.5 {
        return None;
    }
    let base_size = (vw * 0.36).min(vh * 0.55).clamp(1.0, 420.0);
    let width = base_size * 0.82_f64.powf(distance);
    let height = width;
    let shift = offset * base_size * 0.52;
    let x = vw * 0.5 + shift - width * 0.5;
    let base_y = ((vh - base_size * 1.42) * 0.5).max(8.0);
    let y = base_y + base_size - height;
    Some(CoverflowPose {
        x,
        y,
        width,
        height,
        depth: 1.0 / (1.0 + distance * 0.38),
        opacity: (1.0 - 0.13 * distance).max(0.35),
    })
}

/// Draw a retained node once, then reuse it through a smooth reflection mask.
pub(crate) fn paint_cover(
    snapshot: &gtk::Snapshot,
    node: &gsk::RenderNode,
    pose: CoverflowPose,
    reflection_strength: f64,
    shadow: &gdk::RGBA,
) {
    let source = node.bounds();
    if source.width() <= 0.0 || source.height() <= 0.0 {
        return;
    }
    let box_w = pose.width as f32;
    let box_h = pose.height as f32;
    let source_w = source.width();
    let source_h = source.height();
    let scale = (box_w / source_w).min(box_h / source_h);
    let w = source_w * scale;
    let h = source_h * scale;
    let x = (box_w - w) * 0.5;
    let y = (box_h - h) * 0.5;
    snapshot.save();
    snapshot.translate(&graphene::Point::new(pose.x as f32, pose.y as f32));
    snapshot.translate(&graphene::Point::new(x, y));
    snapshot.push_opacity(pose.opacity);
    let rect = graphene::Rect::new(0.0, 0.0, w, h);
    let outline = gsk::RoundedRect::from_rect(rect, 3.0);
    let shadow = gdk::RGBA::new(
        shadow.red(),
        shadow.green(),
        shadow.blue(),
        shadow.alpha() * pose.depth as f32,
    );
    snapshot.append_outset_shadow(&outline, &shadow, 0.0, 12.0, 0.0, 22.0);
    snapshot.save();
    snapshot.scale(w / source_w, h / source_h);
    snapshot.translate(&graphene::Point::new(-source.x(), -source.y()));
    snapshot.append_node(node);
    snapshot.restore();

    let reflection = reflection_strength.clamp(0.0, 1.0);
    if reflection > 0.0 {
        let reflected_height = h * 0.42;
        let top = h + 5.0;
        snapshot.save();
        snapshot.push_clip(&graphene::Rect::new(0.0, top, w, reflected_height));
        snapshot.push_mask(gsk::MaskMode::Alpha);
        snapshot.append_linear_gradient(
            &graphene::Rect::new(0.0, top, w, reflected_height),
            &graphene::Point::new(0.0, top),
            &graphene::Point::new(0.0, top + reflected_height),
            &[
                gsk::ColorStop::new(
                    0.0,
                    gdk::RGBA::new(1.0, 1.0, 1.0, (reflection * 0.38) as f32),
                ),
                gsk::ColorStop::new(1.0, gdk::RGBA::new(1.0, 1.0, 1.0, 0.0)),
            ],
        );
        snapshot.pop();
        snapshot.translate(&graphene::Point::new(0.0, h * 2.0 + 5.0));
        snapshot.scale(w / source_w, -h / source_h);
        snapshot.translate(&graphene::Point::new(-source.x(), -source.y()));
        snapshot.append_node(node);
        snapshot.pop();
        snapshot.pop();
        snapshot.restore();
    }
    snapshot.pop();
    snapshot.restore();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn center_and_neighbors_have_stable_depth_and_bounded_width() {
        let center = coverflow_pose(4, 4.0, 9, (1200.0, 700.0)).unwrap();
        let left = coverflow_pose(3, 4.0, 9, (1200.0, 700.0)).unwrap();
        let right = coverflow_pose(5, 4.0, 9, (1200.0, 700.0)).unwrap();
        assert!((center.x + center.width / 2.0 - 600.0).abs() < 1.0);
        assert!(left.x < center.x && right.x > center.x);
        assert!(center.depth > left.depth && center.depth > right.depth);
        let far = coverflow_pose(2, 4.0, 9, (1200.0, 700.0)).unwrap();
        assert!(center.width > left.width && left.width > far.width);
        assert_eq!(center.width, center.height);
        assert_eq!(left.width, left.height);
    }

    #[test]
    fn empty_single_and_distant_items_stay_safe() {
        assert!(coverflow_pose(0, 0.0, 0, (800.0, 600.0)).is_none());
        assert!(coverflow_pose(9, 0.0, 1, (800.0, 600.0)).is_none());
        let only = coverflow_pose(0, 0.0, 1, (320.0, 460.0)).unwrap();
        assert!(only.x >= 0.0 && only.x + only.width <= 320.0);
        assert!(coverflow_pose(9, 0.0, 20, (800.0, 600.0)).is_none());
    }

    #[test]
    fn fractional_focus_moves_without_a_jump() {
        let at_four = coverflow_pose(4, 4.0, 9, (1200.0, 700.0)).unwrap();
        let halfway = coverflow_pose(4, 4.5, 9, (1200.0, 700.0)).unwrap();
        let at_five = coverflow_pose(4, 5.0, 9, (1200.0, 700.0)).unwrap();
        assert!(at_four.x > halfway.x && halfway.x > at_five.x);
        assert!(at_four.depth > halfway.depth && halfway.depth > at_five.depth);
    }

    #[test]
    #[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
    fn painting_a_reflection_extends_below_the_cover() {
        gtk::init().unwrap();
        let source = gtk::Snapshot::new();
        source.append_color(
            &gdk::RGBA::new(0.8, 0.2, 0.1, 1.0),
            &graphene::Rect::new(0.0, 0.0, 100.0, 120.0),
        );
        let node = source.to_node().unwrap();
        let pose = coverflow_pose(0, 0.0, 1, (900.0, 700.0)).unwrap();
        let scene = gtk::Snapshot::new();
        paint_cover(
            &scene,
            &node,
            pose,
            0.6,
            &gdk::RGBA::new(0.0, 0.0, 0.0, 0.4),
        );
        let drawn = scene
            .to_node()
            .expect("cover and reflection should be painted");
        assert!(drawn.bounds().height() > pose.height as f32 + 30.0);
    }
}
