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

    // Reference and sensor RGB values are display encoded. Decode to linear
    // sRGB for luminance and relative channel-share measurements. The latter
    // separates brightness mismatch from white balance / hue mismatch, but
    // is only a diagnostic (not perceptual Delta E or camera calibration).
    fn srgb_linear(value: u8) -> f64 {
        let x = f64::from(value) / 255.0;
        if x <= 0.04045 {
            x / 12.92
        } else {
            ((x + 0.055) / 1.055).powf(2.4)
        }
    }

    #[derive(Default, Debug)]
    struct GroupColor {
        count: u64,
        linear_luma_sensor: f64,
        linear_luma_camera: f64,
        chroma_delta: [f64; 3],
    }

    impl GroupColor {
        fn record(&mut self, sensor: [u8; 3], camera: [u8; 3]) {
            let s = sensor.map(srgb_linear);
            let c = camera.map(srgb_linear);
            let ssum: f64 = s.iter().sum();
            let csum: f64 = c.iter().sum();
            // Near-black or clipped material cannot determine relative
            // colour accurately. Do not let those pixels dominate hue stats.
            if ssum < 0.025 || csum < 0.025
                || sensor.iter().any(|&v| v >= 252)
                || camera.iter().any(|&v| v >= 252) {
                return;
            }
            self.count += 1;
            self.linear_luma_sensor += 0.2126 * s[0] + 0.7152 * s[1] + 0.0722 * s[2];
            self.linear_luma_camera += 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
            for i in 0..3 {
                self.chroma_delta[i] += s[i] / ssum - c[i] / csum;
            }
        }

        fn print(&self, label: &str) {
            if self.count == 0 {
                eprintln!("PIC_RAW_QA_LIGHT group={label} valid=0");
                return;
            }
            let n = self.count as f64;
            eprintln!(
                "PIC_RAW_QA_LIGHT group={} valid={} mean_linear_luma_sensor={:.4} camera={:.4} \
                 relative_luma_ratio={:.3} chroma_fraction_delta_RGB={:+.4},{:+.4},{:+.4}",
                label,
                self.count,
                self.linear_luma_sensor / n,
                self.linear_luma_camera / n,
                self.linear_luma_sensor / self.linear_luma_camera.max(f64::EPSILON),
                self.chroma_delta[0] / n,
                self.chroma_delta[1] / n,
                self.chroma_delta[2] / n,
            );
        }
    }

    #[derive(Default, Debug)]
    struct Report {
        samples: u64,
        error: [u64; 3],
        signed: [i64; 3],
        neutral_samples: u64,
        neutral_signed: [i64; 3],
        skin_candidate_samples: u64,
        skin_signed: [i64; 3],
        scene_color: GroupColor,
        neutral_color: GroupColor,
        warm_color: GroupColor,
    }

    impl Report {
        fn record(&mut self, actual: [u8; 3], reference: [u8; 3]) {
            self.samples += 1;
            self.scene_color.record(actual, reference);
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
                self.neutral_color.record(actual, reference);
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
                self.warm_color.record(actual, reference);
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
            self.scene_color.print("all_nonclipped");
            self.neutral_color.print("camera_neutral_candidates");
            self.warm_color.print("camera_warm_candidates");
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
    fn colour_statistics_separate_brightness_from_cast() {
        // Same channel ratios, different brightness: luminance changes,
        // relative chroma stays close (allow sRGB transfer curvature).
        let reference = [120, 120, 120];
        let dark = [80, 80, 80];
        let green = [80, 150, 80];
        let mut brightness = GroupColor::default();
        brightness.record(dark, reference);
        assert_eq!(brightness.count, 1);
        assert!(brightness.linear_luma_sensor < brightness.linear_luma_camera);
        assert!(brightness.chroma_delta.iter().all(|v| v.abs() < 1e-6));

        let mut colour = GroupColor::default();
        colour.record(green, reference);
        assert_eq!(colour.count, 1);
        assert!(colour.chroma_delta[1] > 0.20, "strong green cast must be measurable");
        assert!(colour.chroma_delta[0] < 0.0);
    }

    #[test]
    fn diagnostic_rejects_misaligned_rasters() {
        let a = RgbImage::new(32, 32);
        let b = RgbImage::new(32, 33);
        assert!(report_pair(&a, &b).is_none());
    }

    fn describe_camera_metadata(raw: &rawler::RawImage) {
        use rawler::imgop::xyz::Illuminant;
        // Report exactly the WB coefficients and the preferred illuminant
        // matrix used by PIC's current RAW developer. These are Nikon
        // metadata/decoder results, never coefficients fitted to the JPEG.
        let selected = raw.color_matrix_find_first([
            Illuminant::D65,
            Illuminant::A,
            Illuminant::B,
            Illuminant::C,
            Illuminant::D50,
            Illuminant::D55,
            Illuminant::D75,
            Illuminant::Daylight,
            Illuminant::Flash,
        ]);
        eprintln!(
            "PIC_RAW_QA_META make={:?} model={:?} bits_per_sample={} \
             white_balance_RGBE={:?} white_balance_available={} \
             matrix_illuminants={:?}",
            raw.clean_make, raw.clean_model, raw.bps,
            raw.wb_coeffs, raw.wb_coeffs[0].is_finite(),
            raw.color_matrix.keys().collect::<Vec<_>>(),
        );
        if let Some((illuminant, matrix)) = selected {
            eprintln!(
                "PIC_RAW_QA_META selected_illuminant={illuminant:?} \
                 selected_xyz_to_camera_matrix={matrix:?} \
                 blacklevels={:?} whitelevels={:?}",
                raw.blacklevel, raw.whitelevel,
            );
        } else {
            eprintln!(
                "PIC_RAW_QA_META selected_illuminant=none matrix=identity_fallback \
                 blacklevels={:?} whitelevels={:?}",
                raw.blacklevel, raw.whitelevel,
            );
        }
    }

    fn write_comparison_preview(rgb: &RgbImage, path: &Path) {
        // 1400px reference images are small enough to share without sending
        // a 150MB TIFF or the original identifiable NEF to a public repository.
        let long = rgb.width().max(rgb.height()).max(1);
        let width = ((u64::from(rgb.width()) * 1400) / u64::from(long)).max(1) as u32;
        let height = ((u64::from(rgb.height()) * 1400) / u64::from(long)).max(1) as u32;
        let reduced = image::imageops::resize(
            rgb, width, height, image::imageops::FilterType::Lanczos3
        );
        reduced.save(path).expect("write colour comparison PNG");
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
        // Decode once, inspect Nikon metadata BEFORE colour development,
        // then apply the same unchanged RawDevelop::default() path used by PIC.
        // The diagnostic does not regress JPEG pixels or change photo data.
        let sensor = rawler_decode(&filename, "nef_qa_sensor_demosaic", || {
            let decoder = rawler::get_decoder(&raw_source)?;
            let params = rawler::decoders::RawDecodeParams::default();
            let raw = decoder.raw_image(&raw_source, &params, false)?;
            describe_camera_metadata(&raw);
            rawler::imgop::develop::RawDevelop::default()
                .develop_intermediate(&raw)?
                .to_dynamic_image()
                .ok_or_else(|| anyhow::anyhow!("Nikon RAW developer returned no RGB image"))
        }).expect("develop sensor only");
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
            write_comparison_preview(&reference, &folder.join("nikon-preview.png"));
            write_comparison_preview(&actual, &folder.join("sensor-preview.png"));
            eprintln!(
                "PIC_RAW_QA outputs={} comparison=nikon-preview.png,sensor-preview.png",
                folder.display()
            );
        }
        assert_eq!(
            original_sha, blake3::hash(&std::fs::read(path).expect("re-read original NEF")),
            "diagnostic must not modify the source image"
        );
    }
}
