use std::sync::OnceLock;
use std::time::Instant;

static START: OnceLock<Instant> = OnceLock::new();
static PRESENT: OnceLock<Instant> = OnceLock::new();

pub fn start() {
    START.get_or_init(Instant::now);
    mark("main_start", None);
}

pub fn mark(stage: &str, since: Option<Instant>) {
    if std::env::var_os("PIC_STARTUP_TRACE").is_none() {
        return;
    }
    let elapsed = elapsed_ms();
    let duration = since.map(|start| start.elapsed().as_millis()).unwrap_or(0);
    eprintln!("PIC_STARTUP stage={stage} elapsed_ms={elapsed} duration_ms={duration}");
}

pub fn elapsed_ms() -> u128 {
    START.get_or_init(Instant::now).elapsed().as_millis()
}

pub fn begin_present() {
    PRESENT.get_or_init(Instant::now);
}

pub fn present_started() -> Option<Instant> {
    PRESENT.get().copied()
}
