//! Experimental GTK GLArea renderer for live edit tone controls.
//!
//! The source texture remains at decoded source dimensions. GtkGLArea itself
//! is kept at the scrolled viewport size, so 24MP photos never create a 24MP
//! render target. `auto` intentionally remains on the CPU path until this
//! renderer has broader performance and platform validation.

use std::cell::RefCell;
use std::ffi::{c_void, CString};
use std::rc::Rc;
use std::sync::OnceLock;
use std::time::Instant;

use gtk::prelude::*;
use gtk4 as gtk;
use libloading::Library;

use super::filters::{self, FilterPreset};
use super::model::{CropRect, EditRecipe};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditRendererMode {
    Auto,
    Gpu,
    Cpu,
}

impl EditRendererMode {
    pub fn from_environment() -> Self {
        Self::parse(std::env::var("PIC_EDIT_RENDERER").ok().as_deref())
    }

    fn parse(value: Option<&str>) -> Self {
        match value {
            Some("gpu") => Self::Gpu,
            Some("cpu") => Self::Cpu,
            _ => Self::Auto,
        }
    }

    pub fn should_attempt_gpu(self) -> bool {
        matches!(self, Self::Gpu)
    }
}

#[derive(Clone, Debug, PartialEq)]
struct SourceKey {
    reference: String,
    rotation: i32,
    crop: CropRect,
    straighten: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct GpuTone {
    exposure: f32,
    contrast: f32,
    saturation: f32,
    temperature: f32,
    fill_light: f32,
    shadows: f32,
    highlights: f32,
    sharpen: f32,
    grain: f32,
    clarity: f32,
    black_white: bool,
    sepia: bool,
    filter_amount: f32,
    filter: FilterPreset,
    auto_color: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct GpuView {
    width: f32,
    height: f32,
    scroll_x: f32,
    scroll_y: f32,
    pad_x: f32,
    pad_y: f32,
    scale: f32,
}

struct GlObjects {
    program: u32,
    vao: u32,
    texture: u32,
    source_revision: u64,
    filter: u32,
    blur_program: u32,
    clarity_luma_texture: u32,
    clarity_blur_texture: u32,
    clarity_luma_fbo: u32,
    clarity_blur_fbo: u32,
    clarity_dimensions: (i32, i32),
    effect_luma_texture: u32,
    effect_blur_texture: u32,
    effect_luma_fbo: u32,
    effect_blur_fbo: u32,
    effect_dimensions: (i32, i32),
    auto_color_program: u32,
    auto_color_stats: u32,
    auto_color_fbo: u32,
    auto_color_revision: u64,
}

struct GpuState {
    key: Option<SourceKey>,
    source: Option<(i32, i32, glib::Bytes)>,
    source_revision: u64,
    tone: GpuTone,
    view: GpuView,
    objects: Option<GlObjects>,
    rendered_frames: u64,
    last_queued: Option<Instant>,
    frame_after_paint: Option<(Instant, Option<Instant>)>,
    failure: Option<String>,
    on_failure: Option<Box<dyn Fn()>>,
    #[cfg(test)]
    readback_requested: bool,
    #[cfg(test)]
    readback_pixels: Option<Vec<u8>>,
}

#[derive(Clone)]
pub struct LiveGpuCanvas {
    area: gtk::GLArea,
    state: Rc<RefCell<GpuState>>,
}

impl LiveGpuCanvas {
    pub fn new() -> Self {
        let area = gtk::GLArea::new();
        area.set_allowed_apis(gtk::gdk::GLAPI::GL);
        area.set_auto_render(false);
        area.set_has_depth_buffer(false);
        area.set_has_stencil_buffer(false);
        area.set_required_version(3, 2);
        area.set_hexpand(false);
        area.set_vexpand(false);
        area.set_halign(gtk::Align::Start);
        area.set_valign(gtk::Align::Start);
        area.set_can_target(false);
        area.set_visible(false);
        area.add_css_class("edit-live-gpu-canvas");

        let state = Rc::new(RefCell::new(GpuState {
            key: None,
            source: None,
            source_revision: 0,
            tone: GpuTone::default(),
            view: GpuView::default(),
            objects: None,
            rendered_frames: 0,
            last_queued: None,
            frame_after_paint: None,
            failure: None,
            on_failure: None,
            #[cfg(test)]
            readback_requested: false,
            #[cfg(test)]
            readback_pixels: None,
        }));

        let error_state = state.clone();
        area.connect_notify_local(Some("error"), move |area, _| {
            if let Some(error) = area.error() {
                fail(area, &error_state, error.to_string());
            }
        });

        let dispose_state = state.clone();
        area.connect_unrealize(move |area| {
            area.make_current();
            if area.error().is_none() {
                if let Some(objects) = dispose_state.borrow_mut().objects.take() {
                    unsafe {
                        gl::DeleteTextures(1, &objects.texture);
                        gl::DeleteTextures(1, &objects.clarity_luma_texture);
                        gl::DeleteTextures(1, &objects.clarity_blur_texture);
                        gl::DeleteTextures(1, &objects.effect_luma_texture);
                        gl::DeleteTextures(1, &objects.effect_blur_texture);
                        gl::DeleteTextures(1, &objects.auto_color_stats);
                        gl::DeleteFramebuffers(1, &objects.clarity_luma_fbo);
                        gl::DeleteFramebuffers(1, &objects.clarity_blur_fbo);
                        gl::DeleteFramebuffers(1, &objects.effect_luma_fbo);
                        gl::DeleteFramebuffers(1, &objects.effect_blur_fbo);
                        gl::DeleteFramebuffers(1, &objects.auto_color_fbo);
                        gl::DeleteVertexArrays(1, &objects.vao);
                        gl::DeleteProgram(objects.program);
                        gl::DeleteProgram(objects.blur_program);
                        gl::DeleteProgram(objects.auto_color_program);
                    }
                }
            }
        });

        let render_state = state.clone();
        let paint_state = state.clone();
        area.connect_realize(move |area| {
            if let Some(clock) = area.frame_clock() {
                let state = paint_state.clone();
                clock.connect_after_paint(move |_| {
                    let frame = state.borrow_mut().frame_after_paint.take();
                    if std::env::var_os("PICASA_TRACE").is_some() {
                        if let Some((rendered_at, queued_at)) = frame {
                            eprintln!(
                                "PIC_EDIT_GPU frame_present_after_paint_ms={:.3} queue_to_after_paint_ms={}",
                                rendered_at.elapsed().as_secs_f64() * 1000.0,
                                queued_at.map_or_else(|| "none".to_owned(), |at| format!("{:.3}", at.elapsed().as_secs_f64() * 1000.0)),
                            );
                        }
                    }
                });
            }
        });
        area.connect_render(move |area, _context| {
            let render_started = Instant::now();
            let init_error = {
                let mut state = render_state.borrow_mut();
                if state.failure.is_some() {
                    return gtk::glib::Propagation::Stop;
                }
                if state.objects.is_none() {
                    match initialize_gl() {
                        Ok(objects) => {
                            state.objects = Some(objects);
                            None
                        }
                        Err(error) => Some(error),
                    }
                } else {
                    None
                }
            };
            if let Some(error) = init_error {
                fail(area, &render_state, error);
                return gtk::glib::Propagation::Stop;
            }

            let view_width = area.width().max(1);
            let view_height = area.height().max(1);
            let device_scale = area.scale_factor().max(1);
            let draw_started = Instant::now();
            let (upload_trace, queue_ms, gl_error, sharpen) = {
                let mut state = render_state.borrow_mut();
                let source = state
                    .source
                    .as_ref()
                    .map(|(width, height, bytes)| (*width, *height, bytes.clone()));
                let source_revision = state.source_revision;
                let tone = state.tone;
                let view = state.view;
                let objects = state.objects.as_mut().expect("GL initialized");
                let mut upload_trace = None;
                #[cfg(test)]
                let mut readback = None;
                unsafe {
                    gl::Viewport(
                        0,
                        0,
                        view_width * device_scale,
                        view_height * device_scale,
                    );
                    gl::ClearColor(0.055, 0.06, 0.07, 1.0);
                    gl::Clear(gl::COLOR_BUFFER_BIT);
                    if let Some((width, height, bytes)) = source.as_ref() {
                        if objects.source_revision != source_revision {
                            gl::ActiveTexture(gl::TEXTURE0);
                            gl::BindTexture(gl::TEXTURE_2D, objects.texture);
                            gl::PixelStorei(gl::UNPACK_ALIGNMENT, 1);
                            let upload_started = Instant::now();
                            gl::TexImage2D(
                                gl::TEXTURE_2D,
                                0,
                                gl::RGBA8 as i32,
                                *width,
                                *height,
                                0,
                                gl::RGBA,
                                gl::UNSIGNED_BYTE,
                                bytes.as_ref().as_ptr().cast(),
                            );
                            let elapsed = upload_started.elapsed().as_secs_f64() * 1000.0;
                            objects.source_revision = source_revision;
                            upload_trace = Some((*width, *height, elapsed));
                        }
                    }

                    let filter = if view.scale < 1.0 {
                        gl::LINEAR
                    } else {
                        gl::NEAREST
                    };
                    if objects.filter != filter {
                        gl::BindTexture(gl::TEXTURE_2D, objects.texture);
                        gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MIN_FILTER, filter as i32);
                        gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MAG_FILTER, filter as i32);
                        objects.filter = filter;
                    }

                    gl::UseProgram(objects.program);
                    gl::BindVertexArray(objects.vao);
                    gl::ActiveTexture(gl::TEXTURE0);
                    gl::BindTexture(gl::TEXTURE_2D, objects.texture);
                    uniform_1i(objects.program, "uSource", 0);
                    uniform_2f(
                        objects.program,
                        "uSourceSize",
                        source.as_ref().map_or(1.0, |s| s.0 as f32),
                        source.as_ref().map_or(1.0, |s| s.1 as f32),
                    );
                    uniform_2f(
                        objects.program,
                        "uViewSize",
                        view_width as f32,
                        view_height as f32,
                    );
                    uniform_1f(objects.program, "uDeviceScale", device_scale as f32);
                    uniform_2f(objects.program, "uScroll", view.scroll_x, view.scroll_y);
                    uniform_2f(objects.program, "uPadding", view.pad_x, view.pad_y);
                    uniform_1f(objects.program, "uScale", view.scale.max(0.0001));
                    uniform_1f(objects.program, "uExposure", tone.exposure);
                    uniform_1f(objects.program, "uContrast", tone.contrast);
                    uniform_1f(objects.program, "uSaturation", tone.saturation);
                    uniform_1f(objects.program, "uTemperature", tone.temperature);
                    uniform_1f(objects.program, "uFillLight", tone.fill_light);
                    uniform_1f(objects.program, "uShadows", tone.shadows);
                    uniform_1f(objects.program, "uHighlights", tone.highlights);
                    uniform_1f(objects.program, "uSharpen", tone.sharpen);
                    uniform_1f(objects.program, "uClarity", tone.clarity);
                    uniform_1f(objects.program, "uGrain", tone.grain);
                    uniform_1f(objects.program, "uBlackWhite", if tone.black_white { 1.0 } else { 0.0 });
                    uniform_1f(objects.program, "uSepia", if tone.sepia { 1.0 } else { 0.0 });
                    uniform_1f(objects.program, "uFilterAmount", tone.filter_amount);
                    uniform_1i(objects.program, "uAutoColor", if tone.auto_color { 1 } else { 0 });
                    let params = filters::params(tone.filter);
                    uniform_1f(objects.program, "uNamedFilter", if tone.filter == FilterPreset::None { 0.0 } else { 1.0 });
                    uniform_1f(objects.program, "uNewspaper", if tone.filter == FilterPreset::Newspaper { 1.0 } else { 0.0 });
                    uniform_1f(objects.program, "uPencilSketch", if tone.filter == FilterPreset::PencilSketch { 1.0 } else { 0.0 });
                    uniform_1f(objects.program, "uDreamGlow", if tone.filter == FilterPreset::DreamGlow { 1.0 } else { 0.0 });
                    uniform_1f(objects.program, "uFilterExposure", params.exposure);
                    uniform_1f(objects.program, "uFilterShadowLift", params.shadow_lift);
                    uniform_3f(objects.program, "uFilterTint", params.tint.0, params.tint.1, params.tint.2);
                    uniform_3f(objects.program, "uFilterShadowTone", params.shadow_tone.0, params.shadow_tone.1, params.shadow_tone.2);
                    uniform_3f(objects.program, "uFilterHighlightTone", params.highlight_tone.0, params.highlight_tone.1, params.highlight_tone.2);
                    uniform_1f(objects.program, "uFilterSplit", params.split_strength);
                    uniform_1f(objects.program, "uFilterSaturation", params.saturation);
                    uniform_1f(objects.program, "uFilterContrast", params.contrast);
                    uniform_1f(objects.program, "uFilterVignette", params.vignette);
                    let film_grain = match tone.filter {
                        FilterPreset::DisposableFilm => 0.035,
                        FilterPreset::DigicamFlash => 0.012,
                        FilterPreset::OldPhoto => 0.075,
                        _ => 0.0,
                    };
                    uniform_1f(objects.program, "uNamedGrain", film_grain);
                    if tone.auto_color {
                        if let Some((width, height, _)) = source.as_ref() {
                            draw_auto_color_stats(objects, *width, *height, source_revision, view_width * device_scale, view_height * device_scale);
                            gl::ActiveTexture(gl::TEXTURE3);
                            gl::BindTexture(gl::TEXTURE_2D, objects.auto_color_stats);
                            uniform_1i(objects.program, "uAutoColorStats", 3);
                            gl::ActiveTexture(gl::TEXTURE0);
                            gl::BindTexture(gl::TEXTURE_2D, objects.texture);
                        }
                    }
                    if matches!(tone.filter, FilterPreset::PencilSketch | FilterPreset::DreamGlow) {
                        if let Some((source_width, source_height, _)) = source.as_ref() {
                            // Blur the 100%-strength base look BEFORE Filter Amount blending.
                            // Dream Glow must not recursively sample its own incomplete blur.
                            uniform_1f(objects.program, "uDreamGlow", 0.0);
                            uniform_1f(objects.program, "uFilterAmount", 1.0);
                            draw_effect_luminance(
                                objects,
                                *source_width,
                                *source_height,
                                view_width * device_scale,
                                view_height * device_scale,
                                tone.filter == FilterPreset::PencilSketch,
                            );
                            uniform_1f(objects.program, "uDreamGlow", if tone.filter == FilterPreset::DreamGlow { 1.0 } else { 0.0 });
                            uniform_1f(objects.program, "uFilterAmount", tone.filter_amount);
                            gl::ActiveTexture(gl::TEXTURE2);
                            gl::BindTexture(gl::TEXTURE_2D, objects.effect_luma_texture);
                            uniform_1i(objects.program, "uEffectBlur", 2);
                            gl::ActiveTexture(gl::TEXTURE0);
                            gl::BindTexture(gl::TEXTURE_2D, objects.texture);
                        }
                    }
                    if tone.clarity.abs() > 0.001 {
                        if let Some((source_width, source_height, _)) = source.as_ref() {
                            draw_clarity_luminance(
                                objects,
                                *source_width,
                                *source_height,
                                view_width * device_scale,
                                view_height * device_scale,
                            );
                        }
                    }
                    uniform_1i(objects.program, "uClarityPrepass", 0);
                    gl::ActiveTexture(gl::TEXTURE1);
                    gl::BindTexture(gl::TEXTURE_2D, objects.clarity_luma_texture);
                    uniform_1i(objects.program, "uClarityLuma", 1);
                    gl::ActiveTexture(gl::TEXTURE0);
                    gl::BindTexture(gl::TEXTURE_2D, objects.texture);
                    gl::DrawArrays(gl::TRIANGLES, 0, 3);
                    #[cfg(test)]
                    if state.readback_requested {
                        let framebuffer_width = view_width * device_scale;
                        let framebuffer_height = view_height * device_scale;
                        let mut pixels = vec![
                            0u8;
                            framebuffer_width as usize * framebuffer_height as usize * 4
                        ];
                        gl::ReadPixels(
                            0,
                            0,
                            framebuffer_width,
                            framebuffer_height,
                            gl::RGBA,
                            gl::UNSIGNED_BYTE,
                            pixels.as_mut_ptr().cast(),
                        );
                        readback = Some(pixels);
                        state.readback_requested = false;
                    }
                    gl::BindVertexArray(0);
                    gl::UseProgram(0);
                }
                let queued_at = state.last_queued.take();
                let queue_ms = queued_at
                    .map(|queued| render_started.saturating_duration_since(queued).as_secs_f64() * 1000.0);
                state.frame_after_paint = Some((Instant::now(), queued_at));
                state.rendered_frames = state.rendered_frames.wrapping_add(1);
                #[cfg(test)]
                if let Some(pixels) = readback {
                    state.readback_pixels = Some(pixels);
                }
                let gl_error = unsafe { gl::GetError() };
                (upload_trace, queue_ms, gl_error, tone.sharpen)
            };
            if gl_error != gl::NO_ERROR {
                fail(
                    area,
                    &render_state,
                    format!("OpenGL draw failed with error 0x{gl_error:04x}"),
                );
                return gtk::glib::Propagation::Stop;
            }

            if std::env::var_os("PICASA_TRACE").is_some() {
                if let Some((width, height, elapsed)) = upload_trace {
                    eprintln!(
                        "PIC_EDIT_GPU source_upload_ms={elapsed:.3} dimensions={width}x{height} bytes={}",
                        width as usize * height as usize * 4,
                    );
                }
                eprintln!(
                    "PIC_EDIT_GPU gpu_draw_submit_ms={:.3} viewport={}x{} framebuffer={}x{} scale_factor={} sharpen={:.3} queue_to_render_ms={}",
                    draw_started.elapsed().as_secs_f64() * 1000.0,
                    view_width,
                    view_height,
                    view_width * device_scale,
                    view_height * device_scale,
                    device_scale,
                    sharpen,
                    queue_ms.map_or_else(|| "none".to_owned(), |ms| format!("{ms:.3}")),
                );
            }
            gtk::glib::Propagation::Stop
        });

        Self { area, state }
    }

    pub fn widget(&self) -> gtk::GLArea {
        self.area.clone()
    }

    pub fn set_failure_handler(&self, handler: impl Fn() + 'static) {
        self.state.borrow_mut().on_failure = Some(Box::new(handler));
    }

    pub fn has_source_for(&self, reference: &str, rotation: i32, recipe: &EditRecipe) -> bool {
        let state = self.state.borrow();
        state.source.is_some()
            && state.key.as_ref().is_some_and(|key| {
                key.reference == reference
                    && key.rotation == rotation
                    && key.crop == recipe.crop
                    && key.straighten == recipe.straighten
            })
            && state.failure.is_none()
    }

    pub fn set_source(
        &self,
        reference: &str,
        rotation: i32,
        recipe: &EditRecipe,
        width: i32,
        height: i32,
        pixels: glib::Bytes,
    ) {
        let mut state = self.state.borrow_mut();
        state.key = Some(SourceKey {
            reference: reference.to_owned(),
            rotation,
            crop: recipe.crop,
            straighten: recipe.straighten,
        });
        state.source = Some((width, height, pixels));
        state.source_revision = state.source_revision.wrapping_add(1);
        state.last_queued = Some(Instant::now());
        self.area.set_visible(true);
        self.area.queue_render();
    }

    pub fn set_recipe(&self, recipe: &EditRecipe) {
        // Returning from CPU fallback may reuse an unchanged GPU recipe.
        // Restore the layer even when no uniform update is needed.
        self.area.set_visible(true);
        let next = GpuTone {
            exposure: recipe.exposure,
            contrast: recipe.contrast,
            saturation: recipe.saturation,
            temperature: recipe.temperature,
            fill_light: recipe.fill_light,
            shadows: recipe.shadows,
            highlights: recipe.highlights,
            sharpen: recipe.sharpen,
            grain: recipe.grain,
            clarity: recipe.clarity,
            black_white: recipe.black_white,
            sepia: recipe.sepia,
            filter_amount: recipe.filter_amount,
            filter: recipe.filter,
            auto_color: recipe.auto_color,
        };
        let mut state = self.state.borrow_mut();
        if state.tone == next {
            return;
        }
        state.tone = next;
        state.last_queued = Some(Instant::now());
        drop(state);
        self.area.queue_render();
    }

    pub fn set_view(
        &self,
        scroll_x: f64,
        scroll_y: f64,
        width: f64,
        height: f64,
        scale: f64,
        pad_x: f64,
        pad_y: f64,
    ) {
        let next = GpuView {
            width: width as f32,
            height: height as f32,
            // The GLArea is positioned with integer GTK margins. Sample from
            // that same origin so fractional GtkAdjustment values do not add
            // a half-pixel wobble while panning at 1:1.
            scroll_x: scroll_x.round() as f32,
            scroll_y: scroll_y.round() as f32,
            pad_x: pad_x as f32,
            pad_y: pad_y as f32,
            scale: scale as f32,
        };
        let mut state = self.state.borrow_mut();
        if state.view == next {
            return;
        }
        let allocation_changed = (
            state.view.width.round() as i32,
            state.view.height.round() as i32,
        ) != (next.width.round() as i32, next.height.round() as i32);
        state.view = next;
        state.last_queued = Some(Instant::now());
        drop(state);
        if allocation_changed {
            self.area.set_margin_start(scroll_x.round() as i32);
            self.area.set_margin_top(scroll_y.round() as i32);
            self.area.set_size_request(
                width.round().max(1.0) as i32,
                height.round().max(1.0) as i32,
            );
        } else {
            self.area.set_margin_start(scroll_x.round() as i32);
            self.area.set_margin_top(scroll_y.round() as i32);
        }
        if std::env::var_os("PICASA_TRACE").is_some() {
            eprintln!(
                "PIC_EDIT_GPU view viewport={}x{} scroll={}x{} scale={:.4} pad={}x{}",
                next.width.round() as i32,
                next.height.round() as i32,
                next.scroll_x,
                next.scroll_y,
                next.scale,
                next.pad_x,
                next.pad_y,
            );
        }
        self.area.queue_render();
    }

    pub fn hide(&self) {
        self.area.set_visible(false);
    }

    #[cfg(test)]
    fn source_uploaded(&self) -> bool {
        let state = self.state.borrow();
        state.objects.as_ref().is_some_and(|objects| {
            state.source.is_some() && objects.source_revision == state.source_revision
        })
    }

    #[cfg(test)]
    fn rendered_frames(&self) -> u64 {
        self.state.borrow().rendered_frames
    }

    #[cfg(test)]
    fn uploaded_revision(&self) -> u64 {
        self.state
            .borrow()
            .objects
            .as_ref()
            .map_or(0, |objects| objects.source_revision)
    }

    #[cfg(test)]
    fn request_readback(&self) {
        self.state.borrow_mut().readback_requested = true;
        self.area.queue_render();
    }

    #[cfg(test)]
    fn take_readback(&self) -> Option<Vec<u8>> {
        self.state.borrow_mut().readback_pixels.take()
    }
}

pub fn gpu_tone_supported(recipe: &EditRecipe) -> bool {
    !recipe.auto_contrast && !(recipe.auto_color && recipe.filter != FilterPreset::None)
}

pub fn gpu_base_recipe(recipe: &EditRecipe) -> EditRecipe {
    let mut base = recipe.clone();
    base.exposure = 0.0;
    base.contrast = 0.0;
    base.saturation = 0.0;
    base.temperature = 0.0;
    base.fill_light = 0.0;
    base.shadows = 0.0;
    base.highlights = 0.0;
    base.sharpen = 0.0;
    base.grain = 0.0;
    base.clarity = 0.0;
    base.filter = FilterPreset::None;
    base.black_white = false;
    base.sepia = false;
    base.auto_color = false;
    base
}

fn fail(area: &gtk::GLArea, state: &Rc<RefCell<GpuState>>, message: String) {
    let callback = {
        let mut state = state.borrow_mut();
        state.failure = Some(message.clone());
        state.on_failure.take()
    };
    area.set_visible(false);
    eprintln!("PIC_EDIT_GPU init_failed reason={message}; falling back to CPU renderer");
    if let Some(callback) = callback {
        callback();
    }
}

fn initialize_gl() -> Result<GlObjects, String> {
    let library = gl_library()?;
    gl::load_with(|symbol| {
        let name = format!("{symbol}\0");
        unsafe {
            library
                .get::<*const c_void>(name.as_bytes())
                .map_or(std::ptr::null(), |address| *address)
        }
    });
    if !gl::CreateShader::is_loaded() || !gl::GenVertexArrays::is_loaded() {
        return Err("OpenGL entry points are unavailable".to_owned());
    }

    let vertex = compile_shader(
        gl::VERTEX_SHADER,
        r#"#version 150
out vec2 vUnused;
void main() {
    vec2 p = gl_VertexID == 0 ? vec2(-1.0, -1.0)
           : gl_VertexID == 1 ? vec2( 3.0, -1.0)
                              : vec2(-1.0,  3.0);
    gl_Position = vec4(p, 0.0, 1.0);
    vUnused = vec2(0.0);
}"#,
    )?;
    let fragment = compile_shader(
        gl::FRAGMENT_SHADER,
        r#"#version 150
uniform sampler2D uSource;
uniform vec2 uSourceSize;
uniform vec2 uViewSize;
uniform vec2 uScroll;
uniform vec2 uPadding;
uniform float uScale;
uniform float uDeviceScale;
uniform float uExposure;
uniform float uContrast;
uniform float uSaturation;
uniform float uTemperature;
uniform float uFillLight;
uniform float uShadows;
uniform float uHighlights;
uniform float uSharpen;
uniform float uGrain;
uniform float uClarity;
uniform sampler2D uClarityLuma;
uniform int uClarityPrepass;
uniform vec2 uClarityPassSize;
uniform float uBlackWhite;
uniform float uSepia;
uniform float uFilterAmount;
uniform float uNamedFilter;
uniform float uNewspaper;
uniform float uPencilSketch;
uniform float uDreamGlow;
uniform sampler2D uEffectBlur;
uniform float uFilterExposure;
uniform float uFilterShadowLift;
uniform vec3 uFilterTint;
uniform vec3 uFilterShadowTone;
uniform vec3 uFilterHighlightTone;
uniform float uFilterSplit;
uniform float uFilterSaturation;
uniform float uFilterContrast;
uniform float uFilterVignette;
uniform float uNamedGrain;
uniform int uAutoColor;
uniform sampler2D uAutoColorStats;
out vec4 color;
float luma(vec3 c) { return dot(c, vec3(0.2126, 0.7152, 0.0722)); }
float smoothMask(float low, float high, float value) {
    float t = clamp((value - low) / (high - low), 0.0, 1.0);
    return t * t * (3.0 - 2.0 * t);
}
float contrastCurve(float value, float amount) {
    float x = clamp(value, 0.0, 1.0);
    float a = clamp(amount, -1.0, 1.0) * 0.55;
    return clamp(x + a * 4.0 * (x - 0.5) * x * (1.0 - x), 0.0, 1.0);
}
vec4 tonePixel(vec4 px) {
    vec3 corrected = px.rgb;
    if (uAutoColor != 0 && px.a >= 16.0/255.0) {
        vec4 sums = texelFetch(uAutoColorStats, ivec2(0,0), 0);
        if (sums.a >= 0.5) {
            vec3 means = sums.rgb / sums.a;
            float neutral = dot(means, vec3(0.2126,0.7152,0.0722));
            vec3 gains = vec3(1.0) + (clamp(vec3(neutral) / max(means,vec3(1.0)), vec3(0.82), vec3(1.22)) - vec3(1.0)) * 0.65;
            corrected = round(clamp(corrected * gains, 0.0, 1.0) * 255.0) / 255.0;
        }
    }
    vec3 c = corrected * exp2(uExposure);
    if (abs(uFillLight) > 0.0001 || abs(uShadows) > 0.0001 ||
        abs(uHighlights) > 0.0001) {
        float y = luma(c);
        if (y > 0.001 && y < 1.0) {
            float fill = pow(y, pow(2.0, -uFillLight * 1.15));
            float shadows = pow(y, pow(2.0, -uShadows * 1.35));
            float highlights = 1.0 - pow(1.0 - y, pow(2.0, uHighlights * 1.20));
            float fillMask = 1.0 - smoothMask(0.48, 0.93, y);
            float shadowMask = 1.0 - smoothMask(0.18, 0.61, y);
            float highlightMask = smoothMask(0.48, 0.87, y);
            float target = clamp(y + (fill-y)*fillMask*0.95 +
                                 (shadows-y)*shadowMask*0.95 +
                                 (highlights-y)*highlightMask*0.92, 0.0, 1.0);
            c *= clamp(target / y, 0.12, 5.0);
        }
    }
    c.r *= 1.0 + uTemperature * 0.18;
    c.b *= 1.0 - uTemperature * 0.18;
    c.g *= 1.0 + abs(uTemperature) * 0.015;
    float gray = luma(c);
    c = vec3(gray) + (c - vec3(gray)) * (1.0 + uSaturation);
    if (abs(uContrast) > 0.0001) {
        c.r = contrastCurve(c.r, uContrast);
        c.g = contrastCurve(c.g, uContrast);
        c.b = contrastCurve(c.b, uContrast);
    }
    // CPU order: manual tone, then optional B&W / Sepia look, then quantization.
    vec3 preLook = c;
    if (uBlackWhite > 0.5) c = vec3(clamp(luma(c), 0.0, 1.0));
    if (uSepia > 0.5) {
        float y = clamp(luma(c), 0.0, 1.0);
        float strength = 0.75 * (1.0 - 0.20 * abs(2.0 * y - 1.0));
        c = vec3(y * (1.0 + 0.080 * strength),
                 y * (1.0 - 0.006 * strength),
                 y * (1.0 - 0.180 * strength));
    }
    if (uBlackWhite > 0.5 || uSepia > 0.5)
        c = mix(preLook, c, clamp(uFilterAmount, 0.0, 1.0));
    c = round(clamp(c, 0.0, 1.0) * 255.0) / 255.0;
    return vec4(c, px.a);
}
// Port of filters::apply_filter's parameterized colour presets. The filter
// is applied and quantized *before* the manual tone pipeline, like CPU export.
float stableNoise(ivec2 coord) {
    ivec2 c = clamp(coord, ivec2(0), ivec2(uSourceSize) - ivec2(1));
    uint hash = uint(c.x) * 374761393u ^ uint(c.y) * 668265263u ^ 0x9e3779b9u;
    hash = (hash ^ (hash >> 13u)) * 1274126177u;
    hash ^= hash >> 16u;
    return float(hash & 65535u) / 65535.0 - 0.5;
}
vec4 namedFilterPixel(vec4 px, ivec2 coord) {
    if (uNamedFilter < 0.5 || uFilterAmount <= 0.0) return px;
    if (uPencilSketch > 0.5) {
        float gray = mix(1.0, luma(px.rgb), px.a);
        float local = texture(uEffectBlur, (vec2(coord) + vec2(0.5)) / uSourceSize).r;
        float dodge = min((gray + 0.015) / (local + 0.015), 1.0);
        float graphite = pow(dodge, 3.0) * (0.88 + 0.12 * gray);
        float shade = round(graphite * 255.0) / 255.0;
        return vec4(round(mix(px.rgb, vec3(shade), clamp(uFilterAmount, 0.0, 1.0)) * 255.0) / 255.0, px.a);
    }
    if (uNewspaper > 0.5) {
        // Same ordered 4x4 Bayer halftone and source-coordinate phase as CPU.
        if (px.a <= 0.0) return px;
        const int bayer[16] = int[16](
            0, 8, 2, 10, 12, 4, 14, 6,
            3, 11, 1, 9, 15, 7, 13, 5);
        int offset = (coord.y % 4) * 4 + (coord.x % 4);
        float threshold = (float(bayer[offset]) + 0.5) / 16.0;
        float ink = pow(clamp(1.0 - luma(px.rgb), 0.0, 1.0), 0.83);
        vec3 halftone = ink >= threshold
            ? vec3(36.0, 34.0, 31.0) / 255.0
            : vec3(231.0, 220.0, 196.0) / 255.0;
        return vec4(round(mix(px.rgb, halftone, clamp(uFilterAmount, 0.0, 1.0)) * 255.0) / 255.0, px.a);
    }
    vec3 c = px.rgb * uFilterExposure;
    c += vec3(uFilterShadowLift) * (vec3(1.0) - c);
    c *= uFilterTint;
    float y = clamp(luma(c), 0.0, 1.0);
    c = mix(c, c * uFilterShadowTone, (1.0 - y) * uFilterSplit);
    c = mix(c, c * uFilterHighlightTone, y * uFilterSplit);
    float gray = luma(c);
    c = vec3(gray) + (c - vec3(gray)) * uFilterSaturation;
    c = vec3(contrastCurve(c.r, uFilterContrast),
             contrastCurve(c.g, uFilterContrast),
             contrastCurve(c.b, uFilterContrast));
    vec2 centre = (uSourceSize - vec2(1.0)) * 0.5;
    vec2 d = vec2(coord) - centre;
    float maxDistance = max(length(centre), 1.0);
    float falloff = 1.0 - uFilterVignette * dot(d, d) / (maxDistance * maxDistance);
    c *= falloff;
    c = round(clamp(c, 0.0, 1.0) * 255.0) / 255.0;
    if (uDreamGlow > 0.5) {
        float glow = texture(uEffectBlur, (vec2(coord) + vec2(0.5)) / uSourceSize).r * 0.45;
        c = round(clamp(c + (vec3(1.0) - c) * glow, 0.0, 1.0) * 255.0) / 255.0;
    }
    if (uNamedGrain > 0.0) {
        float noise = stableNoise(coord) * uNamedGrain * 255.0;
        c = round(clamp(c * 255.0 + vec3(noise), 0.0, 255.0)) / 255.0;
    }
    // CPU blend_filter_amount quantizes the blend of original and look.
    c = round(clamp(mix(px.rgb, c, clamp(uFilterAmount, 0.0, 1.0)),
                    0.0, 1.0) * 255.0) / 255.0;
    return vec4(c, px.a);
}
vec4 sampleTone(ivec2 coord) {
    ivec2 limit = ivec2(uSourceSize) - ivec2(1);
    ivec2 valid = clamp(coord, ivec2(0), limit);
    return tonePixel(namedFilterPixel(texelFetch(uSource, valid, 0), valid));
}
vec3 clarified(ivec2 coord) {
    vec3 base = sampleTone(coord).rgb;
    if (abs(uClarity) <= 0.001) return base;
    float y = luma(base);
    float localY = texture(uClarityLuma,
        (vec2(coord) + vec2(0.5)) / uSourceSize).r;
    float midtones = clamp(4.0 * y * (1.0 - y), 0.0, 1.0);
    float detail = clamp(y - localY, -0.24, 0.24);
    float target = clamp(y + detail * midtones * uClarity * 1.85, 0.0, 1.0);
    float gain = clamp(target / max(y, 0.01), 0.25, 3.0);
    return round(clamp(base * gain, 0.0, 1.0) * 255.0) / 255.0;
}
vec3 sharpened(ivec2 coord) {
    vec3 center = clarified(coord);
    if (uSharpen <= 0.0001) return center;
    const float k0 = 0.07076637;
    const float k1 = 0.24446040;
    const float k2 = 0.36954646;
    const float kernel[5] = float[5](k0, k1, k2, k1, k0);
    vec3 blurred = vec3(0.0);
    for (int y = -2; y <= 2; ++y) {
        for (int x = -2; x <= 2; ++x) {
            blurred += clarified(coord + ivec2(x, y)) * kernel[x + 2] * kernel[y + 2];
        }
    }
    blurred = round(clamp(blurred, 0.0, 1.0) * 255.0) / 255.0;
    return clamp(center + (center - blurred) * (uSharpen * 1.6), 0.0, 1.0);
}
// Match the CPU's stable, source-coordinate based grain hash.
// Unsigned arithmetic wraps as required by the Rust u32 implementation.
vec3 grainPixel(ivec2 coord, vec3 rgb) {
    if (uGrain <= 0.001) return rgb;
    float noise = stableNoise(coord) * uGrain * 0.24 * 255.0;
    return round(clamp(rgb * 255.0 + vec3(noise), 0.0, 255.0)) / 255.0;
}
vec3 processedPixel(ivec2 coord) {
    return grainPixel(coord, sharpened(coord));
}
void main() {
    if (uClarityPrepass != 0) {
        vec2 uv = gl_FragCoord.xy / uClarityPassSize;
        ivec2 coord = ivec2(floor(uv * uSourceSize));
        vec4 px = texture(uSource, uv);
        float y;
        if (uClarityPrepass == 2) {
            if (uPencilSketch > 0.5) {
                y = mix(1.0, luma(px.rgb), px.a);
            } else {
                float baseLuma = luma(namedFilterPixel(px, coord).rgb);
                y = clamp((baseLuma - 0.55) / 0.45, 0.0, 1.0) * px.a;
            }
        } else {
            y = luma(tonePixel(namedFilterPixel(px, coord)).rgb);
        }
        color = vec4(y, y, y, 1.0);
        return;
    }
    vec2 pixel = gl_FragCoord.xy / max(uDeviceScale, 1.0);
    vec2 topLeft = vec2(pixel.x, uViewSize.y - pixel.y);
    vec2 sourcePos = (topLeft + uScroll - uPadding) / max(uScale, 0.0001);
    if (sourcePos.x < 0.0 || sourcePos.y < 0.0 ||
        sourcePos.x >= uSourceSize.x || sourcePos.y >= uSourceSize.y) {
        color = vec4(0.055, 0.06, 0.07, 1.0);
        return;
    }
    if (uSharpen <= 0.0001 && uGrain <= 0.001 && abs(uClarity) <= 0.001 && uNamedFilter < 0.5) {
        color = tonePixel(texture(uSource, sourcePos / uSourceSize));
        return;
    }
    if (uScale < 1.0) {
        vec2 texel = sourcePos - vec2(0.5);
        ivec2 lo = ivec2(floor(texel));
        vec2 f = fract(texel);
        vec3 top = mix(processedPixel(lo), processedPixel(lo + ivec2(1, 0)), f.x);
        vec3 bottom = mix(processedPixel(lo + ivec2(0, 1)), processedPixel(lo + ivec2(1, 1)), f.x);
        color = vec4(mix(top, bottom, f.y), 1.0);
        return;
    }
    ivec2 coord = ivec2(floor(sourcePos));
    vec4 px = sampleTone(coord);
    color = vec4(processedPixel(coord), px.a);
}"#,
    )?;

    let blur_fragment = compile_shader(gl::FRAGMENT_SHADER, r#"#version 150
uniform sampler2D uInput;
uniform vec2 uTexel;
uniform vec2 uDirection;
uniform float uSigma;
uniform int uRadius;
out vec4 color;
void main() {
    vec2 uv = gl_FragCoord.xy * uTexel;
    float value = 0.0;
    float total = 0.0;
    for (int offset = -uRadius; offset <= uRadius; ++offset) {
        float f = float(offset);
        float weight = exp(-0.5 * f * f / (uSigma * uSigma));
        value += texture(uInput, uv + uDirection * uTexel * f).r * weight;
        total += weight;
    }
    float y = value / max(total, 0.0001);
    color = vec4(y, y, y, 1.0);
}"#)?;

    let auto_vertex = compile_shader(gl::VERTEX_SHADER, r#"#version 150
uniform sampler2D uSource;
uniform int uWidth;
uniform int uSampleStep;
out vec4 vWeighted;
void main() {
    int idx = gl_VertexID * uSampleStep;
    ivec2 at = ivec2(idx % uWidth, idx / uWidth);
    vec4 px = texelFetch(uSource, at, 0);
    float y = dot(px.rgb, vec3(0.2126, 0.7152, 0.0722));
    float hi = max(px.r, max(px.g, px.b));
    float lo = min(px.r, min(px.g, px.b));
    float chroma = (hi - lo) / max(hi, 1.0/255.0);
    float weight = (px.a < 16.0/255.0 || y < 0.07 || y > 0.93 || chroma > 0.32)
        ? 0.0 : pow(1.0 - chroma / 0.32, 2.0);
    vWeighted = vec4(px.rgb * (255.0 * weight), weight);
    gl_Position = vec4(0.0, 0.0, 0.0, 1.0);
    gl_PointSize = 1.0;
}"#)?;
    let auto_fragment = compile_shader(gl::FRAGMENT_SHADER, r#"#version 150
in vec4 vWeighted;
out vec4 color;
void main() { color = vWeighted; }
"#)?;

    let program;
    let blur_program;
    let auto_color_program;
    let mut vao = 0;
    let mut texture = 0;
    unsafe {
        program = gl::CreateProgram();
        gl::AttachShader(program, vertex);
        gl::AttachShader(program, fragment);
        gl::LinkProgram(program);
        blur_program = gl::CreateProgram();
        gl::AttachShader(blur_program, vertex);
        gl::AttachShader(blur_program, blur_fragment);
        gl::LinkProgram(blur_program);
        auto_color_program = gl::CreateProgram();
        gl::AttachShader(auto_color_program, auto_vertex);
        gl::AttachShader(auto_color_program, auto_fragment);
        gl::LinkProgram(auto_color_program);
        let mut auto_linked = 0;
        gl::GetProgramiv(auto_color_program, gl::LINK_STATUS, &mut auto_linked);
        let mut blur_linked = 0;
        gl::GetProgramiv(blur_program, gl::LINK_STATUS, &mut blur_linked);
        let mut linked = 0;
        gl::GetProgramiv(program, gl::LINK_STATUS, &mut linked);
        gl::DeleteShader(vertex);
        gl::DeleteShader(fragment);
        gl::DeleteShader(blur_fragment);
        gl::DeleteShader(auto_vertex);
        gl::DeleteShader(auto_fragment);
        if auto_linked == 0 {
            let message = program_log(auto_color_program);
            gl::DeleteProgram(auto_color_program);
            gl::DeleteProgram(blur_program);
            gl::DeleteProgram(program);
            return Err(format!("auto colour shader link failed: {message}"));
        }
        if blur_linked == 0 {
            let message = program_log(blur_program);
            gl::DeleteProgram(blur_program);
            gl::DeleteProgram(program);
            return Err(format!("clarity blur shader link failed: {message}"));
        }
        if linked == 0 {
            let message = program_log(program);
            gl::DeleteProgram(program);
            return Err(format!("shader link failed: {message}"));
        }
        gl::GenVertexArrays(1, &mut vao);
        gl::BindVertexArray(vao);
        gl::GenTextures(1, &mut texture);
        gl::BindTexture(gl::TEXTURE_2D, texture);
        gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MIN_FILTER, gl::NEAREST as i32);
        gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MAG_FILTER, gl::NEAREST as i32);
        gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_WRAP_S, gl::CLAMP_TO_EDGE as i32);
        gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_WRAP_T, gl::CLAMP_TO_EDGE as i32);
        gl::BindVertexArray(0);
    }
    Ok(GlObjects {
        program,
        vao,
        texture,
        source_revision: 0,
        filter: gl::NEAREST,
        blur_program,
        clarity_luma_texture: 0,
        clarity_blur_texture: 0,
        clarity_luma_fbo: 0,
        clarity_blur_fbo: 0,
        clarity_dimensions: (0, 0),
        effect_luma_texture: 0,
        effect_blur_texture: 0,
        effect_luma_fbo: 0,
        effect_blur_fbo: 0,
        effect_dimensions: (0, 0),
        auto_color_program,
        auto_color_stats: 0,
        auto_color_fbo: 0,
        auto_color_revision: 0,
    })
}

/// GPU reduction into a one-pixel RGBA32F sum. Matches CPU flattened
/// neutral sampling without a source-sized CPU processing pass or GL readback.
unsafe fn draw_auto_color_stats(
    objects: &mut GlObjects, width: i32, height: i32,
    revision: u64, viewport_width: i32, viewport_height: i32,
) {
    if objects.auto_color_revision == revision { return; }
    let mut previous_fbo = 0;
    let mut previous_blend_src = 0;
    let mut previous_blend_dst = 0;
    gl::GetIntegerv(gl::FRAMEBUFFER_BINDING, &mut previous_fbo);
    gl::GetIntegerv(gl::BLEND_SRC_RGB, &mut previous_blend_src);
    gl::GetIntegerv(gl::BLEND_DST_RGB, &mut previous_blend_dst);
    let was_blending = gl::IsEnabled(gl::BLEND) == gl::TRUE;
    if objects.auto_color_stats == 0 {
        gl::GenTextures(1, &mut objects.auto_color_stats);
        gl::GenFramebuffers(1, &mut objects.auto_color_fbo);
        gl::BindTexture(gl::TEXTURE_2D, objects.auto_color_stats);
        gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MIN_FILTER, gl::NEAREST as i32);
        gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MAG_FILTER, gl::NEAREST as i32);
        gl::TexImage2D(gl::TEXTURE_2D, 0, gl::RGBA32F as i32,
            1, 1, 0, gl::RGBA, gl::FLOAT, std::ptr::null());
        gl::BindFramebuffer(gl::FRAMEBUFFER, objects.auto_color_fbo);
        gl::FramebufferTexture2D(gl::FRAMEBUFFER, gl::COLOR_ATTACHMENT0,
            gl::TEXTURE_2D, objects.auto_color_stats, 0);
    } else {
        gl::BindFramebuffer(gl::FRAMEBUFFER, objects.auto_color_fbo);
    }
    gl::Viewport(0, 0, 1, 1);
    gl::ClearColor(0.0, 0.0, 0.0, 0.0);
    gl::Clear(gl::COLOR_BUFFER_BIT);
    gl::Enable(gl::BLEND);
    gl::BlendEquation(gl::FUNC_ADD);
    gl::BlendFunc(gl::ONE, gl::ONE);
    gl::UseProgram(objects.auto_color_program);
    gl::ActiveTexture(gl::TEXTURE0);
    gl::BindTexture(gl::TEXTURE_2D, objects.texture);
    uniform_1i(objects.auto_color_program, "uSource", 0);
    uniform_1i(objects.auto_color_program, "uWidth", width);
    let pixels = i64::from(width) * i64::from(height);
    let step = (pixels / 200_000).max(1);
    uniform_1i(objects.auto_color_program, "uSampleStep", step as i32);
    gl::DrawArrays(gl::POINTS, 0, ((pixels + step - 1) / step) as i32);
    gl::BlendFunc(previous_blend_src as u32, previous_blend_dst as u32);
    if !was_blending { gl::Disable(gl::BLEND); }
    gl::BindFramebuffer(gl::FRAMEBUFFER, previous_fbo as u32);
    gl::Viewport(0, 0, viewport_width, viewport_height);
    gl::UseProgram(objects.program);
    gl::ActiveTexture(gl::TEXTURE0);
    gl::BindTexture(gl::TEXTURE_2D, objects.texture);
    objects.auto_color_revision = revision;
}

fn cpu_gaussian_kernel_radius(sigma: f32) -> i32 {
    // Mirror image::imageops::GaussianBlurParameters (image 0.25.10).
    // Keeping the sample support identical matters for Pencil Sketch edges.
    let mut size = ((((sigma - 0.8) / 0.3 + 1.0) * 2.0 + 1.0).max(3.0)) as i32;
    if size % 2 == 0 { size += 1; }
    (size - 1) / 2
}

// Clarity uses a source-sized luminance *coordinate system*, but its blur is
// computed once per draw in a <=1200px GPU buffer (not via CPU imageops).
// The original GtkGLArea framebuffer must be restored, never framebuffer 0.
unsafe fn draw_clarity_luminance(
    objects: &mut GlObjects,
    source_width: i32,
    source_height: i32,
    view_width_px: i32,
    view_height_px: i32,
) {
    let downsample = (source_width.max(source_height) as f32 / 1200.0).max(1.0);
    let width = (source_width as f32 / downsample).round().max(1.0) as i32;
    let height = (source_height as f32 / downsample).round().max(1.0) as i32;
    let mut original_framebuffer = 0;
    gl::GetIntegerv(gl::FRAMEBUFFER_BINDING, &mut original_framebuffer);
    if objects.clarity_dimensions != (width, height) {
        if objects.clarity_luma_texture == 0 {
            gl::GenTextures(1, &mut objects.clarity_luma_texture);
            gl::GenTextures(1, &mut objects.clarity_blur_texture);
            gl::GenFramebuffers(1, &mut objects.clarity_luma_fbo);
            gl::GenFramebuffers(1, &mut objects.clarity_blur_fbo);
        }
        for (texture, framebuffer) in [
            (objects.clarity_luma_texture, objects.clarity_luma_fbo),
            (objects.clarity_blur_texture, objects.clarity_blur_fbo),
        ] {
            gl::BindTexture(gl::TEXTURE_2D, texture);
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MIN_FILTER, gl::LINEAR as i32);
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MAG_FILTER, gl::LINEAR as i32);
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_WRAP_S, gl::CLAMP_TO_EDGE as i32);
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_WRAP_T, gl::CLAMP_TO_EDGE as i32);
            gl::TexImage2D(
                gl::TEXTURE_2D, 0, gl::R8 as i32, width, height, 0,
                gl::RED, gl::UNSIGNED_BYTE, std::ptr::null(),
            );
            gl::BindFramebuffer(gl::FRAMEBUFFER, framebuffer);
            gl::FramebufferTexture2D(
                gl::FRAMEBUFFER, gl::COLOR_ATTACHMENT0, gl::TEXTURE_2D, texture, 0,
            );
        }
        objects.clarity_dimensions = (width, height);
    }
    // GLArea owns the framebuffer saved before allocation; never bind 0.
    gl::BindFramebuffer(gl::FRAMEBUFFER, objects.clarity_luma_fbo);
    gl::Viewport(0, 0, width, height);
    gl::UseProgram(objects.program);
    gl::ActiveTexture(gl::TEXTURE0);
    gl::BindTexture(gl::TEXTURE_2D, objects.texture);
    uniform_1i(objects.program, "uClarityPrepass", 1);
    uniform_2f(objects.program, "uClarityPassSize", width as f32, height as f32);
    gl::DrawArrays(gl::TRIANGLES, 0, 3);

    let sigma = ((width.min(height) as f32) * 0.008).clamp(1.5, 20.0);
    // image 0.25.10 GaussianBlurParameters::kernel_size_from_sigma;
    // a generic 3*sigma support mismatches its small-radius behaviour.
    let radius = cpu_gaussian_kernel_radius(sigma);
    gl::UseProgram(objects.blur_program);
    uniform_1i(objects.blur_program, "uInput", 0);
    uniform_2f(objects.blur_program, "uTexel", 1.0 / width as f32, 1.0 / height as f32);
    uniform_1f(objects.blur_program, "uSigma", sigma);
    gl::Uniform1i(uniform_location(objects.blur_program, "uRadius"), radius);
    for (input, output, x, y) in [
        (objects.clarity_luma_texture, objects.clarity_blur_fbo, 1.0, 0.0),
        (objects.clarity_blur_texture, objects.clarity_luma_fbo, 0.0, 1.0),
    ] {
        gl::BindFramebuffer(gl::FRAMEBUFFER, output);
        gl::BindTexture(gl::TEXTURE_2D, input);
        uniform_2f(objects.blur_program, "uDirection", x, y);
        gl::DrawArrays(gl::TRIANGLES, 0, 3);
    }

    gl::BindFramebuffer(gl::FRAMEBUFFER, original_framebuffer as u32);
    gl::Viewport(0, 0, view_width_px, view_height_px);
    gl::UseProgram(objects.program);
    uniform_1i(objects.program, "uClarityPrepass", 0);
    gl::ActiveTexture(gl::TEXTURE0);
    gl::BindTexture(gl::TEXTURE_2D, objects.texture);
}

// GPU blur of the unadjusted Pencil Sketch luminance or Dream Glow highlights.
unsafe fn draw_effect_luminance(
    objects: &mut GlObjects,
    source_width: i32,
    source_height: i32,
    view_width_px: i32,
    view_height_px: i32,
    pencil: bool,
) {
    let downsample = (source_width.max(source_height) as f32 / 1200.0).max(1.0);
    let width = (source_width as f32 / downsample).round().max(1.0) as i32;
    let height = (source_height as f32 / downsample).round().max(1.0) as i32;
    let mut original_framebuffer = 0;
    gl::GetIntegerv(gl::FRAMEBUFFER_BINDING, &mut original_framebuffer);
    if objects.effect_dimensions != (width, height) {
        if objects.effect_luma_texture == 0 {
            gl::GenTextures(1, &mut objects.effect_luma_texture);
            gl::GenTextures(1, &mut objects.effect_blur_texture);
            gl::GenFramebuffers(1, &mut objects.effect_luma_fbo);
            gl::GenFramebuffers(1, &mut objects.effect_blur_fbo);
        }
        for (texture, framebuffer) in [
            (objects.effect_luma_texture, objects.effect_luma_fbo),
            (objects.effect_blur_texture, objects.effect_blur_fbo),
        ] {
            gl::BindTexture(gl::TEXTURE_2D, texture);
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MIN_FILTER, gl::LINEAR as i32);
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MAG_FILTER, gl::LINEAR as i32);
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_WRAP_S, gl::CLAMP_TO_EDGE as i32);
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_WRAP_T, gl::CLAMP_TO_EDGE as i32);
            gl::TexImage2D(
                gl::TEXTURE_2D, 0, gl::R8 as i32, width, height, 0,
                gl::RED, gl::UNSIGNED_BYTE, std::ptr::null(),
            );
            gl::BindFramebuffer(gl::FRAMEBUFFER, framebuffer);
            gl::FramebufferTexture2D(
                gl::FRAMEBUFFER, gl::COLOR_ATTACHMENT0, gl::TEXTURE_2D, texture, 0,
            );
        }
        objects.effect_dimensions = (width, height);
    }
    // GLArea owns the framebuffer saved before allocation; never bind 0.
    gl::BindFramebuffer(gl::FRAMEBUFFER, objects.effect_luma_fbo);
    gl::Viewport(0, 0, width, height);
    gl::UseProgram(objects.program);
    gl::ActiveTexture(gl::TEXTURE0);
    gl::BindTexture(gl::TEXTURE_2D, objects.texture);
    uniform_1i(objects.program, "uClarityPrepass", 2);
    uniform_2f(objects.program, "uClarityPassSize", width as f32, height as f32);
    gl::DrawArrays(gl::TRIANGLES, 0, 3);

    let sigma = ((width.min(height) as f32) * if pencil { 0.004 } else { 0.008 }).max(0.8);
    // image 0.25.10 GaussianBlurParameters::kernel_size_from_sigma;
    // a generic 3*sigma support mismatches its small-radius behaviour.
    let radius = cpu_gaussian_kernel_radius(sigma);
    gl::UseProgram(objects.blur_program);
    uniform_1i(objects.blur_program, "uInput", 0);
    uniform_2f(objects.blur_program, "uTexel", 1.0 / width as f32, 1.0 / height as f32);
    uniform_1f(objects.blur_program, "uSigma", sigma);
    gl::Uniform1i(uniform_location(objects.blur_program, "uRadius"), radius);
    for (input, output, x, y) in [
        (objects.effect_luma_texture, objects.effect_blur_fbo, 1.0, 0.0),
        (objects.effect_blur_texture, objects.effect_luma_fbo, 0.0, 1.0),
    ] {
        gl::BindFramebuffer(gl::FRAMEBUFFER, output);
        gl::BindTexture(gl::TEXTURE_2D, input);
        uniform_2f(objects.blur_program, "uDirection", x, y);
        gl::DrawArrays(gl::TRIANGLES, 0, 3);
    }

    gl::BindFramebuffer(gl::FRAMEBUFFER, original_framebuffer as u32);
    gl::Viewport(0, 0, view_width_px, view_height_px);
    gl::UseProgram(objects.program);
    uniform_1i(objects.program, "uClarityPrepass", 0);
    gl::ActiveTexture(gl::TEXTURE0);
    gl::BindTexture(gl::TEXTURE_2D, objects.texture);
}

fn gl_library() -> Result<&'static Library, String> {
    static LIBRARY: OnceLock<Result<Library, String>> = OnceLock::new();
    LIBRARY
        .get_or_init(|| unsafe { Library::new("libGL.so.1").map_err(|error| error.to_string()) })
        .as_ref()
        .map_err(Clone::clone)
}

fn compile_shader(kind: u32, source: &str) -> Result<u32, String> {
    let source = CString::new(source).map_err(|error| error.to_string())?;
    let shader;
    unsafe {
        shader = gl::CreateShader(kind);
        let pointer = source.as_ptr();
        gl::ShaderSource(shader, 1, &pointer, std::ptr::null());
        gl::CompileShader(shader);
        let mut compiled = 0;
        gl::GetShaderiv(shader, gl::COMPILE_STATUS, &mut compiled);
        if compiled == 0 {
            let message = shader_log(shader);
            gl::DeleteShader(shader);
            return Err(format!("shader compile failed: {message}"));
        }
    }
    Ok(shader)
}

fn shader_log(shader: u32) -> String {
    unsafe {
        let mut length = 0;
        gl::GetShaderiv(shader, gl::INFO_LOG_LENGTH, &mut length);
        let mut bytes = vec![0u8; length.max(1) as usize];
        gl::GetShaderInfoLog(
            shader,
            length,
            std::ptr::null_mut(),
            bytes.as_mut_ptr().cast(),
        );
        String::from_utf8_lossy(&bytes)
            .trim_matches('\0')
            .to_owned()
    }
}

fn program_log(program: u32) -> String {
    unsafe {
        let mut length = 0;
        gl::GetProgramiv(program, gl::INFO_LOG_LENGTH, &mut length);
        let mut bytes = vec![0u8; length.max(1) as usize];
        gl::GetProgramInfoLog(
            program,
            length,
            std::ptr::null_mut(),
            bytes.as_mut_ptr().cast(),
        );
        String::from_utf8_lossy(&bytes)
            .trim_matches('\0')
            .to_owned()
    }
}

unsafe fn uniform_location(program: u32, name: &str) -> i32 {
    let name = CString::new(name).expect("static uniform name");
    gl::GetUniformLocation(program, name.as_ptr())
}

unsafe fn uniform_1i(program: u32, name: &str, value: i32) {
    gl::Uniform1i(uniform_location(program, name), value);
}

unsafe fn uniform_1f(program: u32, name: &str, value: f32) {
    gl::Uniform1f(uniform_location(program, name), value);
}

unsafe fn uniform_2f(program: u32, name: &str, x: f32, y: f32) {
    gl::Uniform2f(uniform_location(program, name), x, y);
}

unsafe fn uniform_3f(program: u32, name: &str, x: f32, y: f32, z: f32) {
    gl::Uniform3f(uniform_location(program, name), x, y, z);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renderer_mode_defaults_to_cpu_safe_auto() {
        assert_eq!(EditRendererMode::parse(None), EditRendererMode::Auto);
        assert_eq!(
            EditRendererMode::parse(Some("auto")),
            EditRendererMode::Auto
        );
        assert_eq!(EditRendererMode::parse(Some("gpu")), EditRendererMode::Gpu);
        assert_eq!(EditRendererMode::parse(Some("cpu")), EditRendererMode::Cpu);
        assert_eq!(EditRendererMode::Auto.should_attempt_gpu(), false);
        assert!(!EditRendererMode::Cpu.should_attempt_gpu());
        assert!(EditRendererMode::Gpu.should_attempt_gpu());
    }

    #[test]
    fn gpu_base_recipe_removes_only_shader_supported_tone_adjustments() {
        let mut recipe = EditRecipe::default();
        recipe.exposure = 0.2;
        recipe.contrast = 0.1;
        recipe.saturation = -0.3;
        recipe.temperature = 0.4;
        recipe.fill_light = 0.5;
        recipe.shadows = -0.4;
        recipe.highlights = 0.3;
        recipe.crop.left = 0.1;
        let base = gpu_base_recipe(&recipe);
        assert_eq!(base.exposure, 0.0);
        assert_eq!(base.contrast, 0.0);
        assert_eq!(base.saturation, 0.0);
        assert_eq!(base.temperature, 0.0);
        assert_eq!(base.fill_light, 0.0);
        assert_eq!(base.shadows, 0.0);
        assert_eq!(base.highlights, 0.0);
        assert_eq!(base.sharpen, 0.0);
        assert_eq!(base.grain, 0.0);
        assert_eq!(base.clarity, 0.0);
        assert_eq!(base.filter, FilterPreset::None);
        assert!(!base.black_white);
        assert!(!base.sepia);
        assert!(!base.auto_color);
        assert_eq!(base.crop, recipe.crop);
    }

    #[test]
    fn gpu_support_includes_clarity_grain_and_monochrome_looks() {
        let recipe = EditRecipe::default();
        assert!(gpu_tone_supported(&recipe));
        let mut sharpened = recipe.clone();
        sharpened.sharpen = 0.35;
        assert!(gpu_tone_supported(&sharpened));
        let mut looks = recipe.clone();
        looks.grain = 0.5;
        looks.black_white = true;
        looks.sepia = true;
        looks.filter_amount = 0.5;
        assert!(gpu_tone_supported(&looks));
        let mut unsupported = recipe;
        unsupported.clarity = 0.1;
        assert!(gpu_tone_supported(&unsupported));
        unsupported.auto_color = true;
        assert!(gpu_tone_supported(&unsupported));
        unsupported.filter = FilterPreset::Clarendon;
        assert!(!gpu_tone_supported(&unsupported));
        for effect_filter in [FilterPreset::PencilSketch, FilterPreset::DreamGlow] {
            let mut preset = EditRecipe::default();
            preset.filter = effect_filter;
            assert!(gpu_tone_supported(&preset));
        }
        for supported_filter in [FilterPreset::Clarendon, FilterPreset::Gingham, FilterPreset::Valencia, FilterPreset::OldPhoto] {
            let mut preset = EditRecipe::default();
            preset.filter = supported_filter;
            assert!(gpu_tone_supported(&preset));
        }
    }

    #[test]
    #[ignore = "requires a GTK display and working OpenGL; run under xvfb-run"]
    fn gtk_gl_area_matches_cpu_reference_at_native_pixel_size() {
        gtk::init().expect("GTK initialization");
        const WIDTH: u32 = 48;
        const HEIGHT: u32 = 32;
        let renderer = LiveGpuCanvas::new();
        renderer
            .widget()
            .set_size_request(WIDTH as i32, HEIGHT as i32);
        let window = gtk::Window::new();
        window.set_default_size(WIDTH as i32, HEIGHT as i32);
        window.set_child(Some(&renderer.widget()));
        window.present();

        // Skin tones, dark hair and fine lash/iris edges exercise the colors
        // and high-frequency content where a GPU preview must match CPU.
        let source = image::RgbaImage::from_fn(WIDTH, HEIGHT, |x, y| {
            let lash = (10..14).contains(&y) && (x % 4 != 0 || y == 11);
            let hair = y < 8 && x % 3 == 0;
            if lash || hair {
                image::Rgba([24, 18, 22, 255])
            } else if (15..25).contains(&x) && (16..27).contains(&y) {
                image::Rgba([76, 108, 111, 255])
            } else {
                image::Rgba([
                    (186 + x % 24) as u8,
                    (119 + y % 28) as u8,
                    (91 + (x + y) % 21) as u8,
                    255,
                ])
            }
        });
        let reset_recipe = EditRecipe::default();
        renderer.set_source(
            "test://gpu-parity",
            0,
            &reset_recipe,
            WIDTH as i32,
            HEIGHT as i32,
            glib::Bytes::from_owned(source.clone().into_raw()),
        );
        renderer.set_recipe(&reset_recipe);
        renderer.set_view(0.0, 0.0, WIDTH as f64, HEIGHT as f64, 1.0, 0.0, 0.0);

        let context = glib::MainContext::default();
        let check_frame = |recipe: &EditRecipe, source: &image::RgbaImage| {
            renderer.request_readback();
            let deadline = Instant::now() + std::time::Duration::from_secs(5);
            let mut pixels = None;
            while Instant::now() < deadline {
                while context.pending() {
                    context.iteration(false);
                }
                if let Some(readback) = renderer.take_readback() {
                    pixels = Some(readback);
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            let actual = pixels.expect("GLArea should present a frame and allow test readback");
            let scale = renderer.widget().scale_factor().max(1) as u32;
            let actual_top_down = actual
                .chunks_exact(WIDTH as usize * scale as usize * 4)
                .rev()
                .flatten()
                .copied()
                .collect::<Vec<_>>();
            let expected = super::super::render::apply_tone(source.clone(), recipe);
            let expected = if scale == 1 {
                expected
            } else {
                image::imageops::resize(
                    &expected,
                    WIDTH * scale,
                    HEIGHT * scale,
                    image::imageops::FilterType::Nearest,
                )
            }
            .into_raw();
            let mut max_delta = 0i16;
            for (index, (actual, expected)) in
                actual_top_down.iter().zip(expected.iter()).enumerate()
            {
                let delta = (*actual as i16 - *expected as i16).abs();
                max_delta = max_delta.max(delta);
                assert!(delta <= 1, "channel {index} differs by {delta}");
            }
            eprintln!(
                "PIC_EDIT_GPU parity_test dimensions={}x{} recipe_default={} max_channel_delta={max_delta}",
                WIDTH,
                HEIGHT,
                recipe.is_default(),
            );
        };

        check_frame(&reset_recipe, &source);
        eprintln!("PIC_EDIT_GPU test_gsk_renderer={:?}", window.renderer().map(|r| r.type_().name().to_owned()));
        let uploaded_revision = renderer.uploaded_revision();
        let mut edited = reset_recipe.clone();
        edited.exposure = 0.17;
        edited.contrast = 0.23;
        edited.saturation = -0.19;
        edited.temperature = 0.31;
        edited.fill_light = 0.2;
        edited.shadows = -0.15;
        edited.highlights = 0.1;
        edited.sharpen = 0.35;
        renderer.set_recipe(&edited);
        check_frame(&edited, &source);
        assert_eq!(renderer.uploaded_revision(), uploaded_revision);
        renderer.set_recipe(&reset_recipe);
        check_frame(&reset_recipe, &source);
        assert_eq!(renderer.uploaded_revision(), uploaded_revision);

        for (grain, black_white, sepia, amount) in [
            (0.7, false, false, 1.0),
            (0.0, true, false, 0.55),
            (0.0, false, true, 0.7),
            (0.25, true, true, 0.5),
        ] {
            let mut effect = reset_recipe.clone();
            effect.grain = grain;
            effect.black_white = black_white;
            effect.sepia = sepia;
            effect.filter_amount = amount;
            assert!(gpu_tone_supported(&effect));
            renderer.set_recipe(&effect);
            check_frame(&effect, &source);
            assert_eq!(renderer.uploaded_revision(), uploaded_revision);
        }
        // Check each ordinary preset against the existing CPU/export look.
        // At 1:1 the GPU must not change source dimensions or upload again.
        for preset in [
            FilterPreset::Clarendon, FilterPreset::Gingham,
            FilterPreset::Juno, FilterPreset::Lark,
            FilterPreset::Valencia, FilterPreset::Walden,
            FilterPreset::DisposableFilm, FilterPreset::DigicamFlash,
            FilterPreset::Cinematic, FilterPreset::GoldenHour,
            FilterPreset::OldPhoto, FilterPreset::Newspaper,
        ] {
            let mut recipe = reset_recipe.clone();
            recipe.filter = preset;
            recipe.filter_amount = 0.65;
            renderer.set_recipe(&recipe);
            check_frame(&recipe, &source);
            assert_eq!(renderer.uploaded_revision(), uploaded_revision);
        }
        renderer.set_recipe(&reset_recipe);
        check_frame(&reset_recipe, &source);

        // Pencil Sketch and Dream Glow use two additional GPU blur passes.
        // Their reduced-resolution kernels have a looser reference tolerance
        // than exact per-pixel colour transforms, but still catch wrong passes.
        for preset in [FilterPreset::PencilSketch, FilterPreset::DreamGlow] {
            let mut recipe = reset_recipe.clone();
            recipe.filter = preset;
            recipe.filter_amount = 1.0;
            assert!(gpu_tone_supported(&recipe));
            renderer.set_recipe(&recipe);
            renderer.request_readback();
            let deadline = Instant::now() + std::time::Duration::from_secs(5);
            let mut result = None;
            while Instant::now() < deadline {
                while context.pending() { context.iteration(false); }
                if let Some(readback) = renderer.take_readback() {
                    result = Some(readback);
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            let actual = result.expect("GPU special-filter readback missing");
            let scale = renderer.widget().scale_factor().max(1) as u32;
            let actual = actual.chunks_exact(WIDTH as usize * scale as usize * 4)
                .rev().flatten().copied().collect::<Vec<_>>();
            let cpu = super::super::render::apply_tone(source.clone(), &recipe);
            let cpu = if scale == 1 { cpu } else {
                image::imageops::resize(&cpu, WIDTH * scale, HEIGHT * scale,
                    image::imageops::FilterType::Nearest)
            }.into_raw();
            assert_eq!(actual.len(), cpu.len());
            let errors = actual.iter().zip(&cpu)
                .map(|(a, b)| (*a as i16 - *b as i16).unsigned_abs() as usize)
                .collect::<Vec<_>>();
            let maximum = errors.iter().copied().max().unwrap_or_default();
            let average = errors.iter().sum::<usize>() as f64 / errors.len().max(1) as f64;
            let worst = errors.iter().enumerate().max_by_key(|(_, delta)| *delta)
                .map(|(index, delta)| {
                    let stride = WIDTH as usize * scale as usize * 4;
                    (index / stride, (index % stride) / 4, index % 4,
                     actual[index], cpu[index], *delta)
                });
            eprintln!("PIC_EDIT_GPU special_filter_parity preset={preset:?} max={maximum} avg={average:.3} worst={worst:?}");
            assert!(maximum <= 35 && average <= 5.0,
                "GPU {preset:?} differs too much from CPU: max={maximum}, avg={average:.3}");
            assert_eq!(renderer.uploaded_revision(), uploaded_revision);
        }
        renderer.set_recipe(&reset_recipe);
        check_frame(&reset_recipe, &source);

        // Clarity requires a GPU luminance/blur pass. Here we first verify it
        // really renders on GL and does not re-upload source data. Pixel parity
        // at arbitrary scales is covered separately before default enablement.
        let mut clarity_recipe = reset_recipe.clone();
        clarity_recipe.clarity = 0.45;
        assert!(gpu_tone_supported(&clarity_recipe));
        renderer.set_recipe(&clarity_recipe);
        renderer.request_readback();
        let deadline = Instant::now() + std::time::Duration::from_secs(5);
        let mut clarity_pixels = None;
        while Instant::now() < deadline {
            while context.pending() { context.iteration(false); }
            if let Some(readback) = renderer.take_readback() {
                clarity_pixels = Some(readback);
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let readback = clarity_pixels.expect("GPU clarity framebuffer readback failed");
        let scale = renderer.widget().scale_factor().max(1) as u32;
        let observed = readback.chunks_exact(WIDTH as usize * scale as usize * 4)
            .rev().flatten().copied().collect::<Vec<_>>();
        let expected = super::super::render::apply_tone(source.clone(), &clarity_recipe);
        let expected = if scale == 1 { expected } else {
            image::imageops::resize(&expected, WIDTH * scale, HEIGHT * scale,
                image::imageops::FilterType::Nearest)
        }.into_raw();
        let maximum_delta = observed.iter().zip(&expected)
            .map(|(a, b)| (*a as i16 - *b as i16).abs())
            .max().unwrap_or_default();
        assert!(maximum_delta <= 12,
            "GPU Clarity differs too much from the CPU reference: max channel delta {maximum_delta}");
        assert_eq!(renderer.uploaded_revision(), uploaded_revision);
        renderer.set_recipe(&reset_recipe);
        check_frame(&reset_recipe, &source);

        // Full effect stack should compose without a CPU fallback, texture
        // re-upload or framebuffer feedback between independent blur passes.
        let mut mixed = reset_recipe.clone();
        mixed.filter = FilterPreset::DreamGlow;
        mixed.filter_amount = 0.7;
        mixed.clarity = 0.4;
        mixed.grain = 0.3;
        mixed.sharpen = 0.25;
        mixed.exposure = 0.12;
        assert!(gpu_tone_supported(&mixed));
        renderer.set_recipe(&mixed);
        renderer.request_readback();
        let deadline = Instant::now() + std::time::Duration::from_secs(5);
        let mut result = None;
        while Instant::now() < deadline {
            while context.pending() { context.iteration(false); }
            if let Some(readback) = renderer.take_readback() {
                result = Some(readback);
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let result = result.expect("mixed GPU effects did not render");
        let scale = renderer.widget().scale_factor().max(1) as u32;
        let gpu = result.chunks_exact(WIDTH as usize * scale as usize * 4)
            .rev().flatten().copied().collect::<Vec<_>>();
        let cpu = super::super::render::apply_tone(source.clone(), &mixed);
        let cpu = if scale == 1 { cpu } else {
            image::imageops::resize(&cpu, WIDTH * scale, HEIGHT * scale,
                image::imageops::FilterType::Nearest)
        }.into_raw();
        let differences = gpu.iter().zip(&cpu)
            .map(|(a,b)| (*a as i16 - *b as i16).unsigned_abs() as usize)
            .collect::<Vec<_>>();
        let max_error = differences.iter().copied().max().unwrap_or_default();
        let mean_error = differences.iter().sum::<usize>() as f64 / differences.len().max(1) as f64;
        eprintln!("PIC_EDIT_GPU mixed_parity max={max_error} mean={mean_error:.3}");
        assert!(max_error <= 45 && mean_error <= 9.0,
            "mixed GPU recipe diverges from CPU: max={max_error} mean={mean_error:.3}");
        assert_eq!(renderer.uploaded_revision(), uploaded_revision);
        renderer.set_recipe(&reset_recipe);
        check_frame(&reset_recipe, &source);

        // A CPU-only effect hides this layer. Reset to an unchanged neutral
        // recipe must show the cached source again, without another upload.
        renderer.hide();
        assert!(!renderer.widget().is_visible());
        renderer.set_recipe(&reset_recipe);
        assert!(renderer.widget().is_visible());
        check_frame(&reset_recipe, &source);
        assert_eq!(renderer.uploaded_revision(), uploaded_revision);

        // At 1:1, a scrolled viewport samples a small region of the full
        // source texture without allocating a source-sized render target.
        renderer.set_view(7.4, 5.6, 16.0, 12.0, 1.0, 0.0, 0.0);
        renderer.request_readback();
        let deadline = Instant::now() + std::time::Duration::from_secs(5);
        let mut viewport = None;
        while Instant::now() < deadline {
            while context.pending() {
                context.iteration(false);
            }
            if let Some(readback) = renderer.take_readback() {
                viewport = Some(readback);
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let expected_viewport = image::imageops::crop_imm(&source, 7, 6, 16, 12).to_image();
        let viewport = viewport.expect("scrolled GL viewport should draw");
        let scale = renderer.widget().scale_factor().max(1) as u32;
        let viewport_top_down = viewport
            .chunks_exact(16 * scale as usize * 4)
            .rev()
            .flatten()
            .copied()
            .collect::<Vec<_>>();
        let expected_viewport = if scale == 1 {
            expected_viewport
        } else {
            image::imageops::resize(
                &expected_viewport,
                16 * scale,
                12 * scale,
                image::imageops::FilterType::Nearest,
            )
        };
        assert_eq!(
            viewport_top_down,
            expected_viewport.into_raw(),
            "viewport should sample only the visible source rectangle"
        );

        // Fit mode keeps the large source texture but renders only this
        // viewport. At half scale the linear sampler matches a 2x2 box on
        // this deliberately synthetic fine-detail fixture.
        renderer.set_view(0.0, 0.0, 24.0, 16.0, 0.5, 0.0, 0.0);
        renderer.request_readback();
        let deadline = Instant::now() + std::time::Duration::from_secs(5);
        let mut fit_readback = None;
        while Instant::now() < deadline {
            while context.pending() {
                context.iteration(false);
            }
            if let Some(readback) = renderer.take_readback() {
                fit_readback = Some(readback);
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let fit_actual = fit_readback.expect("Fit viewport should render");
        let scale = renderer.widget().scale_factor().max(1) as u32;
        let fit_actual = fit_actual
            .chunks_exact(24 * scale as usize * 4)
            .rev()
            .flatten()
            .copied()
            .collect::<Vec<_>>();
        let fit_expected = image::RgbaImage::from_fn(24, 16, |x, y| {
            let mut sum = [0u16; 4];
            for dy in 0..2 {
                for dx in 0..2 {
                    let pixel = source.get_pixel(x * 2 + dx, y * 2 + dy);
                    for channel in 0..4 {
                        sum[channel] += pixel[channel] as u16;
                    }
                }
            }
            image::Rgba(sum.map(|channel| ((channel + 2) / 4) as u8))
        });
        let fit_expected = if scale == 1 {
            fit_expected
        } else {
            image::imageops::resize(
                &fit_expected,
                24 * scale,
                16 * scale,
                image::imageops::FilterType::Nearest,
            )
        }
        .into_raw();
        for (index, (actual, expected)) in fit_actual.iter().zip(&fit_expected).enumerate() {
            assert!(
                (*actual as i16 - *expected as i16).abs() <= 1,
                "Fit channel {index} differs from 2x2 linear reference"
            );
        }

        let mut fit_sharpen = reset_recipe.clone();
        fit_sharpen.sharpen = 0.35;
        renderer.set_recipe(&fit_sharpen);
        renderer.request_readback();
        let deadline = Instant::now() + std::time::Duration::from_secs(5);
        let mut sharpened_fit = None;
        while Instant::now() < deadline {
            while context.pending() {
                context.iteration(false);
            }
            if let Some(readback) = renderer.take_readback() {
                sharpened_fit = Some(readback);
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let sharpened_fit = sharpened_fit.expect("sharpened Fit viewport should render");
        let sharpened_fit = sharpened_fit
            .chunks_exact(24 * scale as usize * 4)
            .rev()
            .flatten()
            .copied()
            .collect::<Vec<_>>();
        let sharpened_source = super::super::render::apply_tone(source.clone(), &fit_sharpen);
        let expected_fit = image::RgbaImage::from_fn(24, 16, |x, y| {
            let mut sum = [0u16; 4];
            for dy in 0..2 {
                for dx in 0..2 {
                    let pixel = sharpened_source.get_pixel(x * 2 + dx, y * 2 + dy);
                    for channel in 0..4 {
                        sum[channel] += pixel[channel] as u16;
                    }
                }
            }
            image::Rgba(sum.map(|channel| ((channel + 2) / 4) as u8))
        });
        let expected_fit = if scale == 1 {
            expected_fit
        } else {
            image::imageops::resize(
                &expected_fit,
                24 * scale,
                16 * scale,
                image::imageops::FilterType::Nearest,
            )
        }
        .into_raw();
        for (index, (actual, expected)) in sharpened_fit.iter().zip(&expected_fit).enumerate() {
            assert!(
                (*actual as i16 - *expected as i16).abs() <= 1,
                "sharpened Fit channel {index} differs from the CPU reference"
            );
        }
        // GPU Auto Colour must match the CPU neutral estimate when many
        // credible near-neutral pixels exist, not merely on no-op photos.
        let neutrals = image::RgbaImage::from_fn(WIDTH, HEIGHT, |x, y| {
            let n = ((x + y) % 14) as u8;
            image::Rgba([112 + n, 124 + n, 137 + n, 255])
        });
        let auto = EditRecipe { auto_color: true, ..Default::default() };
        assert!(gpu_tone_supported(&auto));
        renderer.set_source(
            "test://autocolor-neutrals", 0, &auto,
            WIDTH as i32, HEIGHT as i32,
            glib::Bytes::from_owned(neutrals.clone().into_raw()),
        );
        renderer.set_view(0.0, 0.0, WIDTH as f64, HEIGHT as f64, 1.0, 0.0, 0.0);
        renderer.set_recipe(&auto);
        check_frame(&auto, &neutrals);
        renderer.set_recipe(&EditRecipe::default());
        check_frame(&EditRecipe::default(), &neutrals);
        renderer.set_source(
            "test://gpu-parity", 0, &reset_recipe,
            WIDTH as i32, HEIGHT as i32,
            glib::Bytes::from_owned(source.clone().into_raw()),
        );
        renderer.set_recipe(&reset_recipe);
        check_frame(&reset_recipe, &source);

        window.close();
    }

    /// Optional private full-resolution regression:
    /// PIC_TEST_EDIT_REFERENCE=/path/to/DSC_5791.NEF
    ///
    /// With no reference, CI generates an anonymous high-frequency 3936x2624
    /// fixture instead. Never commit personal photographs to the public repo.
    #[test]
    #[ignore = "requires a GTK display + OpenGL; run under xvfb-run"]
    fn gtk_gl_area_full_resolution_sharpness_regression() {
        gtk::init().expect("GTK initialization");
        const VIEW: i32 = 256;
        let image = if let Ok(path) = std::env::var("PIC_TEST_EDIT_REFERENCE") {
            super::super::render::decode_base_for_viewer(&path, 0, u32::MAX, u32::MAX)
                .expect("real photo must decode using PIC's viewer path")
        } else {
            image::RgbaImage::from_fn(3936, 2624, |x, y| {
                // Fine one/two-pixel strands mixed with broad photographic
                // gradients. A 960-pixel draft would destroy these details.
                let hair = (x % 7) == 0 && (y % 13) < 7;
                let eyelash = y % 23 == 0 && x % 4 < 2;
                let r = if hair || eyelash { 22 } else { (60 + x % 147) as u8 };
                let g = if hair || eyelash { 17 } else { (35 + y % 188) as u8 };
                let b = if hair || eyelash { 26 } else { (45 + (x+y) % 173) as u8 };
                image::Rgba([r, g, b, 255])
            })
        };
        let (width, height) = image.dimensions();
        assert!(width >= VIEW as u32 && height >= VIEW as u32);
        let x = width / 2 - VIEW as u32 / 2;
        let y = height / 2 - VIEW as u32 / 2;
        let renderer = LiveGpuCanvas::new();
        renderer.widget().set_size_request(VIEW, VIEW);
        let window = gtk::Window::new();
        window.set_default_size(VIEW, VIEW);
        window.set_child(Some(&renderer.widget()));
        window.present();

        let reference = EditRecipe::default();
        renderer.set_source(
            "test://full-detail", 0, &reference,
            width as i32, height as i32,
            glib::Bytes::from_owned(image.clone().into_raw()),
        );
        renderer.set_view(x as f64, y as f64, VIEW as f64, VIEW as f64, 1.0, 0.0, 0.0);

        let context = glib::MainContext::default();
        let read_frame = |recipe: &EditRecipe| {
            renderer.set_recipe(recipe);
            renderer.request_readback();
            let deadline = Instant::now() + std::time::Duration::from_secs(20);
            let result = loop {
                while context.pending() {
                    context.iteration(false);
                }
                if let Some(pixels) = renderer.take_readback() {
                    break pixels;
                }
                assert!(Instant::now() < deadline, "full-resolution GPU frame timed out");
                std::thread::sleep(std::time::Duration::from_millis(5));
            };
            let scale = renderer.widget().scale_factor().max(1) as usize;
            assert_eq!(result.len(), (VIEW as usize * scale).pow(2) * 4);
            result.chunks_exact(VIEW as usize * scale * 4)
                .rev().flatten().copied().collect::<Vec<u8>>()
        };
        let neutral = read_frame(&reference);
        let revision = renderer.uploaded_revision();
        let mut exposure = reference.clone();
        exposure.exposure = -0.03;
        let exposed = read_frame(&exposure);
        assert_eq!(renderer.uploaded_revision(), revision,
            "Exposure-only edit must reuse the full-size uploaded texture");
        let cpu = super::super::render::apply_tone(image.clone(), &exposure);
        let expected = image::imageops::crop_imm(&cpu, x, y, VIEW as u32, VIEW as u32).to_image();
        let scale = renderer.widget().scale_factor().max(1) as u32;
        let expected = if scale == 1 { expected } else {
            image::imageops::resize(
                &expected,
                VIEW as u32 * scale,
                VIEW as u32 * scale,
                image::imageops::FilterType::Nearest,
            )
        }.into_raw();
        assert_eq!(exposed.len(), expected.len());
        let max_delta = exposed.iter().zip(&expected)
            .map(|(a, b)| (*a as i16 - *b as i16).abs())
            .max().unwrap_or_default();
        assert!(max_delta <= 1,
            "full-source exposure GPU/CPU parity failed: max channel delta {max_delta}");
        // RGB edge strength within the same native 1:1 viewport. Exposure must
        // not resample or erase the image texture's one-pixel detail.
        let edge_energy = |rgba: &[u8]| -> f64 {
            let stride = VIEW as usize * scale as usize * 4;
            let mut sum = 0u64;
            for row in rgba.chunks_exact(stride) {
                for x in 1..(VIEW as usize * scale as usize) {
                    let idx = x * 4;
                    for channel in 0..3 {
                        sum += (row[idx+channel] as i16 - row[idx-4+channel] as i16)
                            .unsigned_abs() as u64;
                    }
                }
            }
            sum as f64
        };
        let source_energy = edge_energy(&neutral);
        let edit_energy = edge_energy(&exposed);
        let ratio = edit_energy / source_energy.max(1.0);
        assert!((0.92..=1.08).contains(&ratio),
            "tiny exposure altered native-pixel sharpness: edge ratio {ratio:.4}");
        let reset = read_frame(&reference);
        assert_eq!(neutral, reset, "exposure reset changed original GPU pixels");
        assert_eq!(renderer.uploaded_revision(), revision);
        eprintln!(
            "PIC_EDIT_GPU full_source_quality source={}x{} viewport={}x{} \
             max_channel_delta={} exposure_edge_ratio={:.4} upload_count=1",
            width, height, VIEW, VIEW, max_delta, ratio
        );
        window.close();
    }

    #[test]
    #[ignore = "measures software/hardware GL upload and viewport rendering; run under xvfb-run"]
    fn gtk_gl_area_benchmarks_full_source_upload_and_viewport_sized_draw() {
        gtk::init().expect("GTK initialization");
        const SOURCE_WIDTH: u32 = 3936;
        const SOURCE_HEIGHT: u32 = 2624;
        const VIEW_WIDTH: i32 = 960;
        const VIEW_HEIGHT: i32 = 640;

        let renderer = LiveGpuCanvas::new();
        renderer.widget().set_size_request(VIEW_WIDTH, VIEW_HEIGHT);
        let window = gtk::Window::new();
        window.set_default_size(VIEW_WIDTH, VIEW_HEIGHT);
        window.set_child(Some(&renderer.widget()));
        window.present();
        let source = if let Ok(reference) = std::env::var("PIC_TEST_EDIT_REFERENCE") {
            super::super::render::decode_base_for_viewer(&reference, 0, u32::MAX, u32::MAX)
                .expect("benchmark photograph must decode")
        } else {
            image::RgbaImage::from_fn(SOURCE_WIDTH, SOURCE_HEIGHT, |x, y| {
                image::Rgba([(x % 251) as u8, (y % 239) as u8, ((x + y) % 233) as u8, 255])
            })
        };
        let (source_width, source_height) = source.dimensions();
        let base_recipe = EditRecipe::default();
        let upload_started = Instant::now();
        renderer.set_source(
            "test://source-benchmark",
            0,
            &base_recipe,
            source_width as i32,
            source_height as i32,
            glib::Bytes::from_owned(source.clone().into_raw()),
        );
        renderer.set_view(
            0.0,
            0.0,
            VIEW_WIDTH as f64,
            VIEW_HEIGHT as f64,
            VIEW_WIDTH as f64 / source_width as f64,
            0.0,
            0.0,
        );

        let context = glib::MainContext::default();
        let deadline = Instant::now() + std::time::Duration::from_secs(15);
        while !renderer.source_uploaded() && Instant::now() < deadline {
            while context.pending() {
                context.iteration(false);
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(
            renderer.source_uploaded(),
            "Full source texture upload failed"
        );
        let ready_ms = upload_started.elapsed().as_secs_f64() * 1000.0;
        renderer.widget().make_current();
        let driver = unsafe {
            let name = gl::GetString(gl::RENDERER);
            if name.is_null() { "unknown".to_owned() }
            else { std::ffi::CStr::from_ptr(name.cast()).to_string_lossy().into_owned() }
        };
        eprintln!("PIC_EDIT_GPU benchmark_gl_renderer={driver} gsk_renderer={:?}", window.renderer().map(|r| r.type_().name().to_owned()));


        let mut recipe = base_recipe.clone();
        recipe.exposure = 0.2;
        recipe.sharpen = 0.35;
        let cpu_started = Instant::now();
        let _cpu_reference = super::super::render::apply_tone(source.clone(), &recipe);
        let cpu_ms = cpu_started.elapsed().as_secs_f64() * 1000.0;

        let prior_frame = renderer.rendered_frames();
        let gpu_started = Instant::now();
        renderer.set_recipe(&recipe);
        while renderer.rendered_frames() <= prior_frame && Instant::now() < deadline {
            while context.pending() {
                context.iteration(false);
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        let uniform_to_draw_ms = gpu_started.elapsed().as_secs_f64() * 1000.0;
        assert!(
            renderer.rendered_frames() > prior_frame,
            "GPU draw was not queued"
        );
        eprintln!(
            "PIC_EDIT_GPU benchmark source={}x{} viewport={}x{} upload_ready_ms={ready_ms:.3} cpu_full_frame_ms={cpu_ms:.3} uniform_to_draw_callback_ms={uniform_to_draw_ms:.3}",
            source_width, source_height, VIEW_WIDTH, VIEW_HEIGHT,
        );
        window.close();
    }
}
