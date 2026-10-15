#[macro_export]
macro_rules! nav_transition_trace {
    ($($argument:tt)*) => {{
        if $crate::nav_transition_trace::enabled() {
            $crate::nav_transition_trace::log(format!($($argument)*));
        }
    }};
}

use std::sync::OnceLock;
use std::time::Instant;

static STARTED: OnceLock<Instant> = OnceLock::new();

pub(crate) fn enabled() -> bool {
    std::env::var_os("PIC_NAV_TRANSITION_TRACE").is_some()
}

pub(crate) fn log(event: impl std::fmt::Display) {
    if enabled() {
        let elapsed = STARTED.get_or_init(Instant::now).elapsed();
        eprintln!(
            "PIC_NAV_TRANSITION_TRACE t_ms={} {event}",
            elapsed.as_millis()
        );
    }
}
