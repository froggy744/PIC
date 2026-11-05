//! Opt-in logging for diagnosing scroll-event routing in the GTK UI.

use gtk4 as gtk;
use gtk::prelude::*;

pub(crate) fn log_scroll_input(
    source: &str,
    controller: &gtk::EventControllerScroll,
    dx: f64,
    dy: f64,
) {
    if std::env::var_os("PIC_SCROLL_TRACE").is_none() {
        return;
    }

    let state = controller.current_event_state();
    eprintln!(
        "PIC_SCROLL_TRACE source={source} dx={dx:.3} dy={dy:.3} unit={:?} state={state:?} ctrl={} shift={} alt={} super={}",
        controller.unit(),
        state.contains(gtk::gdk::ModifierType::CONTROL_MASK),
        state.contains(gtk::gdk::ModifierType::SHIFT_MASK),
        state.contains(gtk::gdk::ModifierType::ALT_MASK),
        state.contains(gtk::gdk::ModifierType::SUPER_MASK),
    );
}
