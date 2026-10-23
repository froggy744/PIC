//! Independent PIC RAW quality diagnostics.
//!
//! This is PIC-authored evaluation code, not another JPEG-to-RAW colour
//! transform. It never changes the on-screen photograph or saved recipe.
//! A genuine Nikon file stays local and is never included in the repository.
//!
//! To run:
//! PIC_TEST_RAW="/path/to/DSC_4814.NEF" \
//! PIC_TEST_OUTPUT_DIR="/tmp/pic-nef-evaluation" \
//! cargo test --locked raw_quality::tests::real_nef_reference_report \
//!   -- --ignored --nocapture --test-threads=1
//!
//! The diagnostic outputs two *different* pictures:
//!   camera-embedded.jpg: Nikon's camera-developed JPEG, unchanged bytes
//!   rawler-sensor16.tiff: rawler's 16-bit developed sensor data
//!
//! Comparing these tells us where colour rendering differs. It is NOT a
//! photographic-quality pass/fail gate and it DOES NOT estimate a colour
//! transform from one photograph.

#[cfg(test)]
mod tests {
    use super::super::*;
    use image::RgbImage;
    use std::path::Path;
    use std::time::Instant;

    #[derive(Default, Debug)]
    struct Report {
        samples: u64,
        error: [u64; 3],
        signed: [i64; 3],
        neutral_samples: u64,
        neutral_signed: [i64; 3],
        skin_candidate_samples: u64,
        skin_signed: [i64; 3],
    }

    impl Report {
        fn record(&mut self, actual: [u8; 3], reference: [u8; 3]) {
            self.samples += 1;
            for i in 0..3 {
                self.error[i] += u64::from(actual[i].abs_diff(reference[i]));
                self.signed[i] += i64::from(actual[i]) - i64::from(reference[i]);
            }
            let (max, min) = (
                *reference.iter().max().unwrap_or(&0),
                *reference.iter().min().unwrap_or(&0),
            );
            // White/gray objects are a useful diagnostic for channel casts.
            // This is a heuristic only: clipped whites and backgrounds do
            // not constitute a measured colour calibration target.
            if max.saturating_sub(min) <= 20 && min >= 48 && max <= 242 {
                self.neutral_samples += 1;
                for i in 0..3 {
                    self.neutral_signed[i] +=
                        i64::from(actual[i]) - i64::from(reference[i]);
                }
            }
            // Warm diffuse colour candidates, NOT a face/skin detector.
            // Their errors are informational and cannot establish fidelity.
            if reference[0] > reference[1].saturating_add(12)
                && reference[1] > reference[2].saturating_add(8)
                && reference[0] > 75
                && reference[0] < 240
            {
                self.skin_candidate_samples += 1;
                for i in 0..3 {
                    self.skin_signed[i] +=
                        i64::from(actual[i]) - i64::from(reference[i]);
                }
            }
        }

        fn print(&self) {
            let denom = self.samples.max(1) as f64;
            eprintln!(
                "PIC_RAW_QA sampled={} mean_absolute_RGB={:.2},{:.2},{:.2} mean_signed_RGB={:+.2},{:+.2},{:+.2}",
                self.samples,
                self.error[0] as f64 / denom,
                self.error[1] as f64 / denom,
                self.error[2] as f64 / denom,
                self.signed[0] as f64 / denom,
                self.signed[1] as f64 / denom,
                self.signed[2] as f64 / denom,
            );
            for (label, count, signed) in [
                ("camera_neutral_candidates", self.neutral_samples, self.neutral_signed),
                ("camera_warm_candidates", self.skin_candidate_samples, self.skin_signed),
            ] {
                if count == 0 {
                    eprintln!("PIC_RAW_QA group={label} count=0 (not sampled)");
                    continue;
                }
                let d = count as f64;
                eprintln!(
                    "PIC_RAW_QA group={} count={} mean_signed_RGB={:+.2},{:+.2},{:+.2}",
                    label, count,
                    signed[0] as f64 / d,
                    signed[1] as f64 / d,
                    signed[2] as f64 / d,
                );
            }
        }
    }

    fn report_pair(actual: &RgbImage, reference: &RgbImage) -> Option<Report> {
        if actual.dimensions() != reference.dimensions() {
            return None;
        }
        let mut report = Report::default();
        for y in (0..actual.height()).step_by(16) {
            for x in (0..actual.width()).step_by(16) {
                report.record(actual.get_pixel(x, y).0, reference.get_pixel(x, y).0);
            }
        }
        Some(report)
    }

    #[test]
    fn diagnostic_reports_channel_cast_without_modifying_pixels() {
        let camera = RgbImage::from_pixel(32, 32, image::Rgb([140, 140, 140]));
        let raw = RgbImage::from_pixel(32, 32, image::Rgb([115, 170, 125]));
        let report = report_pair(&raw, &camera).expect("equal sizes");
        assert_eq!(report.samples, 4);
        assert_eq!(report.signed, [-100, 120, -60]);
        assert_eq!(report.neutral_samples, 4);
        assert_eq!(camera.get_pixel(0, 0).0, [140, 140, 140]);
    }

    #[test]
    fn diagnostic_rejects_misaligned_rasters() {
        let a = RgbImage::new(32, 32);
        let b = RgbImage::new(32, 33);
        assert!(report_pair(&a, &b).is_none());
    }

    #[test]
    #[ignore = "requires PIC_TEST_RAW pointing to a local Nikon NEF; NEVER commit the test image"]
    fn real_nef_reference_report() {
        let filename = std::env::var("PIC_TEST_RAW").expect("set PIC_TEST_RAW to an existing Nikon NEF");
        assert!(is_nikon_raw(&filename), "the fixture must be Nikon NEF/NRW");
        let path = Path::new(&filename);
        let original_sha = blake3::hash(&std::fs::read(path).expect("read source NEF"));
        let camera_jpeg = nef_embedded_preview(path)
            .expect("inspect Nikon embedded JPEG")
            .expect("Nikon JPEG required for reference comparison");

        let started = Instant::now();
        let raw_source = rawler::rawsource::RawSource::new(path).expect("open test RAW");
        // Intentionally bypass match_nef_sensor_to_camera_jpeg(): this test
        // measures the existing RAW developer independently of any
        // per-photograph fitting or reference JPEG corrections.
        let sensor = develop_nef_sensor(&raw_source, &filename).expect("develop sensor only");
        let sensor_ms = started.elapsed().as_secs_f64() * 1000.0;
        let sensor_kind = format!("{:?}", sensor.color());
        let (camera_w, camera_h) = jpeg_dimensions(&camera_jpeg).expect("read JPEG dimensions");

        eprintln!(
            "PIC_RAW_QA source={} sensor={}x{} kind={} sensor_develop_ms={:.2} camera={}x{}",
            path.file_name().and_then(|name| name.to_str()).unwrap_or("unknown"),
            sensor.width(), sensor.height(), sensor_kind,
            sensor_ms, camera_w, camera_h
        );
        let reference = decode_viewer_jpeg(&camera_jpeg, camera_w, camera_h, &filename)
            .expect("colour-manage Nikon JPEG").to_rgb8();
        let actual = sensor.to_rgb8();

        match report_pair(&actual, &reference) {
            Some(report) => report.print(),
            None => eprintln!(
                "PIC_RAW_QA result=dimensions_mismatch sensor={}x{} camera={}x{} \
                 statistical comparison skipped: raster alignment must be established first",
                actual.width(), actual.height(), camera_w, camera_h
            ),
        }
        if let Some(dir) = std::env::var_os("PIC_TEST_OUTPUT_DIR") {
            std::fs::create_dir_all(&dir).expect("create private report directory");
            let folder = Path::new(&dir);
            std::fs::write(folder.join("camera-embedded.jpg"), &camera_jpeg)
                .expect("write camera JPEG unchanged");
            sensor.save(folder.join("rawler-sensor16.tiff"))
                .expect("write 16-bit sensor-developed TIFF");
            eprintln!("PIC_RAW_QA outputs={}", folder.display());
        }
        assert_eq!(
            original_sha, blake3::hash(&std::fs::read(path).expect("re-read original NEF")),
            "diagnostic must not modify the source image"
        );
    }
}
