//! Regular pointy-top hexagons. Gap is measured perpendicular to every edge.
pub(super) const GAP: f64 = 2.0;

// GTK allocations are integral; fit a truly regular polygon inside them
// rather than stretching six vertices to the rounded rectangular bounds.
fn fitted_bounds(w: f64, h: f64) -> (f64, f64, f64, f64) {
    let radius = (w / 3.0_f64.sqrt()).min(h / 2.0);
    let width = radius * 3.0_f64.sqrt();
    let height = radius * 2.0;
    ((w - width) / 2.0, (h - height) / 2.0, width, height)
}
pub(super) fn vertices(w: f64, h: f64) -> [(f64, f64); 6] {
    let (x, y, w, h) = fitted_bounds(w, h);
    [
        (x + w / 2.0, y),
        (x + w, y + h / 4.0),
        (x + w, y + 3.0 * h / 4.0),
        (x + w / 2.0, y + h),
        (x, y + 3.0 * h / 4.0),
        (x, y + h / 4.0),
    ]
}
pub(super) fn contains(w: f64, h: f64, x: f64, y: f64) -> bool {
    let (ox, oy, w, h) = fitted_bounds(w, h);
    let (x, y) = (x - ox, y - oy);
    w > 0.0
        && h > 0.0
        && x >= 0.0
        && x <= w
        && y >= 0.0
        && y <= h
        && (x - w / 2.0).abs() / w * 0.5 <= y.min(h - y) / h
}
pub(super) fn metrics(viewport: f64, target: f64, gap: f64) -> (usize, f64, f64, f64, f64) {
    let viewport = viewport.max(1.0);
    let columns = (viewport / target.max(1.0)).round().max(1.0) as usize;
    let dx = viewport / columns as f64;
    let w = dx - gap.min(dx * 0.5);
    let h = 2.0 * w / 3.0_f64.sqrt();
    (columns, w, h, dx, 3.0_f64.sqrt() * dx / 2.0)
}

/// Separating-axis test; no allocations and no selection through transparent corners.
pub(super) fn intersects_rect(w: f64, h: f64, x: f64, y: f64, rw: f64, rh: f64) -> bool {
    if x > w || y > h || x + rw < 0.0 || y + rh < 0.0 {
        return false;
    }
    let hex = vertices(w, h);
    let rect = [(x, y), (x + rw, y), (x + rw, y + rh), (x, y + rh)];
    for i in 0..6 {
        let (ax, ay) = (hex[(i + 1) % 6].1 - hex[i].1, hex[i].0 - hex[(i + 1) % 6].0);
        let project = |(px, py): (f64, f64)| px * ax + py * ay;
        let min = hex
            .iter()
            .map(|&p| project(p))
            .fold(f64::INFINITY, f64::min);
        let max = hex
            .iter()
            .map(|&p| project(p))
            .fold(f64::NEG_INFINITY, f64::max);
        let rmin = rect
            .iter()
            .map(|&p| project(p))
            .fold(f64::INFINITY, f64::min);
        let rmax = rect
            .iter()
            .map(|&p| project(p))
            .fold(f64::NEG_INFINITY, f64::max);
        if rmax < min || rmin > max {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rubberband_does_not_select_empty_corners() {
        assert!(!intersects_rect(180.0, 208.0, 0.0, 0.0, 10.0, 10.0));
        assert!(intersects_rect(180.0, 208.0, 85.0, 0.0, 10.0, 10.0));
        assert!(intersects_rect(180.0, 208.0, -10.0, 80.0, 200.0, 10.0));
        assert!(!intersects_rect(180.0, 208.0, 185.0, 80.0, 10.0, 10.0));
    }
    #[test]
    fn integer_gtk_allocations_keep_regular_edges() {
        let p = vertices(181.0, 209.0);
        let edge = (p[0].0 - p[1].0).hypot(p[0].1 - p[1].1);
        for i in 0..6 {
            assert!(
                ((p[i].0 - p[(i + 1) % 6].0).hypot(p[i].1 - p[(i + 1) % 6].1) - edge).abs() < 1e-8
            );
        }
    }
    #[test]
    fn regular_shape_and_corner_hit_testing() {
        let h = 200.0;
        let w = h * 3.0_f64.sqrt() / 2.0;
        let p = vertices(w, h);
        for i in 0..6 {
            assert!(
                ((p[i].0 - p[(i + 1) % 6].0).hypot(p[i].1 - p[(i + 1) % 6].1) - 100.0).abs() < 1e-8
            );
        }
        assert!(contains(w, h, w / 2.0, 1.0));
        assert!(contains(w, h, 1.0, h / 2.0));
        assert!(!contains(w, h, 1.0, 1.0));
        assert!(!contains(w, h, w - 1.0, h - 1.0));
        assert!(!contains(w, h, -1.0, h / 2.0));
    }
    #[test]
    fn uniform_perpendicular_gaps_and_narrow_windows() {
        for viewport in [1.0, 60.0, 200.0, 800.0, 1600.0] {
            let (columns, w, h, dx, dy) = metrics(viewport, 180.0, GAP);
            assert!(columns >= 1);
            assert!(w <= viewport);
            assert!((w / h - 3.0_f64.sqrt() / 2.0).abs() < 1e-8);
            assert!((dx - w - GAP.min(dx * 0.5)).abs() < 1e-8);
            assert!((dy * 2.0 / 3.0_f64.sqrt() - dx).abs() < 1e-8);
            assert!((columns as f64 * dx - viewport).abs() < 1e-8);
        }
    }
}
