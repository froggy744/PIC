//! Lightbox large-image benchmark. Ignored; run one scenario per process,
//! under Xvfb with the AGENTS.md display-test command. It only uses the
//! public lightbox API so the same file runs on older commits for comparison.
//!
//! PIC_BENCH_LIGHTBOX_SCENARIO  cold_fit | fit_100 | fit_100_fit_100 |
//!                              png_jpeg_raw | png_cached | rapid | close_decoding
//! PIC_BENCH_LIGHTBOX_PNG       large PNG
//! PIC_BENCH_LIGHTBOX_JPEG      JPEG
//! PIC_BENCH_LIGHTBOX_RAW       RAW
//! PIC_BENCH_LIGHTBOX_SET       '|'-separated photos for rapid navigation

use super::*;
use std::time::{Duration, Instant};

fn bench_env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name} is not set"))
}

fn bench_photo(path: &str, id: i64) -> PhotoObject {
    use std::os::unix::fs::MetadataExt;
    let meta = std::fs::metadata(path).unwrap();
    let (width, height) = image::image_dimensions(path).unwrap_or((4000, 3000));
    glib::Object::builder()
        .property("id", id)
        .property("path", path)
        .property("width", width as i64)
        .property("height", height as i64)
        .property("mtime", meta.mtime())
        .property("size-bytes", meta.size() as i64)
        .property("original-available", true)
        .build()
}

fn proc_mb(field: &str) -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .unwrap()
        .lines()
        .find(|line| line.starts_with(field))
        .and_then(|line| line.split_whitespace().nth(1)?.parse::<u64>().ok())
        .unwrap_or(0)
        / 1024
}

fn reset_peak() {
    // "5" resets VmHWM (peak RSS) to the current RSS.
    let _ = std::fs::write("/proc/self/clear_refs", "5");
}

fn pump(duration: Duration) {
    let context = glib::MainContext::default();
    let until = Instant::now() + duration;
    while Instant::now() < until {
        while context.pending() {
            context.iteration(false);
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn shown(lightbox: &Lightbox) -> Option<gtk::gdk::Texture> {
    lightbox
        .picture
        .paintable()
        .and_then(|paintable| paintable.downcast::<gtk::gdk::Texture>().ok())
}

/// Milliseconds until the picture shows a texture other than `previous`
/// that satisfies `accept`.
fn wait_texture(
    lightbox: &Lightbox,
    previous: Option<&gtk::gdk::Texture>,
    accept: impl Fn(&gtk::gdk::Texture) -> bool,
) -> (u128, gtk::gdk::Texture) {
    let context = glib::MainContext::default();
    let started = Instant::now();
    loop {
        while context.pending() {
            context.iteration(false);
        }
        if let Some(texture) = shown(lightbox) {
            if previous.is_none_or(|previous| previous != &texture) && accept(&texture) {
                return (started.elapsed().as_millis(), texture);
            }
        }
        assert!(
            started.elapsed() < Duration::from_secs(30),
            "timed out waiting for texture"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn report(scenario: &str, step: &str, ms: Option<u128>) {
    // Let prefetch and late workers finish so memory is the settled value.
    pump(Duration::from_millis(1200));
    println!(
        "BENCH scenario={scenario} step={step} ms={} rss_mb={} peak_mb={}",
        ms.map_or("-".to_string(), |ms| ms.to_string()),
        proc_mb("VmRSS:"),
        proc_mb("VmHWM:"),
    );
}

#[test]
#[ignore = "benchmark: needs a GTK display and PIC_BENCH_LIGHTBOX_* files"]
fn bench_lightbox_large_images() {
    gtk::init().unwrap();
    let scenario = bench_env("PIC_BENCH_LIGHTBOX_SCENARIO");
    let png = || bench_photo(&bench_env("PIC_BENCH_LIGHTBOX_PNG"), 1);
    let jpeg = || bench_photo(&bench_env("PIC_BENCH_LIGHTBOX_JPEG"), 2);
    let raw = || bench_photo(&bench_env("PIC_BENCH_LIGHTBOX_RAW"), 3);

    let lightbox = Lightbox::new();
    let window = gtk::Window::builder()
        .default_width(1600)
        .default_height(900)
        .child(&lightbox.root)
        .build();
    window.present();
    pump(Duration::from_millis(500));
    reset_peak();
    println!(
        "BENCH scenario={scenario} step=baseline rss_mb={}",
        proc_mb("VmRSS:")
    );

    let native =
        |width: i64| move |texture: &gtk::gdk::Texture| i64::from(texture.width()) == width;
    let fit = |width: i64| move |texture: &gtk::gdk::Texture| i64::from(texture.width()) < width;

    match scenario.as_str() {
        "cold_fit" => {
            let photo = png();
            lightbox.open(vec![photo.clone()], 0);
            let (ms, _) = wait_texture(&lightbox, None, fit(photo.width()));
            report(&scenario, "fit", Some(ms));
        }
        "fit_100" | "fit_100_fit_100" => {
            let photo = png();
            lightbox.open(vec![photo.clone()], 0);
            let (ms, fitted) = wait_texture(&lightbox, None, fit(photo.width()));
            report(&scenario, "fit", Some(ms));
            lightbox.set_one_to_one(true);
            let (ms, full) = wait_texture(&lightbox, Some(&fitted), native(photo.width()));
            report(&scenario, "100", Some(ms));
            if scenario == "fit_100_fit_100" {
                lightbox.set_one_to_one(false);
                let (ms, fitted) = wait_texture(&lightbox, Some(&full), fit(photo.width()));
                report(&scenario, "fit_again", Some(ms));
                lightbox.set_one_to_one(true);
                let (ms, _) = wait_texture(&lightbox, Some(&fitted), native(photo.width()));
                report(&scenario, "100_again", Some(ms));
            }
        }
        "png_jpeg_raw" => {
            lightbox.open(vec![png(), jpeg(), raw()], 0);
            let (ms, first) = wait_texture(&lightbox, None, |_| true);
            report(&scenario, "png", Some(ms));
            lightbox.navigate_photo(1);
            let (ms, second) = wait_texture(&lightbox, Some(&first), |_| true);
            report(&scenario, "jpeg", Some(ms));
            lightbox.navigate_photo(1);
            let (ms, _) = wait_texture(&lightbox, Some(&second), |_| true);
            report(&scenario, "raw", Some(ms));
        }
        "png_cached" => {
            lightbox.open(vec![jpeg(), png()], 0);
            let (ms, first) = wait_texture(&lightbox, None, |_| true);
            report(&scenario, "jpeg", Some(ms));
            lightbox.navigate_photo(1);
            let (ms, second) = wait_texture(&lightbox, Some(&first), |_| true);
            report(&scenario, "png", Some(ms));
            lightbox.navigate_photo(-1);
            let (ms, _) = wait_texture(&lightbox, Some(&second), |_| true);
            report(&scenario, "cached_jpeg", Some(ms));
        }
        "rapid" => {
            let photos: Vec<PhotoObject> = bench_env("PIC_BENCH_LIGHTBOX_SET")
                .split('|')
                .enumerate()
                .map(|(index, path)| bench_photo(path, 10 + index as i64))
                .collect();
            let count = photos.len();
            lightbox.open(photos, 0);
            wait_texture(&lightbox, None, |_| true);
            let started = Instant::now();
            // Step about as fast as a person tapping the arrow key: prefetch
            // starts 250 ms after each photo settles, so it runs concurrently.
            for _ in 1..count {
                lightbox.navigate_photo(1);
                pump(Duration::from_millis(400));
            }
            // The foreground generation returns to 0 once the last photo shows.
            let context = glib::MainContext::default();
            while VIEWER_FOREGROUND_GENERATION.load(Ordering::Acquire) != 0 {
                while context.pending() {
                    context.iteration(false);
                }
                assert!(started.elapsed() < Duration::from_secs(120));
                std::thread::sleep(Duration::from_millis(1));
            }
            report(&scenario, "all", Some(started.elapsed().as_millis()));
        }
        "close_decoding" => {
            lightbox.open(vec![png()], 0);
            pump(Duration::from_millis(150));
            lightbox.close();
            report(&scenario, "closed", None);
        }
        other => panic!("unknown scenario {other}"),
    }
    lightbox.close();
    report(&scenario, "after_close", None);
    window.close();
}
