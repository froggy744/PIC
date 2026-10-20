#[derive(Clone, Copy)]
enum OneToOneAnchor {
    Cursor {
        normalized_x: f64,
        normalized_y: f64,
        pointer_x: f64,
        pointer_y: f64,
    },
    Center,
}

/// Scale from GTK widget units to physical display pixels. Prefer the
/// fractional surface scale; an unrealized widget has no surface yet, so use
/// GTK's integer widget scale as a fallback.
fn widget_device_scale(widget: &gtk::Widget) -> f64 {
    widget
        .root()
        .and_then(|root| root.surface())
        .map(|surface| surface.scale())
        .filter(|scale| scale.is_finite() && *scale > 0.0)
        .unwrap_or_else(|| widget.scale_factor().max(1) as f64)
}

fn logical_scale_for_physical(physical_scale: f64, device_scale: f64) -> f64 {
    physical_scale.max(0.0) / device_scale.max(f64::EPSILON)
}

fn logical_canvas_size(
    dimensions: (i32, i32),
    physical_scale: f64,
    device_scale: f64,
) -> (f64, f64) {
    let logical_scale = logical_scale_for_physical(physical_scale, device_scale);
    (
        dimensions.0.max(1) as f64 * logical_scale,
        dimensions.1.max(1) as f64 * logical_scale,
    )
}

fn rounded_canvas_size(size: (f64, f64)) -> (i32, i32) {
    (size.0.round().max(1.0) as i32, size.1.round().max(1.0) as i32)
}

fn fit_zoom_for_canvas(scroll: &gtk::ScrolledWindow, dimensions: (i32, i32)) -> f64 {
    let source_w = dimensions.0.max(1) as f64;
    let source_h = dimensions.1.max(1) as f64;
    let device_scale = widget_device_scale(scroll.upcast_ref());
    // Leave a small breathing margin so the first + click visibly enlarges
    // the photo instead of jumping straight from Fit to native 100%.
    let viewport_w = (scroll.width() - 24).max(1) as f64;
    let viewport_h = (scroll.height() - 24).max(1) as f64;
    ((viewport_w * device_scale) / source_w)
        .min((viewport_h * device_scale) / source_h)
        .min(1.0)
        .max(0.01)
}

fn canvas_size_for_zoom(
    scroll: &gtk::ScrolledWindow,
    dimensions: (i32, i32),
    zoom: f64,
) -> (f64, f64) {
    let fit = fit_zoom_for_canvas(scroll, dimensions);
    let effective_zoom = (fit * zoom).clamp(0.01, 8.0);
    logical_canvas_size(
        dimensions,
        effective_zoom,
        widget_device_scale(scroll.upcast_ref()),
    )
}

fn native_canvas_size(scroll: &gtk::ScrolledWindow, dimensions: (i32, i32)) -> (i32, i32) {
    rounded_canvas_size(logical_canvas_size(
        dimensions,
        1.0,
        widget_device_scale(scroll.upcast_ref()),
    ))
}

fn anchored_scroll_target(
    current_scroll: f64,
    pointer: f64,
    viewport: f64,
    old_content: f64,
    new_content: f64,
) -> f64 {
    let old_content = old_content.max(1.0);
    let new_content = new_content.max(1.0);
    let old_padding = ((viewport - old_content) * 0.5).max(0.0);
    let new_padding = ((viewport - new_content) * 0.5).max(0.0);
    let image_position = (current_scroll + pointer - old_padding).clamp(0.0, old_content);
    let normalized = image_position / old_content;
    normalized * new_content + new_padding - pointer
}

#[cfg(test)]
fn pointer_over_content(pointer: f64, viewport: f64, content: f64) -> bool {
    if !pointer.is_finite() || !viewport.is_finite() || !content.is_finite() {
        return false;
    }
    let padding = ((viewport - content) * 0.5).max(0.0);
    let visible_end = if content >= viewport {
        viewport
    } else {
        padding + content
    };
    pointer >= padding && pointer <= visible_end
}

fn fit_photo_rect(
    container_w: f64,
    container_h: f64,
    image_w: f64,
    image_h: f64,
) -> (f64, f64, f64, f64) {
    contained_rect(container_w, container_h, image_w, image_h)
}

fn normalized_image_point(
    pointer_x: f64,
    pointer_y: f64,
    photo_x: f64,
    photo_y: f64,
    photo_width: f64,
    photo_height: f64,
    h_scroll: f64,
    v_scroll: f64,
) -> Option<(f64, f64)> {
    if !pointer_x.is_finite()
        || !pointer_y.is_finite()
        || photo_width <= 0.0
        || photo_height <= 0.0
        || pointer_x < photo_x
        || pointer_y < photo_y
        || pointer_x > photo_x + photo_width
        || pointer_y > photo_y + photo_height
    {
        return None;
    }
    Some((
        ((h_scroll + pointer_x - photo_x) / photo_width).clamp(0.0, 1.0),
        ((v_scroll + pointer_y - photo_y) / photo_height).clamp(0.0, 1.0),
    ))
}

fn normalized_scroll_target(
    normalized: f64,
    pointer: f64,
    viewport: f64,
    native_content: f64,
) -> f64 {
    let padding = ((viewport - native_content) * 0.5).max(0.0);
    normalized.clamp(0.0, 1.0) * native_content + padding - pointer
}

fn one_to_one_scroll_target(
    current_scroll: f64,
    pointer: f64,
    viewport: f64,
    old_content: f64,
    native_content: f64,
    anchor_cursor: bool,
) -> f64 {
    if anchor_cursor {
        anchored_scroll_target(
            current_scroll,
            pointer,
            viewport,
            old_content,
            native_content,
        )
    } else {
        ((native_content - viewport) * 0.5).max(0.0)
    }
}

/// Only the FIRST entrance to 1:1 or an actual change of native geometry may
/// move the scroll offsets; tone/colour/filter updates only replace pixels.
fn native_preview_needs_reanchor(
    old_dimensions: (i32, i32),
    display_dimensions: (i32, i32),
    entering_with_anchor: bool,
) -> bool {
    entering_with_anchor || old_dimensions != display_dimensions
}

#[cfg(test)]
mod native_preview_geometry_tests {
    use super::*;

    #[test]
    fn native_refinement_does_not_recenter_unchanged_image() {
        let native = (4000, 3000);
        assert!(!native_preview_needs_reanchor(native, native, false));
    }

    #[test]
    fn first_one_to_one_and_real_geometry_changes_still_reanchor() {
        assert!(native_preview_needs_reanchor((1800, 1400), (4000, 3000), true));
        assert!(native_preview_needs_reanchor((4000, 3000), (3600, 2800), false));
    }

    #[test]
    fn native_canvas_maps_source_pixels_to_device_pixels_at_integer_and_fractional_scales() {
        assert_eq!(logical_canvas_size((6016, 4016), 1.0, 1.0), (6016.0, 4016.0));
        assert_eq!(logical_canvas_size((6016, 4016), 1.0, 2.0), (3008.0, 2008.0));
        assert_eq!(logical_canvas_size((6000, 4000), 1.0, 1.5), (4000.0, 4000.0 / 1.5));
        assert_eq!(logical_scale_for_physical(1.0, 2.0) * 2.0, 1.0);
        assert_eq!(logical_scale_for_physical(1.0, 1.5) * 1.5, 1.0);
    }

    #[test]
    fn fit_and_manual_zoom_use_physical_source_scale_but_logical_canvas_geometry() {
        let fit_physical = (800.0_f64 * 2.0 / 6000.0).min(600.0 * 2.0 / 4000.0);
        let fit_canvas = logical_canvas_size((6000, 4000), fit_physical, 2.0);
        assert_eq!(fit_canvas, (800.0, 800.0 * 2.0 / 3.0));

        let manual_physical = fit_physical * 2.0;
        let manual_canvas = logical_canvas_size((6000, 4000), manual_physical, 2.0);
        assert_eq!(manual_canvas, (1600.0, 1600.0 * 2.0 / 3.0));
    }

    #[test]
    fn normalized_cursor_anchor_survives_logical_canvas_resizing() {
        let normalized = (0.37, 0.62);
        let viewport = (900.0, 700.0);
        let pointer = (247.0, 381.0);
        for device_scale in [1.0, 2.0, 1.5, 1.75] {
            let logical = rounded_canvas_size(logical_canvas_size(
                (6016, 4016),
                1.0,
                device_scale,
            ));
            let target_x = normalized_scroll_target(
                normalized.0,
                pointer.0,
                viewport.0,
                logical.0 as f64,
            );
            let target_y = normalized_scroll_target(
                normalized.1,
                pointer.1,
                viewport.1,
                logical.1 as f64,
            );
            let padding_x = ((viewport.0 - logical.0 as f64) * 0.5).max(0.0);
            let padding_y = ((viewport.1 - logical.1 as f64) * 0.5).max(0.0);
            let restored_x = (target_x + pointer.0 - padding_x) / logical.0 as f64;
            let restored_y = (target_y + pointer.1 - padding_y) / logical.1 as f64;
            assert!((restored_x - normalized.0).abs() < 1e-12);
            assert!((restored_y - normalized.1).abs() < 1e-12);
        }
    }
}

fn apply_canvas_zoom(
    picture: &gtk::Picture,
    scroll: &gtk::ScrolledWindow,
    dimensions: (i32, i32),
    zoom: f64,
) {
    if zoom <= 0.0 {
        picture.set_can_shrink(true);
        picture.set_hexpand(true);
        picture.set_vexpand(true);
        picture.set_halign(gtk::Align::Fill);
        picture.set_valign(gtk::Align::Fill);
        picture.set_size_request(1, 1);
        scroll.set_cursor_from_name(None);
        scroll.hadjustment().set_value(0.0);
        scroll.vadjustment().set_value(0.0);
        return;
    }
    // Positive zoom values are multipliers of the current Fit scale.
    let (width, height) = canvas_size_for_zoom(scroll, dimensions, zoom);
    let width = width.round() as i32;
    let height = height.round() as i32;
    // Keep shrinking enabled: disabling it makes GtkPicture insist on the
    // paintable's intrinsic size and defeats incremental size requests.
    picture.set_can_shrink(true);
    picture.set_hexpand(false);
    picture.set_vexpand(false);
    picture.set_halign(gtk::Align::Center);
    picture.set_valign(gtk::Align::Center);
    picture.set_size_request(width, height);
    scroll.set_cursor_from_name(if zoom > 1.0 { Some("grab") } else { None });
    picture.queue_resize();
}

fn apply_canvas_one_to_one(
    picture: &gtk::Picture,
    scroll: &gtk::ScrolledWindow,
    dimensions: (i32, i32),
) {
    let (width, height) = native_canvas_size(scroll, dimensions);
    picture.set_can_shrink(true);
    picture.set_hexpand(false);
    picture.set_vexpand(false);
    picture.set_halign(gtk::Align::Center);
    picture.set_valign(gtk::Align::Center);
    picture.set_size_request(width, height);
    scroll.set_cursor_from_name(Some("grab"));
    picture.queue_resize();
}
