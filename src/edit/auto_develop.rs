//! Conservative, deterministic display-referred automatic corrections.
//!
//! Called on decoded photo pixels by PIC's shared tone pipeline. Analysis is
//! bounded by a 2-D sampling grid, independent of the full export pixel count.
//! This is deliberately not a replacement for RAW-scene white balance or a
//! learned subject-aware model; it protects the source's highlight/chroma data.

use image::RgbaImage;
use rayon::prelude::*;

const HIST_BINS: usize = 256;
const NEUTRAL_MAX_CHROMA: f32 = 0.18;
const GRID_X: usize = 4;
const GRID_Y: usize = 4;

#[derive(Clone, Copy, Debug, Default)]
struct Region {
    pixels: u32,
    dark: u32,
    bright: u32,
    skin: u32,
    sum_y: f64,
}

#[derive(Clone, Copy, Debug, Default)]
struct Challenges {
    shadow_need: f32,
    exposure_need: f32,
    highlight_risk: f32,
    low_contrast: f32,
    color_cast: f32,
    confidence: f32,
}

fn score_challenges(stats: &Analysis) -> Challenges {
    let grid = &stats.regions;
    let mut dark_regions = 0usize;
    let mut bright_regions = 0usize;
    let mut mixed_regions = 0usize;
    for r in grid.iter().filter(|r| r.pixels >= 4) {
        let fraction_dark = r.dark as f32 / r.pixels as f32;
        let fraction_bright = r.bright as f32 / r.pixels as f32;
        if fraction_dark > 0.42 {
            dark_regions += 1;
        }
        if fraction_bright > 0.20 {
            bright_regions += 1;
        }
        if fraction_dark > 0.25 && fraction_bright > 0.12 {
            mixed_regions += 1;
        }
    }
    let dark_coverage = dark_regions as f32 / (GRID_X * GRID_Y) as f32;
    let bright_coverage = bright_regions as f32 / (GRID_X * GRID_Y) as f32;
    let highlight_risk = ((stats.p99 - 0.82) / 0.17)
        .clamp(0.0, 1.0)
        .max((bright_coverage * 1.5).clamp(0.0, 1.0));
    let shadow_need =
        ((0.40 - stats.p50) / 0.30).clamp(0.0, 1.0) * (0.30 + 0.70 * dark_coverage.clamp(0.0, 1.0));
    // Strong highlights should suppress broad exposure, not selective shadows.
    let exposure_need = ((0.34 - stats.p50) / 0.27).clamp(0.0, 1.0) * (1.0 - highlight_risk);
    let spread = stats.p88 - stats.p12;
    let low_contrast = ((0.48 - spread) / 0.35).clamp(0.0, 1.0) * (1.0 - bright_coverage).max(0.0);
    let color_cast = if stats.neutral_weight > 2.5 {
        let means = stats.neutrals.map(|n| n / stats.neutral_weight);
        let max = means.iter().copied().fold(0.0_f64, f64::max);
        let min = means.iter().copied().fold(f64::MAX, f64::min);
        ((max - min) / max.max(1.0) / 0.25).clamp(0.0, 1.0) as f32
    } else {
        0.0
    };
    Challenges {
        shadow_need,
        exposure_need,
        highlight_risk,
        low_contrast,
        color_cast,
        confidence: (stats.samples as f32 / 4096.0).clamp(0.0, 1.0)
            * (1.0 - 0.25 * mixed_regions as f32 / 16.0),
    }
}

#[derive(Clone, Copy, Debug)]
struct Analysis {
    p12: f32,
    p50: f32,
    p88: f32,
    p99: f32,
    neutrals: [f64; 3],
    neutral_weight: f64,
    samples: u64,
    regions: [Region; GRID_X * GRID_Y],
    portrait_signal: bool,
}

#[derive(Clone, Copy, Debug)]
struct Plan {
    wb: [f32; 3],
    exposure: f32,
    shadow_lift: f32,
    contrast: f32,
    portrait_guard: bool,
}

#[inline]
fn luma(rgb: [f32; 3]) -> f32 {
    0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2]
}

/// Conservative display-RGB skin-tone heuristic. It only limits portrait
/// lifting when skin-like pixels form a meaningful, spatially compact region;
/// it is not used for white balance or to alter colour.
#[inline]
fn is_skin_tone(rgb: [f32; 3]) -> bool {
    let [r, g, b] = rgb;
    let y = (0.299 * r + 0.587 * g + 0.114 * b) / 255.0;
    if !(0.18..=0.70).contains(&y) {
        return false;
    }
    let cb = 128.0 - 0.168_736 * r - 0.331_264 * g + 0.5 * b;
    let cr = 128.0 + 0.5 * r - 0.418_688 * g - 0.081_312 * b;
    (77.0..=127.0).contains(&cb) && (133.0..=173.0).contains(&cr)
}

#[inline]
fn smoothstep(a: f32, b: f32, value: f32) -> f32 {
    let t = ((value - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn percentile(histogram: &[u64; HIST_BINS], total: u64, fraction: f32) -> f32 {
    let threshold = ((total.saturating_sub(1)) as f64 * fraction as f64) as u64;
    let mut cumulative = 0;
    for (i, n) in histogram.iter().enumerate() {
        cumulative += n;
        if cumulative > threshold {
            return i as f32 / 255.0;
        }
    }
    1.0
}

impl Analysis {
    fn measure(image: &RgbaImage) -> Option<Self> {
        let width = image.width() as usize;
        let height = image.height() as usize;
        if width == 0 || height == 0 {
            return None;
        }
        // Sample across BOTH axes. A flat stride can repeatedly sample just
        // one column of a tiled/cropped photograph and badly bias auto tone.
        let sx = (width / 384).max(1);
        let sy = (height / 288).max(1);
        let data = image.as_raw();
        let mut hist = [0_u64; HIST_BINS];
        let mut count = 0_u64;
        let mut regions = [Region::default(); GRID_X * GRID_Y];
        let mut neutral_sum = [0.0_f64; 3];
        let mut neutral_weight = 0.0_f64;
        let mut skin_samples = 0_u64;
        for y in (0..height).step_by(sy) {
            for x in (0..width).step_by(sx) {
                let i = (y * width + x) * 4;
                if data[i + 3] < 16 {
                    continue;
                }
                let rgb = [data[i] as f32, data[i + 1] as f32, data[i + 2] as f32];
                let lum = luma(rgb) / 255.0;
                hist[(lum * 255.0).round().clamp(0.0, 255.0) as usize] += 1;
                count += 1;
                let region_id = (y * GRID_Y / height).min(GRID_Y - 1) * GRID_X
                    + (x * GRID_X / width).min(GRID_X - 1);
                let region = &mut regions[region_id];
                region.pixels += 1;
                region.sum_y += lum as f64;
                if lum < 0.34 {
                    region.dark += 1;
                }
                if lum > 0.65 {
                    region.bright += 1;
                }
                if is_skin_tone(rgb) {
                    region.skin += 1;
                    skin_samples += 1;
                }
                if !(0.15..=0.90).contains(&lum) {
                    continue;
                }
                let top = rgb[0].max(rgb[1]).max(rgb[2]);
                let bottom = rgb[0].min(rgb[1]).min(rgb[2]);
                let chroma = (top - bottom) / top.max(1.0);
                if chroma >= NEUTRAL_MAX_CHROMA {
                    continue;
                }
                let weight = (1.0 - chroma / NEUTRAL_MAX_CHROMA).powi(2) as f64;
                for channel in 0..3 {
                    neutral_sum[channel] += rgb[channel] as f64 * weight;
                }
                neutral_weight += weight;
            }
        }
        if count < 16 {
            return None;
        }
        let skin_regions = regions
            .iter()
            .filter(|region| {
                region.pixels >= 4 && region.skin as f32 / region.pixels as f32 >= 0.08
            })
            .count();
        Some(Self {
            p12: percentile(&hist, count, 0.12),
            p50: percentile(&hist, count, 0.50),
            p88: percentile(&hist, count, 0.88),
            p99: percentile(&hist, count, 0.99),
            neutrals: neutral_sum,
            neutral_weight,
            samples: count,
            regions,
            portrait_signal: skin_samples as f32 / count as f32 >= 0.05
                && (1..=6).contains(&skin_regions),
        })
    }
}

impl Plan {
    fn estimate(stats: Analysis, color: bool, contrast: bool, brightness: bool) -> Self {
        let mut plan = Self {
            wb: [1.0; 3],
            exposure: 1.0,
            shadow_lift: 0.0,
            contrast: 0.0,
            portrait_guard: stats.portrait_signal,
        };
        if color
            && stats.neutral_weight >= 2.5
            && stats.neutral_weight / stats.samples as f64 >= 0.003
        {
            let means = [
                stats.neutrals[0] / stats.neutral_weight,
                stats.neutrals[1] / stats.neutral_weight,
                stats.neutrals[2] / stats.neutral_weight,
            ];
            let target = 0.2126 * means[0] + 0.7152 * means[1] + 0.0722 * means[2];
            for c in 0..3 {
                plan.wb[c] = ((target / means[c].max(1.0)) as f32)
                    .powf(0.70)
                    .clamp(0.92, 1.08);
            }
        }
        let issues = score_challenges(&stats);
        if brightness {
            // A bright upper tail is NOT evidence that the subject is bright:
            // sunsets and product shots can contain large dark regions and
            // valuable near-white content in the same frame. Protect those
            // highlights *per pixel*, rather than blocking shadow work for
            // the entire photograph.
            let low_key = stats.p88 < 0.30 && stats.p99 < 0.50;
            let highlight_anchored = stats.p88 >= 0.70 || stats.p99 >= 0.90;
            let max_ev = if stats.portrait_signal {
                0.14
            } else if low_key {
                0.20
            } else if highlight_anchored {
                0.0
            } else {
                0.42
            };
            if stats.p50 < 0.33 && max_ev > 0.0 && issues.exposure_need > 0.1 {
                let desired = if low_key { 0.23 } else { 0.39 };
                let stops = (desired / stats.p50.max(0.06)).log2().clamp(0.0, max_ev);
                plan.exposure = 2.0_f32.powf(stops);
            }
            // Region-aware challenge scoring: don't let a luminous sky suppress
            // foreground correction. Keep overall exposure independent.
            let need = issues.shadow_need * issues.confidence;
            if need > 0.08 && stats.p12 < 0.31 {
                plan.shadow_lift = (0.055 + 0.16 * need).clamp(0.0, 0.19);
            }
            // Preserve already well-lit photographs.
            if stats.p50 >= 0.62 {
                if stats.p12 < 0.38 && issues.highlight_risk > 0.3 {
                    // Backlit subject against a bright wall; percentile median
                    // is dominated by background rather than the subject.
                    plan.shadow_lift = plan.shadow_lift.max(0.065);
                } else {
                    plan.shadow_lift *= 0.45;
                }
            }
            if stats.portrait_signal {
                // Keep some background recovery, but don't broadly lift a
                // dim portrait's hair, clothing and skin to the same target.
                plan.shadow_lift *= 0.65;
            }
        }
        if contrast {
            let spread = stats.p88 - stats.p12;
            // A bounded S curve preserves the original black and white and
            // avoids percentile-based multiplication up to 3.2x.
            // Avoid "improving" already-bright scenes. Their narrow
            // upper tonal range is usually white surfaces, not flat contrast.
            if spread < 0.58 && spread > 0.035 && stats.p88 < 0.84 {
                plan.contrast = ((0.58 - spread) * 0.32 * issues.low_contrast).clamp(0.0, 0.10);
            }
        }
        plan
    }
}

#[inline]
fn map_luminance(y: f32, plan: Plan) -> f32 {
    if y <= 0.000_001 {
        return 0.0;
    }
    let mut mapped = y;
    if plan.exposure > 1.0 {
        // As the original signal approaches the white point, reduce exposure
        // contribution rather than clip sunlit skin or the white background.
        let protect = 1.0 - smoothstep(0.55, 0.95, y);
        mapped += y * (plan.exposure - 1.0) * protect;
    }
    if plan.shadow_lift > 0.0 {
        let black_protect = smoothstep(0.01, 0.10, y);
        let highlight_protect = 1.0 - smoothstep(0.46, 0.69, y);
        mapped += plan.shadow_lift * black_protect * highlight_protect;
    }
    if plan.contrast > 0.0 {
        mapped += plan.contrast * 4.0 * (mapped - 0.5) * mapped * (1.0 - mapped);
    }
    if plan.shadow_lift > 0.0 {
        mapped = mapped.max(y);
    }
    mapped.clamp(0.0, 1.0)
}

/// Apply all enabled automatic edits together from one set of source statistics.
/// Auto adjustments are always recomputed from the *unedited* decoded frame;
/// they are not baked into the photo, and toggles preserve alpha and dimensions.
pub fn apply(image: &mut RgbaImage, color: bool, contrast: bool, brightness: bool) {
    if !(color || contrast || brightness) {
        return;
    }
    let Some(stats) = Analysis::measure(image) else {
        return;
    };
    let plan = Plan::estimate(stats, color, contrast, brightness);
    let issues = score_challenges(&stats);
    if std::env::var_os("PICASA_TRACE").is_some() {
        eprintln!(
            "PIC_EDIT_AUTO_ENGINE samples={} p12={:.3} p50={:.3} p88={:.3} p99={:.3} exposure={:.3} shadows={:.3} contrast={:.3} portrait={} wb={:?} challenges=shadow:{:.2},exposure:{:.2},highlights:{:.2},contrast:{:.2},cast:{:.2},confidence:{:.2}",
            stats.samples,
            stats.p12,
            stats.p50,
            stats.p88,
            stats.p99,
            plan.exposure,
            plan.shadow_lift,
            plan.contrast,
            plan.portrait_guard,
            plan.wb,
            issues.shadow_need,
            issues.exposure_need,
            issues.highlight_risk,
            issues.low_contrast,
            issues.color_cast,
            issues.confidence
        );
    }
    image.as_mut().par_chunks_mut(4).for_each(|pixel| {
        if pixel[3] < 16 {
            return;
        }
        let src = [
            pixel[0] as f32 / 255.0,
            pixel[1] as f32 / 255.0,
            pixel[2] as f32 / 255.0,
        ];
        let y_before = luma(src);
        if y_before < 0.000_001 {
            return;
        }
        let wb_fade = smoothstep(0.04, 0.16, y_before) * (1.0 - smoothstep(0.84, 0.99, y_before));
        let mut rgb = [0.0_f32; 3];
        for c in 0..3 {
            rgb[c] = src[c] * (1.0 + (plan.wb[c] - 1.0) * wb_fade);
        }
        // White balance must not inadvertently alter brightness.
        let balanced_y = luma(rgb);
        let neutralize_gain = y_before / balanced_y.max(0.000_001);
        for c in 0..3 {
            rgb[c] *= neutralize_gain;
        }
        let mut pixel_plan = plan;
        if plan.portrait_guard && is_skin_tone([pixel[0] as f32, pixel[1] as f32, pixel[2] as f32])
        {
            pixel_plan.exposure = 1.0 + (plan.exposure - 1.0) * 0.25;
            pixel_plan.shadow_lift *= 0.15;
            pixel_plan.contrast *= 0.25;
        }
        let corrected_y = map_luminance(y_before, pixel_plan);
        // One gain for all channels retains the ratio/hue of the neutralised
        // pixel. Limiting to the headroom of the highest channel prevents
        // independently clipped channels from shifting saturated colours.
        let intended_gain = corrected_y / y_before;
        let channel_max = rgb[0].max(rgb[1]).max(rgb[2]);
        let safe_gain = intended_gain.min(1.0 / channel_max.max(0.000_001));
        for c in 0..3 {
            pixel[c] = (rgb[c] * safe_gain * 255.0).round().clamp(0.0, 255.0) as u8;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    #[test]
    fn neutral_midgray_is_stable_and_alpha_survives() {
        let mut img = RgbaImage::from_pixel(48, 48, Rgba([128, 128, 128, 193]));
        apply(&mut img, true, true, true);
        assert_eq!(img.get_pixel(24, 24).0, [128, 128, 128, 193]);
    }

    #[test]
    fn bright_background_survives_shadow_recovery() {
        let mut img = RgbaImage::from_fn(100, 100, |x, _| {
            if x < 20 {
                Rgba([70, 78, 83, 255])
            } else {
                Rgba([225, 225, 225, 255])
            }
        });
        apply(&mut img, false, false, true);
        assert!(img.get_pixel(10, 50)[1] >= 83);
        assert!(img.get_pixel(10, 50)[1] <= 120);
        assert!(img.get_pixel(60, 50)[1] >= 222);
    }

    #[test]
    fn coloured_sports_uniform_does_not_trigger_false_wb() {
        let mut img = RgbaImage::from_pixel(90, 90, Rgba([205, 42, 36, 255]));
        let original = img.clone();
        apply(&mut img, true, false, false);
        assert_eq!(img, original);
    }

    #[test]
    fn neutral_cast_is_corrected_conservatively() {
        let mut img = RgbaImage::from_pixel(100, 100, Rgba([136, 126, 115, 255]));
        apply(&mut img, true, false, false);
        let px = img.get_pixel(50, 50).0;
        assert!(px[0] < 136 && px[2] > 115);
        assert!(px[0] > 127 && px[2] < 125);
        assert_eq!(px[3], 255);
    }

    #[test]
    fn near_black_does_not_turn_gray() {
        let mut img = RgbaImage::from_pixel(64, 64, Rgba([2, 3, 4, 128]));
        apply(&mut img, true, true, true);
        assert!(img.get_pixel(20, 20)[1] <= 6);
        assert_eq!(img.get_pixel(20, 20)[3], 128);
    }

    #[test]
    fn contrast_is_gentle_on_narrow_tonal_ranges() {
        let mut image = RgbaImage::from_fn(100, 80, |x, _| {
            let gray = if x < 50 { 100 } else { 120 };
            Rgba([gray, gray, gray, 255])
        });
        apply(&mut image, false, true, false);
        let a = image.get_pixel(10, 20)[0] as i16;
        let b = image.get_pixel(80, 20)[0] as i16;
        assert!(b - a > 20, "contrast should add separation");
        assert!(b - a < 40, "contrast must not exaggerate narrow ranges");
        assert!(a >= 85 && b <= 135);
    }

    #[test]
    fn clean_high_key_scene_is_not_forced_to_middle_gray() {
        let mut image = RgbaImage::from_pixel(64, 64, Rgba([242, 241, 239, 240]));
        let original = image.clone();
        apply(&mut image, true, true, true);
        assert_eq!(image, original);
    }

    #[test]
    fn fully_transparent_pixels_are_not_modified() {
        let mut image = RgbaImage::from_pixel(60, 60, Rgba([180, 80, 35, 0]));
        let original = image.clone();
        apply(&mut image, true, true, true);
        assert_eq!(image, original);
    }

    #[test]
    fn dark_foreground_and_bright_background_never_get_global_exposure() {
        // An image can have a dark median while still containing a large
        // highlight region with no safe room for an overall EV boost.
        let stats = Analysis {
            p12: 0.08,
            p50: 0.25,
            p88: 0.91,
            p99: 0.98,
            neutrals: [0.0; 3],
            neutral_weight: 0.0,
            samples: 1000,
            regions: [Region::default(); GRID_X * GRID_Y],
            portrait_signal: false,
        };
        let plan = Plan::estimate(stats, false, true, true);
        assert_eq!(plan.exposure, 1.0);
        assert_eq!(plan.contrast, 0.0);
    }

    #[test]
    fn sunset_foreground_recovers_without_brightening_sky() {
        let mut img = RgbaImage::from_fn(100, 100, |x, _| {
            if x < 65 {
                Rgba([31, 36, 42, 255])
            } else {
                Rgba([245, 183, 95, 255])
            }
        });
        let before_sky = *img.get_pixel(85, 50);
        let before_shadow = *img.get_pixel(12, 50);
        apply(&mut img, false, false, true);
        let after_shadow = img.get_pixel(12, 50);
        let after_sky = img.get_pixel(85, 50);
        assert!(
            after_shadow[1] >= before_shadow[1] + 6,
            "shadow must visibly lift"
        );
        for c in 0..3 {
            assert!(
                (after_sky[c] as i16 - before_sky[c] as i16).abs() <= 2,
                "highlight sky must remain stable"
            );
        }
    }

    #[test]
    fn black_product_lifts_while_white_label_stays_white() {
        let mut img = RgbaImage::from_fn(100, 100, |x, _| {
            if x < 90 {
                Rgba([20, 36, 59, 255])
            } else {
                Rgba([238, 240, 239, 255])
            }
        });
        let light = *img.get_pixel(95, 50);
        apply(&mut img, false, false, true);
        assert!(
            img.get_pixel(20, 50)[1] >= 42,
            "dark material should gain useful visibility"
        );
        for c in 0..3 {
            assert!(
                (img.get_pixel(95, 50)[c] as i16 - light[c] as i16).abs() <= 2,
                "white lettering/label must stay protected"
            );
        }
    }

    #[test]
    fn moderately_underexposed_scene_may_gain_exposure() {
        let stats = Analysis {
            p12: 0.12,
            p50: 0.27,
            p88: 0.58,
            p99: 0.73,
            neutrals: [0.0; 3],
            neutral_weight: 0.0,
            samples: 1000,
            regions: [Region::default(); GRID_X * GRID_Y],
            portrait_signal: false,
        };
        let plan = Plan::estimate(stats, false, false, true);
        assert!(plan.exposure > 1.0 && plan.exposure <= 2.0_f32.powf(0.42));
    }

    #[test]
    fn region_scoring_identifies_sunset_and_rejects_global_exposure() {
        let image = RgbaImage::from_fn(160, 160, |_, y| {
            if y < 60 {
                Rgba([245, 180, 100, 255])
            } else {
                Rgba([35, 40, 45, 255])
            }
        });
        let stats = Analysis::measure(&image).unwrap();
        let c = score_challenges(&stats);
        assert!(c.highlight_risk > 0.3);
        assert!(c.shadow_need > 0.15);
        let p = Plan::estimate(stats, false, false, true);
        assert_eq!(p.exposure, 1.0);
        assert!(p.shadow_lift > 0.0);
    }

    #[test]
    fn shadow_auto_does_not_darken_any_visible_pixel() {
        let mut img = RgbaImage::from_fn(160, 160, |x, y| {
            if y < 55 {
                Rgba([250, 175, 90, 255])
            } else {
                let v = 15 + ((x + y) % 55) as u8;
                Rgba([v, v, v, 255])
            }
        });
        let old = img.clone();
        apply(&mut img, false, false, true);
        for (before, after) in old.pixels().zip(img.pixels()) {
            assert!(
                luma([after[0] as f32, after[1] as f32, after[2] as f32]) + 1.0
                    >= luma([before[0] as f32, before[1] as f32, before[2] as f32])
            );
        }
    }

    #[test]
    fn result_is_deterministic_for_original_source() {
        let src = RgbaImage::from_fn(73, 61, |x, y| {
            Rgba([
                ((x * 3 + y) % 256) as u8,
                ((x + y * 4) % 256) as u8,
                ((x * 2 + y * 2) % 256) as u8,
                255,
            ])
        });
        let mut a = src.clone();
        let mut b = src;
        apply(&mut a, true, true, true);
        apply(&mut b, true, true, true);
        assert_eq!(a, b);
    }
}
