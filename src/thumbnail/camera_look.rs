//! Nikon camera-look approximation for full-resolution sensor pixels.
//!
//! The embedded JPEG is used ONLY to estimate one global transform at reduced
//! resolution. It is never blended or copied into the sensor-rendered frame.
//! This avoids the edge halos and speckles from mixing independently developed
//! rasters. This is not a replacement for a calibrated camera DCP profile.
use image::{ImageBuffer, Rgb, RgbImage};
use rayon::prelude::*;

type SensorImage = ImageBuffer<Rgb<u16>, Vec<u16>>;
const SAMPLE_STEP: usize = 2;
const TONAL_BLEND: f32 = 0.85;
const MIN_SAMPLES: usize = 64;

#[derive(Clone)]
struct CameraLook {
    matrix: [[f32; 4]; 3],
    tone: [f32; 256],
    saturation: f32,
}

fn luminance(rgb: [f32; 3]) -> f32 {
    (0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2]).clamp(0.0, 255.0)
}

fn sensor_at(sensor: &SensorImage, x: u32, y: u32, width: u32, height: u32) -> [f32; 4] {
    // Map the centre of each reduced JPEG pixel to the sensor. The alignment
    // guard is in the caller, which checks the JPEG's *native* dimensions.
    let sx = (((x as u64 * 2 + 1) * sensor.width() as u64) / (2 * width as u64))
        .min(sensor.width() as u64 - 1) as u32;
    let sy = (((y as u64 * 2 + 1) * sensor.height() as u64) / (2 * height as u64))
        .min(sensor.height() as u64 - 1) as u32;
    let p = sensor.get_pixel(sx, sy).0;
    [
        p[0] as f32 / 65535.0,
        p[1] as f32 / 65535.0,
        p[2] as f32 / 65535.0,
        1.0,
    ]
}

fn apply_matrix(matrix: &[[f32; 4]; 3], input: [f32; 4]) -> [f32; 3] {
    std::array::from_fn(|ch| {
        let row = matrix[ch];
        (row[0] * input[0] + row[1] * input[1] + row[2] * input[2] + row[3])
            .clamp(0.0, 255.0)
    })
}

fn solve(mut gram: [[f64; 4]; 4], rhs: [f64; 4]) -> Option<[f32; 4]> {
    let mut b = rhs;
    for col in 0..4 {
        let pivot = (col..4).max_by(|&a, &b| gram[a][col].abs().total_cmp(&gram[b][col].abs()))?;
        if gram[pivot][col].abs() < 1e-10 {
            return None;
        }
        gram.swap(pivot, col);
        b.swap(pivot, col);
        let scale = gram[col][col];
        for k in col..4 {
            gram[col][k] /= scale;
        }
        b[col] /= scale;
        for row in 0..4 {
            if row == col {
                continue;
            }
            let factor = gram[row][col];
            for k in col..4 {
                gram[row][k] -= factor * gram[col][k];
            }
            b[row] -= factor * b[col];
        }
    }
    let result = b.map(|v| v as f32);
    result.iter().all(|v| v.is_finite()).then_some(result)
}

impl CameraLook {
    fn estimate(sensor: &SensorImage, jpeg: &RgbImage) -> Option<Self> {
        let (w, h) = jpeg.dimensions();
        if w < 16 || h < 16 || sensor.width() < w || sensor.height() < h {
            return None;
        }
        let mut gram = [[0.0f64; 4]; 4];
        let mut rhs = [[0.0f64; 3]; 4];
        let mut count = 0usize;
        for y in (1..h - 1).step_by(SAMPLE_STEP) {
            for x in (1..w - 1).step_by(SAMPLE_STEP) {
                let target = jpeg.get_pixel(x, y).0;
                // Ignore clipped areas and strong JPEG boundaries when
                // fitting colour. The two demosaicers need not agree at edges.
                let bright = target.iter().copied().max().unwrap_or(0);
                let dark = target.iter().copied().min().unwrap_or(0);
                if bright >= 250 || dark <= 4 {
                    continue;
                }
                let neighbor = jpeg.get_pixel(x + 1, y).0;
                if target.iter().zip(neighbor).any(|(&a, b)| a.abs_diff(b) > 35) {
                    continue;
                }
                let input = sensor_at(sensor, x, y, w, h);
                for row in 0..4 {
                    for col in 0..4 {
                        gram[row][col] += (input[row] * input[col]) as f64;
                    }
                    for channel in 0..3 {
                        rhs[row][channel] += input[row] as f64 * target[channel] as f64;
                    }
                }
                count += 1;
            }
        }
        if count < MIN_SAMPLES {
            return None;
        }
        // Strong identity prior prevents a monochrome/flat scene from
        // producing a wildly ill-conditioned or oversaturated matrix.
        let prior = count as f64 * 0.004;
        for i in 0..4 {
            gram[i][i] += prior;
        }
        let mut matrix = [[0.0f32; 4]; 3];
        for channel in 0..3 {
            matrix[channel] = solve(gram, std::array::from_fn(|i| {
                rhs[i][channel] + if i == channel { 255.0 * prior } else { 0.0 }
            }))?;
        }

        // Estimate a monotone camera contrast curve from whole-image luma
        // histograms, à la an auto-matched camera tone curve. It uses *no*
        // pixelwise JPEG/RAW switching, hence cannot make double edges.
        let mut source_hist = [0u32; 256];
        let mut target_hist = [0u32; 256];
        let mut source_saturation = 0.0f64;
        let mut target_saturation = 0.0f64;
        let mut total = 0u32;
        for y in (0..h).step_by(SAMPLE_STEP) {
            for x in (0..w).step_by(SAMPLE_STEP) {
                let input = sensor_at(sensor, x, y, w, h);
                let mapped = apply_matrix(&matrix, input);
                let expected = jpeg.get_pixel(x, y).0.map(|c| c as f32);
                source_hist[luminance(mapped).round() as usize] += 1;
                target_hist[luminance(expected).round() as usize] += 1;
                let chroma = |rgb: [f32; 3]| {
                    let high = rgb.iter().copied().fold(0.0f32, f32::max);
                    let low = rgb.iter().copied().fold(255.0f32, f32::min);
                    (high - low) / high.max(16.0)
                };
                source_saturation += chroma(mapped) as f64;
                target_saturation += chroma(expected) as f64;
                total += 1;
            }
        }
        if total == 0 {
            return None;
        }
        let mut target_cdf = [0u32; 256];
        let mut accum = 0u32;
        for (idx, item) in target_cdf.iter_mut().enumerate() {
            accum += target_hist[idx];
            *item = accum;
        }
        let mut tone = [0.0f32; 256];
        let mut src_accum = 0u32;
        let mut j = 0usize;
        for i in 0..256 {
            src_accum += source_hist[i];
            while j < 255 && target_cdf[j] < src_accum {
                j += 1;
            }
            // The camera JPEG contains scene-dependent processing; limit
            // per-photo tone adaptation to avoid pumping noise and highlights.
            let destination = i as f32 * (1.0 - TONAL_BLEND) + j as f32 * TONAL_BLEND;
            tone[i] = destination.clamp((i as f32 - 42.0).max(0.0), (i as f32 + 42.0).min(255.0));
        }
        // Smooth the curve while maintaining monotonicity. Pure CDF matching
        // on sparse histograms otherwise introduces posterisation.
        let original = tone;
        for i in 1..255 {
            tone[i] = (original[i - 1] + 2.0 * original[i] + original[i + 1]) * 0.25;
        }
        for i in 1..256 {
            tone[i] = tone[i].max(tone[i - 1]);
        }
        let saturation = if source_saturation > 1.0 {
            (target_saturation / source_saturation) as f32
        } else {
            1.0
        }.clamp(0.90, 1.15);
        Some(Self { matrix, tone, saturation })
    }

    fn pixel(&self, raw: [u16; 3]) -> [u8; 3] {
        let input = [
            raw[0] as f32 / 65535.0,
            raw[1] as f32 / 65535.0,
            raw[2] as f32 / 65535.0,
            1.0,
        ];
        let rgb = apply_matrix(&self.matrix, input);
        let old_y = luminance(rgb);
        let index = old_y.floor() as usize;
        let fraction = old_y - index as f32;
        let new_y = self.tone[index] * (1.0 - fraction)
            + self.tone[(index + 1).min(255)] * fraction;
        // Apply tone using shared luma gain, not independent R/G/B curves.
        // Global chroma gain is limited; it cannot introduce spatial halos.
        let gain = (new_y / old_y.max(1.0)).clamp(0.65, 1.45);
        let y = old_y * gain;
        std::array::from_fn(|ch| {
            (y + (rgb[ch] - old_y) * gain * self.saturation)
                .round().clamp(0.0, 255.0) as u8
        })
    }
}

/// Returns None if there are insufficient useful calibration samples.
/// Peak additional storage: a single RGB8 output and a scaled JPEG reference.
pub(super) fn render(sensor: &SensorImage, reference: &RgbImage) -> Option<RgbImage> {
    let look = CameraLook::estimate(sensor, reference)?;
    let mut pixels = vec![0u8; sensor.as_raw().len()];
    pixels.par_chunks_exact_mut(3)
        .zip(sensor.as_raw().par_chunks_exact(3))
        .for_each(|(out, src)| {
            out.copy_from_slice(&look.pixel([src[0], src[1], src[2]]));
        });
    RgbImage::from_raw(sensor.width(), sensor.height(), pixels)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fitted_camera_look_preserves_sensor_detail_and_colour() {
        let sensor = SensorImage::from_fn(256, 256, |x, y| {
            let v = 0.12 + x as f32 / 400.0 + y as f32 / 1800.0;
            Rgb([(v * 65535.0) as u16, (v * 0.67 * 65535.0) as u16,
                (v * 0.41 * 65535.0) as u16])
        });
        let reference = RgbImage::from_fn(32, 32, |x, y| {
            let v = 0.12 + ((x * 8 + 4) as f32) / 400.0
                + ((y * 8 + 4) as f32) / 1800.0;
            Rgb([(v * 215.0 + 7.0).round() as u8,
                (v * 0.67 * 235.0 + 7.0).round() as u8,
                (v * 0.41 * 240.0 + 7.0).round() as u8])
        });
        let output = render(&sensor, &reference).expect("synthetic calibration");
        assert_eq!(output.dimensions(), sensor.dimensions());
        assert_ne!(output.get_pixel(10, 10), output.get_pixel(240, 240),
            "camera match must not replace the sensor with flat JPEG pixels");
        assert!(output.get_pixel(120, 120)[0] > output.get_pixel(120, 120)[1]);
        assert!(output.get_pixel(120, 120)[1] > output.get_pixel(120, 120)[2]);
    }

    #[test]
    fn monotone_curve_does_not_reverse_neutral_gradient() {
        let sensor = SensorImage::from_fn(256, 256, |x, _| {
            let v = ((x * 220 / 255 + 16) * 257) as u16;
            Rgb([v, v, v])
        });
        let reference = RgbImage::from_fn(32, 32, |x, _| {
            let v = (x * 6 + 25) as u8;
            Rgb([v, v, v])
        });
        let result = render(&sensor, &reference).expect("neutral calibration");
        for x in 1..256 {
            let a = result.get_pixel(x - 1, 120)[0];
            let b = result.get_pixel(x, 120)[0];
            assert!(b >= a, "tone correction inverted a gradient at {x}");
        }
    }

    #[test]
    fn rejects_insufficient_reference_without_allocating_output() {
        let sensor = SensorImage::from_pixel(32, 32, Rgb([15000u16, 14000, 13000]));
        let reference = RgbImage::from_pixel(4, 4, Rgb([95u8, 92, 89]));
        assert!(render(&sensor, &reference).is_none());
    }
}
