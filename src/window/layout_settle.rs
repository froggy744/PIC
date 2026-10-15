/// Coalesces a short burst of width changes (for example an animated sidebar)
/// into one layout update after the width has stopped changing for a few
/// frames. When inactive, normal responsive layout updates pass straight
/// through.
#[derive(Debug)]
pub(super) struct WidthSettleGate {
    active: bool,
    last_width: i32,
    stable_frames: u8,
    required_stable_frames: u8,
}

impl WidthSettleGate {
    pub(super) fn new(required_stable_frames: u8) -> Self {
        Self {
            active: false,
            last_width: 0,
            stable_frames: 0,
            required_stable_frames: required_stable_frames.max(1),
        }
    }

    /// Start coalescing width changes until the allocation has settled.
    pub(super) fn begin(&mut self) {
        self.active = true;
        self.last_width = 0;
        self.stable_frames = 0;
    }

    /// Cancel a pending settle (used when the explicit divider-drag path takes
    /// ownership of resizing).
    pub(super) fn cancel(&mut self) {
        self.active = false;
        self.last_width = 0;
        self.stable_frames = 0;
    }

    /// Returns true when the caller should apply the current layout width.
    pub(super) fn observe(&mut self, width: i32) -> bool {
        if !self.active {
            return true;
        }

        if width != self.last_width {
            self.last_width = width;
            self.stable_frames = 0;
            return false;
        }

        self.stable_frames = self.stable_frames.saturating_add(1);
        if self.stable_frames < self.required_stable_frames {
            return false;
        }

        self.cancel();
        true
    }

    #[cfg(test)]
    fn is_active(&self) -> bool {
        self.active
    }
}

/// Width-driven gallery reflow is allowed only when neither an explicit
/// sidebar divider drag nor a temporary hover-overlay animation owns the
/// allocation. Pin/unpin animations use `WidthSettleGate` instead: they are
/// allowed to finish, then produce one final responsive update.
pub(super) fn should_observe_width(divider_drag_active: bool, hover_overlay_active: bool) -> bool {
    !divider_drag_active && !hover_overlay_active
}

/// Only the virtualized Folder Grid uses a column-dependent vertical row
/// model. Coalesce inspector slides there; never change Photo Wall/Masonry or
/// interfere with ordinary gallery/window resizing.
pub(super) fn should_defer_inspector_folder_grid_reflow(
    inspector_transition_active: bool,
    is_grid: bool,
    is_sectioned_folder: bool,
) -> bool {
    inspector_transition_active && is_grid && is_sectioned_folder
}

#[cfg(test)]
mod tests {
    use super::WidthSettleGate;

    #[test]
    fn waits_for_stable_frames_before_reflowing() {
        let mut gate = WidthSettleGate::new(3);
        gate.begin();

        assert!(!gate.observe(1200));
        assert!(!gate.observe(1160));
        assert!(!gate.observe(1120));
        assert!(!gate.observe(1120));
        assert!(!gate.observe(1120));
        assert!(gate.observe(1120));
        assert!(!gate.is_active());
    }

    #[test]
    fn width_changes_reset_the_stability_count() {
        let mut gate = WidthSettleGate::new(2);
        gate.begin();

        assert!(!gate.observe(1000));
        assert!(!gate.observe(1000));
        assert!(!gate.observe(980));
        assert!(!gate.observe(980));
        assert!(gate.observe(980));
    }

    #[test]
    fn inactive_gate_allows_normal_layout_updates() {
        let mut gate = WidthSettleGate::new(3);
        assert!(gate.observe(900));
        assert!(gate.observe(880));
    }
    #[test]
    fn inspector_slide_coalesces_only_folder_grid() {
        use super::should_defer_inspector_folder_grid_reflow as defer;
        assert!(defer(true, true, true), "Folder Grid must settle once");
        assert!(!defer(false, true, true), "ordinary resize remains live");
        assert!(!defer(true, false, true), "Photo Wall/Masonry stays live");
        assert!(!defer(true, true, false), "regular GtkGridView stays live");
    }

    #[test]
    fn hover_overlay_freezes_width_reflow() {
        assert!(!super::should_observe_width(false, true));
        assert!(!super::should_observe_width(true, false));
        assert!(super::should_observe_width(false, false));
    }
}
