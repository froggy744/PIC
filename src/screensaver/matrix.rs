//! Matrix treatment is baked into worker-owned pixels once per decoded image.
//! Camera transforms therefore move the photograph and its code together.
use super::{Pixels, Style};

// The film's code is built from Japanese katakana. Pango selects a CJK font
// when the default UI font lacks the characters.
pub(super) const KATAKANA: [char; 32] = [
    'ア', 'イ', 'ウ', 'エ', 'オ', 'カ', 'キ', 'ク',
    'ケ', 'コ', 'サ', 'シ', 'ス', 'セ', 'ソ', 'タ',
    'チ', 'ツ', 'テ', 'ト', 'ナ', 'ニ', 'ヌ', 'ネ',
    'ノ', 'ハ', 'ヒ', 'フ', 'ヘ', 'ホ', 'マ', 'ム',
];
const CELL_W: usize = 12;
const CELL_H: usize = 17;

struct GlyphAtlas { masks: Vec<Vec<u8>> }
impl GlyphAtlas {
    fn new() -> Self {
        let masks = KATAKANA.iter().map(|glyph| {
            let mut surface = gtk4::cairo::ImageSurface::create(
                gtk4::cairo::Format::ARgb32, CELL_W as i32, CELL_H as i32,
            ).expect("katakana mask surface");
            {
                let cr = gtk4::cairo::Context::new(&surface).expect("katakana mask context");
                cr.set_source_rgb(1.0, 1.0, 1.0);
                let layout = pangocairo::functions::create_layout(&cr);
                let font = gtk4::pango::FontDescription::from_string("Sans 10");
                layout.set_font_description(Some(&font));
                layout.set_text(&glyph.to_string());
                let (tw, th) = layout.pixel_size();
                cr.move_to((CELL_W as i32 - tw).max(0) as f64 / 2.0,
                    (CELL_H as i32 - th).max(0) as f64 / 2.0);
                pangocairo::functions::show_layout(&cr, &layout);
            }
            surface.flush();
            let stride = surface.stride() as usize;
            let data = surface.data().expect("katakana mask pixels");
            let mut mask = Vec::with_capacity(CELL_W * CELL_H);
            for y in 0..CELL_H {
                for x in 0..CELL_W {
                    mask.push(data[y * stride + x * 4 + 3]);
                }
            }
            mask
        }).collect();
        Self { masks }
    }
}
thread_local! { static GLYPH_ATLAS: GlyphAtlas = GlyphAtlas::new(); }

fn hash(x: usize, y: usize) -> usize {
    let n = (x as u32).wrapping_mul(374761393)
        .wrapping_add((y as u32).wrapping_mul(668265263));
    ((n ^ (n >> 13)).wrapping_mul(1274126177) >> 16) as usize
}

pub(super) fn prepare(mut pixels: Pixels, style: Style) -> Pixels {
    if style != Style::MatrixKenBurns { return pixels; }
    let w = pixels.width as usize;
    let h = pixels.height as usize;
    if w == 0 || h == 0 { return pixels; }
    let tones: [f64; 256] = std::array::from_fn(|i| 245.0 * (i as f64 / 255.0).powf(0.70));
    GLYPH_ATLAS.with(|atlas| {
        for y in 0..h {
            for x in 0..w {
                let cx = x / CELL_W;
                let shifted_y = y + hash(cx, 0) % CELL_H;
                let cy = shifted_y / CELL_H;
                let gy = shifted_y % CELL_H;
                let gx = x % CELL_W;
                let seed = hash(cx, cy);
                let mask = atlas.masks[seed % atlas.masks.len()][gy * CELL_W + gx] as f64 / 255.0;
                let offset = (y * w + x) * 4;
                let px = &mut pixels.data[offset..offset + 4];
                let luma = (54 * px[0] as usize + 183 * px[1] as usize + 19 * px[2] as usize) / 256;
                let tone = tones[luma];
                let green = if style == Style::MatrixKenBurns {
                    // A full photographic image carries the Ken Burns shot.
                    // Sparse, fixed code streams sit on top and pan with it.
                    let base = tone * 0.94;
                    let phase = (cy + hash(cx, 37) % 22) % 22;
                    let stream = if hash(cx, 17) % 3 != 0 { 0.0 } else {
                        match phase {
                            0..=2 => 0.96,
                            3..=8 => 0.58,
                            9..=12 => 0.28,
                            _ => 0.0,
                        }
                    };
                    (base + (35.0 + tone * 0.38) * mask * stream).min(255.0)
                } else {
                    // Rain photos are formed from denser, luminous code.
                    let background = tone * 0.36;
                    let column = 0.82 + (hash(cx, 11) % 37) as f64 / 100.0;
                    let trail = match (cy + hash(cx, 29) % 23) % 23 {
                        0 => 1.18, 1..=5 => 1.02, 6..=15 => 0.87, _ => 0.68,
                    };
                    let emission = (tone * 1.38 * column * trail).min(255.0);
                    let stream_mask = if seed % 31 == 0 { 0.0 } else { mask };
                    background + (emission - background) * stream_mask
                };
                let highlight = (green / 255.0).powi(2);
                px[0] = (green * (0.07 + 0.30 * highlight)) as u8;
                px[1] = green as u8;
                px[2] = (green * (0.15 + 0.28 * highlight)) as u8;
            }
        }
    });
    pixels
}

/// Screen-space rain; only the Rain style calls this. Time comes from the
/// existing playback clock, so pause/resume freezes trails as well as slides.
pub(super) fn draw_rain(cr: &gtk4::cairo::Context, w: f64, h: f64, seconds: f64) {
    let cell = (w / 100.0).max(13.0);
    let line = cell * 1.42;
    let layout = pangocairo::functions::create_layout(cr);
    let font = gtk4::pango::FontDescription::from_string(&format!("Sans {}", (cell * 0.80).round() as i32));
    layout.set_font_description(Some(&font));
    let tick = (seconds * 12.0) as usize;
    for col in 0..(w / cell).ceil() as usize {
        let seed = hash(col, 17);
        let speed = 42.0 + (seed % 85) as f64;
        let cycle = h + line * 20.0;
        let head = (seconds * speed + (seed % 2000) as f64).rem_euclid(cycle);
        let mut text = String::new();
        for tail in (0..18).rev() {
            if !text.is_empty() { text.push('\n'); }
            text.push(KATAKANA[hash(col, tail + tick) % KATAKANA.len()]);
        }
        layout.set_text(&text);
        let top = head - layout.pixel_size().1 as f64;
        let gradient = gtk4::cairo::LinearGradient::new(0.0, top, 0.0, head);
        gradient.add_color_stop_rgba(0.0, 0.02, 0.45, 0.10, 0.0);
        gradient.add_color_stop_rgba(0.75, 0.04, 0.80, 0.20, 0.36);
        gradient.add_color_stop_rgba(1.0, 0.65, 1.0, 0.72, 0.90);
        let _ = cr.set_source(&gradient);
        cr.move_to(col as f64 * cell, top);
        pangocairo::functions::show_layout(cr, &layout);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn photo() -> Pixels {
        let mut data = Vec::new();
        for y in 0..96 {
            for x in 0..192 {
                data.extend_from_slice(&[x as u8, y as u8, 220, 127]);
            }
        }
        Pixels { width: 192, height: 96, data }
    }

    #[test]
    fn matrix_conversion_preserves_geometry_alpha_and_is_fixed() {
        let a = prepare(photo(), Style::MatrixKenBurns);
        let b = prepare(photo(), Style::MatrixKenBurns);
        assert_eq!((a.width, a.height), (192, 96));
        assert_eq!(a.data, b.data);
        assert!(a.data.chunks_exact(4).all(|p| p[1] > p[0] && p[1] > p[2] && p[3] == 127));
        assert!(a.data.chunks_exact(4).map(|p| p[1]).max().unwrap() > 70);
    }

    #[test]
    fn ken_burns_keeps_photographic_tones_beneath_katakana() {
        assert!(KATAKANA.iter().all(|ch| ('\u{30a0}'..='\u{30ff}').contains(ch)));
        let atlas = GlyphAtlas::new();
        assert!(atlas.masks.iter().all(|mask| mask.iter().any(|a| *a > 0)));
        assert!(atlas.masks.iter().collect::<std::collections::HashSet<_>>().len() >= 20,
            "Katakana font fallback rendered missing-glyph boxes");
        let rendered = |level: u8| {
            let image = Pixels { width: 192, height: 102,
                data: [level, level, level, 255].repeat(192 * 102) };
            prepare(image, Style::MatrixKenBurns).data.chunks_exact(4)
                .map(|px| px[1]).collect::<Vec<_>>()
        };
        let dark = rendered(70);
        let light = rendered(180);
        let median = |data: &[u8]| {
            let mut values = data.to_vec();
            values.sort_unstable();
            values[values.len() / 2]
        };
        assert!(median(&light) > median(&dark) + 65, "photo contrast was lost");
        assert!(median(&light) > 150, "photo is too dark to read");
        assert!(light.iter().max().unwrap() - light.iter().min().unwrap() > 25,
            "Katakana streams are not visible");
    }

    #[test]
    fn ordinary_styles_keep_original_colors() {
        for style in Style::CHOICES.into_iter().filter(|s| *s != Style::MatrixKenBurns) {
            assert_eq!(prepare(photo(), style).data, photo().data);
        }
    }
}
