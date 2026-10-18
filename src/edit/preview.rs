#[derive(Clone)]
struct PreviewBase {
    path: String,
    rotation: i32,
    target_width: u32,
    target_height: u32,
    image: image::RgbaImage,
}

impl PreviewBase {
    fn matches(&self, path: &str, rotation: i32, target_width: u32, target_height: u32) -> bool {
        self.path == path
            && self.rotation == rotation
            && self.target_width == target_width
            && self.target_height == target_height
    }
}

const PREVIEW_BASE_CACHE_CAPACITY: usize = 2;

fn preview_target_dimensions(
    source_width: i64,
    source_height: i64,
    rotation: i32,
) -> (u32, u32) {
    // Edit renders retain the full source resolution at every zoom level.
    // The canvas scales that texture for Fit mode; keeping the source pixels
    // avoids a resolution drop when an adjustment is applied.
    let (mut width, mut height) = (
        if source_width > 0 {
            source_width as u32
        } else {
            u32::MAX
        },
        if source_height > 0 {
            source_height as u32
        } else {
            u32::MAX
        },
    );
    if matches!(rotation.rem_euclid(360), 90 | 270) {
        std::mem::swap(&mut width, &mut height);
    }
    (width, height)
}

fn cached_preview_base(
    cache: &Arc<Mutex<Vec<PreviewBase>>>,
    path: &str,
    rotation: i32,
    target_width: u32,
    target_height: u32,
) -> Option<image::RgbaImage> {
    cache
        .lock()
        .ok()?
        .iter()
        .find(|entry| entry.matches(path, rotation, target_width, target_height))
        .map(|entry| entry.image.clone())
}

fn store_preview_base(cache: &Arc<Mutex<Vec<PreviewBase>>>, entry: PreviewBase) {
    let Ok(mut cache) = cache.lock() else {
        return;
    };
    cache.retain(|existing| {
        !existing.matches(
            &entry.path,
            entry.rotation,
            entry.target_width,
            entry.target_height,
        )
    });
    cache.push(entry);
    while cache.len() > PREVIEW_BASE_CACHE_CAPACITY {
        cache.remove(0);
    }
}

#[derive(Clone)]
struct PreviewGeometry {
    path: String,
    rotation: i32,
    target_width: u32,
    target_height: u32,
    crop: CropRect,
    straighten: f32,
    image: image::RgbaImage,
}

impl PreviewGeometry {
    fn matches(
        &self,
        path: &str,
        rotation: i32,
        target_width: u32,
        target_height: u32,
        crop: CropRect,
        straighten: f32,
    ) -> bool {
        self.path == path
            && self.rotation == rotation
            && self.target_width == target_width
            && self.target_height == target_height
            && self.crop == crop.normalized()
            && self.straighten == straighten
    }
}

struct PreviewJob {
    generation: u64,
    path: String,
    rotation: i32,
    target_width: u32,
    target_height: u32,
    recipe: EditRecipe,
    result_sender: std::sync::mpsc::Sender<anyhow::Result<(u64, u32, u32, Vec<u8>)>>,
}

fn spawn_preview_worker(
    base_cache: Arc<Mutex<Vec<PreviewBase>>>,
) -> std::sync::mpsc::Sender<PreviewJob> {
    let (job_sender, job_receiver) = std::sync::mpsc::channel::<PreviewJob>();
    std::thread::spawn(move || {
        let mut geometry_cache: Option<PreviewGeometry> = None;
        let mut pending: Option<PreviewJob> = None;

        loop {
            let mut job = match pending.take() {
                Some(job) => job,
                None => match job_receiver.recv() {
                    Ok(job) => job,
                    Err(_) => break,
                },
            };

            while let Ok(newer) = job_receiver.try_recv() {
                job = newer;
            }


            let result = render_preview_job(&job, &base_cache, &mut geometry_cache).map(|image| {
                (
                    job.generation,
                    image.width(),
                    image.height(),
                    image.into_raw(),
                )
            });

            // If newer slider requests arrived while this frame was rendering,
            // keep only the newest one. The just-completed frame is stale, but
            // its decoded/geometry cache work remains useful to the next job.
            let mut newest = None;
            while let Ok(next) = job_receiver.try_recv() {
                newest = Some(next);
            }
            if let Some(next) = newest {
                pending = Some(next);
                continue;
            }

            let _ = job.result_sender.send(result);
        }
    });
    job_sender
}

fn render_preview_job(
    job: &PreviewJob,
    base_cache: &Arc<Mutex<Vec<PreviewBase>>>,
    geometry_cache: &mut Option<PreviewGeometry>,
) -> anyhow::Result<image::RgbaImage> {
    let crop = job.recipe.crop.normalized();
    let straighten = job.recipe.straighten;
    let geometry_needed = straighten.abs() >= 0.01 || !crop.is_full();

    let geometry_hit = geometry_needed
        && geometry_cache.as_ref().is_some_and(|entry| {
            entry.matches(
                &job.path,
                job.rotation,
                job.target_width,
                job.target_height,
                crop,
                straighten,
            )
        });


    let render_started = std::time::Instant::now();
    let geometry = if geometry_hit {
        geometry_cache
            .as_ref()
            .expect("geometry cache matched")
            .image
            .clone()
    } else {
        let cached = cached_preview_base(
            base_cache,
            &job.path,
            job.rotation,
            job.target_width,
            job.target_height,
        );
        let (base, cache_status) = if let Some(image) = cached {
            (image, "hit")
        } else {
            let image = super::render::decode_base_for_viewer(
                &job.path,
                job.rotation,
                job.target_width,
                job.target_height,
            )?;
            store_preview_base(
                base_cache,
                PreviewBase {
                    path: job.path.clone(),
                    rotation: job.rotation,
                    target_width: job.target_width,
                    target_height: job.target_height,
                    image: image.clone(),
                },
            );
            (image, "miss")
        };
        if std::env::var_os("PICASA_TRACE").is_some() {
            eprintln!(
                "PIC_EDIT source_base cache={} target={}x{} decoded={}x{} uri={}",
                cache_status,
                job.target_width,
                job.target_height,
                base.width(),
                base.height(),
                crate::thumbnail::viewer_trace_reference(&job.path),
            );
        }

        if geometry_needed {
            let image = super::render::apply_geometry(base, &job.recipe);
            geometry_cache.replace(PreviewGeometry {
                path: job.path.clone(),
                rotation: job.rotation,
                target_width: job.target_width,
                target_height: job.target_height,
                crop,
                straighten,
                image: image.clone(),
            });
            image
        } else {
            base
        }
    };

    let geometry_dimensions = (geometry.width(), geometry.height());
    let tone_started = std::time::Instant::now();
    let rendered = super::render::apply_tone(geometry, &job.recipe);
    let tone_elapsed_ms = tone_started.elapsed().as_secs_f64() * 1000.0;
    if std::env::var_os("PICASA_TRACE").is_some() {
        eprintln!(
            "PIC_EDIT adjustment_buffer geometry={}x{} output={}x{} recipe_default={} sharpen={:.3} clarity={:.3} grain={:.3} filter={:?} tone_ms={:.3} job_ms={:.3} uri={}",
            geometry_dimensions.0,
            geometry_dimensions.1,
            rendered.width(),
            rendered.height(),
            job.recipe.is_default(),
            job.recipe.sharpen,
            job.recipe.clarity,
            job.recipe.grain,
            job.recipe.filter,
            tone_elapsed_ms,
            render_started.elapsed().as_secs_f64() * 1000.0,
            crate::thumbnail::viewer_trace_reference(&job.path),
        );
    }


    Ok(rendered)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edit_preview_at_fit_and_one_to_one_keeps_source_resolution() {
        let source = (3936, 2624);
        assert_eq!(preview_target_dimensions(3936, 2624, 0), source);
        assert_eq!(preview_target_dimensions(3936, 2624, 90), (2624, 3936));
        assert_eq!(preview_target_dimensions(0, 0, 0), (u32::MAX, u32::MAX));
    }

    #[test]
    fn one_to_one_preview_keeps_native_dimensions_and_detail_through_edit_and_reset() {
        const WIDTH: u32 = 3936;
        const HEIGHT: u32 = 2624;
        let path = std::env::temp_dir().join(format!(
            "pic-edit-preview-quality-{}-{}.jpg",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let source = image::RgbImage::from_fn(WIDTH, HEIGHT, |x, y| {
            // A fine repeating edge pattern stands in for eyelashes and hair.
            let stripe = ((x / 5 + y / 11) % 2) as u8;
            let value = if stripe == 0 { 48 } else { 205 };
            image::Rgb([value, value, value])
        });
        source.save(&path).unwrap();
        let reference = path.to_string_lossy().into_owned();
        let cache = Arc::new(Mutex::new(Vec::new()));
        let mut geometry_cache = None;
        let render =
            |generation, recipe: EditRecipe, geometry_cache: &mut Option<PreviewGeometry>| {
                let (sender, _receiver) = std::sync::mpsc::channel();
                let job = PreviewJob {
                    generation,
                    path: reference.clone(),
                    rotation: 0,
                    target_width: WIDTH,
                    target_height: HEIGHT,
                    recipe,
                    result_sender: sender,
                };
                render_preview_job(&job, &cache, geometry_cache).unwrap()
            };

        let original = render(1, EditRecipe::default(), &mut geometry_cache);
        let mut edited_recipe = EditRecipe::default();
        edited_recipe.exposure = -0.03;
        let edited = render(2, edited_recipe, &mut geometry_cache);
        let reset = render(3, EditRecipe::default(), &mut geometry_cache);

        for (label, image) in [("zero", &original), ("edited", &edited), ("reset", &reset)] {
            assert_eq!((image.width(), image.height()), (WIDTH, HEIGHT), "{label}");
        }
        assert_eq!(
            original, reset,
            "reset must restore the decoded preview pixels"
        );
        let edge_count = |image: &image::RgbaImage| {
            (0..WIDTH - 1)
                .step_by(2)
                .filter(|&x| {
                    let left = image.get_pixel(x, 100)[0] as i16;
                    let right = image.get_pixel(x + 1, 100)[0] as i16;
                    (left - right).abs() > 35
                })
                .count()
        };
        let baseline_edges = edge_count(&original);
        let edited_edges = edge_count(&edited);
        eprintln!(
            "PIC_EDIT test_detail dimensions={}x{} fine_edges_zero={} fine_edges_exposure={}",
            WIDTH, HEIGHT, baseline_edges, edited_edges
        );
        assert!(baseline_edges > 100, "fixture should contain fine detail");
        assert!(
            edited_edges * 4 >= baseline_edges * 3,
            "minor exposure edit lost fine edges"
        );
        // Unsupported effects must use the same cached full source, too.
        for (generation, sharpen, clarity) in [(4, 0.425, 0.0), (5, 0.0, 0.201)] {
            let mut recipe = EditRecipe::default();
            recipe.sharpen = sharpen;
            recipe.clarity = clarity;
            let adjusted = render(generation, recipe, &mut geometry_cache);
            assert_eq!(adjusted.dimensions(), (WIDTH, HEIGHT));
            assert!(edge_count(&adjusted) >= baseline_edges * 3 / 4);
        }
        assert_eq!(cache.lock().unwrap().len(), 1, "effects must reuse one decoded source");
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    #[ignore = "requires PIC_TEST_EDIT_REFERENCE for a real local or NFS photo"]
    fn real_photo_zero_exposure_reset_keep_editor_preview_resolution() {
        let reference = std::env::var("PIC_TEST_EDIT_REFERENCE")
            .expect("set PIC_TEST_EDIT_REFERENCE to a local path or NFS URI");
        let cache = Arc::new(Mutex::new(Vec::new()));
        let mut geometry_cache = None;
        let render =
            |generation, recipe: EditRecipe, geometry_cache: &mut Option<PreviewGeometry>| {
                let (sender, _receiver) = std::sync::mpsc::channel();
                let job = PreviewJob {
                    generation,
                    path: reference.clone(),
                    rotation: 0,
                    target_width: 3960,
                    target_height: 2640,
                    recipe,
                    result_sender: sender,
                };
                render_preview_job(&job, &cache, geometry_cache).unwrap()
            };

        let original = render(1, EditRecipe::default(), &mut geometry_cache);
        let mut recipe = EditRecipe::default();
        recipe.exposure = -0.03;
        let edited = render(2, recipe, &mut geometry_cache);
        let reset = render(3, EditRecipe::default(), &mut geometry_cache);
        let dimensions = (original.width(), original.height());
        assert!(dimensions.0 > 2000 && dimensions.1 > 1000, "unexpected source resolution: {dimensions:?}");
        assert_eq!((edited.width(), edited.height()), dimensions);
        assert_eq!((reset.width(), reset.height()), dimensions);
        assert_eq!(original, reset);
        eprintln!(
            "PIC_EDIT real_photo dimensions={}x{} exposure_dimensions={}x{} reset=identical",
            dimensions.0,
            dimensions.1,
            edited.width(),
            edited.height(),
        );
    }
}
