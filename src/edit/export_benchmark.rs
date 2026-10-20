//! Debug-only benchmark for an existing album, invoked through `cargo run`.

use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use image::RgbaImage;

use super::export_batch::{self, ExportFormat, ExportJob};
use super::model::EditRecipe;

#[derive(Clone, Copy)]
enum ResizeMethod {
    ImageLanczos3,
    SimdLanczos3,
    SimdHamming,
}

impl ResizeMethod {
    fn name(self) -> &'static str {
        match self {
            Self::ImageLanczos3 => "image_lanczos3_reference",
            Self::SimdLanczos3 => "simd_lanczos3",
            Self::SimdHamming => "simd_hamming",
        }
    }
}

fn fit_with_method(image: RgbaImage, max_edge: u32, method: ResizeMethod) -> Result<RgbaImage> {
    let (width, height) = image.dimensions();
    let long_edge = width.max(height);
    if max_edge == 0 || long_edge <= max_edge {
        return Ok(image);
    }
    let scale = max_edge as f64 / long_edge as f64;
    let target_width = ((width as f64 * scale).round() as u32).max(1);
    let target_height = ((height as f64 * scale).round() as u32).max(1);
    match method {
        ResizeMethod::ImageLanczos3 => Ok(image::imageops::resize(
            &image,
            target_width,
            target_height,
            image::imageops::FilterType::Lanczos3,
        )),
        ResizeMethod::SimdLanczos3 | ResizeMethod::SimdHamming => {
            let source = fast_image_resize::images::Image::from_vec_u8(
                width,
                height,
                image.into_raw(),
                fast_image_resize::PixelType::U8x4,
            )?;
            let mut destination = fast_image_resize::images::Image::new(
                target_width,
                target_height,
                fast_image_resize::PixelType::U8x4,
            );
            let filter = match method {
                ResizeMethod::SimdLanczos3 => fast_image_resize::FilterType::Lanczos3,
                ResizeMethod::SimdHamming => fast_image_resize::FilterType::Hamming,
                ResizeMethod::ImageLanczos3 => unreachable!(),
            };
            let options = fast_image_resize::ResizeOptions::new()
                .resize_alg(fast_image_resize::ResizeAlg::Convolution(filter));
            fast_image_resize::Resizer::new().resize(&source, &mut destination, &options)?;
            RgbaImage::from_raw(target_width, target_height, destination.into_vec())
                .context("benchmark resizer returned an invalid RGBA buffer")
        }
    }
}

fn image_error(reference: &RgbaImage, candidate: &RgbaImage) -> (f64, f64) {
    let mut absolute = 0u64;
    let mut squared = 0u64;
    let mut samples = 0u64;
    for (a, b) in reference
        .as_raw()
        .chunks_exact(4)
        .zip(candidate.as_raw().chunks_exact(4))
    {
        for channel in 0..3 {
            let difference = (i32::from(a[channel]) - i32::from(b[channel])).unsigned_abs() as u64;
            absolute += difference;
            squared += difference * difference;
            samples += 1;
        }
    }
    let mae = absolute as f64 / samples.max(1) as f64;
    let mse = squared as f64 / samples.max(1) as f64;
    let psnr = if mse == 0.0 {
        f64::INFINITY
    } else {
        10.0 * (255.0f64 * 255.0 / mse).log10()
    };
    (mae, psnr)
}

fn median(values: &mut [f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.sort_by(f64::total_cmp);
    values[values.len() / 2]
}

fn bench_resizers(jobs: &[ExportJob], max_edge: u32) -> Result<()> {
    let methods = [
        ResizeMethod::ImageLanczos3,
        ResizeMethod::SimdLanczos3,
        ResizeMethod::SimdHamming,
    ];
    let mut durations: [Vec<f64>; 3] = std::array::from_fn(|_| Vec::new());
    let mut errors: [(f64, f64); 3] = [(0.0, 0.0); 3];
    let mut rendered_count = 0usize;
    let render_started = Instant::now();
    for job in jobs {
        let rendered = export_batch::render_job(job).with_context(|| {
            format!(
                "render failed for {}",
                Path::new(&job.source_path)
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
            )
        })?;
        let mut outputs = Vec::with_capacity(methods.len());
        for (index, method) in methods.iter().copied().enumerate() {
            // Exclude the input clone from resize timing; real Export already
            // owns its decoded pixels and passes them straight to the resizer.
            let input = rendered.clone();
            let started = Instant::now();
            let output = fit_with_method(input, max_edge, method)?;
            durations[index].push(started.elapsed().as_secs_f64() * 1000.0);
            outputs.push(output);
        }
        for index in 1..methods.len() {
            let (mae, psnr) = image_error(&outputs[0], &outputs[index]);
            errors[index].0 += mae;
            errors[index].1 += psnr;
        }
        rendered_count += 1;
    }
    let render_ms = render_started.elapsed().as_secs_f64() * 1000.0;
    eprintln!(
        "PIC_EXPORT_BENCH phase=render_for_comparison photos={} elapsed_ms={render_ms:.1}",
        rendered_count,
    );
    for (index, method) in methods.iter().copied().enumerate() {
        let mut values = durations[index].clone();
        let total: f64 = values.iter().sum();
        if index == 0 {
            eprintln!(
                "PIC_EXPORT_BENCH method={} resize_total_ms={total:.1} resize_median_ms={:.1} quality=reference",
                method.name(), median(&mut values),
            );
        } else {
            let photos = rendered_count.max(1) as f64;
            eprintln!(
                "PIC_EXPORT_BENCH method={} resize_total_ms={total:.1} resize_median_ms={:.1} mean_mae_vs_reference={:.3} mean_psnr_vs_reference_db={:.2}",
                method.name(), median(&mut values), errors[index].0 / photos, errors[index].1 / photos,
            );
        }
    }
    Ok(())
}

fn make_jobs(photos: Vec<crate::db::Photo>) -> Vec<ExportJob> {
    photos
        .into_iter()
        .map(|photo| {
            let file_name = Path::new(&photo.path)
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            let mut job = ExportJob::from_record(
                photo.path,
                photo.rotation,
                photo.edit_recipe,
                photo.width.unwrap_or(0),
                photo.height.unwrap_or(0),
                &file_name,
            );
            job.file_name = export_batch::file_name_for_format(&job.file_name, ExportFormat::Jpeg);
            job
        })
        .collect()
}

fn repeat_jobs(jobs: &[ExportJob], repeat_count: usize) -> Result<Vec<ExportJob>> {
    let capacity = jobs
        .len()
        .checked_mul(repeat_count)
        .context("repeated benchmark job count overflowed")?;
    let mut repeated = Vec::with_capacity(capacity);
    for _ in 0..repeat_count {
        repeated.extend(jobs.iter().cloned());
    }
    Ok(repeated)
}

fn validate_export_jpegs(output: &Path, max_edge: u32) -> Result<usize> {
    let mut count = 0;
    for entry in std::fs::read_dir(output)? {
        let entry = entry?;
        if entry.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        let path = entry.path();
        let decoded = image::open(&path).with_context(|| {
            format!(
                "benchmark JPEG did not decode: {}",
                entry.file_name().to_string_lossy()
            )
        })?;
        let (width, height) = (decoded.width(), decoded.height());
        if width > max_edge || height > max_edge {
            anyhow::bail!("benchmark JPEG exceeds the selected 4K edge limit");
        }
        let file = std::fs::File::open(&path)?;
        let metadata = exif::Reader::new()
            .read_from_container(&mut BufReader::new(file))
            .context("benchmark JPEG has unreadable EXIF")?;
        let field = |tag| metadata.get_field(tag, exif::In::PRIMARY);
        if field(exif::Tag::Orientation).and_then(|field| field.value.get_uint(0)) != Some(1)
            || field(exif::Tag::PixelXDimension).and_then(|field| field.value.get_uint(0))
                != Some(width)
            || field(exif::Tag::PixelYDimension).and_then(|field| field.value.get_uint(0))
                != Some(height)
            || field(exif::Tag::GPSLatitude).is_some()
            || field(exif::Tag::GPSLongitude).is_some()
        {
            anyhow::bail!("benchmark JPEG metadata validation failed");
        }
        count += 1;
    }
    Ok(count)
}

fn run_real_batch(
    jobs: &[ExportJob],
    output: &Path,
    max_edge: u32,
    workers: usize,
) -> Result<Duration> {
    std::fs::create_dir_all(output).with_context(|| {
        format!(
            "could not create benchmark output directory {}",
            output.display()
        )
    })?;
    let started = Instant::now();
    let outcome = export_batch::run_batch_export_with_workers(
        jobs,
        output,
        max_edge,
        export_batch::default_export_quality(),
        ExportFormat::Jpeg,
        workers,
        export_batch::render_job,
        |_, _, _, _| {},
        None,
    );
    let elapsed = started.elapsed();
    if outcome.failed > 0 || outcome.skipped > 0 || outcome.exported != jobs.len() {
        anyhow::bail!(
            "batch incomplete: exported={} skipped={} failed={} errors={:?}",
            outcome.exported,
            outcome.skipped,
            outcome.failed,
            outcome.errors,
        );
    }
    let validated = validate_export_jpegs(output, max_edge)?;
    if validated != jobs.len() {
        anyhow::bail!(
            "validated {validated} JPEGs for {} input photos",
            jobs.len()
        );
    }
    Ok(elapsed)
}

fn load_album_jobs(album_name: &str) -> Result<(String, Vec<ExportJob>)> {
    let database_path = crate::db::database_path()?;
    let connection = crate::db::open_existing(&database_path)?;
    let album = crate::db::albums(&connection)?
        .into_iter()
        .find(|album| album.name.eq_ignore_ascii_case(album_name))
        .with_context(|| {
            format!("album {album_name:?} was not found in the default PIC library")
        })?;
    let jobs = make_jobs(crate::db::photos_in_album(&connection, album.id, None)?);
    if jobs.is_empty() {
        anyhow::bail!("album {album_name:?} contains no photos");
    }
    for job in &jobs {
        if !crate::source::file_available(&job.source_path) {
            anyhow::bail!("an album source is unavailable; benchmark aborted before exporting");
        }
    }
    Ok((album.name, jobs))
}

fn peak_rss_kib() -> Result<u64> {
    let status = std::fs::read_to_string("/proc/self/status")?;
    let line = status
        .lines()
        .find(|line| line.starts_with("VmHWM:"))
        .context("VmHWM was not available in /proc/self/status")?;
    Ok(line
        .split_whitespace()
        .nth(1)
        .context("VmHWM value was missing")?
        .parse()?)
}

fn run_one_worker_configuration(album_name: &str, workers: usize, output: &Path) -> Result<()> {
    const MAX_EDGE: u32 = 3840;
    let (_, base_jobs) = load_album_jobs(album_name)?;
    let repeat_count = std::env::var("PIC_EXPORT_BENCH_REPEAT")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|&value| (1..=32).contains(&value))
        .unwrap_or(1);
    let jobs = repeat_jobs(&base_jobs, repeat_count)?;
    std::fs::create_dir_all(output)?;
    let started = Instant::now();
    let result = run_real_batch(&jobs, output, MAX_EDGE, workers);
    let peak_rss = peak_rss_kib()?;
    let elapsed = result?;
    std::fs::write(
        output.join(".benchmark-summary"),
        format!(
            "elapsed_ms={}\npeak_rss_kib={}\n",
            elapsed.as_millis(),
            peak_rss
        ),
    )?;
    eprintln!(
        "PIC_EXPORT_BENCH child=pass workers={workers} photos={} batch_ms={} peak_rss_kib={} wall_ms={}",
        jobs.len(), elapsed.as_millis(), peak_rss, started.elapsed().as_millis(),
    );
    Ok(())
}

#[derive(Default)]
struct GpuSample {
    start_busy: Option<u64>,
    peak_busy: Option<u64>,
    start_vram: Option<u64>,
    peak_vram: Option<u64>,
}

fn read_gpu_counters() -> Option<(u64, u64)> {
    let cards = std::fs::read_dir("/sys/class/drm").ok()?;
    for card in cards.flatten() {
        let name = card.file_name();
        if !name.to_string_lossy().starts_with("card") || name.to_string_lossy().contains('-') {
            continue;
        }
        let device = card.path().join("device");
        let busy = std::fs::read_to_string(device.join("gpu_busy_percent"))
            .ok()?
            .trim()
            .parse()
            .ok()?;
        let vram = std::fs::read_to_string(device.join("mem_info_vram_used"))
            .ok()?
            .trim()
            .parse()
            .ok()?;
        return Some((busy, vram));
    }
    None
}

fn read_child_peak_rss(child_pid: u32) -> Option<u64> {
    let status = std::fs::read_to_string(format!("/proc/{child_pid}/status")).ok()?;
    status
        .lines()
        .find(|line| line.starts_with("VmRSS:"))?
        .split_whitespace()
        .nth(1)?
        .parse()
        .ok()
}

struct WorkerRun {
    workers: usize,
    output: PathBuf,
    elapsed_ms: u128,
    peak_rss_kib: u64,
    sampled_rss_kib: u64,
    gpu: GpuSample,
    trace_path: PathBuf,
}

fn run_worker_child(album: &str, workers: usize, parent_dir: &Path) -> Result<WorkerRun> {
    let output = parent_dir.join(format!("workers-{workers}"));
    std::fs::create_dir_all(&output)?;
    let trace_path = output.join(".trace.log");
    let trace = std::fs::File::create(&trace_path)?;
    let exe = std::env::current_exe()?;
    let mut child = Command::new(exe)
        .env("PIC_EXPORT_BENCH_ALBUM", album)
        .env("PIC_EXPORT_BENCH_WORKERS", workers.to_string())
        .env("PIC_EXPORT_BENCH_OUTPUT", &output)
        .env("PICASA_TRACE", "1")
        .env("PIC_OVERLAY_TRACE", "1")
        .stdout(Stdio::null())
        .stderr(Stdio::from(trace))
        .spawn()
        .context("could not start isolated export benchmark worker")?;
    let child_pid = child.id();
    let mut gpu = GpuSample::default();
    if let Some((busy, vram)) = read_gpu_counters() {
        gpu.start_busy = Some(busy);
        gpu.start_vram = Some(vram);
    }
    let mut sampled_rss_kib = 0;
    let status = loop {
        if let Some((busy, vram)) = read_gpu_counters() {
            gpu.peak_busy = Some(gpu.peak_busy.unwrap_or(0).max(busy));
            gpu.peak_vram = Some(gpu.peak_vram.unwrap_or(0).max(vram));
        }
        sampled_rss_kib = sampled_rss_kib.max(read_child_peak_rss(child_pid).unwrap_or(0));
        if let Some(status) = child.try_wait()? {
            break status;
        }
        std::thread::sleep(Duration::from_millis(25));
    };
    if !status.success() {
        let trace = std::fs::read_to_string(&trace_path).unwrap_or_default();
        anyhow::bail!(
            "{} worker benchmark failed; trace tail: {}",
            workers,
            trace
                .lines()
                .rev()
                .take(6)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect::<Vec<_>>()
                .join(" | ")
        );
    }
    let summary = std::fs::read_to_string(output.join(".benchmark-summary"))?;
    let parse = |key: &str| -> Result<u64> {
        summary
            .lines()
            .find_map(|line| line.strip_prefix(&format!("{key}=")))
            .with_context(|| format!("child summary omitted {key}"))?
            .parse()
            .map_err(Into::into)
    };
    Ok(WorkerRun {
        workers,
        output,
        elapsed_ms: parse("elapsed_ms")? as u128,
        peak_rss_kib: parse("peak_rss_kib")?,
        sampled_rss_kib,
        gpu,
        trace_path,
    })
}

fn compare_outputs(
    reference: &Path,
    candidate: &Path,
    jobs: &[ExportJob],
) -> Result<(usize, u64, f64)> {
    let mut exact = 0;
    let mut max_delta = 0u64;
    let mut absolute = 0u64;
    let mut samples = 0u64;
    for job in jobs {
        let baseline = image::open(reference.join(&job.file_name))?.to_rgba8();
        let current = image::open(candidate.join(&job.file_name))?.to_rgba8();
        if baseline.dimensions() != current.dimensions() {
            anyhow::bail!("worker output dimensions differ from the 1-worker baseline");
        }
        let mut same = true;
        for (a, b) in baseline.as_raw().iter().zip(current.as_raw()) {
            let delta = u64::from(a.abs_diff(*b));
            max_delta = max_delta.max(delta);
            absolute += delta;
            samples += 1;
            same &= delta == 0;
        }
        exact += usize::from(same);
    }
    Ok((exact, max_delta, absolute as f64 / samples.max(1) as f64))
}

fn trace_stage_summary(path: &Path) -> Result<String> {
    let trace = std::fs::read_to_string(path)?;
    let mut lines = Vec::new();
    for stage in [
        "decode_rotate",
        "geometry",
        "tone",
        "overlays",
        "fit_max_edge",
        "metadata",
        "encode",
    ] {
        let mut values = trace
            .lines()
            .filter_map(|line| {
                let rest = line.strip_prefix(&format!("PIC_EXPORT stage={stage} elapsed_ms="))?;
                rest.split_whitespace().next()?.parse::<f64>().ok()
            })
            .collect::<Vec<_>>();
        if !values.is_empty() {
            let total: f64 = values.iter().sum();
            values.sort_by(f64::total_cmp);
            lines.push(format!(
                "{stage}_sum_ms={total:.1}_median_ms={:.1}",
                values[values.len() / 2]
            ));
        }
    }
    Ok(lines.join(" "))
}

fn source_fingerprints(jobs: &[ExportJob]) -> Result<Vec<(u64, u128, blake3::Hash)>> {
    jobs.iter()
        .map(|job| {
            let path = Path::new(&job.source_path);
            let metadata = std::fs::metadata(path)?;
            let modified = metadata
                .modified()?
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos();
            let mut file = std::fs::File::open(path)?;
            let mut hasher = blake3::Hasher::new();
            std::io::copy(&mut file, &mut hasher)?;
            Ok((metadata.len(), modified, hasher.finalize()))
        })
        .collect()
}

pub fn run(album_name: &str) -> Result<()> {
    const MAX_EDGE: u32 = 3840;
    if let (Some(workers), Some(output)) = (
        std::env::var("PIC_EXPORT_BENCH_WORKERS")
            .ok()
            .and_then(|value| value.parse::<usize>().ok()),
        std::env::var_os("PIC_EXPORT_BENCH_OUTPUT"),
    ) {
        return run_one_worker_configuration(album_name, workers, Path::new(&output));
    }

    let (album, base_jobs) = load_album_jobs(album_name)?;
    let repeat_count = std::env::var("PIC_EXPORT_BENCH_REPEAT")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|&value| (1..=32).contains(&value))
        .unwrap_or(1);
    let jobs = repeat_jobs(&base_jobs, repeat_count)?;
    let overlay_jobs = jobs
        .iter()
        .filter(|job| {
            EditRecipe::decode(&job.edit_recipe)
                .overlays
                .iter()
                .any(|overlay| overlay.visible && overlay.opacity > 0.001)
        })
        .count();
    eprintln!(
        "PIC_EXPORT_BENCH album={} photos={} overlays_in={} max_edge={} format=JPEG quality={}",
        album,
        jobs.len(),
        overlay_jobs,
        MAX_EDGE,
        export_batch::default_export_quality(),
    );
    if repeat_count == 1 {
        bench_resizers(&jobs, MAX_EDGE)?;
    } else {
        eprintln!(
            "PIC_EXPORT_BENCH resize_comparison=skipped repeat_count={repeat_count} base_photos={}",
            base_jobs.len(),
        );
    }

    let fingerprints_before = source_fingerprints(&jobs)?;
    let parent_dir = std::env::temp_dir().join(format!("pic-export-bench-{}", std::process::id()));
    if parent_dir.exists() {
        std::fs::remove_dir_all(&parent_dir)?;
    }
    std::fs::create_dir_all(&parent_dir)?;
    let mut runs = Vec::new();
    let mut run_result = Ok(());
    for workers in [1, 2, 4, 6] {
        match run_worker_child(&album, workers, &parent_dir) {
            Ok(run) => runs.push(run),
            Err(error) => {
                run_result = Err(error);
                break;
            }
        }
    }
    if let Err(error) = run_result {
        let _ = std::fs::remove_dir_all(&parent_dir);
        return Err(error);
    }
    let fingerprints_after = source_fingerprints(&jobs)?;
    if fingerprints_before != fingerprints_after {
        let _ = std::fs::remove_dir_all(&parent_dir);
        anyhow::bail!("source files changed during benchmark");
    }
    let baseline = &runs[0].output;
    let mut winner: Option<&WorkerRun> = None;
    for run in &runs {
        let (exact, max_delta, mae) = compare_outputs(baseline, &run.output, &jobs)?;
        let trace_summary = trace_stage_summary(&run.trace_path)?;
        eprintln!(
            "PIC_EXPORT_BENCH workers={} batch_ms={} peak_rss_mib={:.1} sampled_rss_mib={:.1} gpu_busy_start={:?} gpu_busy_peak={:?} gpu_vram_start_mib={:?} gpu_vram_peak_mib={:?} exact_outputs={}/{} max_pixel_delta={} mean_pixel_delta={:.6} stages=[{}]",
            run.workers,
            run.elapsed_ms,
            run.peak_rss_kib as f64 / 1024.0,
            run.sampled_rss_kib as f64 / 1024.0,
            run.gpu.start_busy,
            run.gpu.peak_busy,
            run.gpu.start_vram.map(|bytes| bytes as f64 / 1_048_576.0),
            run.gpu.peak_vram.map(|bytes| bytes as f64 / 1_048_576.0),
            exact,
            jobs.len(),
            max_delta,
            mae,
            trace_summary,
        );
        if exact == jobs.len()
            && max_delta == 0
            && winner.is_none_or(|current| run.elapsed_ms < current.elapsed_ms)
        {
            winner = Some(run);
        }
    }
    let winner = winner.context("no worker configuration matched the 1-worker image baseline")?;
    let estimated_minutes = winner.elapsed_ms as f64 / jobs.len() as f64 * 3000.0 / 60_000.0;
    eprintln!(
        "PIC_EXPORT_BENCH winner_workers={} batch_ms={} estimated_3000_minutes={estimated_minutes:.1} image_match=exact source_fingerprints=unchanged",
        winner.workers, winner.elapsed_ms,
    );
    eprintln!("PIC_EXPORT_BENCH gpu_export_candidate=unavailable reason=export_renderer_is_cpu_and_live_gpu_requires_gtk_glarea_context");
    std::fs::remove_dir_all(&parent_dir).with_context(|| {
        format!(
            "could not remove benchmark outputs {}",
            parent_dir.display()
        )
    })?;
    Ok(())
}
