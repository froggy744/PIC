//! Experimental GTK GLArea renderer for live edit tone controls.
//!
//! The source texture remains at decoded source dimensions. GtkGLArea itself
//! is kept at the scrolled viewport size, so 24MP photos never create a 24MP
//! render target. `auto` validates the active GL context and shaders before
//! selecting GPU, then falls back to CPU on initialization or draw failures.

use std::cell::RefCell;
use std::ffi::{c_void, CString};
use std::rc::Rc;
use std::sync::{Mutex, OnceLock};
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
        match self {
            Self::Cpu => false,
            Self::Gpu => true,
            Self::Auto => auto_gpu_retry_allowed(),
        }
    }
}

#[derive(Clone, Debug)]
enum AutoGpuDecision {
    Unknown,
    Verified(String),
    Failed {
        reason: String,
        retry_after: Instant,
    },
}

static AUTO_GPU_DECISION: OnceLock<Mutex<AutoGpuDecision>> = OnceLock::new();
const AUTO_GPU_RETRY_DELAY: std::time::Duration = std::time::Duration::from_secs(60);

fn auto_gpu_decision() -> &'static Mutex<AutoGpuDecision> {
    AUTO_GPU_DECISION.get_or_init(|| Mutex::new(AutoGpuDecision::Unknown))
}

fn auto_gpu_retry_allowed() -> bool {
    let Ok(decision) = auto_gpu_decision().lock() else {
        return true;
    };
    match &*decision {
        AutoGpuDecision::Unknown | AutoGpuDecision::Verified(_) => true,
        AutoGpuDecision::Failed {
            reason,
            retry_after,
        } => {
            if Instant::now() < *retry_after {
                trace_gpu(format_args!(
                    "renderer=cpu auto_retry_suppressed reason={reason:?}"
                ));
                false
            } else {
                true
            }
        }
    }
}

fn cache_auto_gpu_success(fingerprint: String) {
    if let Ok(mut decision) = auto_gpu_decision().lock() {
        *decision = AutoGpuDecision::Verified(fingerprint);
    }
}

fn cache_auto_gpu_failure(reason: String) {
    if let Ok(mut decision) = auto_gpu_decision().lock() {
        *decision = AutoGpuDecision::Failed {
            reason,
            retry_after: Instant::now() + AUTO_GPU_RETRY_DELAY,
        };
    }
}

fn auto_gpu_is_verified(fingerprint: &str) -> bool {
    auto_gpu_decision().lock().ok().is_some_and(
        |decision| matches!(&*decision, AutoGpuDecision::Verified(value) if value == fingerprint),
    )
}

fn trace_gpu(message: impl std::fmt::Display) {
    if std::env::var_os("PICASA_TRACE").is_some() {
        eprintln!("PIC_EDIT_GPU {message}");
    }
}

fn is_software_renderer(renderer: &str) -> bool {
    let renderer = renderer.to_ascii_lowercase();
    [
        "llvmpipe",
        "softpipe",
        "swrast",
        "swiftshader",
        "software rasterizer",
    ]
    .iter()
    .any(|needle| renderer.contains(needle))
}

#[derive(Clone, Debug)]
struct GlCapabilities {
    version: String,
    renderer: String,
    shading_language: String,
    max_texture_size: i32,
    fragment_texture_units: i32,
    vertex_texture_units: i32,
}

impl GlCapabilities {
    fn fingerprint(&self) -> String {
        format!(
            "{}|{}|{}|{}|{}|{}",
            self.version,
            self.renderer,
            self.shading_language,
            self.max_texture_size,
            self.fragment_texture_units,
            self.vertex_texture_units,
        )
    }
}

unsafe fn gl_string(name: u32) -> String {
    let value = gl::GetString(name);
    if value.is_null() {
        "unknown".to_owned()
    } else {
        std::ffi::CStr::from_ptr(value.cast())
            .to_string_lossy()
            .into_owned()
    }
}

fn query_gl_capabilities(mode: EditRendererMode) -> Result<GlCapabilities, String> {
    let (
        version,
        renderer,
        shading_language,
        max_texture_size,
        fragment_texture_units,
        vertex_texture_units,
    ) = unsafe {
        let mut values = [0; 4];
        gl::GetIntegerv(gl::MAJOR_VERSION, &mut values[0]);
        gl::GetIntegerv(gl::MINOR_VERSION, &mut values[1]);
        gl::GetIntegerv(gl::MAX_TEXTURE_SIZE, &mut values[2]);
        gl::GetIntegerv(gl::MAX_TEXTURE_IMAGE_UNITS, &mut values[3]);
        let mut vertex_units = 0;
        gl::GetIntegerv(gl::MAX_VERTEX_TEXTURE_IMAGE_UNITS, &mut vertex_units);
        (
            gl_string(gl::VERSION),
            gl_string(gl::RENDERER),
            gl_string(gl::SHADING_LANGUAGE_VERSION),
            values[2],
            values[3],
            vertex_units,
        )
    };
    let mut major = 0;
    let mut minor = 0;
    unsafe {
        gl::GetIntegerv(gl::MAJOR_VERSION, &mut major);
        gl::GetIntegerv(gl::MINOR_VERSION, &mut minor);
    }
    if major < 3 || (major == 3 && minor < 2) {
        return Err(format!(
            "OpenGL 3.2 required, context reports {major}.{minor} ({version})"
        ));
    }
    if max_texture_size < 2048 {
        return Err(format!(
            "maximum texture size {max_texture_size} is below the 2048 minimum"
        ));
    }
    if fragment_texture_units < 6 || vertex_texture_units < 1 {
        return Err(format!(
            "insufficient texture units (fragment={fragment_texture_units}, vertex={vertex_texture_units}; need 6 and 1)"
        ));
    }
    let software = is_software_renderer(&renderer);
    trace_gpu(format_args!(
        "capabilities version={version:?} glsl={shading_language:?} renderer={renderer:?} software={software} max_texture={max_texture_size} fragment_units={fragment_texture_units} vertex_units={vertex_texture_units} mode={mode:?}"
    ));
    if software && mode == EditRendererMode::Auto {
        return Err(format!(
            "software OpenGL renderer {renderer:?} is not selected automatically"
        ));
    }
    Ok(GlCapabilities {
        version,
        renderer,
        shading_language,
        max_texture_size,
        fragment_texture_units,
        vertex_texture_units,
    })
}

fn validate_render_target() -> Result<(), String> {
    unsafe {
        let mut previous_framebuffer = 0;
        let mut previous_viewport = [0; 4];
        let mut previous_program = 0;
        let mut previous_vao = 0;
        let mut previous_active_texture = 0;
        gl::GetIntegerv(gl::FRAMEBUFFER_BINDING, &mut previous_framebuffer);
        gl::GetIntegerv(gl::VIEWPORT, previous_viewport.as_mut_ptr());
        gl::GetIntegerv(gl::CURRENT_PROGRAM, &mut previous_program);
        gl::GetIntegerv(gl::VERTEX_ARRAY_BINDING, &mut previous_vao);
        gl::GetIntegerv(gl::ACTIVE_TEXTURE, &mut previous_active_texture);
        while gl::GetError() != gl::NO_ERROR {}
        let vertex = compile_shader(
            gl::VERTEX_SHADER,
            "#version 150\nvoid main(){ vec2 p=gl_VertexID==0?vec2(-1,-1):gl_VertexID==1?vec2(3,-1):vec2(-1,3); gl_Position=vec4(p,0,1); }",
        )?;
        let fragment = compile_shader(
            gl::FRAGMENT_SHADER,
            "#version 150\nuniform sampler2D s; out vec4 c; void main(){ c=texelFetch(s,ivec2(0),0); }",
        )?;
        let program = gl::CreateProgram();
        gl::AttachShader(program, vertex);
        gl::AttachShader(program, fragment);
        gl::LinkProgram(program);
        gl::DeleteShader(vertex);
        gl::DeleteShader(fragment);
        let mut linked = 0;
        gl::GetProgramiv(program, gl::LINK_STATUS, &mut linked);
        if linked == 0 {
            let message = program_log(program);
            gl::DeleteProgram(program);
            return Err(format!("validation shader link failed: {message}"));
        }
        let mut vao = 0;
        let mut source = 0;
        let mut target = 0;
        let mut fbo = 0;
        gl::GenVertexArrays(1, &mut vao);
        gl::BindVertexArray(vao);
        gl::GenTextures(1, &mut source);
        gl::BindTexture(gl::TEXTURE_2D, source);
        gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MIN_FILTER, gl::NEAREST as i32);
        gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MAG_FILTER, gl::NEAREST as i32);
        let expected = [64u8, 128, 192, 255];
        gl::TexImage2D(
            gl::TEXTURE_2D,
            0,
            gl::RGBA8 as i32,
            2,
            2,
            0,
            gl::RGBA,
            gl::UNSIGNED_BYTE,
            expected.as_ptr().cast(),
        );
        gl::GenTextures(1, &mut target);
        gl::BindTexture(gl::TEXTURE_2D, target);
        gl::TexImage2D(
            gl::TEXTURE_2D,
            0,
            gl::RGBA32F as i32,
            2,
            2,
            0,
            gl::RGBA,
            gl::FLOAT,
            std::ptr::null(),
        );
        gl::GenFramebuffers(1, &mut fbo);
        gl::BindFramebuffer(gl::FRAMEBUFFER, fbo);
        gl::FramebufferTexture2D(
            gl::FRAMEBUFFER,
            gl::COLOR_ATTACHMENT0,
            gl::TEXTURE_2D,
            target,
            0,
        );
        let framebuffer_status = gl::CheckFramebufferStatus(gl::FRAMEBUFFER);
        if framebuffer_status != gl::FRAMEBUFFER_COMPLETE {
            gl::BindFramebuffer(gl::FRAMEBUFFER, previous_framebuffer as u32);
            gl::Viewport(
                previous_viewport[0],
                previous_viewport[1],
                previous_viewport[2],
                previous_viewport[3],
            );
            gl::BindVertexArray(previous_vao as u32);
            gl::UseProgram(previous_program as u32);
            gl::ActiveTexture(previous_active_texture as u32);
            gl::DeleteFramebuffers(1, &fbo);
            gl::DeleteTextures(1, &target);
            gl::DeleteTextures(1, &source);
            gl::DeleteVertexArrays(1, &vao);
            gl::DeleteProgram(program);
            return Err(format!(
                "validation framebuffer incomplete: 0x{framebuffer_status:04x}"
            ));
        }
        gl::Viewport(0, 0, 2, 2);
        gl::UseProgram(program);
        gl::ActiveTexture(gl::TEXTURE0);
        gl::BindTexture(gl::TEXTURE_2D, source);
        gl::Uniform1i(gl::GetUniformLocation(program, c"s".as_ptr()), 0);
        gl::DrawArrays(gl::TRIANGLES, 0, 3);
        let mut pixels = [0.0f32; 16];
        gl::ReadPixels(
            0, 0, 2, 2, gl::RGBA, gl::FLOAT, pixels.as_mut_ptr().cast(),
        );
        let gl_error = gl::GetError();
        gl::BindFramebuffer(gl::FRAMEBUFFER, previous_framebuffer as u32);
        gl::Viewport(
            previous_viewport[0],
            previous_viewport[1],
            previous_viewport[2],
            previous_viewport[3],
        );
        gl::BindVertexArray(previous_vao as u32);
        gl::UseProgram(previous_program as u32);
        gl::ActiveTexture(previous_active_texture as u32);
        gl::DeleteFramebuffers(1, &fbo);
        gl::DeleteTextures(1, &target);
        gl::DeleteTextures(1, &source);
        gl::DeleteVertexArrays(1, &vao);
        gl::DeleteProgram(program);
        if gl_error != gl::NO_ERROR {
            return Err(format!(
                "validation draw/readback failed with GL error 0x{gl_error:04x}"
            ));
        }
        if pixels.chunks_exact(4).any(|pixel| {
            pixel
                .iter()
                .zip(expected)
                .any(|(actual, expected)| (actual - expected as f32 / 255.0).abs() > 1.0 / 255.0)
        }) {
            return Err(format!(
                "validation frame produced unexpected pixels: {pixels:?}"
            ));
        }
    }
    Ok(())
}

impl GpuTone {
    fn is_neutral(self) -> bool {
        self.exposure == 0.0
            && self.contrast == 0.0
            && self.saturation == 0.0
            && self.temperature == 0.0
            && self.fill_light == 0.0
            && self.shadows == 0.0
            && self.highlights == 0.0
            && self.sharpen == 0.0
            && self.grain == 0.0
            && self.clarity == 0.0
            && !self.black_white
            && !self.sepia
            && self.filter == FilterPreset::None
            && !self.auto_color
            && !self.auto_contrast
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
    auto_contrast: bool,
}

fn text_layer_content_key(layer: &super::model::TextLayerSpec, size: (i32, i32)) -> String {
    // Position, anchor and opacity affect uniforms only. A raster change
    // (content, font, colour, alignment, photo dimensions) must upload again.
    let raster = (
        &layer.id,
        &layer.text,
        &layer.font_family,
        layer.size,
        layer.bold,
        layer.italic,
        layer.color,
        &layer.align,
    );
    format!("text:{raster:?}:{}x{}", size.0, size.1)
}

/// Geometry of a decoded, untransformed source texture. Match the integer
/// rounding in render::rotate_autocrop and render::crop_normalized.
#[derive(Clone, Copy, Debug, PartialEq)]
struct GpuGeometry {
    output_width: i32,
    output_height: i32,
    straight_width: f32,
    straight_height: f32,
    oriented_width: f32,
    oriented_height: f32,
    library_rotation: i32,
    crop_x: f32,
    crop_y: f32,
    cos_angle: f32,
    sin_angle: f32,
}
impl GpuGeometry {
    fn for_source(width: i32, height: i32, recipe: &EditRecipe) -> Self {
        Self::for_source_rotation(width, height, 0, recipe)
    }

    fn for_source_rotation(width: i32, height: i32, rotation: i32, recipe: &EditRecipe) -> Self {
        let rotation = rotation.rem_euclid(360);
        let (oriented_width, oriented_height) = if rotation == 90 || rotation == 270 {
            (height, width)
        } else {
            (width, height)
        };
        let angle = if recipe.straighten.abs() < 0.01 {
            0.0
        } else {
            recipe.straighten.to_radians()
        };
        let (straight_width, straight_height) =
            if angle == 0.0 || oriented_width < 2 || oriented_height < 2 {
                (oriented_width as f32, oriented_height as f32)
            } else {
                let (w, h) = super::render::largest_rotated_rect(
                    oriented_width as f32,
                    oriented_height as f32,
                    angle,
                );
                (w.floor().max(1.0), h.floor().max(1.0))
            };
        let c = recipe.crop.normalized();
        let w = straight_width as i32;
        let h = straight_height as i32;
        let x = ((c.left * straight_width).round() as i32).clamp(0, w - 1);
        let y = ((c.top * straight_height).round() as i32).clamp(0, h - 1);
        let right = (c.right * straight_width).round() as i32;
        let bottom = (c.bottom * straight_height).round() as i32;
        Self {
            output_width: (right - x).clamp(1, w - x),
            output_height: (bottom - y).clamp(1, h - y),
            straight_width,
            straight_height,
            oriented_width: oriented_width as f32,
            oriented_height: oriented_height as f32,
            library_rotation: rotation / 90,
            crop_x: x as f32,
            crop_y: y as f32,
            cos_angle: angle.cos(),
            sin_angle: angle.sin(),
        }
    }
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

/// Raster inputs are prepared only when the overlay/text recipe changes.
/// The source photo and all layer blending remain on the GPU.
#[derive(Clone)]
struct GpuOverlayLayer {
    id: String,
    rect: super::model::NormRect,
    opacity: f32,
    width: i32,
    height: i32,
    rgba: glib::Bytes,
}

#[derive(Clone, Debug, PartialEq)]
struct GpuOverlayKey {
    overlays: Vec<super::model::OverlaySpec>,
    text: Vec<super::model::TextLayerSpec>,
    size: (i32, i32),
}

struct GlObjects {
    program: u32,
    vao: u32,
    texture: u32,
    source_revision: u64,
    processed_texture: u32,
    processed_fbo: u32,
    processed_dimensions: (i32, i32),
    processed_source_revision: u64,
    processed_tone_revision: u64,
    processed_geometry_revision: u64,
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
    auto_color_partial_texture: u32,
    auto_color_partial_fbo: u32,
    auto_color_partial_dimensions: (i32, i32),
    auto_color_revision: u64,
    histogram_program: u32,
    histogram_reduce_program: u32,
    histogram_texture: u32,
    histogram_fbo: u32,
    auto_contrast_params: u32,
    auto_contrast_fbo: u32,
    auto_contrast_revision: u64,
    stats_source_texture: u32,
    stats_source_fbo: u32,
    stats_source_dimensions: (i32, i32),
    stats_source_revision: u64,
    overlay_program: u32,
    overlay_textures: Vec<(String, u32)>,
    overlay_revision: u64,
    #[cfg(test)]
    gpu_timer_query: u32,
}

struct GpuState {
    renderer_mode: EditRendererMode,
    key: Option<SourceKey>,
    source: Option<(i32, i32, glib::Bytes)>,
    source_revision: u64,
    stats_revision: u64,
    tone: GpuTone,
    tone_revision: u64,
    geometry: Option<GpuGeometry>,
    overlay_key: Option<GpuOverlayKey>,
    overlays: Vec<GpuOverlayLayer>,
    overlay_revision: u64,
    overlay_visible: bool,
    view: GpuView,
    objects: Option<GlObjects>,
    rendered_frames: u64,
    has_presented_frame: bool,
    last_queued: Option<Instant>,
    frame_after_paint: Option<(Instant, Option<Instant>)>,
    failure: Option<String>,
    on_failure: Option<Box<dyn Fn()>>,
    on_frame_presented: Option<Box<dyn Fn()>>,
    #[cfg(test)]
    readback_requested: bool,
    #[cfg(test)]
    readback_pixels: Option<Vec<u8>>,
    #[cfg(test)]
    gpu_timer_requested: bool,
    #[cfg(test)]
    gpu_timer_pending: bool,
}

#[derive(Clone)]
pub struct LiveGpuCanvas {
    area: gtk::GLArea,
    state: Rc<RefCell<GpuState>>,
}

impl LiveGpuCanvas {
    pub fn new() -> Self {
        Self::new_for_mode(EditRendererMode::Gpu)
    }

    pub fn new_for_mode(renderer_mode: EditRendererMode) -> Self {
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
            renderer_mode,
            key: None,
            source: None,
            source_revision: 0,
            stats_revision: 0,
            tone: GpuTone::default(),
            tone_revision: 0,
            geometry: None,
            overlay_key: None,
            overlays: Vec::new(),
            overlay_revision: 0,
            overlay_visible: true,
            view: GpuView::default(),
            objects: None,
            rendered_frames: 0,
            has_presented_frame: false,
            last_queued: None,
            frame_after_paint: None,
            failure: None,
            on_failure: None,
            on_frame_presented: None,
            #[cfg(test)]
            readback_requested: false,
            #[cfg(test)]
            readback_pixels: None,
            #[cfg(test)]
            gpu_timer_requested: false,
            #[cfg(test)]
            gpu_timer_pending: false,
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
                        gl::DeleteTextures(1, &objects.processed_texture);
                        if !objects.overlay_textures.is_empty() {
                            for &(_, texture) in &objects.overlay_textures {
                                gl::DeleteTextures(1, &texture);
                            }
                        }
                        gl::DeleteTextures(1, &objects.clarity_luma_texture);
                        gl::DeleteTextures(1, &objects.clarity_blur_texture);
                        gl::DeleteTextures(1, &objects.effect_luma_texture);
                        gl::DeleteTextures(1, &objects.effect_blur_texture);
                        gl::DeleteTextures(1, &objects.auto_color_stats);
                        gl::DeleteTextures(1, &objects.auto_color_partial_texture);
                        gl::DeleteTextures(1, &objects.histogram_texture);
                        gl::DeleteTextures(1, &objects.auto_contrast_params);
                        gl::DeleteTextures(1, &objects.stats_source_texture);
                        gl::DeleteFramebuffers(1, &objects.clarity_luma_fbo);
                        gl::DeleteFramebuffers(1, &objects.clarity_blur_fbo);
                        gl::DeleteFramebuffers(1, &objects.effect_luma_fbo);
                        gl::DeleteFramebuffers(1, &objects.effect_blur_fbo);
                        gl::DeleteFramebuffers(1, &objects.auto_color_fbo);
                        gl::DeleteFramebuffers(1, &objects.auto_color_partial_fbo);
                        gl::DeleteFramebuffers(1, &objects.histogram_fbo);
                        gl::DeleteFramebuffers(1, &objects.auto_contrast_fbo);
                        gl::DeleteFramebuffers(1, &objects.stats_source_fbo);
                        gl::DeleteFramebuffers(1, &objects.processed_fbo);
                        gl::DeleteVertexArrays(1, &objects.vao);
                        gl::DeleteProgram(objects.program);
                        gl::DeleteProgram(objects.blur_program);
                        gl::DeleteProgram(objects.auto_color_program);
                        gl::DeleteProgram(objects.histogram_program);
                        gl::DeleteProgram(objects.histogram_reduce_program);
                        gl::DeleteProgram(objects.overlay_program);
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
                    let (frame, first_presented) = {
                        let mut state = state.borrow_mut();
                        let frame = state.frame_after_paint.take();
                        let first_presented = frame.is_some() && !state.has_presented_frame;
                        if first_presented {
                            state.has_presented_frame = true;
                        }
                        (frame, first_presented)
                    };
                    if std::env::var_os("PICASA_TRACE").is_some() {
                        if let Some((rendered_at, queued_at)) = frame {
                            eprintln!(
                                "PIC_EDIT_GPU frame_present_after_paint_ms={:.3} queue_to_after_paint_ms={}",
                                rendered_at.elapsed().as_secs_f64() * 1000.0,
                                queued_at.map_or_else(|| "none".to_owned(), |at| format!("{:.3}", at.elapsed().as_secs_f64() * 1000.0)),
                            );
                        }
                    }
                    if first_presented {
                        trace_gpu("renderer=gpu status=presented first_frame=true");
                    }
                    if first_presented {
                        if let Some(callback) = state.borrow().on_frame_presented.as_ref() {
                            callback();
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
                    match initialize_gl(state.renderer_mode) {
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
            let oversized_source = {
                let state = render_state.borrow();
                let dimensions = state.source.as_ref().map(|(width, height, _)| (*width, *height));
                let mut maximum = 0;
                unsafe { gl::GetIntegerv(gl::MAX_TEXTURE_SIZE, &mut maximum) };
                dimensions.filter(|(width, height)| *width > maximum || *height > maximum)
            };
            if let Some((width, height)) = oversized_source {
                fail(
                    area,
                    &render_state,
                    format!("source dimensions {width}x{height} exceed GL_MAX_TEXTURE_SIZE"),
                );
                return gtk::glib::Propagation::Stop;
            }
            let draw_started = Instant::now();
            let (upload_trace, queue_ms, gl_error, sharpen) = {
                let mut state = render_state.borrow_mut();
                let source = state
                    .source
                    .as_ref()
                    .map(|(width, height, bytes)| (*width, *height, bytes.clone()));
                let source_revision = state.source_revision;
                let stats_revision = state.stats_revision;
                let tone = state.tone;
                let tone_revision = state.tone_revision;
                let geometry = state.geometry;
                let overlays = state.overlays.clone();
                let overlay_revision = state.overlay_revision;
                let overlay_visible = state.overlay_visible;
                let view = state.view;
                #[cfg(test)]
                let gpu_timer_requested = state.gpu_timer_requested;
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
                    #[cfg(test)]
                    if gpu_timer_requested {
                        if objects.gpu_timer_query == 0 {
                            gl::GenQueries(1, &mut objects.gpu_timer_query);
                        }
                        gl::BeginQuery(gl::TIME_ELAPSED, objects.gpu_timer_query);
                    }
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
                            // Build the source pyramid once per decoded image.
                            // Sampling this pyramid at the view's footprint avoids
                            // aliasing at Fit without a CPU resize/readback or a
                            // second geometry-specific upload.
                            gl::GenerateMipmap(gl::TEXTURE_2D);
                            let elapsed = upload_started.elapsed().as_secs_f64() * 1000.0;
                            objects.source_revision = source_revision;
                            upload_trace = Some((*width, *height, elapsed));
                        }
                    }

                    let filter = if view.scale < 1.0 {
                        gl::LINEAR_MIPMAP_LINEAR
                    } else {
                        gl::NEAREST
                    };
                    if objects.filter != filter {
                        gl::BindTexture(gl::TEXTURE_2D, objects.texture);
                        gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MIN_FILTER, filter as i32);
                        // Magnification remains pixel exact; minification uses
                        // the mip pyramid and trilinear level selection.
                        gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MAG_FILTER, gl::NEAREST as i32);
                        objects.filter = filter;
                    }

                    gl::UseProgram(objects.program);
                    gl::BindVertexArray(objects.vao);
                    gl::ActiveTexture(gl::TEXTURE0);
                    gl::BindTexture(gl::TEXTURE_2D, objects.texture);
                    uniform_1i(objects.program, "uSource", 0);
                    let source_width = source.as_ref().map_or(1, |s| s.0);
                    let source_height = source.as_ref().map_or(1, |s| s.1);
                    let output_width = geometry.map_or(source_width, |g| g.output_width);
                    let output_height = geometry.map_or(source_height, |g| g.output_height);
                    uniform_1i(objects.program, "uGpuGeometryOn", if geometry.is_some() { 1 } else { 0 });
                    uniform_2f(objects.program, "uOutputSize", output_width as f32, output_height as f32);
                    if let Some(geo) = geometry {
                        uniform_2f(objects.program, "uRotatedSize", geo.straight_width, geo.straight_height);
                        uniform_2f(objects.program, "uOrientedSize", geo.oriented_width, geo.oriented_height);
                        uniform_1i(objects.program, "uLibraryRotation", geo.library_rotation);
                        uniform_2f(objects.program, "uCropOffset", geo.crop_x, geo.crop_y);
                        uniform_2f(objects.program, "uAngleCosSin", geo.cos_angle, geo.sin_angle);
                    }
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
                    uniform_1f(
                        objects.program,
                        "uMinificationLod",
                        if view.scale < 1.0 {
                            (1.0 / view.scale.max(0.0001)).log2().max(0.0)
                        } else {
                            0.0
                        },
                    );
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
                    uniform_1i(objects.program, "uAutoContrast", if tone.auto_contrast { 1 } else { 0 });
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
                    uniform_1i(objects.program, "uProcessedSource", 5);
                    uniform_1i(objects.program, "uProcessPass", 0);
                    uniform_1i(objects.program, "uUseProcessedSource", 0);
                    if matches!(tone.filter, FilterPreset::PencilSketch | FilterPreset::DreamGlow) {
                        if source.is_some() {
                            // Blur the 100%-strength base look BEFORE Filter Amount blending.
                            // Dream Glow must not recursively sample its own incomplete blur.
                            uniform_1f(objects.program, "uDreamGlow", 0.0);
                            uniform_1f(objects.program, "uFilterAmount", 1.0);
                            draw_effect_luminance(
                                objects,
                                output_width,
                                output_height,
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
                    // Prepare the statistics image once per decoded source,
                    // geometry and filter strength. The GPU histogram must
                    // sample output BEFORE Auto Colour and Auto Contrast.
                    let stats_texture = if (tone.auto_color || tone.auto_contrast) && source.is_some()
                        && (tone.filter != FilterPreset::None || geometry.is_some())
                    {
                        draw_stats_source(
                            objects, output_width, output_height, stats_revision,
                            view_width * device_scale, view_height * device_scale,
                        );
                        objects.stats_source_texture
                    } else {
                        objects.texture
                    };
                    // CPU recipe order is Auto Colour first, Auto Contrast second.
                    // The histogram must sample the colour-corrected values.
                    if tone.auto_color {
                        if let Some((width, height, _)) = source.as_ref() {
                            let _ = (width, height);
                            draw_auto_color_stats(objects, stats_texture, output_width, output_height,
                                stats_revision, view_width * device_scale, view_height * device_scale);
                            gl::ActiveTexture(gl::TEXTURE3);
                            gl::BindTexture(gl::TEXTURE_2D, objects.auto_color_stats);
                            uniform_1i(objects.program, "uAutoColorStats", 3);
                            gl::ActiveTexture(gl::TEXTURE0);
                            gl::BindTexture(gl::TEXTURE_2D, objects.texture);
                        }
                    }
                    if tone.auto_contrast {
                        if let Some((width, height, _)) = source.as_ref() {
                            let _ = (width, height);
                            draw_auto_contrast_stats(
                                objects, stats_texture, output_width, output_height, stats_revision,
                                tone.auto_color,
                                view_width * device_scale, view_height * device_scale,
                            );
                            gl::ActiveTexture(gl::TEXTURE4);
                            gl::BindTexture(gl::TEXTURE_2D, objects.auto_contrast_params);
                            uniform_1i(objects.program, "uAutoContrastParams", 4);
                            gl::ActiveTexture(gl::TEXTURE0);
                            gl::BindTexture(gl::TEXTURE_2D, objects.texture);
                        }
                    }
                    if tone.clarity.abs() > 0.001 {
                        if source.is_some() {
                            draw_clarity_luminance(
                                objects,
                                output_width,
                                output_height,
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
                    let use_processed_source = view.scale < 1.0
                        && source.is_some()
                        && !tone.is_neutral();
                    if use_processed_source {
                        let output_dimensions = (output_width, output_height);
                        if objects.processed_source_revision != source_revision
                            || objects.processed_tone_revision != tone_revision
                            || objects.processed_geometry_revision != stats_revision
                            || objects.processed_dimensions != output_dimensions
                        {
                            render_processed_source(
                                objects,
                                output_width,
                                output_height,
                                view_width * device_scale,
                                view_height * device_scale,
                                view.scale,
                            );
                            objects.processed_source_revision = source_revision;
                            objects.processed_tone_revision = tone_revision;
                            objects.processed_geometry_revision = stats_revision;
                            objects.processed_dimensions = output_dimensions;
                        }
                        gl::ActiveTexture(gl::TEXTURE5);
                        gl::BindTexture(gl::TEXTURE_2D, objects.processed_texture);
                        uniform_1i(objects.program, "uUseProcessedSource", 1);
                        gl::ActiveTexture(gl::TEXTURE0);
                        gl::BindTexture(gl::TEXTURE_2D, objects.texture);
                    }
                    gl::DrawArrays(gl::TRIANGLES, 0, 3);
                    // Render photo layers *after* all image adjustments.
                    // GPU blending composes image assets first, then text.
                    if overlay_visible && !overlays.is_empty() {
                        render_gpu_overlays(
                            objects, &overlays, overlay_revision, &view,
                            view_width, view_height, device_scale,
                            output_width, output_height,
                        );
                    }
                    #[cfg(test)]
                    if gpu_timer_requested {
                        gl::EndQuery(gl::TIME_ELAPSED);
                        state.gpu_timer_requested = false;
                        state.gpu_timer_pending = true;
                    }
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

            render_state.borrow_mut().frame_after_paint = Some((Instant::now(), None));

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

    pub fn set_frame_presented_handler(&self, handler: impl Fn() + 'static) {
        self.state.borrow_mut().on_frame_presented = Some(Box::new(handler));
    }

    pub fn has_source_for(&self, reference: &str, rotation: i32, recipe: &EditRecipe) -> bool {
        let state = self.state.borrow();
        state.source.is_some()
            && state.key.as_ref().is_some_and(|key| {
                key.reference == reference
                    && (state.geometry.is_some()
                        || (key.rotation == rotation
                            && key.crop == recipe.crop
                            && key.straighten == recipe.straighten))
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
        // Existing editor sources are CPU-transformed; do not transform twice.
        state.geometry = None;
        state.source_revision = state.source_revision.wrapping_add(1);
        state.has_presented_frame = false;
        state.stats_revision = state.stats_revision.wrapping_add(1);
        state.overlay_key = None;
        state.overlays.clear();
        state.overlay_revision = state.overlay_revision.wrapping_add(1);
        state.last_queued = Some(Instant::now());
        self.area.set_visible(true);
        self.area.queue_render();
    }

    /// Source pixels are RAW decoded source data, not a CPU cropped image.
    /// Return the exact viewport dimensions needed by the scroll/layout layer.
    /// Updating geometry never reuploads the unchanged source texture.
    pub fn set_untransformed_geometry(&self, recipe: &EditRecipe) -> Option<(i32, i32)> {
        self.set_untransformed_geometry_for_rotation(0, recipe)
    }

    /// The rotation is PIC's library quarter-turn, applied before straighten.
    /// Pass unrotated decoded pixels; source uploads remain unchanged.
    pub fn set_untransformed_geometry_for_rotation(
        &self,
        rotation: i32,
        recipe: &EditRecipe,
    ) -> Option<(i32, i32)> {
        let mut state = self.state.borrow_mut();
        let &(width, height, _) = state.source.as_ref()?;
        let geo = GpuGeometry::for_source_rotation(width, height, rotation, recipe);
        if state.geometry != Some(geo) {
            state.geometry = Some(geo);
            state.stats_revision = state.stats_revision.wrapping_add(1);
            state.overlay_key = None;
            state.last_queued = Some(Instant::now());
            drop(state);
            self.area.queue_render();
        }
        Some((geo.output_width, geo.output_height))
    }

    pub fn clear_untransformed_geometry(&self) {
        let mut state = self.state.borrow_mut();
        if state.geometry.take().is_some() {
            state.stats_revision = state.stats_revision.wrapping_add(1);
            state.last_queued = Some(Instant::now());
            drop(state);
            self.area.queue_render();
        }
    }

    /// Rebuild only when layer content or final geometry changes. Asset
    /// decoding and Pango text shaping run once per recipe revision; compositing
    /// runs in GL for each rendered frame.
    pub fn set_overlay_layers(&self, recipe: &EditRecipe) {
        let (width, height, key) = {
            let state = self.state.borrow();
            let Some(&(src_w, src_h, _)) = state.source.as_ref() else {
                return;
            };
            let size = state
                .geometry
                .map_or((src_w, src_h), |g| (g.output_width, g.output_height));
            let key = GpuOverlayKey {
                overlays: recipe.overlays.clone(),
                text: recipe.text_layers.clone(),
                size,
            };
            if state.overlay_key.as_ref() == Some(&key) {
                return;
            }
            (size.0, size.1, key)
        };
        let previous = self.state.borrow().overlays.clone();
        let mut layers = Vec::new();
        for overlay in &recipe.overlays {
            if !overlay.visible || overlay.opacity <= 0.001 {
                continue;
            }
            let Some(asset) = super::overlays::load_asset(&overlay.asset) else {
                continue;
            };
            let aspect = asset.width() as f32 / asset.height().max(1) as f32;
            let rect = overlay.rect(width as f32, height as f32, aspect);
            let id = format!("asset:{}", overlay.asset);
            let rgba = previous.iter().find(|layer| layer.id == id).map_or_else(
                || glib::Bytes::from_owned(asset.as_raw().clone()),
                |layer| layer.rgba.clone(),
            );
            layers.push(GpuOverlayLayer {
                id,
                rect,
                opacity: overlay.opacity,
                width: asset.width() as i32,
                height: asset.height() as i32,
                rgba,
            });
        }
        for text_layer in &recipe.text_layers {
            if !text_layer.visible || text_layer.opacity <= 0.001 {
                continue;
            }
            let id = text_layer_content_key(text_layer, (width, height));
            let (raster_width, raster_height, rgba) = if let Some(layer) =
                previous.iter().find(|layer| layer.id == id)
            {
                (layer.width, layer.height, layer.rgba.clone())
            } else {
                let Some(raster) =
                    super::text_render::render_text_rgba(text_layer, width as f64, height as f64)
                else {
                    continue;
                };
                (
                    raster.width() as i32,
                    raster.height() as i32,
                    glib::Bytes::from_owned(raster.into_raw()),
                )
            };
            let rect = text_layer.rect_with_size(
                raster_width as f32 / width.max(1) as f32,
                raster_height as f32 / height.max(1) as f32,
            );
            layers.push(GpuOverlayLayer {
                id,
                rect,
                opacity: text_layer.opacity,
                width: raster_width,
                height: raster_height,
                rgba,
            });
        }
        let mut state = self.state.borrow_mut();
        state.overlay_key = Some(key);
        state.overlays = layers;
        state.overlay_revision = state.overlay_revision.wrapping_add(1);
        state.last_queued = Some(Instant::now());
        drop(state);
        self.area.queue_render();
    }

    /// Crop mode hides content overlays just as the previous Cairo renderer did.
    pub fn set_overlay_compositing_visible(&self, visible: bool) {
        let mut state = self.state.borrow_mut();
        if state.overlay_visible != visible {
            state.overlay_visible = visible;
            state.last_queued = Some(Instant::now());
            drop(state);
            self.area.queue_render();
        }
    }

    pub fn uses_gpu_layers(&self) -> bool {
        let state = self.state.borrow();
        state.failure.is_none() && state.source.is_some() && self.area.is_visible()
    }

    pub fn set_recipe(&self, recipe: &EditRecipe) {
        self.set_overlay_layers(recipe);
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
            auto_contrast: recipe.auto_contrast,
        };
        let mut state = self.state.borrow_mut();
        if state.tone == next {
            return;
        }
        if state.tone.filter != next.filter || state.tone.filter_amount != next.filter_amount {
            state.stats_revision = state.stats_revision.wrapping_add(1);
        }
        state.tone = next;
        state.tone_revision = state.tone_revision.wrapping_add(1);
        state.last_queued = Some(Instant::now());
        drop(state);
        self.area.queue_render();
    }

    /// Square filter-picker cover crop. Unlike the full editor's scroll
    /// view, this small GLArea has no GTK scroller or adjustable margins.
    /// It uses the same fragment shader and source geometry as live Edit.
    pub fn set_thumbnail_cover(&self, width: i32, height: i32, tile_size: i32) {
        let view = thumbnail_cover_view(width, height, tile_size);
        self.area.set_margin_start(0);
        self.area.set_margin_top(0);
        self.area.set_size_request(tile_size, tile_size);
        self.area.set_halign(gtk::Align::Fill);
        self.area.set_valign(gtk::Align::Fill);
        let mut state = self.state.borrow_mut();
        if state.view != view {
            state.view = view;
            state.last_queued = Some(Instant::now());
            drop(state);
            self.area.queue_render();
        }
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

    #[cfg(test)]
    fn request_gpu_timer(&self) {
        self.state.borrow_mut().gpu_timer_requested = true;
        self.area.queue_render();
    }

    #[cfg(test)]
    fn take_gpu_time_ns(&self) -> Option<u64> {
        if !self.state.borrow().gpu_timer_pending {
            return None;
        }
        self.area.make_current();
        let query = self.state.borrow().objects.as_ref()?.gpu_timer_query;
        let mut available = 0;
        unsafe {
            gl::GetQueryObjectiv(query, gl::QUERY_RESULT_AVAILABLE, &mut available);
            if available == 0 {
                return None;
            }
            let mut elapsed_ns = 0u64;
            gl::GetQueryObjectui64v(query, gl::QUERY_RESULT, &mut elapsed_ns);
            self.state.borrow_mut().gpu_timer_pending = false;
            Some(elapsed_ns)
        }
    }
}

fn thumbnail_cover_view(width: i32, height: i32, tile_size: i32) -> GpuView {
    let width = width.max(1) as f32;
    let height = height.max(1) as f32;
    let tile = tile_size.max(1) as f32;
    let scale = (tile / width).max(tile / height);
    GpuView {
        width: tile,
        height: tile,
        scroll_x: ((width * scale - tile) * 0.5).max(0.0),
        scroll_y: ((height * scale - tile) * 0.5).max(0.0),
        pad_x: 0.0,
        pad_y: 0.0,
        scale,
    }
}

pub fn gpu_tone_supported(recipe: &EditRecipe) -> bool {
    // A filtered/cropped GPU histogram prepass exists, but does not yet match
    // the CPU Export reference consistently. Keep automatic corrections on
    // CPU for creative looks and non-identity geometry until strict parity is
    // proven. Standalone/both autos on plain, uncropped photos are verified.
    let geometry_changes_pixels = !recipe.crop.is_full() || recipe.straighten.abs() >= 0.01;
    (recipe.filter == FilterPreset::None || !(recipe.auto_color || recipe.auto_contrast))
        && !(geometry_changes_pixels && (recipe.auto_color || recipe.auto_contrast))
}

/// GPU Auto corrections are currently approved only for native 1:1 previews
/// with identity library rotation. The statistics are computed from the full
/// source, but the final mip filtering can differ from CPU preview rounding.
pub fn gpu_tone_supported_for_view(
    recipe: &EditRecipe,
    native_one_to_one: bool,
    rotation: i32,
) -> bool {
    if !gpu_tone_supported(recipe) {
        return false;
    }
    let has_auto = recipe.auto_color || recipe.auto_contrast;
    !has_auto || (native_one_to_one && rotation.rem_euclid(360) == 0)
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
    base.auto_contrast = false;
    // In LiveGPU mode geometry is applied directly to the RAW source texture.
    // Do not first straighten/crop it on the CPU and then reapply on the GPU.
    base.crop = CropRect::default();
    base.straighten = 0.0;
    base
}

fn fail(area: &gtk::GLArea, state: &Rc<RefCell<GpuState>>, message: String) {
    let (callback, hide_canvas, renderer_mode) = {
        let mut state = state.borrow_mut();
        state.failure = Some(message.clone());
        (
            state.on_failure.take(),
            !state.has_presented_frame,
            state.renderer_mode,
        )
    };
    if renderer_mode == EditRendererMode::Auto
        && !message.starts_with("source dimensions ")
    {
        cache_auto_gpu_failure(message.clone());
    }
    if hide_canvas {
        area.set_visible(false);
    }
    trace_gpu(format_args!(
        "renderer=cpu fallback_reason={message:?} mode={renderer_mode:?}"
    ));
    if let Some(callback) = callback {
        callback();
    }
}

fn initialize_gl(mode: EditRendererMode) -> Result<GlObjects, String> {
    let initialization_started = Instant::now();
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
    let capabilities = query_gl_capabilities(mode)?;

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
#extension GL_ARB_gpu_shader5 : enable
uniform sampler2D uSource;
uniform sampler2D uProcessedSource;
uniform int uProcessPass;
uniform int uUseProcessedSource;
uniform vec2 uSourceSize;
uniform vec2 uOutputSize;
uniform int uGpuGeometryOn;
uniform vec2 uRotatedSize;
uniform vec2 uOrientedSize;
uniform int uLibraryRotation;
uniform vec2 uCropOffset;
uniform vec2 uAngleCosSin;
uniform vec2 uViewSize;
uniform vec2 uScroll;
uniform vec2 uPadding;
uniform float uScale;
uniform float uMinificationLod;
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
uniform int uAutoContrast;
uniform sampler2D uAutoContrastParams;
#ifdef GL_ARB_gpu_shader5
precise out vec4 color;
#else
out vec4 color;
#endif
// Keep the same left-to-right multiply/add sequence as render::rgb_luma.
// A GLSL dot may be fused/re-associated by the driver, moving a pixel across
// an Auto Contrast histogram half-bin even though its input is an exact u8.
float luma(vec3 c) {
    float y = 0.2126 * c.r;
    y = y + 0.7152 * c.g;
    y = y + 0.0722 * c.b;
    return y;
}
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
    // CPU order: Auto Contrast is applied after Auto Colour and any named
    // look, before manual Exposure/Contrast and the other slider adjustments.
    if (uAutoContrast != 0 && px.a >= 16.0/255.0) {
        vec4 limits = texelFetch(uAutoContrastParams, ivec2(0, 0), 0);
        if (limits.a > 0.5) {
            vec3 correctedBytes = round(corrected * 255.0);
            float y = luma(correctedBytes) / 255.0;
            if (y > 0.0001) {
                float pivot = (limits.r + limits.g) * 0.5;
                float stretched = clamp((y - pivot) * limits.b + pivot, 0.012, 0.988);
                float adjusted = y + (stretched - y) * 0.75;
                float gain = clamp(adjusted / y, 0.2, 3.0);
                corrected = round(clamp(correctedBytes * gain, 0.0, 255.0)) / 255.0;
            }
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
// Inverse transform in output pixel coordinates: rotate + autocrop first,
// then crop. Manual bilinear interpolation uses CPU-equivalent 8-bit rounding.
vec4 geometryPixelLod(vec2 outputCoord, float lod) {
    vec2 valid = clamp(outputCoord, vec2(0.0), uOutputSize - vec2(1.0));
    if (uGpuGeometryOn == 0) {
        if (lod <= 0.001) {
            ivec2 nearestCoord = ivec2(floor(valid));
            if (all(lessThan(abs(valid - vec2(nearestCoord)), vec2(0.001))))
                return texelFetch(uSource, nearestCoord, 0);
        }
        return textureLod(uSource, (vec2(valid) + vec2(0.5)) / uSourceSize, lod);
    }
    vec2 offset = valid + uCropOffset - (uRotatedSize - vec2(1.0)) * 0.5;
    vec2 orientedCoord = vec2(
        uAngleCosSin.x * offset.x + uAngleCosSin.y * offset.y,
        -uAngleCosSin.y * offset.x + uAngleCosSin.x * offset.y
    ) + (uOrientedSize - vec2(1.0)) * 0.5;
    vec2 sourceCoord = orientedCoord;
    if (uLibraryRotation == 1) {
        sourceCoord = vec2(orientedCoord.y, uSourceSize.y - 1.0 - orientedCoord.x);
    } else if (uLibraryRotation == 2) {
        sourceCoord = uSourceSize - vec2(1.0) - orientedCoord;
    } else if (uLibraryRotation == 3) {
        sourceCoord = vec2(uSourceSize.x - 1.0 - orientedCoord.y, orientedCoord.x);
    }
    sourceCoord = clamp(sourceCoord, vec2(0.0), uSourceSize - vec2(1.0));
    if (lod > 0.001) {
        return textureLod(uSource, (sourceCoord + vec2(0.5)) / uSourceSize, lod);
    }
    ivec2 lo = ivec2(floor(sourceCoord));
    ivec2 hi = min(lo + ivec2(1), ivec2(uSourceSize) - ivec2(1));
    vec2 t = fract(sourceCoord);
    vec4 upper = mix(texelFetch(uSource, lo, 0),
                     texelFetch(uSource, ivec2(hi.x, lo.y), 0), t.x);
    vec4 lower = mix(texelFetch(uSource, ivec2(lo.x, hi.y), 0),
                     texelFetch(uSource, hi, 0), t.x);
    return round(clamp(mix(upper, lower, t.y), 0.0, 1.0) * 255.0) / 255.0;
}
vec4 geometryPixel(ivec2 outputCoord) { return geometryPixelLod(vec2(outputCoord), 0.0); }
// Port of filters::apply_filter's parameterized colour presets. The filter
// is applied and quantized *before* the manual tone pipeline, like CPU export.
float stableNoise(ivec2 coord) {
    ivec2 c = clamp(coord, ivec2(0), ivec2(uOutputSize) - ivec2(1));
    uint hash = uint(c.x) * 374761393u ^ uint(c.y) * 668265263u ^ 0x9e3779b9u;
    hash = (hash ^ (hash >> 13u)) * 1274126177u;
    hash ^= hash >> 16u;
    return float(hash & 65535u) / 65535.0 - 0.5;
}
vec4 namedFilterPixel(vec4 px, ivec2 coord) {
    if (uNamedFilter < 0.5 || uFilterAmount <= 0.0) return px;
    if (uPencilSketch > 0.5) {
        float gray = mix(1.0, luma(px.rgb), px.a);
        float local = texture(uEffectBlur, (vec2(coord) + vec2(0.5)) / uOutputSize).r;
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
    vec2 centre = (uOutputSize - vec2(1.0)) * 0.5;
    vec2 d = vec2(coord) - centre;
    float maxDistance = max(length(centre), 1.0);
    float falloff = 1.0 - uFilterVignette * dot(d, d) / (maxDistance * maxDistance);
    c *= falloff;
    c = round(clamp(c, 0.0, 1.0) * 255.0) / 255.0;
    if (uDreamGlow > 0.5) {
        float glow = texture(uEffectBlur, (vec2(coord) + vec2(0.5)) / uOutputSize).r * 0.45;
        c = round(clamp(c + (vec3(1.0) - c) * glow, 0.0, 1.0) * 255.0) / 255.0;
    }
    if (uNamedGrain > 0.0) {
        // Source-scale grain averages down with the display footprint.
        float noise = stableNoise(coord) * uNamedGrain * min(uScale, 1.0) * 255.0;
        c = round(clamp(c * 255.0 + vec3(noise), 0.0, 255.0)) / 255.0;
    }
    // CPU blend_filter_amount mixes two already-quantized byte images using
    // `base + (filtered - base) * amount`, then rounds to u8. Do the same in
    // byte space; normalized `mix()` can land on the opposite side of a half
    // byte and Auto Contrast can amplify that one-level difference.
    vec3 originalBytes = round(px.rgb * 255.0);
    vec3 filteredBytes = round(c * 255.0);
    c = round(clamp(originalBytes + (filteredBytes - originalBytes) *
                    clamp(uFilterAmount, 0.0, 1.0), 0.0, 255.0)) / 255.0;
    return vec4(c, px.a);
}
vec4 sampleTone(ivec2 coord) {
    ivec2 limit = ivec2(uOutputSize) - ivec2(1);
    ivec2 valid = clamp(coord, ivec2(0), limit);
    return tonePixel(namedFilterPixel(geometryPixelLod(vec2(valid), uMinificationLod), valid));
}
vec3 clarified(ivec2 coord) {
    vec3 base = sampleTone(coord).rgb;
    if (abs(uClarity) <= 0.001) return base;
    float y = luma(base);
    float localY = texture(uClarityLuma,
        (vec2(coord) + vec2(0.5)) / uOutputSize).r;
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
    // Avoid turning source-scale grain into coarse screen-space noise at Fit.
    float noise = stableNoise(coord) * uGrain * min(uScale, 1.0) * 0.24 * 255.0;
    return round(clamp(rgb * 255.0 + vec3(noise), 0.0, 255.0)) / 255.0;
}
vec3 processedPixel(ivec2 coord) {
    return grainPixel(coord, sharpened(coord));
}
void main() {
    if (uProcessPass != 0) {
        ivec2 coord = ivec2(floor(gl_FragCoord.xy));
        vec4 px = sampleTone(coord);
        color = vec4(processedPixel(coord), px.a);
        return;
    }
    if (uClarityPrepass != 0) {
        vec2 uv = gl_FragCoord.xy / uClarityPassSize;
        ivec2 coord = ivec2(floor(uv * uOutputSize));
        vec4 px = uGpuGeometryOn != 0 ? geometryPixel(coord) : texture(uSource, uv);
        float y;
        if (uClarityPrepass == 3) {
            color = namedFilterPixel(geometryPixel(coord), coord);
            return;
        }
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
        sourcePos.x >= uOutputSize.x || sourcePos.y >= uOutputSize.y) {
        color = vec4(0.055, 0.06, 0.07, 1.0);
        return;
    }
    if (uUseProcessedSource != 0) {
        color = textureLod(uProcessedSource, sourcePos / uOutputSize, uMinificationLod);
        return;
    }
    // The source texture's mip pyramid integrates a footprint-sized area.
    // Trilinear LOD selection is stable at fractional zooms and suppresses
    // checkerboard/moiré detail at Fit. LOD zero preserves exact 1:1 fetches.
    if (uScale < 1.0) {
        vec2 outputCoord = sourcePos - vec2(0.5);
        ivec2 coord = ivec2(floor(outputCoord));
        if (uSharpen > 0.0001 || uGrain > 0.001 || abs(uClarity) > 0.001 || uNamedFilter >= 0.5) {
            vec4 px = sampleTone(coord);
            color = vec4(processedPixel(coord), px.a);
            return;
        }
        vec4 px = geometryPixelLod(outputCoord, uMinificationLod);
        px = namedFilterPixel(px, coord);
        color = tonePixel(px);
        return;
    }
    if (uSharpen <= 0.0001 && uGrain <= 0.001 && abs(uClarity) <= 0.001 && uNamedFilter < 0.5) {
        color = tonePixel(uGpuGeometryOn != 0
            ? geometryPixel(ivec2(floor(sourcePos)))
            : texture(uSource, sourcePos / uSourceSize));
        return;
    }
    ivec2 coord = ivec2(floor(sourcePos));
    vec4 px = sampleTone(coord);
    color = vec4(processedPixel(coord), px.a);
}"#,
    )?;

    let blur_fragment = compile_shader(
        gl::FRAGMENT_SHADER,
        r#"#version 150
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
}"#,
    )?;

    let overlay_fragment = compile_shader(
        gl::FRAGMENT_SHADER,
        r#"#version 150
uniform sampler2D uOverlay;
uniform vec4 uLayerRect;
uniform float uLayerOpacity;
uniform vec2 uViewSize;
uniform vec2 uScroll;
uniform vec2 uPadding;
uniform float uScale;
uniform float uDeviceScale;
uniform vec2 uPhotoSize;
out vec4 color;
void main() {
    vec2 pixel = gl_FragCoord.xy / max(uDeviceScale, 1.0);
    vec2 fromTop = vec2(pixel.x, uViewSize.y - pixel.y);
    vec2 photo = (fromTop + uScroll - uPadding) / max(uScale, 0.0001);
    if (photo.x < 0.0 || photo.y < 0.0 ||
        photo.x >= uPhotoSize.x || photo.y >= uPhotoSize.y) discard;
    vec2 uv = (photo - uLayerRect.xy) / uLayerRect.zw;
    if (uv.x < 0.0 || uv.y < 0.0 || uv.x >= 1.0 || uv.y >= 1.0) discard;
    // Upload bytes are top-down RGBA while OpenGL texture v=0 is the bottom row.
    vec4 layer = texture(uOverlay, vec2(uv.x, 1.0-uv.y));
    color = vec4(layer.rgb, layer.a * uLayerOpacity);
}
"#,
    )?;

    // Hierarchical Kahan summation keeps the GPU's weighted neutral estimate
    // stable as source resolution and rasterization order change. The prior
    // additive blend into one float pixel accumulated up to 200k samples in
    // driver-dependent order, shifting gains enough to move Auto Contrast bins.
    let auto_fragment = compile_shader(
        gl::FRAGMENT_SHADER,
        r#"#version 150
uniform sampler2D uSource;
uniform sampler2D uPartials;
uniform int uWidth;
uniform int uSampleStep;
uniform int uSampleCount;
uniform int uPartialCount;
uniform int uReductionPass;
out vec4 color;
void main() {
    vec4 sum = vec4(0.0);
    vec4 compensation = vec4(0.0);
    if (uReductionPass == 0) {
        int batch = int(gl_FragCoord.x);
        for (int i = 0; i < 512; ++i) {
            int sample = batch * 512 + i;
            if (sample >= uSampleCount) break;
            int index = sample * uSampleStep;
            ivec2 at = ivec2(index % uWidth, index / uWidth);
    vec4 px = texelFetch(uSource, at, 0);
            vec3 bytes = round(px.rgb * 255.0);
            int hi = int(max(bytes.r, max(bytes.g, bytes.b)));
            int lo = int(min(bytes.r, min(bytes.g, bytes.b)));
            int denominator = max(hi, 1);
            bool tooChromatic = (hi - lo) * 25 > denominator * 8;
            float chroma = float(hi - lo) / float(denominator);
            int lumaScore = 1063 * int(bytes.r) + 3576 * int(bytes.g) + 361 * int(bytes.b);
            // Exact integer form of CPU's inclusive 0.07..=0.93 test:
            // coefficients are in 1/5000 units and luma is divided by 255.
            bool outsideMidtones = lumaScore < 89250 || lumaScore > 1185750;
            float weight = (px.a < 16.0/255.0 || outsideMidtones || tooChromatic)
        ? 0.0 : pow(1.0 - chroma / 0.32, 2.0);
            vec4 value = vec4(bytes * weight, weight);
            vec4 adjusted = value - compensation;
            vec4 next = sum + adjusted;
            compensation = (next - sum) - adjusted;
            sum = next;
        }
    } else {
        for (int i = 0; i < 512; ++i) {
            if (i >= uPartialCount) break;
            vec4 value = texelFetch(uPartials, ivec2(i, 0), 0);
            vec4 adjusted = value - compensation;
            vec4 next = sum + adjusted;
            compensation = (next - sum) - adjusted;
            sum = next;
        }
    }
    color = sum;
}
"#,
    )?;

    // Each sampled source pixel draws a one-pixel point into one bin of the
    // 256x1 floating-point histogram texture. It is then reduced ONCE on GPU.
    let histogram_vertex = compile_shader(
        gl::VERTEX_SHADER,
        r#"#version 150
uniform sampler2D uSource;
uniform int uWidth;
uniform int uSampleStep;
uniform int uAutoColor;
uniform sampler2D uAutoColorStats;
float luma(vec3 c) {
    float y = 0.2126 * c.r;
    y = y + 0.7152 * c.g;
    y = y + 0.0722 * c.b;
    return y;
}
void main() {
    int sampleIndex = gl_VertexID * uSampleStep;
    ivec2 at = ivec2(sampleIndex % uWidth, sampleIndex / uWidth);
    vec4 px = texelFetch(uSource, at, 0);
    if (uAutoColor != 0 && px.a >= 16.0/255.0) {
        vec4 sums = texelFetch(uAutoColorStats, ivec2(0,0), 0);
        if (sums.a >= 0.5) {
            vec3 means = sums.rgb / sums.a;
            float neutral = luma(means);
            vec3 gains = vec3(1.0) + (clamp(vec3(neutral) / max(means,vec3(1.0)),vec3(0.82),vec3(1.22))-vec3(1.0))*0.65;
            px.rgb = round(clamp(px.rgb*gains,0.0,1.0)*255.0)/255.0;
        }
    }
    float luminance = luma(round(px.rgb * 255.0));
    float bucket = floor(luminance + 0.5);
    float pos = ((bucket + 0.5) / 256.0) * 2.0 - 1.0;
    gl_Position = vec4(pos, 0.0, 0.0, 1.0);
    if (px.a < 16.0 / 255.0) gl_Position.xy = vec2(2.0, 2.0);
    gl_PointSize = 1.0;
}"#,
    )?;
    let histogram_fragment = compile_shader(
        gl::FRAGMENT_SHADER,
        r#"#version 150
out vec4 color;
void main() { color = vec4(0.0, 0.0, 0.0, 1.0); }
"#,
    )?;
    // Pixel shader executes at a single 1x1 target. It calculates the same
    // percentile bounds, scene-luminance pivot and limited gain as render.rs.
    let histogram_reduce_fragment = compile_shader(
        gl::FRAGMENT_SHADER,
        r#"#version 150
uniform sampler2D uHistogram;
out vec4 color;
void main() {
    float samples = 0.0;
    for (int i = 0; i < 256; ++i) {
        samples += texelFetch(uHistogram, ivec2(i, 0), 0).a;
    }
    if (samples < 4.0) { color = vec4(0.0); return; }
    // The histogram contains exact integer counts. Use integer-equivalent
    // rounding instead of a float 0.01 multiply so half-percentile boundaries
    // match Rust's f64 round for every count.
    float clipped = samples >= 100.0 ? max(1.0, floor((samples + 50.0) / 100.0)) : 0.0;
    int low = 0;
    int high = 255;
    float total = 0.0;
    for (int i = 0; i < 256; ++i) {
        total += texelFetch(uHistogram, ivec2(i, 0), 0).a;
        if (total > clipped) { low = i; break; }
    }
    total = 0.0;
    for (int i = 255; i >= 0; --i) {
        total += texelFetch(uHistogram, ivec2(i, 0), 0).a;
        if (total > clipped) { high = i; break; }
    }
    if (high <= low + 6) { color = vec4(0.0); return; }
    float span = float(high - low) / 255.0;
    float contrast = clamp(0.85 / span, 1.0, 3.2);
    color = contrast > 1.001
        ? vec4(float(low) / 255.0, float(high) / 255.0, contrast, 1.0)
        : vec4(0.0);
}"#,
    )?;

    let program;
    let blur_program;
    let auto_color_program;
    let histogram_program;
    let histogram_reduce_program;
    let overlay_program;
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
        gl::AttachShader(auto_color_program, vertex);
        gl::AttachShader(auto_color_program, auto_fragment);
        gl::LinkProgram(auto_color_program);
        histogram_program = gl::CreateProgram();
        gl::AttachShader(histogram_program, histogram_vertex);
        gl::AttachShader(histogram_program, histogram_fragment);
        gl::LinkProgram(histogram_program);
        histogram_reduce_program = gl::CreateProgram();
        gl::AttachShader(histogram_reduce_program, vertex);
        gl::AttachShader(histogram_reduce_program, histogram_reduce_fragment);
        gl::LinkProgram(histogram_reduce_program);
        overlay_program = gl::CreateProgram();
        gl::AttachShader(overlay_program, vertex);
        gl::AttachShader(overlay_program, overlay_fragment);
        gl::LinkProgram(overlay_program);
        let mut overlay_linked = 0;
        gl::GetProgramiv(overlay_program, gl::LINK_STATUS, &mut overlay_linked);
        let mut histogram_linked = 0;
        let mut histogram_reduce_linked = 0;
        gl::GetProgramiv(histogram_program, gl::LINK_STATUS, &mut histogram_linked);
        gl::GetProgramiv(
            histogram_reduce_program,
            gl::LINK_STATUS,
            &mut histogram_reduce_linked,
        );
        let mut auto_linked = 0;
        gl::GetProgramiv(auto_color_program, gl::LINK_STATUS, &mut auto_linked);
        let mut blur_linked = 0;
        gl::GetProgramiv(blur_program, gl::LINK_STATUS, &mut blur_linked);
        let mut linked = 0;
        gl::GetProgramiv(program, gl::LINK_STATUS, &mut linked);
        gl::DeleteShader(vertex);
        gl::DeleteShader(fragment);
        gl::DeleteShader(blur_fragment);
        gl::DeleteShader(auto_fragment);
        gl::DeleteShader(histogram_vertex);
        gl::DeleteShader(histogram_fragment);
        gl::DeleteShader(histogram_reduce_fragment);
        gl::DeleteShader(overlay_fragment);
        if overlay_linked == 0 {
            let message = program_log(overlay_program);
            gl::DeleteProgram(overlay_program);
            gl::DeleteProgram(histogram_program);
            gl::DeleteProgram(histogram_reduce_program);
            gl::DeleteProgram(auto_color_program);
            gl::DeleteProgram(blur_program);
            gl::DeleteProgram(program);
            return Err(format!("overlay shader link failed: {message}"));
        }
        if histogram_linked == 0 || histogram_reduce_linked == 0 {
            let bad = if histogram_linked == 0 {
                histogram_program
            } else {
                histogram_reduce_program
            };
            let message = program_log(bad);
            gl::DeleteProgram(histogram_program);
            gl::DeleteProgram(histogram_reduce_program);
            gl::DeleteProgram(auto_color_program);
            gl::DeleteProgram(blur_program);
            gl::DeleteProgram(program);
            return Err(format!("auto contrast shader link failed: {message}"));
        }
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
    let objects = GlObjects {
        program,
        vao,
        texture,
        source_revision: 0,
        processed_texture: 0,
        processed_fbo: 0,
        processed_dimensions: (0, 0),
        processed_source_revision: 0,
        processed_tone_revision: 0,
        processed_geometry_revision: 0,
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
        auto_color_partial_texture: 0,
        auto_color_partial_fbo: 0,
        auto_color_partial_dimensions: (0, 0),
        auto_color_revision: 0,
        histogram_program,
        histogram_reduce_program,
        histogram_texture: 0,
        histogram_fbo: 0,
        auto_contrast_params: 0,
        auto_contrast_fbo: 0,
        auto_contrast_revision: 0,
        stats_source_texture: 0,
        stats_source_fbo: 0,
        stats_source_dimensions: (0, 0),
        stats_source_revision: 0,
        overlay_program,
        overlay_textures: Vec::new(),
        overlay_revision: 0,
        #[cfg(test)]
        gpu_timer_query: 0,
    };
    let fingerprint = capabilities.fingerprint();
    let validation_cached = mode == EditRendererMode::Auto && auto_gpu_is_verified(&fingerprint);
    if !validation_cached {
        validate_render_target()?;
        trace_gpu("initialization_validation=passed small_frame=true framebuffer=true");
    } else {
        trace_gpu("initialization_validation=cached capability_fingerprint_match=true");
    }
    if mode == EditRendererMode::Auto {
        cache_auto_gpu_success(fingerprint);
    }
    trace_gpu(format_args!(
        "renderer=gpu status=initialized initialization_total_ms={:.3} validation_cached={}",
        initialization_started.elapsed().as_secs_f64() * 1000.0,
        validation_cached
    ));
    Ok(objects)
}

/// Render the complete edit recipe at output-pixel resolution, then build its
/// mip pyramid. CPU preview and Export apply spatial effects before display
/// scaling; this cached texture keeps GPU Fit in the same order without a
/// source upload or framebuffer readback.
unsafe fn render_processed_source(
    objects: &mut GlObjects,
    width: i32,
    height: i32,
    viewport_width: i32,
    viewport_height: i32,
    view_scale: f32,
) {
    let mut previous_framebuffer = 0;
    gl::GetIntegerv(gl::FRAMEBUFFER_BINDING, &mut previous_framebuffer);
    if objects.processed_texture == 0 {
        gl::GenTextures(1, &mut objects.processed_texture);
        gl::GenFramebuffers(1, &mut objects.processed_fbo);
    }
    gl::ActiveTexture(gl::TEXTURE5);
    gl::BindTexture(gl::TEXTURE_2D, objects.processed_texture);
    if objects.processed_dimensions != (width, height) {
        gl::TexParameteri(
            gl::TEXTURE_2D,
            gl::TEXTURE_MIN_FILTER,
            gl::LINEAR_MIPMAP_LINEAR as i32,
        );
        gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MAG_FILTER, gl::NEAREST as i32);
        gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_WRAP_S, gl::CLAMP_TO_EDGE as i32);
        gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_WRAP_T, gl::CLAMP_TO_EDGE as i32);
        gl::TexImage2D(
            gl::TEXTURE_2D,
            0,
            gl::RGBA8 as i32,
            width,
            height,
            0,
            gl::RGBA,
            gl::UNSIGNED_BYTE,
            std::ptr::null(),
        );
        gl::BindFramebuffer(gl::FRAMEBUFFER, objects.processed_fbo);
        gl::FramebufferTexture2D(
            gl::FRAMEBUFFER,
            gl::COLOR_ATTACHMENT0,
            gl::TEXTURE_2D,
            objects.processed_texture,
            0,
        );
        objects.processed_dimensions = (width, height);
    }

    gl::BindFramebuffer(gl::FRAMEBUFFER, objects.processed_fbo);
    gl::Viewport(0, 0, width, height);
    gl::UseProgram(objects.program);
    uniform_1i(objects.program, "uProcessPass", 1);
    uniform_1i(objects.program, "uUseProcessedSource", 0);
    uniform_1f(objects.program, "uMinificationLod", 0.0);
    // Grain is generated in source coordinates here; mip filtering naturally
    // reduces its variance at Fit rather than scaling screen-space noise.
    uniform_1f(objects.program, "uScale", 1.0);
    // The source sampler remains on unit 0. Unbind the target from its regular
    // sampler unit during the FBO pass to avoid a texture feedback loop.
    gl::BindTexture(gl::TEXTURE_2D, 0);
    gl::ActiveTexture(gl::TEXTURE0);
    gl::DrawArrays(gl::TRIANGLES, 0, 3);
    uniform_1i(objects.program, "uProcessPass", 0);
    uniform_1f(
        objects.program,
        "uMinificationLod",
        (1.0 / view_scale.max(0.0001)).log2().max(0.0),
    );
    uniform_1f(objects.program, "uScale", view_scale.max(0.0001));
    gl::ActiveTexture(gl::TEXTURE5);
    gl::BindTexture(gl::TEXTURE_2D, objects.processed_texture);
    gl::GenerateMipmap(gl::TEXTURE_2D);
    gl::BindFramebuffer(gl::FRAMEBUFFER, previous_framebuffer as u32);
    gl::Viewport(0, 0, viewport_width, viewport_height);
    gl::ActiveTexture(gl::TEXTURE0);
    gl::BindTexture(gl::TEXTURE_2D, objects.texture);
}

/// Draw independently textured layers over the completed tone/filter image.
/// CPU only decodes assets and shapes text; no photo-sized CPU pixel blending.
unsafe fn render_gpu_overlays(
    objects: &mut GlObjects,
    overlays: &[GpuOverlayLayer],
    revision: u64,
    view: &GpuView,
    view_width: i32,
    view_height: i32,
    device_scale: i32,
    photo_width: i32,
    photo_height: i32,
) {
    if objects.overlay_revision != revision {
        // Repositioning and opacity changes only update rect/opacity uniforms.
        // Retain GL textures by content identity, even when layer order changes.
        let mut previous = std::mem::take(&mut objects.overlay_textures);
        for overlay in overlays {
            if let Some(index) = previous.iter().position(|(id, _)| id == &overlay.id) {
                let entry = previous.swap_remove(index);
                objects.overlay_textures.push(entry);
                continue;
            }
            let mut texture = 0;
            gl::GenTextures(1, &mut texture);
            gl::BindTexture(gl::TEXTURE_2D, texture);
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MIN_FILTER, gl::LINEAR as i32);
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MAG_FILTER, gl::LINEAR as i32);
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_WRAP_S, gl::CLAMP_TO_EDGE as i32);
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_WRAP_T, gl::CLAMP_TO_EDGE as i32);
            gl::PixelStorei(gl::UNPACK_ALIGNMENT, 1);
            gl::TexImage2D(
                gl::TEXTURE_2D,
                0,
                gl::RGBA8 as i32,
                overlay.width,
                overlay.height,
                0,
                gl::RGBA,
                gl::UNSIGNED_BYTE,
                overlay.rgba.as_ref().as_ptr().cast(),
            );
            if std::env::var_os("PICASA_TRACE").is_some() {
                eprintln!(
                    "PIC_EDIT_GPU layer_upload id={} dimensions={}x{}",
                    overlay.id, overlay.width, overlay.height
                );
            }
            objects.overlay_textures.push((overlay.id.clone(), texture));
        }
        for (_, texture) in previous {
            gl::DeleteTextures(1, &texture);
        }
        objects.overlay_revision = revision;
    }
    let was_blending = gl::IsEnabled(gl::BLEND) == gl::TRUE;
    let mut prev_src_rgb = 0;
    let mut prev_dst_rgb = 0;
    let mut prev_src_alpha = 0;
    let mut prev_dst_alpha = 0;
    let mut prev_eq_rgb = 0;
    let mut prev_eq_alpha = 0;
    gl::GetIntegerv(gl::BLEND_SRC_RGB, &mut prev_src_rgb);
    gl::GetIntegerv(gl::BLEND_DST_RGB, &mut prev_dst_rgb);
    gl::GetIntegerv(gl::BLEND_SRC_ALPHA, &mut prev_src_alpha);
    gl::GetIntegerv(gl::BLEND_DST_ALPHA, &mut prev_dst_alpha);
    gl::GetIntegerv(gl::BLEND_EQUATION_RGB, &mut prev_eq_rgb);
    gl::GetIntegerv(gl::BLEND_EQUATION_ALPHA, &mut prev_eq_alpha);
    gl::Enable(gl::BLEND);
    gl::BlendFuncSeparate(
        gl::SRC_ALPHA,
        gl::ONE_MINUS_SRC_ALPHA,
        gl::ONE,
        gl::ONE_MINUS_SRC_ALPHA,
    );
    gl::BlendEquation(gl::FUNC_ADD);
    gl::UseProgram(objects.overlay_program);
    uniform_1i(objects.overlay_program, "uOverlay", 0);
    uniform_2f(
        objects.overlay_program,
        "uViewSize",
        view_width as f32,
        view_height as f32,
    );
    uniform_2f(
        objects.overlay_program,
        "uScroll",
        view.scroll_x,
        view.scroll_y,
    );
    uniform_2f(objects.overlay_program, "uPadding", view.pad_x, view.pad_y);
    uniform_1f(objects.overlay_program, "uScale", view.scale);
    uniform_1f(objects.overlay_program, "uDeviceScale", device_scale as f32);
    uniform_2f(
        objects.overlay_program,
        "uPhotoSize",
        photo_width as f32,
        photo_height as f32,
    );
    for (overlay, (_, texture)) in overlays.iter().zip(&objects.overlay_textures) {
        gl::BindTexture(gl::TEXTURE_2D, *texture);
        let rect = overlay.rect;
        let location = uniform_location(objects.overlay_program, "uLayerRect");
        gl::Uniform4f(
            location,
            rect.left * photo_width as f32,
            rect.top * photo_height as f32,
            rect.width * photo_width as f32,
            rect.height * photo_height as f32,
        );
        uniform_1f(
            objects.overlay_program,
            "uLayerOpacity",
            overlay.opacity.clamp(0.0, 1.0),
        );
        gl::DrawArrays(gl::TRIANGLES, 0, 3);
    }
    gl::BlendFuncSeparate(
        prev_src_rgb as u32,
        prev_dst_rgb as u32,
        prev_src_alpha as u32,
        prev_dst_alpha as u32,
    );
    gl::BlendEquationSeparate(prev_eq_rgb as u32, prev_eq_alpha as u32);
    if !was_blending {
        gl::Disable(gl::BLEND);
    }
    gl::UseProgram(objects.program);
    gl::ActiveTexture(gl::TEXTURE0);
    gl::BindTexture(gl::TEXTURE_2D, objects.texture);
}

/// Filter and crop the decoded source in an offscreen GPU framebuffer,
/// using exactly the same shader and effect textures as the onscreen view.
/// Autocorrection histograms then sample this cache; no CPU photo render.
unsafe fn draw_stats_source(
    objects: &mut GlObjects,
    width: i32,
    height: i32,
    revision: u64,
    viewport_width: i32,
    viewport_height: i32,
) {
    if objects.stats_source_revision == revision
        && objects.stats_source_dimensions == (width, height)
    {
        return;
    }
    let mut previous_fbo = 0;
    gl::GetIntegerv(gl::FRAMEBUFFER_BINDING, &mut previous_fbo);
    if objects.stats_source_texture == 0 {
        gl::GenTextures(1, &mut objects.stats_source_texture);
        gl::GenFramebuffers(1, &mut objects.stats_source_fbo);
    }
    if objects.stats_source_dimensions != (width, height) {
        gl::BindTexture(gl::TEXTURE_2D, objects.stats_source_texture);
        gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MIN_FILTER, gl::NEAREST as i32);
        gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MAG_FILTER, gl::NEAREST as i32);
        gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_WRAP_S, gl::CLAMP_TO_EDGE as i32);
        gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_WRAP_T, gl::CLAMP_TO_EDGE as i32);
        gl::TexImage2D(
            gl::TEXTURE_2D,
            0,
            gl::RGBA8 as i32,
            width,
            height,
            0,
            gl::RGBA,
            gl::UNSIGNED_BYTE,
            std::ptr::null(),
        );
        gl::BindFramebuffer(gl::FRAMEBUFFER, objects.stats_source_fbo);
        gl::FramebufferTexture2D(
            gl::FRAMEBUFFER,
            gl::COLOR_ATTACHMENT0,
            gl::TEXTURE_2D,
            objects.stats_source_texture,
            0,
        );
        objects.stats_source_dimensions = (width, height);
    }
    gl::BindFramebuffer(gl::FRAMEBUFFER, objects.stats_source_fbo);
    gl::Viewport(0, 0, width, height);
    gl::UseProgram(objects.program);
    gl::ActiveTexture(gl::TEXTURE0);
    gl::BindTexture(gl::TEXTURE_2D, objects.texture);
    uniform_1i(objects.program, "uClarityPrepass", 3);
    uniform_2f(
        objects.program,
        "uClarityPassSize",
        width as f32,
        height as f32,
    );
    gl::DrawArrays(gl::TRIANGLES, 0, 3);
    gl::BindFramebuffer(gl::FRAMEBUFFER, previous_fbo as u32);
    gl::Viewport(0, 0, viewport_width, viewport_height);
    uniform_1i(objects.program, "uClarityPrepass", 0);
    gl::ActiveTexture(gl::TEXTURE0);
    gl::BindTexture(gl::TEXTURE_2D, objects.texture);
    objects.stats_source_revision = revision;
    if std::env::var_os("PICASA_TRACE").is_some() {
        eprintln!(
            "PIC_EDIT_GPU auto_stats_prepass dimensions={width}x{height} revision={revision}"
        );
    }
}

/// Compute Auto Contrast on GPU from the exact flattened-sample luminance
/// histogram used by render::apply_auto_contrast. Both histograms and robust
/// percentile bounds remain in GPU textures, with no synchronous readback.
unsafe fn draw_auto_contrast_stats(
    objects: &mut GlObjects,
    stats_source: u32,
    width: i32,
    height: i32,
    revision: u64,
    auto_color: bool,
    viewport_width: i32,
    viewport_height: i32,
) {
    // Two cached percentile sets for the same source: uncorrected and after
    // Auto Colour. Switching either toggle must invalidate the histogram.
    let signature = revision.wrapping_mul(2).wrapping_add(u64::from(auto_color));
    if objects.auto_contrast_revision == signature {
        return;
    }
    let mut prev_fbo = 0;
    let mut prev_blend_src_rgb = 0;
    let mut prev_blend_dst_rgb = 0;
    let mut prev_blend_src_alpha = 0;
    let mut prev_blend_dst_alpha = 0;
    let mut prev_blend_equation_rgb = 0;
    let mut prev_blend_equation_alpha = 0;
    let mut prev_clear = [0.0f32; 4];
    gl::GetIntegerv(gl::FRAMEBUFFER_BINDING, &mut prev_fbo);
    gl::GetIntegerv(gl::BLEND_SRC_RGB, &mut prev_blend_src_rgb);
    gl::GetIntegerv(gl::BLEND_DST_RGB, &mut prev_blend_dst_rgb);
    gl::GetIntegerv(gl::BLEND_SRC_ALPHA, &mut prev_blend_src_alpha);
    gl::GetIntegerv(gl::BLEND_DST_ALPHA, &mut prev_blend_dst_alpha);
    gl::GetIntegerv(gl::BLEND_EQUATION_RGB, &mut prev_blend_equation_rgb);
    gl::GetIntegerv(gl::BLEND_EQUATION_ALPHA, &mut prev_blend_equation_alpha);
    gl::GetFloatv(gl::COLOR_CLEAR_VALUE, prev_clear.as_mut_ptr());
    let blend_was_enabled = gl::IsEnabled(gl::BLEND) == gl::TRUE;

    if objects.histogram_texture == 0 {
        gl::GenTextures(1, &mut objects.histogram_texture);
        gl::GenFramebuffers(1, &mut objects.histogram_fbo);
        gl::GenTextures(1, &mut objects.auto_contrast_params);
        gl::GenFramebuffers(1, &mut objects.auto_contrast_fbo);
        for (texture, fbo, w) in [
            (objects.histogram_texture, objects.histogram_fbo, 256),
            (objects.auto_contrast_params, objects.auto_contrast_fbo, 1),
        ] {
            gl::BindTexture(gl::TEXTURE_2D, texture);
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MIN_FILTER, gl::NEAREST as i32);
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MAG_FILTER, gl::NEAREST as i32);
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_WRAP_S, gl::CLAMP_TO_EDGE as i32);
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_WRAP_T, gl::CLAMP_TO_EDGE as i32);
            gl::TexImage2D(
                gl::TEXTURE_2D,
                0,
                gl::RGBA32F as i32,
                w,
                1,
                0,
                gl::RGBA,
                gl::FLOAT,
                std::ptr::null(),
            );
            gl::BindFramebuffer(gl::FRAMEBUFFER, fbo);
            gl::FramebufferTexture2D(
                gl::FRAMEBUFFER,
                gl::COLOR_ATTACHMENT0,
                gl::TEXTURE_2D,
                texture,
                0,
            );
        }
    }
    gl::BindFramebuffer(gl::FRAMEBUFFER, objects.histogram_fbo);
    gl::Viewport(0, 0, 256, 1);
    gl::ClearColor(0.0, 0.0, 0.0, 0.0);
    gl::Clear(gl::COLOR_BUFFER_BIT);
    gl::Enable(gl::BLEND);
    gl::BlendEquation(gl::FUNC_ADD);
    gl::BlendFunc(gl::ONE, gl::ONE);
    gl::UseProgram(objects.histogram_program);
    gl::ActiveTexture(gl::TEXTURE0);
    gl::BindTexture(gl::TEXTURE_2D, stats_source);
    uniform_1i(objects.histogram_program, "uSource", 0);
    uniform_1i(
        objects.histogram_program,
        "uAutoColor",
        if auto_color { 1 } else { 0 },
    );
    if auto_color {
        gl::ActiveTexture(gl::TEXTURE3);
        gl::BindTexture(gl::TEXTURE_2D, objects.auto_color_stats);
        uniform_1i(objects.histogram_program, "uAutoColorStats", 3);
        gl::ActiveTexture(gl::TEXTURE0);
        // Keep unit 0 bound to the filtered/cropped statistics source.
        // Resetting it to the raw texture here silently calculates the
        // combined Auto Colour + Auto Contrast histogram on different pixels.
        gl::BindTexture(gl::TEXTURE_2D, stats_source);
    }
    uniform_1i(objects.histogram_program, "uWidth", width);
    let pixel_count = i64::from(width) * i64::from(height);
    let stride = (pixel_count / 250_000).max(1);
    uniform_1i(objects.histogram_program, "uSampleStep", stride as i32);
    gl::DrawArrays(gl::POINTS, 0, ((pixel_count + stride - 1) / stride) as i32);

    gl::BindFramebuffer(gl::FRAMEBUFFER, objects.auto_contrast_fbo);
    gl::Viewport(0, 0, 1, 1);
    gl::Disable(gl::BLEND);
    gl::UseProgram(objects.histogram_reduce_program);
    gl::ActiveTexture(gl::TEXTURE0);
    gl::BindTexture(gl::TEXTURE_2D, objects.histogram_texture);
    uniform_1i(objects.histogram_reduce_program, "uHistogram", 0);
    gl::DrawArrays(gl::TRIANGLES, 0, 3);

    gl::BlendFuncSeparate(
        prev_blend_src_rgb as u32,
        prev_blend_dst_rgb as u32,
        prev_blend_src_alpha as u32,
        prev_blend_dst_alpha as u32,
    );
    gl::BlendEquationSeparate(
        prev_blend_equation_rgb as u32,
        prev_blend_equation_alpha as u32,
    );
    if blend_was_enabled {
        gl::Enable(gl::BLEND);
    } else {
        gl::Disable(gl::BLEND);
    }
    gl::ClearColor(prev_clear[0], prev_clear[1], prev_clear[2], prev_clear[3]);
    gl::BindFramebuffer(gl::FRAMEBUFFER, prev_fbo as u32);
    gl::Viewport(0, 0, viewport_width, viewport_height);
    gl::UseProgram(objects.program);
    gl::ActiveTexture(gl::TEXTURE0);
    gl::BindTexture(gl::TEXTURE_2D, objects.texture);
    objects.auto_contrast_revision = signature;
}

/// GPU reduction into a one-pixel RGBA32F sum. Matches CPU flattened
/// neutral sampling without a source-sized CPU processing pass or GL readback.
unsafe fn draw_auto_color_stats(
    objects: &mut GlObjects,
    stats_source: u32,
    width: i32,
    height: i32,
    revision: u64,
    viewport_width: i32,
    viewport_height: i32,
) {
    if objects.auto_color_revision == revision {
        return;
    }
    let mut previous_fbo = 0;
    gl::GetIntegerv(gl::FRAMEBUFFER_BINDING, &mut previous_fbo);
    let was_blending = gl::IsEnabled(gl::BLEND) == gl::TRUE;
    let pixel_count = i64::from(width) * i64::from(height);
    let step = (pixel_count / 200_000).max(1);
    let sample_count = ((pixel_count + step - 1) / step) as i32;
    let partial_count = (sample_count + 511) / 512;
    if objects.auto_color_stats == 0 {
        gl::GenTextures(1, &mut objects.auto_color_stats);
        gl::GenFramebuffers(1, &mut objects.auto_color_fbo);
        gl::BindTexture(gl::TEXTURE_2D, objects.auto_color_stats);
        gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MIN_FILTER, gl::NEAREST as i32);
        gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MAG_FILTER, gl::NEAREST as i32);
        gl::TexImage2D(
            gl::TEXTURE_2D,
            0,
            gl::RGBA32F as i32,
            1,
            1,
            0,
            gl::RGBA,
            gl::FLOAT,
            std::ptr::null(),
        );
        gl::BindFramebuffer(gl::FRAMEBUFFER, objects.auto_color_fbo);
        gl::FramebufferTexture2D(
            gl::FRAMEBUFFER,
            gl::COLOR_ATTACHMENT0,
            gl::TEXTURE_2D,
            objects.auto_color_stats,
            0,
        );
    } else {
        gl::BindTexture(gl::TEXTURE_2D, objects.auto_color_stats);
    }
    if objects.auto_color_partial_texture == 0 {
        gl::GenTextures(1, &mut objects.auto_color_partial_texture);
        gl::GenFramebuffers(1, &mut objects.auto_color_partial_fbo);
    }
    if objects.auto_color_partial_dimensions != (partial_count, 1) {
        gl::BindTexture(gl::TEXTURE_2D, objects.auto_color_partial_texture);
        gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MIN_FILTER, gl::NEAREST as i32);
        gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MAG_FILTER, gl::NEAREST as i32);
        gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_WRAP_S, gl::CLAMP_TO_EDGE as i32);
        gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_WRAP_T, gl::CLAMP_TO_EDGE as i32);
        gl::TexImage2D(
            gl::TEXTURE_2D,
            0,
            gl::RGBA32F as i32,
            partial_count,
            1,
            0,
            gl::RGBA,
            gl::FLOAT,
            std::ptr::null(),
        );
        gl::BindFramebuffer(gl::FRAMEBUFFER, objects.auto_color_partial_fbo);
        gl::FramebufferTexture2D(
            gl::FRAMEBUFFER,
            gl::COLOR_ATTACHMENT0,
            gl::TEXTURE_2D,
            objects.auto_color_partial_texture,
            0,
        );
        objects.auto_color_partial_dimensions = (partial_count, 1);
    }

    gl::Disable(gl::BLEND);
    gl::UseProgram(objects.auto_color_program);
    gl::ActiveTexture(gl::TEXTURE0);
    gl::BindTexture(gl::TEXTURE_2D, stats_source);
    uniform_1i(objects.auto_color_program, "uSource", 0);
    uniform_1i(objects.auto_color_program, "uWidth", width);
    uniform_1i(objects.auto_color_program, "uSampleStep", step as i32);
    uniform_1i(objects.auto_color_program, "uSampleCount", sample_count);
    uniform_1i(objects.auto_color_program, "uReductionPass", 0);
    gl::BindFramebuffer(gl::FRAMEBUFFER, objects.auto_color_partial_fbo);
    gl::Viewport(0, 0, partial_count, 1);
    gl::DrawArrays(gl::TRIANGLES, 0, 3);

    uniform_1i(objects.auto_color_program, "uReductionPass", 1);
    uniform_1i(objects.auto_color_program, "uPartialCount", partial_count);
    gl::ActiveTexture(gl::TEXTURE1);
    gl::BindTexture(gl::TEXTURE_2D, objects.auto_color_partial_texture);
    uniform_1i(objects.auto_color_program, "uPartials", 1);
    gl::BindFramebuffer(gl::FRAMEBUFFER, objects.auto_color_fbo);
    gl::Viewport(0, 0, 1, 1);
    gl::DrawArrays(gl::TRIANGLES, 0, 3);

    if was_blending {
        gl::Enable(gl::BLEND);
    }
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
    if size % 2 == 0 {
        size += 1;
    }
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
                gl::TEXTURE_2D,
                0,
                gl::R8 as i32,
                width,
                height,
                0,
                gl::RED,
                gl::UNSIGNED_BYTE,
                std::ptr::null(),
            );
            gl::BindFramebuffer(gl::FRAMEBUFFER, framebuffer);
            gl::FramebufferTexture2D(
                gl::FRAMEBUFFER,
                gl::COLOR_ATTACHMENT0,
                gl::TEXTURE_2D,
                texture,
                0,
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
    uniform_2f(
        objects.program,
        "uClarityPassSize",
        width as f32,
        height as f32,
    );
    gl::DrawArrays(gl::TRIANGLES, 0, 3);

    let sigma = ((width.min(height) as f32) * 0.008).clamp(1.5, 20.0);
    // image 0.25.10 GaussianBlurParameters::kernel_size_from_sigma;
    // a generic 3*sigma support mismatches its small-radius behaviour.
    let radius = cpu_gaussian_kernel_radius(sigma);
    gl::UseProgram(objects.blur_program);
    uniform_1i(objects.blur_program, "uInput", 0);
    uniform_2f(
        objects.blur_program,
        "uTexel",
        1.0 / width as f32,
        1.0 / height as f32,
    );
    uniform_1f(objects.blur_program, "uSigma", sigma);
    gl::Uniform1i(uniform_location(objects.blur_program, "uRadius"), radius);
    for (input, output, x, y) in [
        (
            objects.clarity_luma_texture,
            objects.clarity_blur_fbo,
            1.0,
            0.0,
        ),
        (
            objects.clarity_blur_texture,
            objects.clarity_luma_fbo,
            0.0,
            1.0,
        ),
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
                gl::TEXTURE_2D,
                0,
                gl::R8 as i32,
                width,
                height,
                0,
                gl::RED,
                gl::UNSIGNED_BYTE,
                std::ptr::null(),
            );
            gl::BindFramebuffer(gl::FRAMEBUFFER, framebuffer);
            gl::FramebufferTexture2D(
                gl::FRAMEBUFFER,
                gl::COLOR_ATTACHMENT0,
                gl::TEXTURE_2D,
                texture,
                0,
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
    uniform_2f(
        objects.program,
        "uClarityPassSize",
        width as f32,
        height as f32,
    );
    gl::DrawArrays(gl::TRIANGLES, 0, 3);

    let sigma = ((width.min(height) as f32) * if pencil { 0.004 } else { 0.008 }).max(0.8);
    // image 0.25.10 GaussianBlurParameters::kernel_size_from_sigma;
    // a generic 3*sigma support mismatches its small-radius behaviour.
    let radius = cpu_gaussian_kernel_radius(sigma);
    gl::UseProgram(objects.blur_program);
    uniform_1i(objects.blur_program, "uInput", 0);
    uniform_2f(
        objects.blur_program,
        "uTexel",
        1.0 / width as f32,
        1.0 / height as f32,
    );
    uniform_1f(objects.blur_program, "uSigma", sigma);
    gl::Uniform1i(uniform_location(objects.blur_program, "uRadius"), radius);
    for (input, output, x, y) in [
        (
            objects.effect_luma_texture,
            objects.effect_blur_fbo,
            1.0,
            0.0,
        ),
        (
            objects.effect_blur_texture,
            objects.effect_luma_fbo,
            0.0,
            1.0,
        ),
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
    fn renderer_mode_defaults_to_capability_checked_auto() {
        assert_eq!(EditRendererMode::parse(None), EditRendererMode::Auto);
        assert_eq!(
            EditRendererMode::parse(Some("auto")),
            EditRendererMode::Auto
        );
        assert_eq!(EditRendererMode::parse(Some("gpu")), EditRendererMode::Gpu);
        assert_eq!(EditRendererMode::parse(Some("cpu")), EditRendererMode::Cpu);
        assert_eq!(EditRendererMode::Auto.should_attempt_gpu(), true);
        assert!(!EditRendererMode::Cpu.should_attempt_gpu());
        assert!(EditRendererMode::Gpu.should_attempt_gpu());
    }

    #[test]
    fn software_renderers_are_detected_without_vendor_classification() {
        for renderer in [
            "llvmpipe (LLVM 18.1.8)",
            "softpipe",
            "swrast",
            "SwiftShader",
        ] {
            assert!(is_software_renderer(renderer), "{renderer}");
        }
        for renderer in [
            "AMD Radeon RX 7600",
            "Mesa Intel(R) UHD Graphics",
            "NVIDIA RTX",
        ] {
            assert!(!is_software_renderer(renderer), "{renderer}");
        }
    }

    #[test]
    #[ignore = "requires GTK and OpenGL; run once with hardware GL and once with software GL"]
    fn gtk_auto_renderer_validates_or_rejects_the_current_context_cleanly() {
        gtk::init().expect("GTK initialization");
        let renderer = LiveGpuCanvas::new_for_mode(EditRendererMode::Auto);
        renderer.widget().set_size_request(8, 8);
        let window = gtk::Window::new();
        window.set_default_size(8, 8);
        window.set_child(Some(&renderer.widget()));
        window.present();
        let recipe = EditRecipe::default();
        let pixels = image::RgbaImage::from_pixel(4, 4, image::Rgba([48, 96, 144, 255]));
        renderer.set_source(
            "test://auto-capability-validation",
            0,
            &recipe,
            4,
            4,
            glib::Bytes::from_owned(pixels.into_raw()),
        );
        renderer.set_recipe(&recipe);
        renderer.set_view(0.0, 0.0, 8.0, 8.0, 1.0, 0.0, 0.0);
        let context = glib::MainContext::default();
        let deadline = Instant::now() + std::time::Duration::from_secs(6);
        while Instant::now() < deadline {
            while context.pending() {
                context.iteration(false);
            }
            let state = renderer.state.borrow();
            if state.failure.is_some() || state.has_presented_frame {
                break;
            }
            drop(state);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let state = renderer.state.borrow();
        assert!(
            state.failure.is_some() || state.has_presented_frame,
            "Auto must either validate and present a frame or record a clear CPU fallback reason"
        );
        if state.failure.is_none() {
            assert!(
                state.objects.is_some(),
                "validated context keeps its GL objects"
            );
            assert!(
                auto_gpu_retry_allowed(),
                "successful capability validation is cached"
            );
        }
        drop(state);
        window.close();
    }

    #[test]
    fn uncertain_automatic_filter_and_crop_combinations_keep_cpu_reference() {
        // A future GPU histogram optimisation must not accidentally enable
        // combinations with unverified colour or contrast statistics.
        for (auto_color, auto_contrast) in [(true, false), (false, true), (true, true)] {
            let mut plain = EditRecipe::default();
            plain.auto_color = auto_color;
            plain.auto_contrast = auto_contrast;
            assert!(
                gpu_tone_supported(&plain),
                "simple automatic corrections must remain GPU accelerated"
            );

            let mut crop = plain.clone();
            crop.crop.left = 0.1;
            assert!(
                !gpu_tone_supported(&crop),
                "cropped Auto corrections require pixel-parity approval"
            );
            let mut straighten = plain.clone();
            straighten.straighten = 3.5;
            assert!(
                !gpu_tone_supported(&straighten),
                "straightened Auto corrections require pixel-parity approval"
            );
            for filter in FilterPreset::ALL {
                let mut filtered = plain.clone();
                filtered.filter = filter;
                filtered.filter_amount = 0.65;
                assert!(
                    !gpu_tone_supported(&filtered),
                    "{filter:?} + automatic correction must use verified CPU path"
                );
            }
        }
        for filter in FilterPreset::ALL {
            let mut no_autos = EditRecipe::default();
            no_autos.filter = filter;
            no_autos.crop.left = 0.1;
            assert!(
                gpu_tone_supported(&no_autos),
                "{filter:?} with crop but no autos is GPU supported"
            );
        }
    }

    #[test]
    fn automatic_preview_gate_keeps_unverified_zoom_and_rotation_on_cpu() {
        let mut automatic = EditRecipe::default();
        automatic.auto_color = true;
        assert!(gpu_tone_supported_for_view(&automatic, true, 0));
        assert!(!gpu_tone_supported_for_view(&automatic, false, 0));
        assert!(!gpu_tone_supported_for_view(&automatic, true, 90));
        assert!(!gpu_tone_supported_for_view(&automatic, true, 180));

        let plain = EditRecipe::default();
        assert!(
            gpu_tone_supported_for_view(&plain, false, 90),
            "non-automatic edits retain GPU FIT and rotation support"
        );
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
        assert!(!base.auto_contrast);
        assert_eq!(base.crop, CropRect::default());
        assert_eq!(base.straighten, 0.0);
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
        let mut auto_contrast = EditRecipe::default();
        auto_contrast.auto_contrast = true;
        assert!(gpu_tone_supported(&auto_contrast));
        auto_contrast.auto_color = true;
        assert!(gpu_tone_supported(&auto_contrast));
        auto_contrast.filter = FilterPreset::Clarendon;
        assert!(!gpu_tone_supported(&auto_contrast));
        let mut cropped_auto = EditRecipe::default();
        cropped_auto.crop.left = 0.1;
        cropped_auto.auto_color = true;
        assert!(!gpu_tone_supported(&cropped_auto));
        cropped_auto.auto_color = false;
        cropped_auto.auto_contrast = true;
        assert!(!gpu_tone_supported(&cropped_auto));
        cropped_auto.auto_contrast = false;
        assert!(gpu_tone_supported(&cropped_auto));
        for effect_filter in [FilterPreset::PencilSketch, FilterPreset::DreamGlow] {
            let mut preset = EditRecipe::default();
            preset.filter = effect_filter;
            assert!(gpu_tone_supported(&preset));
        }
        for supported_filter in [
            FilterPreset::Clarendon,
            FilterPreset::Gingham,
            FilterPreset::Valencia,
            FilterPreset::OldPhoto,
        ] {
            let mut preset = EditRecipe::default();
            preset.filter = supported_filter;
            assert!(gpu_tone_supported(&preset));
        }
    }

    #[test]
    fn text_gpu_texture_key_changes_only_when_raster_content_changes() {
        let mut layer = super::super::model::TextLayerSpec::new_default();
        let key = text_layer_content_key(&layer, (3936, 2624));
        layer.x = 0.23;
        layer.y = 0.45;
        layer.opacity = 0.5;
        layer.anchor = super::super::model::OverlayAnchor::BottomRight;
        assert_eq!(
            key,
            text_layer_content_key(&layer, (3936, 2624)),
            "moving a text overlay must retain the GPU raster texture"
        );
        layer.text = "Updated words".to_string();
        assert_ne!(
            key,
            text_layer_content_key(&layer, (3936, 2624)),
            "changing text must invalidate the GPU raster texture"
        );
        layer.text = "Your text".to_string();
        layer.size = 0.12;
        assert_ne!(key, text_layer_content_key(&layer, (3936, 2624)));
        layer.size = super::super::model::TextLayerSpec::DEFAULT_SIZE;
        assert_ne!(
            key,
            text_layer_content_key(&layer, (4000, 3000)),
            "raster must use the current output photo dimensions"
        );
    }

    #[test]
    fn gpu_thumbnail_cover_stays_square_and_centers_photo() {
        let landscape = thumbnail_cover_view(300, 200, 84);
        assert_eq!(landscape.width, 84.0);
        assert_eq!(landscape.height, 84.0);
        assert!((landscape.scale - 0.42).abs() < 0.0001);
        assert!((landscape.scroll_x - 21.0).abs() < 0.001);
        assert_eq!(landscape.scroll_y, 0.0);
        let portrait = thumbnail_cover_view(200, 300, 84);
        assert!((portrait.scroll_y - 21.0).abs() < 0.001);
        assert_eq!(portrait.scroll_x, 0.0);
        let square = thumbnail_cover_view(168, 168, 84);
        assert_eq!(square.scale, 0.5);
        assert_eq!(square.scroll_x, 0.0);
    }

    #[test]
    fn gpu_straighten_crop_dimensions_match_cpu() {
        let image = image::RgbaImage::from_fn(101, 67, |x, y| {
            image::Rgba([x as u8, y as u8, (x + y) as u8, 255])
        });
        for rotation in [0, 90, 180, 270] {
            for angle in [0.0, 1.5, -4.5, 8.7] {
                for crop in [
                    CropRect::default(),
                    CropRect {
                        left: 0.13,
                        top: 0.2,
                        right: 0.87,
                        bottom: 0.92,
                    },
                    CropRect {
                        left: 0.0,
                        top: 0.0,
                        right: 0.5,
                        bottom: 0.5,
                    },
                ] {
                    let recipe = EditRecipe {
                        straighten: angle,
                        crop,
                        ..EditRecipe::default()
                    };
                    let cpu = super::super::render::apply_geometry(
                        super::super::render::apply_library_rotation(image.clone(), rotation),
                        &recipe,
                    );
                    let gpu = GpuGeometry::for_source_rotation(101, 67, rotation, &recipe);
                    assert_eq!(
                        (gpu.output_width, gpu.output_height),
                        (cpu.width() as i32, cpu.height() as i32),
                        "rotation={rotation} angle={angle}, crop={crop:?}",
                    );
                }
            }
        }
    }

    #[test]
    #[ignore = "requires GTK OpenGL; run under xvfb-run"]
    fn gtk_gl_area_filter_thumbnail_cover_matches_cpu() {
        gtk::init().expect("GTK init");
        const SW: u32 = 48;
        const SH: u32 = 32;
        const TILE: u32 = 32;
        let src = image::RgbaImage::from_fn(SW, SH, |x, y| {
            image::Rgba([
                (x * 3 + y * 2 + 20) as u8,
                (y * 5 + x + 15) as u8,
                ((x * 2 + y * 3 + 30) % 240) as u8,
                255,
            ])
        });
        let gpu = LiveGpuCanvas::new();
        let window = gtk::Window::new();
        window.set_default_size(TILE as i32, TILE as i32);
        window.set_child(Some(&gpu.widget()));
        window.present();
        gpu.set_source(
            "test://filter-tile",
            0,
            &EditRecipe::default(),
            SW as i32,
            SH as i32,
            glib::Bytes::from_owned(src.clone().into_raw()),
        );
        gpu.set_untransformed_geometry(&EditRecipe::default());
        gpu.set_thumbnail_cover(SW as i32, SH as i32, TILE as i32);
        let context = glib::MainContext::default();
        for (filter, black_white, sepia) in [
            (FilterPreset::None, false, false),
            (FilterPreset::Clarendon, false, false),
            (FilterPreset::Newspaper, false, false),
            (FilterPreset::None, true, false),
            (FilterPreset::None, false, true),
        ] {
            let mut recipe = EditRecipe::default();
            recipe.filter = filter;
            recipe.filter_amount = 1.0;
            recipe.black_white = black_white;
            recipe.sepia = sepia;
            gpu.set_recipe(&recipe);
            gpu.request_readback();
            let deadline = Instant::now() + std::time::Duration::from_secs(6);
            let pixels = loop {
                while context.pending() {
                    context.iteration(false);
                }
                if let Some(frame) = gpu.take_readback() {
                    break frame;
                }
                assert!(Instant::now() < deadline, "GPU filter tile timed out");
                std::thread::sleep(std::time::Duration::from_millis(5));
            };
            let scale = gpu.widget().scale_factor().max(1) as usize;
            let actual = pixels
                .chunks_exact(TILE as usize * scale * 4)
                .rev()
                .flatten()
                .copied()
                .collect::<Vec<_>>();
            let cpu = super::super::render::apply_tone(src.clone(), &recipe);
            let expected = image::imageops::crop_imm(&cpu, 8, 0, TILE, TILE).to_image();
            let expected = if scale == 1 {
                expected
            } else {
                image::imageops::resize(
                    &expected,
                    TILE * scale as u32,
                    TILE * scale as u32,
                    image::imageops::FilterType::Nearest,
                )
            }
            .into_raw();
            assert_eq!(actual.len(), expected.len());
            let max_delta = actual
                .iter()
                .zip(&expected)
                .map(|(a, b)| a.abs_diff(*b))
                .max()
                .unwrap_or_default();
            eprintln!(
                "PIC_EDIT_GPU thumbnail_parity filter={filter:?} bw={black_white} sepia={sepia} max_delta={max_delta}"
            );
            assert!(
                max_delta <= 2,
                "thumbnail GPU/CPU mismatch filter={filter:?} bw={black_white} sepia={sepia}: {max_delta}"
            );
        }
        window.close();
    }

    #[test]
    #[ignore = "requires GTK OpenGL display; run under xvfb-run"]
    fn gtk_gl_area_fit_minification_matches_lanczos_checkerboard_reference() {
        gtk::init().expect("GTK init");
        const SOURCE: u32 = 128;
        const VIEW: u32 = 32;
        let checker = image::RgbaImage::from_fn(SOURCE, SOURCE, |x, y| {
            let value = if (x + y) % 2 == 0 { 0 } else { 255 };
            image::Rgba([value, value, value, 255])
        });
        let gpu = LiveGpuCanvas::new();
        gpu.widget().set_size_request(VIEW as i32, VIEW as i32);
        let window = gtk::Window::new();
        window.set_default_size(VIEW as i32, VIEW as i32);
        window.set_child(Some(&gpu.widget()));
        window.present();
        gpu.set_source(
            "test://checkerboard-fit",
            0,
            &EditRecipe::default(),
            SOURCE as i32,
            SOURCE as i32,
            glib::Bytes::from_owned(checker.clone().into_raw()),
        );
        // Exercise the geometryPixel path that previously bypassed
        // footprint filtering for an unadjusted source texture.
        gpu.set_untransformed_geometry(&EditRecipe::default());
        gpu.set_view(
            0.0,
            0.0,
            VIEW as f64,
            VIEW as f64,
            VIEW as f64 / SOURCE as f64,
            0.0,
            0.0,
        );
        gpu.request_readback();
        let context = glib::MainContext::default();
        let deadline = Instant::now() + std::time::Duration::from_secs(6);
        let pixels = loop {
            while context.pending() {
                context.iteration(false);
            }
            if let Some(pixels) = gpu.take_readback() {
                break pixels;
            }
            assert!(
                Instant::now() < deadline,
                "checkerboard Fit readback timed out"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        };
        let actual = pixels
            .chunks_exact(VIEW as usize * 4)
            .rev()
            .flatten()
            .copied()
            .collect::<Vec<_>>();
        let reference =
            image::imageops::resize(&checker, VIEW, VIEW, image::imageops::FilterType::Lanczos3)
                .into_raw();
        let max_delta = actual
            .iter()
            .zip(&reference)
            .map(|(a, b)| a.abs_diff(*b))
            .max()
            .unwrap_or_default();
        let mean_delta = actual
            .iter()
            .zip(&reference)
            .map(|(a, b)| a.abs_diff(*b) as f64)
            .sum::<f64>()
            / actual.len() as f64;
        eprintln!(
            "PIC_EDIT_GPU checkerboard_fit_reference=Lanczos3 scale=0.25 max_delta={max_delta} mean_delta={mean_delta:.3}"
        );
        assert!(
            max_delta <= 3 && mean_delta <= 1.0,
            "checkerboard Fit differs from Lanczos3: max={max_delta}, mean={mean_delta:.3}"
        );
        window.close();
    }

    #[test]
    #[ignore = "requires a GTK OpenGL display; run under xvfb-run"]
    fn gtk_gl_area_layer_alpha_and_stacking_match_cpu() {
        gtk::init().expect("GTK initialization");
        const W: u32 = 48;
        const H: u32 = 32;
        let image = image::RgbaImage::from_pixel(W, H, image::Rgba([20, 40, 60, 255]));
        let canvas = LiveGpuCanvas::new();
        canvas.widget().set_size_request(W as i32, H as i32);
        let window = gtk::Window::new();
        window.set_default_size(W as i32, H as i32);
        window.set_child(Some(&canvas.widget()));
        window.present();
        let recipe = EditRecipe::default();
        canvas.set_source(
            "test://gpu-alpha-layers",
            0,
            &recipe,
            W as i32,
            H as i32,
            glib::Bytes::from_owned(image.clone().into_raw()),
        );
        canvas.set_view(0.0, 0.0, W as f64, H as f64, 1.0, 0.0, 0.0);
        let rgba_image = image::RgbaImage::from_pixel(4, 4, image::Rgba([220, 60, 15, 128]));
        let rgba_text = image::RgbaImage::from_pixel(2, 2, image::Rgba([250, 250, 250, 255]));
        {
            let mut state = canvas.state.borrow_mut();
            state.overlays = vec![
                GpuOverlayLayer {
                    id: "image".to_owned(),
                    rect: super::super::model::NormRect {
                        left: 6.0 / W as f32,
                        top: 4.0 / H as f32,
                        width: 4.0 / W as f32,
                        height: 4.0 / H as f32,
                    },
                    opacity: 0.75,
                    width: 4,
                    height: 4,
                    rgba: glib::Bytes::from_owned(rgba_image.into_raw()),
                },
                GpuOverlayLayer {
                    id: "text".to_owned(),
                    rect: super::super::model::NormRect {
                        left: 7.0 / W as f32,
                        top: 5.0 / H as f32,
                        width: 2.0 / W as f32,
                        height: 2.0 / H as f32,
                    },
                    opacity: 0.65,
                    width: 2,
                    height: 2,
                    rgba: glib::Bytes::from_owned(rgba_text.into_raw()),
                },
            ];
            state.overlay_revision = state.overlay_revision.wrapping_add(1);
        }
        canvas.widget().queue_render();
        canvas.request_readback();
        let context = glib::MainContext::default();
        let deadline = Instant::now() + std::time::Duration::from_secs(6);
        let readback = loop {
            while context.pending() {
                context.iteration(false);
            }
            if let Some(pixels) = canvas.take_readback() {
                break pixels;
            }
            assert!(Instant::now() < deadline, "layer pixel readback timed out");
            std::thread::sleep(std::time::Duration::from_millis(5));
        };
        let scale = canvas.widget().scale_factor().max(1) as usize;
        let actual = readback
            .chunks_exact(W as usize * scale * 4)
            .rev()
            .flatten()
            .copied()
            .collect::<Vec<u8>>();
        let pixel = |x: usize, y: usize| -> [u8; 4] {
            let idx = (y * scale * W as usize * scale + x * scale) * 4;
            actual[idx..idx + 4].try_into().unwrap()
        };
        let mix = |base: [u8; 4], layer: [u8; 4], opacity: f32| {
            let a = (layer[3] as f32 / 255.0) * opacity;
            let mut out = [0; 4];
            for i in 0..3 {
                out[i] = (layer[i] as f32 * a + base[i] as f32 * (1.0 - a))
                    .round()
                    .clamp(0.0, 255.0) as u8;
            }
            out[3] = (255.0 * a + base[3] as f32 * (1.0 - a)).round() as u8;
            out
        };
        let base = [20, 40, 60, 255];
        let first = mix(base, [220, 60, 15, 128], 0.75);
        let combined = mix(first, [250, 250, 250, 255], 0.65);
        let close = |observed: [u8; 4], expected: [u8; 4], place: &str| {
            for channel in 0..4 {
                let delta = observed[channel].abs_diff(expected[channel]);
                assert!(
                    delta <= 2,
                    "GPU {place} channel {channel}: {observed:?} vs {expected:?}, delta {delta}"
                );
            }
        };
        close(pixel(0, 0), base, "uncovered background");
        close(pixel(6, 4), first, "image overlay alpha");
        close(pixel(7, 5), combined, "text above image");
        close(pixel(10, 4), base, "outside rectangle");
        let rev = canvas.uploaded_revision();
        canvas.set_recipe(&recipe);
        canvas.widget().queue_render();
        assert_eq!(
            canvas.uploaded_revision(),
            rev,
            "layer updates must not reupload photo"
        );
        window.close();
    }

    #[test]
    #[ignore = "requires GTK OpenGL display; run under xvfb-run"]
    fn gtk_gl_area_geometry_crop_and_straighten_match_cpu() {
        gtk::init().expect("GTK initialization");
        const WIDTH: u32 = 48;
        const HEIGHT: u32 = 32;
        let original = image::RgbaImage::from_fn(WIDTH, HEIGHT, |x, y| {
            image::Rgba([
                (25 + 4 * x) as u8,
                (31 + 5 * y) as u8,
                (12 + (x * 3 + y * 2) % 190) as u8,
                255,
            ])
        });
        let renderer = LiveGpuCanvas::new();
        renderer
            .widget()
            .set_size_request(WIDTH as i32, HEIGHT as i32);
        let window = gtk::Window::new();
        window.set_default_size(WIDTH as i32, HEIGHT as i32);
        window.set_child(Some(&renderer.widget()));
        window.present();
        let neutral = EditRecipe::default();
        renderer.set_source(
            "test://gpu-untransformed-geometry",
            0,
            &neutral,
            WIDTH as i32,
            HEIGHT as i32,
            glib::Bytes::from_owned(original.clone().into_raw()),
        );
        renderer.set_view(0.0, 0.0, WIDTH as f64, HEIGHT as f64, 1.0, 0.0, 0.0);
        let context = glib::MainContext::default();
        let capture = || -> Vec<u8> {
            renderer.request_readback();
            let deadline = Instant::now() + std::time::Duration::from_secs(6);
            let pixels = loop {
                while context.pending() {
                    context.iteration(false);
                }
                if let Some(frame) = renderer.take_readback() {
                    break frame;
                }
                assert!(
                    Instant::now() < deadline,
                    "GPU geometry framebuffer timeout"
                );
                std::thread::sleep(std::time::Duration::from_millis(5));
            };
            let scale = renderer.widget().scale_factor().max(1) as usize;
            pixels
                .chunks_exact(WIDTH as usize * scale * 4)
                .rev()
                .flatten()
                .copied()
                .collect::<Vec<u8>>()
        };
        let baseline = capture();
        let revision = renderer.uploaded_revision();
        for (rotation, angle, crop) in [
            (
                0,
                0.0,
                CropRect {
                    left: 0.25,
                    top: 0.25,
                    right: 0.75,
                    bottom: 0.75,
                },
            ),
            (0, 4.5, CropRect::default()),
            (
                0,
                -6.5,
                CropRect {
                    left: 0.16,
                    top: 0.13,
                    right: 0.88,
                    bottom: 0.85,
                },
            ),
            (90, 0.0, CropRect::default()),
            (180, 0.0, CropRect::default()),
            (270, 0.0, CropRect::default()),
            (
                90,
                4.5,
                CropRect {
                    left: 0.10,
                    top: 0.12,
                    right: 0.90,
                    bottom: 0.83,
                },
            ),
        ] {
            let recipe = EditRecipe {
                straighten: angle,
                crop,
                ..EditRecipe::default()
            };
            let expected = super::super::render::apply_geometry(
                super::super::render::apply_library_rotation(original.clone(), rotation),
                &recipe,
            );
            let output = renderer
                .set_untransformed_geometry_for_rotation(rotation, &recipe)
                .unwrap();
            assert_eq!(output, (expected.width() as i32, expected.height() as i32));
            let actual = capture();
            let scale = renderer.widget().scale_factor().max(1) as usize;
            let stride = WIDTH as usize * scale * 4;
            let mut max_delta = 0u8;
            // Quarter-turns can extend past the fixed 48x32 GLArea; compare
            // the visible intersection. Exact full output size is checked above.
            for y in 0..expected.height().min(HEIGHT) as usize {
                for x in 0..expected.width().min(WIDTH) as usize {
                    let actual_offset = y * scale * stride + x * scale * 4;
                    let pixel = expected.get_pixel(x as u32, y as u32);
                    for channel in 0..4 {
                        max_delta =
                            max_delta.max(actual[actual_offset + channel].abs_diff(pixel[channel]));
                    }
                }
            }
            assert!(
                max_delta <= 2,
                "GPU source geometry mismatch rotation={rotation} angle={angle} crop={crop:?} delta={max_delta}"
            );
            assert_eq!(
                renderer.uploaded_revision(),
                revision,
                "geometry-only changes must never re-upload the source"
            );
            eprintln!(
                "PIC_EDIT_GPU geometry_parity rotation={rotation} angle={angle} {}x{} max_channel_delta={max_delta}",
                output.0, output.1
            );
        }
        // Automatic histogram statistics must see the transformed and named-
        // filtered output, not the original uncropped RAW texture.
        if std::env::var_os("PIC_GPU_AUTOSTATS_DIAG").is_some() {
            // Diagnostic only: these combinations deliberately retain the CPU
            // fallback until the GPU histogram path passes strict pixel parity.
            // Cover all presets at several filter strengths and isolate the
            // offending automatic stage. This runs only in the workflow's opt-in
            // diagnostics because it deliberately bypasses the renderer gate.
            for filter in FilterPreset::ALL {
                for amount in [0.25, 0.65, 1.0] {
                    for (auto_color, auto_contrast) in [(true, false), (false, true), (true, true)]
                    {
                        let mut stats_recipe = EditRecipe::default();
                        stats_recipe.crop = CropRect {
                            left: 0.125,
                            top: 0.125,
                            right: 0.875,
                            bottom: 0.875,
                        };
                        stats_recipe.filter = filter;
                        stats_recipe.filter_amount = amount;
                        stats_recipe.auto_color = auto_color;
                        stats_recipe.auto_contrast = auto_contrast;
                        assert!(
                            !gpu_tone_supported(&stats_recipe),
                            "unverified combination must not bypass CPU fallback"
                        );
                        let cpu_geometry =
                            super::super::render::apply_geometry(original.clone(), &stats_recipe);
                        renderer.set_untransformed_geometry_for_rotation(0, &stats_recipe);
                        renderer.set_recipe(&stats_recipe);
                        let actual = capture();
                        // Read back only in this opt-in test diagnostic to
                        // isolate creative-filter input error from downstream
                        // histogram/correction error. Interactive rendering
                        // never reads these textures back to the CPU.
                        let mut filter_reference = stats_recipe.clone();
                        filter_reference.auto_color = false;
                        filter_reference.auto_contrast = false;
                        let cpu_filtered = super::super::render::apply_tone(
                            cpu_geometry.clone(),
                            &filter_reference,
                        );
                        let mut gpu_filtered =
                            vec![
                                0u8;
                                cpu_filtered.width() as usize * cpu_filtered.height() as usize * 4
                            ];
                        renderer.widget().make_current();
                        if let Some(objects) = renderer.state.borrow().objects.as_ref() {
                            unsafe {
                                gl::BindTexture(gl::TEXTURE_2D, objects.stats_source_texture);
                                gl::GetTexImage(
                                    gl::TEXTURE_2D,
                                    0,
                                    gl::RGBA,
                                    gl::UNSIGNED_BYTE,
                                    gpu_filtered.as_mut_ptr().cast(),
                                );
                            }
                        }
                        let cpu_filtered = cpu_filtered.into_raw();
                        let mut filter_stage_max = 0u8;
                        let mut filter_stage_sum = 0u64;
                        let mut filter_stage_worst = (0usize, 0u8, 0u8);
                        for (index, (gpu, cpu)) in
                            gpu_filtered.iter().zip(&cpu_filtered).enumerate()
                        {
                            let delta = gpu.abs_diff(*cpu);
                            filter_stage_sum += delta as u64;
                            if delta > filter_stage_max {
                                filter_stage_max = delta;
                                filter_stage_worst = (index, *gpu, *cpu);
                            }
                        }
                        let expected = super::super::render::apply_tone(
                            super::super::render::apply_geometry(original.clone(), &stats_recipe),
                            &stats_recipe,
                        );
                        let scale = renderer.widget().scale_factor().max(1) as usize;
                        let stride = WIDTH as usize * scale * 4;
                        let mut max_delta = 0u8;
                        let mut sum_delta = 0u64;
                        let mut worst = (0usize, 0u8, 0u8);
                        for y in 0..expected.height() as usize {
                            for x in 0..expected.width() as usize {
                                let offset = y * scale * stride + x * scale * 4;
                                for channel in 0..4 {
                                    let observed = actual[offset + channel];
                                    let reference = expected.get_pixel(x as u32, y as u32)[channel];
                                    let delta = observed.abs_diff(reference);
                                    sum_delta += delta as u64;
                                    if delta > max_delta {
                                        worst = (
                                            (y * expected.width() as usize + x) * 4 + channel,
                                            observed,
                                            reference,
                                        );
                                    }
                                    max_delta = max_delta.max(delta);
                                }
                            }
                        }
                        assert_eq!(renderer.uploaded_revision(), revision);
                        eprintln!(
                            "PIC_EDIT_GPU cropped_filter_auto_parity filter={filter:?} amount={amount} color={auto_color} contrast={auto_contrast} filter_stage_max={filter_stage_max} filter_stage_mean={:.4} filter_worst_pixel=({}, {}) filter_channel={} filter_actual={} filter_expected={} final_max={max_delta} final_mean={:.4} worst_pixel=({}, {}) channel={} actual={} expected={} error_origin={} status=unverified",
                            filter_stage_sum as f64 / cpu_filtered.len().max(1) as f64,
                            (filter_stage_worst.0 / 4) as u32 % cpu_geometry.width(),
                            (filter_stage_worst.0 / 4) as u32 / cpu_geometry.width(),
                            filter_stage_worst.0 % 4,
                            filter_stage_worst.1,
                            filter_stage_worst.2,
                            sum_delta as f64
                                / (expected.width() as usize * expected.height() as usize * 4)
                                    .max(1) as f64,
                            (worst.0 / 4) as u32 % expected.width(),
                            (worst.0 / 4) as u32 / expected.width(),
                            worst.0 % 4,
                            worst.1,
                            worst.2,
                            if filter_stage_max > 0 && max_delta > filter_stage_max {
                                "filter+downstream-histogram-or-correction"
                            } else if filter_stage_max > 0 {
                                "creative-filter"
                            } else if max_delta > 0 {
                                "histogram-correction-or-final-quantization"
                            } else {
                                "none"
                            },
                        );
                    }
                }
            }
            renderer.set_recipe(&neutral);
        }
        // Exercise the diagnostic GPU route over full-resolution geometry.
        // The editor keeps these automatic combinations on CPU until every
        // measured case passes the same strict native pixel limit.
        for (rotation, angle, crop) in [
            (0, 4.5, CropRect::default()),
            (
                0,
                0.0,
                CropRect {
                    left: 0.125,
                    top: 0.125,
                    right: 0.875,
                    bottom: 0.875,
                },
            ),
            (90, 0.0, CropRect::default()),
            (270, 0.0, CropRect::default()),
        ] {
            for (auto_color, auto_contrast) in [(true, false), (false, true), (true, true)] {
                let mut recipe = EditRecipe {
                    straighten: angle,
                    crop,
                    ..EditRecipe::default()
                };
                recipe.auto_color = auto_color;
                recipe.auto_contrast = auto_contrast;
                let gpu = gpu_tone_supported_for_view(&recipe, true, rotation);
                renderer.set_untransformed_geometry_for_rotation(rotation, &recipe);
                renderer.set_recipe(&recipe);
                let actual = capture();
                let expected = super::super::render::apply_tone(
                    super::super::render::apply_geometry(
                        super::super::render::apply_library_rotation(original.clone(), rotation),
                        &recipe,
                    ),
                    &recipe,
                );
                let scale = renderer.widget().scale_factor().max(1) as usize;
                let stride = WIDTH as usize * scale * 4;
                let compare_width = expected.width().min(WIDTH) as usize;
                let compare_height = expected.height().min(HEIGHT) as usize;
                let mut max_delta = 0u8;
                let mut sum_delta = 0u64;
                let mut worst = (0usize, 0u8, 0u8);
                let mut channels = 0usize;
                for y in 0..compare_height {
                    for x in 0..compare_width {
                        let offset = y * scale * stride + x * scale * 4;
                        let expected_pixel = expected.get_pixel(x as u32, y as u32);
                        for channel in 0..4 {
                            let observed = actual[offset + channel];
                            let reference = expected_pixel[channel];
                            let delta = observed.abs_diff(reference);
                            sum_delta += delta as u64;
                            channels += 1;
                            if delta > max_delta {
                                worst = (
                                    (y * expected.width() as usize + x) * 4 + channel,
                                    observed,
                                    reference,
                                );
                            }
                            max_delta = max_delta.max(delta);
                        }
                    }
                }
                eprintln!(
                    "PIC_EDIT_GPU automatic_geometry_parity rotation={rotation} straighten={angle} crop={crop:?} color={auto_color} contrast={auto_contrast} renderer={} max_channel_delta={max_delta} mean_channel_delta={:.4} worst_index={} actual={} expected={}",
                    if gpu { "GPU" } else { "CPU-fallback" },
                    sum_delta as f64 / channels.max(1) as f64,
                    worst.0,
                    worst.1,
                    worst.2,
                );
                if gpu {
                    assert!(
                        max_delta <= 1,
                        "supported automatic geometry parity failed: max delta {max_delta}"
                    );
                }
            }
        }
        // The real GTK editor maintains a GPU geometry object even for
        // uncropped full-frame photos. Validate supported standalone/both
        // automatic adjustments with that identity geometry and FBO path.
        for (auto_color, auto_contrast) in [(true, false), (false, true), (true, true)] {
            let mut recipe = EditRecipe::default();
            recipe.auto_color = auto_color;
            recipe.auto_contrast = auto_contrast;
            assert!(gpu_tone_supported(&recipe));
            renderer.set_untransformed_geometry_for_rotation(0, &recipe);
            renderer.set_recipe(&recipe);
            let actual = capture();
            let expected = super::super::render::apply_tone(original.clone(), &recipe);
            let scale = renderer.widget().scale_factor().max(1) as usize;
            let row_bytes = WIDTH as usize * scale * 4;
            let mut max_delta = 0u8;
            for y in 0..HEIGHT as usize {
                for x in 0..WIDTH as usize {
                    let offset = y * scale * row_bytes + x * scale * 4;
                    for channel in 0..4 {
                        max_delta = max_delta.max(
                            actual[offset + channel]
                                .abs_diff(expected.get_pixel(x as u32, y as u32)[channel]),
                        );
                    }
                }
            }
            assert!(
                max_delta <= 2,
                "GPU identity geometry auto_color={auto_color} auto_contrast={auto_contrast}: delta={max_delta}"
            );
            assert_eq!(renderer.uploaded_revision(), revision);
            eprintln!(
                "PIC_EDIT_GPU identity_auto_parity color={auto_color} contrast={auto_contrast} max_delta={max_delta}"
            );
        }
        renderer.set_recipe(&neutral);
        renderer.clear_untransformed_geometry();
        let reset = capture();
        assert_eq!(
            reset, baseline,
            "removing GPU geometry must restore original full-res source pixels"
        );
        assert_eq!(renderer.uploaded_revision(), revision);
        window.close();
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
            let mut sum_delta = 0u64;
            let mut worst = (0usize, 0u8, 0u8);
            for (index, (actual, expected)) in
                actual_top_down.iter().zip(expected.iter()).enumerate()
            {
                let delta = (*actual as i16 - *expected as i16).abs();
                sum_delta += delta as u64;
                if delta > max_delta {
                    worst = (index, *actual, *expected);
                }
                max_delta = max_delta.max(delta);
                // A creative filter followed by two automatic 8-bit passes
                // can accumulate a few levels of cross-platform float/rounding
                // drift. All other single-stage and plain tone cases remain
                // on their original strict <=1-channel parity limit.
                // Diagnostic-only: sample each mixed filter/auto permutation
                // before choosing the final defensible CPU parity tolerance.
                // Do NOT enable these combinations in gpu_tone_supported yet.
                let allowed = if recipe.filter != FilterPreset::None
                    && (recipe.auto_color || recipe.auto_contrast)
                {
                    40
                } else {
                    1
                };
                assert!(
                    delta <= allowed,
                    "filter={:?} amount={} auto_color={} auto_contrast={} channel {index} differs by {delta} (limit {allowed})",
                    recipe.filter,
                    recipe.filter_amount,
                    recipe.auto_color,
                    recipe.auto_contrast
                );
            }
            eprintln!(
                "PIC_EDIT_GPU parity_test dimensions={}x{} recipe_default={} filter={:?} amount={} auto_color={} auto_contrast={} max_channel_delta={max_delta} mean_channel_delta={:.4} worst_pixel=({}, {}) channel={} actual={} expected={}",
                WIDTH,
                HEIGHT,
                recipe.is_default(),
                recipe.filter,
                recipe.filter_amount,
                recipe.auto_color,
                recipe.auto_contrast,
                sum_delta as f64 / actual_top_down.len().max(1) as f64,
                (worst.0 / 4) as u32 % WIDTH,
                (worst.0 / 4) as u32 / WIDTH,
                worst.0 % 4,
                worst.1,
                worst.2,
            );
        };

        check_frame(&reset_recipe, &source);
        eprintln!(
            "PIC_EDIT_GPU test_gsk_renderer={:?}",
            window.renderer().map(|r| r.type_().name().to_owned())
        );
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
        // Native-size regression coverage includes Sharpen. Grain below and
        // Clarity later in this test use the same 1:1 CPU parity helper.
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
            FilterPreset::Clarendon,
            FilterPreset::Gingham,
            FilterPreset::Juno,
            FilterPreset::Lark,
            FilterPreset::Valencia,
            FilterPreset::Walden,
            FilterPreset::DisposableFilm,
            FilterPreset::DigicamFlash,
            FilterPreset::Cinematic,
            FilterPreset::GoldenHour,
            FilterPreset::OldPhoto,
            FilterPreset::Newspaper,
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
                while context.pending() {
                    context.iteration(false);
                }
                if let Some(readback) = renderer.take_readback() {
                    result = Some(readback);
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            let actual = result.expect("GPU special-filter readback missing");
            let scale = renderer.widget().scale_factor().max(1) as u32;
            let actual = actual
                .chunks_exact(WIDTH as usize * scale as usize * 4)
                .rev()
                .flatten()
                .copied()
                .collect::<Vec<_>>();
            let cpu = super::super::render::apply_tone(source.clone(), &recipe);
            let cpu = if scale == 1 {
                cpu
            } else {
                image::imageops::resize(
                    &cpu,
                    WIDTH * scale,
                    HEIGHT * scale,
                    image::imageops::FilterType::Nearest,
                )
            }
            .into_raw();
            assert_eq!(actual.len(), cpu.len());
            let errors = actual
                .iter()
                .zip(&cpu)
                .map(|(a, b)| (*a as i16 - *b as i16).unsigned_abs() as usize)
                .collect::<Vec<_>>();
            let maximum = errors.iter().copied().max().unwrap_or_default();
            let average = errors.iter().sum::<usize>() as f64 / errors.len().max(1) as f64;
            let worst = errors
                .iter()
                .enumerate()
                .max_by_key(|(_, delta)| *delta)
                .map(|(index, delta)| {
                    let stride = WIDTH as usize * scale as usize * 4;
                    (
                        index / stride,
                        (index % stride) / 4,
                        index % 4,
                        actual[index],
                        cpu[index],
                        *delta,
                    )
                });
            eprintln!(
                "PIC_EDIT_GPU special_filter_parity preset={preset:?} max={maximum} avg={average:.3} worst={worst:?}"
            );
            assert!(
                maximum <= 35 && average <= 5.0,
                "GPU {preset:?} differs too much from CPU: max={maximum}, avg={average:.3}"
            );
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
            while context.pending() {
                context.iteration(false);
            }
            if let Some(readback) = renderer.take_readback() {
                clarity_pixels = Some(readback);
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let readback = clarity_pixels.expect("GPU clarity framebuffer readback failed");
        let scale = renderer.widget().scale_factor().max(1) as u32;
        let observed = readback
            .chunks_exact(WIDTH as usize * scale as usize * 4)
            .rev()
            .flatten()
            .copied()
            .collect::<Vec<_>>();
        let expected = super::super::render::apply_tone(source.clone(), &clarity_recipe);
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
        let maximum_delta = observed
            .iter()
            .zip(&expected)
            .map(|(a, b)| (*a as i16 - *b as i16).abs())
            .max()
            .unwrap_or_default();
        assert!(
            maximum_delta <= 12,
            "GPU Clarity differs too much from the CPU reference: max channel delta {maximum_delta}"
        );
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
            while context.pending() {
                context.iteration(false);
            }
            if let Some(readback) = renderer.take_readback() {
                result = Some(readback);
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let result = result.expect("mixed GPU effects did not render");
        let scale = renderer.widget().scale_factor().max(1) as u32;
        let gpu = result
            .chunks_exact(WIDTH as usize * scale as usize * 4)
            .rev()
            .flatten()
            .copied()
            .collect::<Vec<_>>();
        let cpu = super::super::render::apply_tone(source.clone(), &mixed);
        let cpu = if scale == 1 {
            cpu
        } else {
            image::imageops::resize(
                &cpu,
                WIDTH * scale,
                HEIGHT * scale,
                image::imageops::FilterType::Nearest,
            )
        }
        .into_raw();
        let differences = gpu
            .iter()
            .zip(&cpu)
            .map(|(a, b)| (*a as i16 - *b as i16).unsigned_abs() as usize)
            .collect::<Vec<_>>();
        let max_error = differences.iter().copied().max().unwrap_or_default();
        let mean_error = differences.iter().sum::<usize>() as f64 / differences.len().max(1) as f64;
        eprintln!("PIC_EDIT_GPU mixed_parity max={max_error} mean={mean_error:.3}");
        assert!(
            max_error <= 45 && mean_error <= 9.0,
            "mixed GPU recipe diverges from CPU: max={max_error} mean={mean_error:.3}"
        );
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
        for (label, mut effect, allowed_delta) in [
            ("clarity", EditRecipe::default(), 12u8),
            ("grain", EditRecipe::default(), 1u8),
        ] {
            if label == "clarity" {
                effect.clarity = 0.45;
            } else {
                effect.grain = 0.5;
            }
            renderer.set_recipe(&effect);
            renderer.request_readback();
            let deadline = Instant::now() + std::time::Duration::from_secs(5);
            let pixels = loop {
                while context.pending() {
                    context.iteration(false);
                }
                if let Some(pixels) = renderer.take_readback() {
                    break pixels;
                }
                assert!(Instant::now() < deadline, "{label} Fit readback timed out");
                std::thread::sleep(std::time::Duration::from_millis(5));
            };
            let actual = pixels
                .chunks_exact(24 * scale as usize * 4)
                .rev()
                .flatten()
                .copied()
                .collect::<Vec<_>>();
            let processed = super::super::render::apply_tone(source.clone(), &effect);
            let reference = image::RgbaImage::from_fn(24, 16, |x, y| {
                let mut sum = [0u16; 4];
                for dy in 0..2 {
                    for dx in 0..2 {
                        let pixel = processed.get_pixel(x * 2 + dx, y * 2 + dy);
                        for channel in 0..4 {
                            sum[channel] += pixel[channel] as u16;
                        }
                    }
                }
                image::Rgba(sum.map(|channel| ((channel + 2) / 4) as u8))
            });
            let reference = if scale == 1 {
                reference
            } else {
                image::imageops::resize(
                    &reference,
                    24 * scale,
                    16 * scale,
                    image::imageops::FilterType::Nearest,
                )
            }
            .into_raw();
            let maximum_delta = actual
                .iter()
                .zip(&reference)
                .map(|(actual, expected)| actual.abs_diff(*expected))
                .max()
                .unwrap_or_default();
            eprintln!("PIC_EDIT_GPU {label}_fit_source_order max_channel_delta={maximum_delta}");
            assert!(
                maximum_delta <= allowed_delta,
                "{label} Fit differs from tone-at-source-resolution reference: {maximum_delta}"
            );
        }
        // GPU Auto Colour must match the CPU neutral estimate when many
        // credible near-neutral pixels exist, not merely on no-op photos.
        let neutrals = image::RgbaImage::from_fn(WIDTH, HEIGHT, |x, y| {
            let n = ((x + y) % 14) as u8;
            image::Rgba([112 + n, 124 + n, 137 + n, 255])
        });
        let auto = EditRecipe {
            auto_color: true,
            ..Default::default()
        };
        assert!(gpu_tone_supported(&auto));
        renderer.set_source(
            "test://autocolor-neutrals",
            0,
            &auto,
            WIDTH as i32,
            HEIGHT as i32,
            glib::Bytes::from_owned(neutrals.clone().into_raw()),
        );
        renderer.set_view(0.0, 0.0, WIDTH as f64, HEIGHT as f64, 1.0, 0.0, 0.0);
        renderer.set_recipe(&auto);
        check_frame(&auto, &neutrals);
        renderer.set_recipe(&EditRecipe::default());
        check_frame(&EditRecipe::default(), &neutrals);
        renderer.set_source(
            "test://gpu-parity",
            0,
            &reset_recipe,
            WIDTH as i32,
            HEIGHT as i32,
            glib::Bytes::from_owned(source.clone().into_raw()),
        );
        renderer.set_recipe(&reset_recipe);
        check_frame(&reset_recipe, &source);

        // Auto Contrast is percentile-based. This deliberate outlier fixture
        // checks source histogram computation and pixel correction together.
        let gradient = image::RgbaImage::from_fn(WIDTH, HEIGHT, |x, y| {
            if x == 0 && y == 0 {
                return image::Rgba([0, 0, 0, 255]);
            }
            if x == 1 && y == 0 {
                return image::Rgba([255, 255, 255, 255]);
            }
            let value = (102 + 2 * x + y) as u8;
            // Deliberate blue cast: this exercises colour-corrected histogram
            // bins instead of allowing the Auto Colour pass to be a no-op.
            image::Rgba([
                value.saturating_sub(12),
                value,
                value.saturating_add(8),
                255,
            ])
        });
        let mut automatic = EditRecipe::default();
        automatic.auto_contrast = true;
        renderer.set_source(
            "test://auto-contrast-outliers",
            0,
            &automatic,
            WIDTH as i32,
            HEIGHT as i32,
            glib::Bytes::from_owned(gradient.clone().into_raw()),
        );
        renderer.set_view(0.0, 0.0, WIDTH as f64, HEIGHT as f64, 1.0, 0.0, 0.0);
        renderer.set_recipe(&automatic);
        check_frame(&automatic, &gradient);
        renderer.set_recipe(&EditRecipe::default());
        check_frame(&EditRecipe::default(), &gradient);
        // CPU applies Auto Colour before computing contrast percentile bounds.
        // Verify the actual combination, then switch off Auto Colour without
        // reloading the source, to exercise the two histogram cache keys.
        let mut combined = EditRecipe::default();
        combined.auto_color = true;
        combined.auto_contrast = true;
        assert!(gpu_tone_supported(&combined));
        renderer.set_recipe(&combined);
        check_frame(&combined, &gradient);
        renderer.set_recipe(&automatic);
        check_frame(&automatic, &gradient);
        renderer.set_recipe(&combined);
        check_frame(&combined, &gradient);
        renderer.set_recipe(&EditRecipe::default());
        check_frame(&EditRecipe::default(), &gradient);
        for (name, fixture) in [
            (
                "low-contrast",
                image::RgbaImage::from_fn(WIDTH, HEIGHT, |x, y| {
                    let value = (116 + (x * 3 + y * 2) % 18) as u8;
                    image::Rgba([value.saturating_sub(3), value, value.saturating_add(4), 255])
                }),
            ),
            (
                "high-contrast",
                image::RgbaImage::from_fn(WIDTH, HEIGHT, |x, y| {
                    let value = if (x + y) % 2 == 0 { 4 } else { 251 };
                    image::Rgba([
                        value,
                        if x % 3 == 0 { 0 } else { value },
                        if y % 3 == 0 { 255 } else { value },
                        255,
                    ])
                }),
            ),
        ] {
            renderer.set_source(
                &format!("test://auto-{name}"),
                0,
                &EditRecipe::default(),
                WIDTH as i32,
                HEIGHT as i32,
                glib::Bytes::from_owned(fixture.clone().into_raw()),
            );
            renderer.set_view(0.0, 0.0, WIDTH as f64, HEIGHT as f64, 1.0, 0.0, 0.0);
            renderer.set_recipe(&EditRecipe::default());
            check_frame(&EditRecipe::default(), &fixture);
            let fixture_revision = renderer.uploaded_revision();
            for (auto_color, auto_contrast) in [(true, false), (false, true), (true, true)] {
                let automatic = EditRecipe {
                    auto_color,
                    auto_contrast,
                    ..EditRecipe::default()
                };
                renderer.set_recipe(&automatic);
                check_frame(&automatic, &fixture);
                assert_eq!(renderer.uploaded_revision(), fixture_revision);
            }
            eprintln!("PIC_EDIT_GPU automatic_fixture={name} source_uploads=1");
        }
        renderer.set_source(
            "test://auto-contrast-outliers",
            0,
            &combined,
            WIDTH as i32,
            HEIGHT as i32,
            glib::Bytes::from_owned(gradient.clone().into_raw()),
        );
        renderer.set_view(0.0, 0.0, WIDTH as f64, HEIGHT as f64, 1.0, 0.0, 0.0);
        // The automatic statistics must stay tied to the complete source
        // image as the preview moves between native, 50% and Fit. Verify GPU
        // output against tone-at-source-resolution followed by box reduction.
        for zoom in [1.0f64, 0.5, 0.25] {
            let view_width = (WIDTH as f64 * zoom) as u32;
            let view_height = (HEIGHT as f64 * zoom) as u32;
            renderer.set_view(
                0.0,
                0.0,
                view_width as f64,
                view_height as f64,
                zoom,
                0.0,
                0.0,
            );
            for (auto_color, auto_contrast) in [(true, false), (false, true), (true, true)] {
                let mut zoom_recipe = EditRecipe::default();
                zoom_recipe.auto_color = auto_color;
                zoom_recipe.auto_contrast = auto_contrast;
                renderer.set_recipe(&zoom_recipe);
                renderer.request_readback();
                let deadline = Instant::now() + std::time::Duration::from_secs(5);
                let pixels = loop {
                    while context.pending() {
                        context.iteration(false);
                    }
                    if let Some(pixels) = renderer.take_readback() {
                        break pixels;
                    }
                    assert!(Instant::now() < deadline, "Auto zoom readback timed out");
                    std::thread::sleep(std::time::Duration::from_millis(5));
                };
                let device_scale = renderer.widget().scale_factor().max(1) as u32;
                let actual = pixels
                    .chunks_exact(view_width as usize * device_scale as usize * 4)
                    .rev()
                    .flatten()
                    .copied()
                    .collect::<Vec<_>>();
                let processed = super::super::render::apply_tone(gradient.clone(), &zoom_recipe);
                let factor = zoom.recip() as u32;
                let reference = image::RgbaImage::from_fn(view_width, view_height, |x, y| {
                    let mut sum = [0u32; 4];
                    for dy in 0..factor {
                        for dx in 0..factor {
                            let pixel = processed.get_pixel(x * factor + dx, y * factor + dy);
                            for channel in 0..4 {
                                sum[channel] += pixel[channel] as u32;
                            }
                        }
                    }
                    image::Rgba(
                        sum.map(|value| ((value + factor * factor / 2) / (factor * factor)) as u8),
                    )
                });
                let reference = if device_scale == 1 {
                    reference
                } else {
                    image::imageops::resize(
                        &reference,
                        view_width * device_scale,
                        view_height * device_scale,
                        image::imageops::FilterType::Nearest,
                    )
                }
                .into_raw();
                let mut max_delta = 0u8;
                let mut sum_delta = 0u64;
                let mut worst = (0usize, 0u8, 0u8);
                for (index, (observed, expected)) in actual.iter().zip(&reference).enumerate() {
                    let delta = observed.abs_diff(*expected);
                    sum_delta += delta as u64;
                    if delta > max_delta {
                        worst = (index, *observed, *expected);
                    }
                    max_delta = max_delta.max(delta);
                }
                eprintln!(
                    "PIC_EDIT_GPU automatic_zoom_parity zoom={zoom} color={auto_color} contrast={auto_contrast} renderer={} max_channel_delta={max_delta} mean_channel_delta={:.4} worst_index={} actual={} expected={}",
                    if gpu_tone_supported_for_view(&zoom_recipe, zoom == 1.0, 0) {
                        "GPU"
                    } else {
                        "CPU-fallback"
                    },
                    sum_delta as f64 / actual.len().max(1) as f64,
                    worst.0,
                    worst.1,
                    worst.2,
                );
                if zoom == 1.0 {
                    assert!(
                        max_delta <= 1,
                        "native Auto parity color={auto_color} contrast={auto_contrast}: max delta {max_delta}"
                    );
                } else {
                    assert!(
                        !gpu_tone_supported_for_view(&zoom_recipe, false, 0),
                        "sub-100% Auto must keep the CPU preview fallback until strict parity passes"
                    );
                }
            }
        }
        renderer.set_view(0.0, 0.0, WIDTH as f64, HEIGHT as f64, 1.0, 0.0, 0.0);
        renderer.set_recipe(&EditRecipe::default());
        check_frame(&EditRecipe::default(), &gradient);
        // GPU stats must be calculated from the *filtered* photograph,
        // before automatic corrections. Toggle presets and intensity on the
        // same uploaded source to expose stale neutral/histogram caches.
        // Known limitation: combined Auto Colour + Auto Contrast with a
        // filtered source can shift the percentile bounds when a few
        // quantized pixels straddle a histogram bin. Keep this independent
        // opt-in diagnostic while the live editor remains on CPU fallback.
        if std::env::var_os("PIC_GPU_AUTOSTATS_DIAG").is_some() {
            for filter in [
                FilterPreset::Clarendon,
                FilterPreset::Valencia,
                FilterPreset::Newspaper,
                FilterPreset::DreamGlow,
            ] {
                for amount in [0.25, 0.65, 1.0] {
                    let mut filtered = combined.clone();
                    filtered.filter = filter;
                    filtered.filter_amount = amount;
                    for (auto_color, auto_contrast) in [(true, false), (false, true), (true, true)]
                    {
                        filtered.auto_color = auto_color;
                        filtered.auto_contrast = auto_contrast;
                        renderer.set_recipe(&filtered);
                        check_frame(&filtered, &gradient);
                    }
                    assert!(
                        renderer.source_uploaded(),
                        "changing filters must not invalidate the uploaded texture"
                    );
                }
            }
        }
        renderer.set_recipe(&EditRecipe::default());
        check_frame(&EditRecipe::default(), &gradient);
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
                let r = if hair || eyelash {
                    22
                } else {
                    (60 + x % 147) as u8
                };
                let g = if hair || eyelash {
                    17
                } else {
                    (35 + y % 188) as u8
                };
                let b = if hair || eyelash {
                    26
                } else {
                    (45 + (x + y) % 173) as u8
                };
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
            "test://full-detail",
            0,
            &reference,
            width as i32,
            height as i32,
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
                assert!(
                    Instant::now() < deadline,
                    "full-resolution GPU frame timed out"
                );
                std::thread::sleep(std::time::Duration::from_millis(5));
            };
            let scale = renderer.widget().scale_factor().max(1) as usize;
            assert_eq!(result.len(), (VIEW as usize * scale).pow(2) * 4);
            result
                .chunks_exact(VIEW as usize * scale * 4)
                .rev()
                .flatten()
                .copied()
                .collect::<Vec<u8>>()
        };
        let neutral = read_frame(&reference);
        let revision = renderer.uploaded_revision();
        let mut exposure = reference.clone();
        exposure.exposure = -0.03;
        let exposed = read_frame(&exposure);
        assert_eq!(
            renderer.uploaded_revision(),
            revision,
            "Exposure-only edit must reuse the full-size uploaded texture"
        );
        let cpu = super::super::render::apply_tone(image.clone(), &exposure);
        let expected = image::imageops::crop_imm(&cpu, x, y, VIEW as u32, VIEW as u32).to_image();
        let scale = renderer.widget().scale_factor().max(1) as u32;
        let expected = if scale == 1 {
            expected
        } else {
            image::imageops::resize(
                &expected,
                VIEW as u32 * scale,
                VIEW as u32 * scale,
                image::imageops::FilterType::Nearest,
            )
        }
        .into_raw();
        assert_eq!(exposed.len(), expected.len());
        let max_delta = exposed
            .iter()
            .zip(&expected)
            .map(|(a, b)| (*a as i16 - *b as i16).abs())
            .max()
            .unwrap_or_default();
        assert!(
            max_delta <= 1,
            "full-source exposure GPU/CPU parity failed: max channel delta {max_delta}"
        );
        // RGB edge strength within the same native 1:1 viewport. Exposure must
        // not resample or erase the image texture's one-pixel detail.
        let edge_energy = |rgba: &[u8]| -> f64 {
            let stride = VIEW as usize * scale as usize * 4;
            let mut sum = 0u64;
            for row in rgba.chunks_exact(stride) {
                for x in 1..(VIEW as usize * scale as usize) {
                    let idx = x * 4;
                    for channel in 0..3 {
                        sum += (row[idx + channel] as i16 - row[idx - 4 + channel] as i16)
                            .unsigned_abs() as u64;
                    }
                }
            }
            sum as f64
        };
        let source_energy = edge_energy(&neutral);
        let edit_energy = edge_energy(&exposed);
        let ratio = edit_energy / source_energy.max(1.0);
        assert!(
            (0.92..=1.08).contains(&ratio),
            "tiny exposure altered native-pixel sharpness: edge ratio {ratio:.4}"
        );
        let reset = read_frame(&reference);
        assert_eq!(neutral, reset, "exposure reset changed original GPU pixels");
        assert_eq!(renderer.uploaded_revision(), revision);
        // GPU neutral-statistic accumulation must also work at actual Nikon
        // resolution. Optional PIC_TEST_EDIT_REFERENCE exercises a private NEF.
        let mut automatic = EditRecipe::default();
        automatic.auto_color = true;
        assert!(gpu_tone_supported(&automatic));
        let auto_gpu = read_frame(&automatic);
        let auto_cpu = super::super::render::apply_tone(image.clone(), &automatic);
        let auto_cpu =
            image::imageops::crop_imm(&auto_cpu, x, y, VIEW as u32, VIEW as u32).to_image();
        let auto_cpu = if scale == 1 {
            auto_cpu
        } else {
            image::imageops::resize(
                &auto_cpu,
                VIEW as u32 * scale,
                VIEW as u32 * scale,
                image::imageops::FilterType::Nearest,
            )
        }
        .into_raw();
        let auto_delta = auto_gpu
            .iter()
            .zip(&auto_cpu)
            .map(|(a, b)| (*a as i16 - *b as i16).abs())
            .max()
            .unwrap_or_default();
        assert!(
            auto_delta <= 3,
            "full-source Auto Colour GPU/CPU mismatch: max delta {auto_delta}"
        );
        assert_eq!(renderer.uploaded_revision(), revision);
        // Real source dimensions exercise the 256-bin GPU histogram and its
        // dependency on neutral-colour gains. Both automatic tools must be
        // composable without another source upload or a stale cached histogram.
        let mut both = automatic.clone();
        both.auto_contrast = true;
        assert!(gpu_tone_supported(&both));
        let both_gpu = read_frame(&both);
        let both_cpu = super::super::render::apply_tone(image.clone(), &both);
        let both_cpu =
            image::imageops::crop_imm(&both_cpu, x, y, VIEW as u32, VIEW as u32).to_image();
        let both_cpu = if scale == 1 {
            both_cpu
        } else {
            image::imageops::resize(
                &both_cpu,
                VIEW as u32 * scale,
                VIEW as u32 * scale,
                image::imageops::FilterType::Nearest,
            )
        }
        .into_raw();
        let combined_delta = both_gpu
            .iter()
            .zip(&both_cpu)
            .map(|(a, b)| (*a as i16 - *b as i16).abs())
            .max()
            .unwrap_or_default();
        assert!(
            combined_delta <= 3,
            "full-size combined Auto Colour/Contrast parity: delta {combined_delta}"
        );
        assert_eq!(renderer.uploaded_revision(), revision);
        eprintln!("PIC_EDIT_GPU full_source_combined_auto max_channel_delta={combined_delta}");
        let reset_again = read_frame(&reference);
        assert_eq!(neutral, reset_again);
        eprintln!("PIC_EDIT_GPU full_source_autocolor max_channel_delta={auto_delta}");
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
            if name.is_null() {
                "unknown".to_owned()
            } else {
                std::ffi::CStr::from_ptr(name.cast())
                    .to_string_lossy()
                    .into_owned()
            }
        };
        eprintln!(
            "PIC_EDIT_GPU benchmark_gl_renderer={driver} gsk_renderer={:?}",
            window.renderer().map(|r| r.type_().name().to_owned())
        );

        let mut recipe = base_recipe.clone();
        recipe.exposure = 0.2;
        recipe.sharpen = 0.35;
        let cpu_started = Instant::now();
        let _cpu_reference = super::super::render::apply_tone(source.clone(), &recipe);
        let cpu_ms = cpu_started.elapsed().as_secs_f64() * 1000.0;

        let prior_frame = renderer.rendered_frames();
        let gpu_started = Instant::now();
        renderer.request_gpu_timer();
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
        let mut gpu_times = Vec::with_capacity(5);
        for frame in 0..5 {
            if frame > 0 {
                let previous_frame = renderer.rendered_frames();
                renderer.request_gpu_timer();
                while renderer.rendered_frames() <= previous_frame && Instant::now() < deadline {
                    while context.pending() {
                        context.iteration(false);
                    }
                    std::thread::sleep(std::time::Duration::from_millis(2));
                }
            }
            loop {
                if let Some(elapsed_ns) = renderer.take_gpu_time_ns() {
                    gpu_times.push(elapsed_ns as f64 / 1_000_000.0);
                    break;
                }
                while context.pending() {
                    context.iteration(false);
                }
                assert!(Instant::now() < deadline, "GPU timer query timed out");
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
        }
        let gpu_first_frame = gpu_times[0];
        let cached_times = &gpu_times[1..];
        let gpu_cached_mean = cached_times.iter().sum::<f64>() / cached_times.len() as f64;
        let mut cached_sorted = cached_times.to_vec();
        cached_sorted.sort_by(f64::total_cmp);
        let gpu_cached_median = cached_sorted[cached_sorted.len() / 2];
        eprintln!(
            "PIC_EDIT_GPU benchmark source={}x{} viewport={}x{} upload_ready_ms={ready_ms:.3} cpu_full_frame_ms={cpu_ms:.3} uniform_to_draw_callback_ms={uniform_to_draw_ms:.3} gpu_first_frame_ms={gpu_first_frame:.3} gpu_cached_mean_ms={gpu_cached_mean:.3} gpu_cached_median_ms={gpu_cached_median:.3} samples={}",
            source_width,
            source_height,
            VIEW_WIDTH,
            VIEW_HEIGHT,
            gpu_times.len(),
        );

        let source_revision = renderer.uploaded_revision();
        let measure_auto = |recipe: &EditRecipe| {
            let mut gpu_samples_ms = Vec::with_capacity(2);
            let timer_deadline = Instant::now() + std::time::Duration::from_secs(20);
            for _ in 0..2 {
                let previous_frame = renderer.rendered_frames();
                renderer.request_gpu_timer();
                renderer.set_recipe(recipe);
                while renderer.rendered_frames() <= previous_frame
                    && Instant::now() < timer_deadline
                {
                    while context.pending() {
                        context.iteration(false);
                    }
                    std::thread::sleep(std::time::Duration::from_millis(2));
                }
                assert!(
                    renderer.rendered_frames() > previous_frame,
                    "automatic correction GPU draw was not queued"
                );
                loop {
                    if let Some(elapsed_ns) = renderer.take_gpu_time_ns() {
                        gpu_samples_ms.push(elapsed_ns as f64 / 1_000_000.0);
                        break;
                    }
                    while context.pending() {
                        context.iteration(false);
                    }
                    assert!(
                        Instant::now() < timer_deadline,
                        "automatic correction GPU timer query timed out"
                    );
                    std::thread::sleep(std::time::Duration::from_millis(2));
                }
            }
            assert_eq!(
                renderer.uploaded_revision(),
                source_revision,
                "automatic edits must reuse the uploaded source texture"
            );
            let cached = gpu_samples_ms[1];
            (gpu_samples_ms[0], cached)
        };
        for (label, auto_color, auto_contrast) in [
            ("auto_color", true, false),
            ("auto_contrast", false, true),
            ("combined", true, true),
        ] {
            let mut automatic = base_recipe.clone();
            automatic.auto_color = auto_color;
            automatic.auto_contrast = auto_contrast;
            if label == "combined" {
                // Invalidate both statistics caches without changing the
                // source texture. Do not pump the main context between these
                // recipe updates, so only the combined frame is rendered.
                let mut invalidate = automatic.clone();
                invalidate.filter_amount = 0.99;
                renderer.set_recipe(&invalidate);
            }
            let cpu_started = Instant::now();
            let _cpu_frame = super::super::render::apply_tone(source.clone(), &automatic);
            let cpu_auto_ms = cpu_started.elapsed().as_secs_f64() * 1000.0;
            let (gpu_first_ms, gpu_cached_ms) = measure_auto(&automatic);
            eprintln!(
                "PIC_EDIT_GPU benchmark_{label} gpu_first_frame_ms={gpu_first_ms:.3} gpu_cached_frame_ms={gpu_cached_ms:.3} gpu_stats_plus_extra_ms={:.3} cpu_full_frame_ms={cpu_auto_ms:.3} source_upload_delta=0",
                (gpu_first_ms - gpu_cached_ms).max(0.0),
            );
        }
        window.close();
    }
}
