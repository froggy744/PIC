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

use super::filters::FilterPreset;
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
    frame_after_paint: Option<(Instant, Option<f64>)>,
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
                        gl::DeleteVertexArrays(1, &objects.vao);
                        gl::DeleteProgram(objects.program);
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
                        if let Some((rendered_at, queue_ms)) = frame {
                            eprintln!(
                                "PIC_EDIT_GPU frame_present_after_paint_ms={:.3} queue_to_after_paint_ms={}",
                                rendered_at.elapsed().as_secs_f64() * 1000.0,
                                queue_ms.map_or_else(|| "none".to_owned(), |ms| format!("{ms:.3}")),
                            );
                        }
                    }
                });
            }
        });
        area.connect_render(move |area, _context| {
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
            let (upload_trace, queue_ms, gl_error) = {
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
                let queue_ms = state
                    .last_queued
                    .take()
                    .map(|queued| queued.elapsed().as_secs_f64() * 1000.0);
                state.frame_after_paint = Some((Instant::now(), queue_ms));
                state.rendered_frames = state.rendered_frames.wrapping_add(1);
                #[cfg(test)]
                if let Some(pixels) = readback {
                    state.readback_pixels = Some(pixels);
                }
                let gl_error = unsafe { gl::GetError() };
                (upload_trace, queue_ms, gl_error)
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
                    "PIC_EDIT_GPU gpu_draw_submit_ms={:.3} viewport={}x{} framebuffer={}x{} scale_factor={} queue_to_render_ms={}",
                    draw_started.elapsed().as_secs_f64() * 1000.0,
                    view_width,
                    view_height,
                    view_width * device_scale,
                    view_height * device_scale,
                    device_scale,
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
        let next = GpuTone {
            exposure: recipe.exposure,
            contrast: recipe.contrast,
            saturation: recipe.saturation,
            temperature: recipe.temperature,
            fill_light: recipe.fill_light,
            shadows: recipe.shadows,
            highlights: recipe.highlights,
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
    recipe.filter == FilterPreset::None
        && !recipe.auto_color
        && !recipe.auto_contrast
        && !recipe.black_white
        && !recipe.sepia
        && recipe.clarity == 0.0
        && recipe.sharpen == 0.0
        && recipe.grain == 0.0
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
void main() {
    vec2 pixel = gl_FragCoord.xy / max(uDeviceScale, 1.0);
    vec2 topLeft = vec2(pixel.x, uViewSize.y - pixel.y);
    vec2 sourcePos = (topLeft + uScroll - uPadding) / max(uScale, 0.0001);
    if (sourcePos.x < 0.0 || sourcePos.y < 0.0 ||
        sourcePos.x >= uSourceSize.x || sourcePos.y >= uSourceSize.y) {
        color = vec4(0.055, 0.06, 0.07, 1.0);
        return;
    }
    vec2 uv = vec2(sourcePos.x / uSourceSize.x,
                   sourcePos.y / uSourceSize.y);
    vec4 px = texture(uSource, uv);
    vec3 c = px.rgb * exp2(uExposure);
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
    color = vec4(clamp(c, 0.0, 1.0), px.a);
}"#,
    )?;

    let program;
    let mut vao = 0;
    let mut texture = 0;
    unsafe {
        program = gl::CreateProgram();
        gl::AttachShader(program, vertex);
        gl::AttachShader(program, fragment);
        gl::LinkProgram(program);
        let mut linked = 0;
        gl::GetProgramiv(program, gl::LINK_STATUS, &mut linked);
        gl::DeleteShader(vertex);
        gl::DeleteShader(fragment);
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
    })
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
        assert_eq!(base.crop, recipe.crop);
    }

    #[test]
    fn gpu_support_is_limited_to_the_initial_four_adjustments() {
        let recipe = EditRecipe::default();
        assert!(gpu_tone_supported(&recipe));
        let mut unsupported = recipe;
        unsupported.clarity = 0.1;
        assert!(!gpu_tone_supported(&unsupported));
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
        let uploaded_revision = renderer.uploaded_revision();
        let mut edited = reset_recipe.clone();
        edited.exposure = 0.17;
        edited.contrast = 0.23;
        edited.saturation = -0.19;
        edited.temperature = 0.31;
        edited.fill_light = 0.2;
        edited.shadows = -0.15;
        edited.highlights = 0.1;
        renderer.set_recipe(&edited);
        check_frame(&edited, &source);
        assert_eq!(renderer.uploaded_revision(), uploaded_revision);
        renderer.set_recipe(&reset_recipe);
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
        let source = image::RgbaImage::from_fn(SOURCE_WIDTH, SOURCE_HEIGHT, |x, y| {
            image::Rgba([(x % 251) as u8, (y % 239) as u8, ((x + y) % 233) as u8, 255])
        });
        let base_recipe = EditRecipe::default();
        let upload_started = Instant::now();
        renderer.set_source(
            "test://24mp-benchmark",
            0,
            &base_recipe,
            SOURCE_WIDTH as i32,
            SOURCE_HEIGHT as i32,
            glib::Bytes::from_owned(source.clone().into_raw()),
        );
        renderer.set_view(
            0.0,
            0.0,
            VIEW_WIDTH as f64,
            VIEW_HEIGHT as f64,
            VIEW_WIDTH as f64 / SOURCE_WIDTH as f64,
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
            "24MP source texture upload failed"
        );
        let ready_ms = upload_started.elapsed().as_secs_f64() * 1000.0;

        let mut recipe = base_recipe.clone();
        recipe.exposure = 0.2;
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
            SOURCE_WIDTH, SOURCE_HEIGHT, VIEW_WIDTH, VIEW_HEIGHT,
        );
        window.close();
    }
}
