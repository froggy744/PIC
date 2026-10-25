//! OpenGL perspective Cover Flow shared by albums and the photo screensaver.
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::ffi::{c_void, CString};
use std::rc::Rc;
use std::sync::OnceLock;

use gtk::prelude::*;
use gtk::{gdk, gsk};
use gtk4 as gtk;
use libloading::Library;

pub(crate) const VISIBLE_RADIUS: usize = 4;
const SIDE_ANGLE: f64 = 60.0_f64.to_radians();

#[derive(Clone, Copy, Debug)]
pub(crate) struct CoverflowPose {
    /// Projected bounds for pointer hit testing, in logical pixels.
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    /// Model-space values consumed by the perspective vertex shader.
    pub center_x: f64,
    pub center_y: f64,
    pub center_z: f64,
    pub card_width: f64,
    pub card_height: f64,
    pub rotation_y: f64,
    pub camera_distance: f64,
    pub opacity: f64,
}

/// Positions full-height cards on a perspective plane. Side-card narrowing is
/// produced by the Y rotation and camera projection, never by a 2D shear.
pub(crate) fn coverflow_pose(
    index: usize,
    focus: f64,
    count: usize,
    viewport: (f64, f64),
) -> Option<CoverflowPose> {
    let (vw, vh) = viewport;
    if count == 0 || index >= count || vw <= 0.0 || vh <= 0.0 {
        return None;
    }
    let offset = index as f64 - focus.clamp(0.0, (count - 1) as f64);
    let distance = offset.abs();
    if distance > VISIBLE_RADIUS as f64 + 0.5 {
        return None;
    }

    let card_height = (vh * 0.58).min(vw * 0.40).max(1.0);
    let card_width = card_height;
    let side_distance = (distance - 1.0).max(0.0);
    let lateral = if distance <= 1.0 {
        offset * 0.72 * card_width
    } else {
        offset.signum() * (0.72 + side_distance * 0.25) * card_width
    };
    let rotation_y = -offset.signum() * SIDE_ANGLE * distance.min(1.0);
    let center_y = vh * 0.36;
    let center_z = -side_distance * card_height * 0.22;
    let camera_distance = vh * 3.4;
    let opacity = (1.0 - distance * 0.07).clamp(0.68, 1.0);
    let mut bounds = (
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    );
    for (local_x, local_y) in [
        (-card_width * 0.5, -card_height * 0.5),
        (card_width * 0.5, -card_height * 0.5),
        (-card_width * 0.5, card_height * 0.5),
        (card_width * 0.5, card_height * 0.5),
    ] {
        let rotated_x = local_x * rotation_y.cos();
        let rotated_z = center_z - local_x * rotation_y.sin();
        let perspective = camera_distance / (camera_distance - rotated_z).max(1.0);
        let screen_x = vw * 0.5 + (lateral + rotated_x) * perspective;
        let screen_y = center_y - local_y * perspective;
        bounds.0 = bounds.0.min(screen_x);
        bounds.1 = bounds.1.min(screen_y);
        bounds.2 = bounds.2.max(screen_x);
        bounds.3 = bounds.3.max(screen_y);
    }
    Some(CoverflowPose {
        x: bounds.0,
        y: bounds.1,
        width: bounds.2 - bounds.0,
        height: bounds.3 - bounds.1,
        center_x: lateral,
        center_y,
        center_z,
        card_width,
        card_height,
        rotation_y,
        camera_distance,
        opacity,
    })
}

#[derive(Clone)]
pub(crate) struct CoverflowItem {
    pub index: usize,
    pub revision: u64,
    pub node: gsk::RenderNode,
}

#[derive(Default)]
struct Scene {
    count: usize,
    focus: f64,
    target: f64,
    last_tick: i64,
    animating: bool,
    items: Vec<CoverflowItem>,
    resources: Option<GlResources>,
    error: Option<String>,
}

struct UploadedTexture {
    revision: u64,
    texture: u32,
}

struct GlResources {
    program: u32,
    vao: u32,
    textures: HashMap<usize, UploadedTexture>,
}

/// A GTK GLArea whose items are GSK scene nodes rasterized once to textures,
/// then projected as real 3D quads. Scene updates retain the bounded neighbor
/// set and GL texture cache; focus animation only changes model uniforms.
pub(crate) struct CoverflowGlArea {
    area: gtk::GLArea,
    scene: Rc<RefCell<Scene>>,
}

impl CoverflowGlArea {
    pub(crate) fn new(css_class: &str, height_request: i32) -> Self {
        let area = gtk::GLArea::new();
        area.set_allowed_apis(gdk::GLAPI::GL);
        area.set_required_version(3, 2);
        area.set_has_depth_buffer(false);
        area.set_has_stencil_buffer(false);
        area.set_auto_render(true);
        area.set_hexpand(true);
        area.set_vexpand(true);
        area.set_size_request(-1, height_request);
        area.set_can_target(false);
        area.add_css_class(css_class);
        let scene = Rc::new(RefCell::new(Scene::default()));

        let draw_scene = scene.clone();
        area.connect_render(move |area, _| {
            let mut scene = draw_scene.borrow_mut();
            if scene.resources.is_none() {
                match initialize_gl() {
                    Ok(resources) => scene.resources = Some(resources),
                    Err(error) => {
                        if scene.error.as_ref() != Some(&error) {
                            eprintln!("PIC Cover Flow OpenGL initialization failed: {error}");
                            scene.error = Some(error);
                        }
                        return glib::Propagation::Stop;
                    }
                }
            }
            if let Err(error) = draw_frame(area, &mut scene) {
                if scene.error.as_ref() != Some(&error) {
                    eprintln!("PIC Cover Flow OpenGL frame failed: {error}");
                    scene.error = Some(error);
                }
            }
            glib::Propagation::Stop
        });

        let release_scene = scene.clone();
        area.connect_unrealize(move |area| {
            area.make_current();
            if area.error().is_none() {
                if let Some(resources) = release_scene.borrow_mut().resources.take() {
                    unsafe {
                        delete_resources(resources);
                    }
                }
            }
        });

        Self { area, scene }
    }

    pub(crate) fn widget(&self) -> &gtk::GLArea {
        &self.area
    }

    pub(crate) fn set_scene(
        &self,
        count: usize,
        focus: f64,
        items: Vec<CoverflowItem>,
        animate: bool,
    ) {
        let mut scene = self.scene.borrow_mut();
        let first = scene.count == 0;
        scene.count = count;
        scene.items = items;
        scene.target = focus.clamp(0.0, count.saturating_sub(1) as f64);
        scene.error = None;
        if first || !animate || !self.area.settings().is_gtk_enable_animations() {
            scene.focus = scene.target;
            scene.animating = false;
        } else if (scene.target - scene.focus).abs() > 0.0001 && !scene.animating {
            scene.animating = true;
            scene.last_tick = 0;
            let weak_scene = Rc::downgrade(&self.scene);
            self.area.add_tick_callback(move |widget, clock| {
                let Some(scene) = weak_scene.upgrade() else {
                    return glib::ControlFlow::Break;
                };
                let mut scene = scene.borrow_mut();
                let previous = std::mem::replace(&mut scene.last_tick, clock.frame_time());
                let dt = if previous == 0 {
                    16_000
                } else {
                    (clock.frame_time() - previous).clamp(1_000, 50_000)
                };
                let blend = 1.0 - (-(dt as f64) / 105_000.0).exp();
                let target = scene.target;
                let next = scene.focus + (target - scene.focus) * blend;
                if (target - next).abs() < 0.0015 {
                    scene.focus = target;
                    scene.animating = false;
                    widget.queue_render();
                    glib::ControlFlow::Break
                } else {
                    scene.focus = next;
                    widget.queue_render();
                    glib::ControlFlow::Continue
                }
            });
        }
        drop(scene);
        self.area.queue_render();
    }
}

fn gl_library() -> Result<&'static Library, String> {
    static LIBRARY: OnceLock<Result<Library, String>> = OnceLock::new();
    LIBRARY
        .get_or_init(|| unsafe { Library::new("libGL.so.1").map_err(|error| error.to_string()) })
        .as_ref()
        .map_err(Clone::clone)
}

fn load_gl() -> Result<(), String> {
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
        return Err("OpenGL 3.2 entry points are unavailable".to_owned());
    }
    Ok(())
}

fn initialize_gl() -> Result<GlResources, String> {
    load_gl()?;
    let vertex = compile_shader(gl::VERTEX_SHADER, VERTEX_SHADER)?;
    let fragment = compile_shader(gl::FRAGMENT_SHADER, FRAGMENT_SHADER)?;
    let program = unsafe { gl::CreateProgram() };
    unsafe {
        gl::AttachShader(program, vertex);
        gl::AttachShader(program, fragment);
        gl::LinkProgram(program);
        gl::DeleteShader(vertex);
        gl::DeleteShader(fragment);
    }
    let mut linked = 0;
    unsafe {
        gl::GetProgramiv(program, gl::LINK_STATUS, &mut linked);
    }
    if linked == 0 {
        let message = program_log(program);
        unsafe {
            gl::DeleteProgram(program);
        }
        return Err(format!("Cover Flow shader link failed: {message}"));
    }
    let mut vao = 0;
    unsafe {
        gl::GenVertexArrays(1, &mut vao);
        gl::BindVertexArray(vao);
        gl::UseProgram(program);
        gl::Uniform1i(uniform(program, "uTexture"), 0);
        gl::Enable(gl::BLEND);
        gl::BlendFunc(gl::SRC_ALPHA, gl::ONE_MINUS_SRC_ALPHA);
        gl::Disable(gl::DEPTH_TEST);
        gl::Disable(gl::CULL_FACE);
    }
    Ok(GlResources {
        program,
        vao,
        textures: HashMap::new(),
    })
}

fn draw_frame(area: &gtk::GLArea, scene: &mut Scene) -> Result<(), String> {
    let scale = area.scale_factor().max(1) as f32;
    let viewport_width = (area.width().max(1) as f32 * scale).round() as i32;
    let viewport_height = (area.height().max(1) as f32 * scale).round() as i32;
    let Some(renderer) = area.native().and_then(|native| native.renderer()) else {
        return Err("Cover Flow has no realized GTK renderer for its card textures".to_owned());
    };
    let resources = scene.resources.as_mut().expect("GL resources initialized");
    let wanted: HashSet<usize> = scene.items.iter().map(|item| item.index).collect();
    let stale: Vec<usize> = resources
        .textures
        .keys()
        .copied()
        .filter(|index| !wanted.contains(index))
        .collect();
    for index in stale {
        if let Some(texture) = resources.textures.remove(&index) {
            unsafe {
                gl::DeleteTextures(1, &texture.texture);
            }
        }
    }

    for item in &scene.items {
        let upload = resources
            .textures
            .get(&item.index)
            .is_none_or(|uploaded| uploaded.revision != item.revision);
        if !upload {
            continue;
        }
        if let Some(previous) = resources.textures.remove(&item.index) {
            unsafe {
                gl::DeleteTextures(1, &previous.texture);
            }
        }
        let bounds = item.node.bounds();
        let texture = renderer.render_texture(&item.node, Some(&bounds));
        let width = texture.width().max(1);
        let height = texture.height().max(1);
        let mut downloader = gdk::TextureDownloader::new(&texture);
        downloader.set_format(gdk::MemoryFormat::R8g8b8a8);
        let (bytes, stride) = downloader.download_bytes();
        let bytes = bytes.as_ref();
        // GSK may switch to its own GL context while rasterizing the node.
        // Rebind the GLArea context before uploading into its texture cache.
        area.make_current();
        if let Some(error) = area.error() {
            return Err(format!("Cover Flow GL context was lost: {error}"));
        }
        let mut gl_texture = 0;
        unsafe {
            gl::GenTextures(1, &mut gl_texture);
            gl::BindTexture(gl::TEXTURE_2D, gl_texture);
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MIN_FILTER, gl::LINEAR as i32);
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MAG_FILTER, gl::LINEAR as i32);
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_WRAP_S, gl::CLAMP_TO_EDGE as i32);
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_WRAP_T, gl::CLAMP_TO_EDGE as i32);
            gl::PixelStorei(gl::UNPACK_ALIGNMENT, 1);
            gl::PixelStorei(gl::UNPACK_ROW_LENGTH, (stride / 4) as i32);
            gl::TexImage2D(
                gl::TEXTURE_2D,
                0,
                gl::RGBA8 as i32,
                width,
                height,
                0,
                gl::RGBA,
                gl::UNSIGNED_BYTE,
                bytes.as_ptr().cast(),
            );
            gl::PixelStorei(gl::UNPACK_ROW_LENGTH, 0);
        }
        resources.textures.insert(
            item.index,
            UploadedTexture {
                revision: item.revision,
                texture: gl_texture,
            },
        );
    }

    let logical_viewport = (area.width().max(1) as f64, area.height().max(1) as f64);
    let mut draw_order: Vec<&CoverflowItem> = scene.items.iter().collect();
    draw_order.sort_by(|a, b| {
        let da = (a.index as f64 - scene.focus).abs();
        let db = (b.index as f64 - scene.focus).abs();
        db.total_cmp(&da)
    });
    unsafe {
        gl::Viewport(0, 0, viewport_width, viewport_height);
        gl::ClearColor(0.0, 0.0, 0.0, 1.0);
        gl::Clear(gl::COLOR_BUFFER_BIT);
        gl::UseProgram(resources.program);
        gl::BindVertexArray(resources.vao);
        gl::ActiveTexture(gl::TEXTURE0);
        uniform_2f(
            resources.program,
            "uViewport",
            viewport_width as f32,
            viewport_height as f32,
        );
        uniform_1i(resources.program, "uReflection", 0);
        for item in draw_order {
            let Some(pose) = coverflow_pose(item.index, scene.focus, scene.count, logical_viewport)
            else {
                continue;
            };
            let Some(texture) = resources.textures.get(&item.index) else {
                continue;
            };
            let scale = area.scale_factor().max(1) as f64;
            uniform_2f(
                resources.program,
                "uCardSize",
                (pose.card_width * scale) as f32,
                (pose.card_height * scale) as f32,
            );
            uniform_3f(
                resources.program,
                "uCenter",
                ((pose.center_x + logical_viewport.0 * 0.5) * scale) as f32,
                (pose.center_y * scale) as f32,
                (pose.center_z * scale) as f32,
            );
            uniform_1f(resources.program, "uYaw", pose.rotation_y as f32);
            uniform_1f(
                resources.program,
                "uCameraDistance",
                (pose.camera_distance * scale) as f32,
            );
            uniform_1f(resources.program, "uOpacity", pose.opacity as f32);
            uniform_1i(resources.program, "uReflection", 1);
            gl::BindTexture(gl::TEXTURE_2D, texture.texture);
            gl::DrawArrays(gl::TRIANGLES, 0, 6);
            uniform_1i(resources.program, "uReflection", 0);
            gl::DrawArrays(gl::TRIANGLES, 0, 6);
        }
        gl::BindVertexArray(0);
        gl::BindTexture(gl::TEXTURE_2D, 0);
        gl::UseProgram(0);
    }
    Ok(())
}

unsafe fn delete_resources(resources: GlResources) {
    for texture in resources.textures.values() {
        gl::DeleteTextures(1, &texture.texture);
    }
    gl::DeleteVertexArrays(1, &resources.vao);
    gl::DeleteProgram(resources.program);
}

const VERTEX_SHADER: &str = r#"#version 150
uniform vec2 uViewport;
uniform vec2 uCardSize;
uniform vec3 uCenter;
uniform float uYaw;
uniform float uCameraDistance;
uniform int uReflection;
out vec2 vTexCoord;
out float vReflectFade;
void main() {
    vec2 corners[6] = vec2[6](
        vec2(-0.5, -0.5), vec2( 0.5, -0.5), vec2(-0.5,  0.5),
        vec2(-0.5,  0.5), vec2( 0.5, -0.5), vec2( 0.5,  0.5)
    );
    vec2 texCoords[6] = vec2[6](
        vec2(0.0, 1.0), vec2(1.0, 1.0), vec2(0.0, 0.0),
        vec2(0.0, 0.0), vec2(1.0, 1.0), vec2(1.0, 0.0)
    );
    vec2 corner = corners[gl_VertexID];
    vec2 uv = texCoords[gl_VertexID];
    float x = corner.x * uCardSize.x;
    float y = corner.y * uCardSize.y;
    if (uReflection != 0) y = -uCardSize.y - y;
    float cosine = cos(uYaw);
    float sine = sin(uYaw);
    float rotatedX = x * cosine;
    float rotatedZ = uCenter.z - x * sine;
    float centerUp = uViewport.y * 0.5 - uCenter.y;
    float camera = max(uCameraDistance, 1.0);
    float perspective = camera / max(camera - rotatedZ, camera * 0.25);
    float screenX = uViewport.x * 0.5 + (uCenter.x - uViewport.x * 0.5 + rotatedX) * perspective;
    float screenY = uViewport.y * 0.5 - (centerUp + y) * perspective;
    gl_Position = vec4(screenX / uViewport.x * 2.0 - 1.0,
                       1.0 - screenY / uViewport.y * 2.0, 0.0, 1.0);
    vTexCoord = uv;
    vReflectFade = uReflection != 0 ? clamp((-y - uCardSize.y * 0.5) / (uCardSize.y * 0.52), 0.0, 1.0) : 0.0;
}"#;

const FRAGMENT_SHADER: &str = r#"#version 150
uniform sampler2D uTexture;
uniform int uReflection;
uniform float uOpacity;
in vec2 vTexCoord;
in float vReflectFade;
out vec4 outColor;
void main() {
    vec2 uv = vTexCoord;
    vec4 color = texture(uTexture, uv);
    if (uReflection != 0) {
        color.rgb *= 0.90;
        color.a *= uOpacity * 0.45 * pow(max(0.0, 1.0 - vReflectFade), 2.0);
    } else {
        color.a *= uOpacity;
    }
    outColor = color;
}"#;

fn compile_shader(kind: u32, source: &str) -> Result<u32, String> {
    let source = CString::new(source).map_err(|error| error.to_string())?;
    let shader = unsafe { gl::CreateShader(kind) };
    unsafe {
        let pointer = source.as_ptr();
        gl::ShaderSource(shader, 1, &pointer, std::ptr::null());
        gl::CompileShader(shader);
    }
    let mut compiled = 0;
    unsafe {
        gl::GetShaderiv(shader, gl::COMPILE_STATUS, &mut compiled);
    }
    if compiled == 0 {
        let message = shader_log(shader);
        unsafe {
            gl::DeleteShader(shader);
        }
        return Err(format!("shader compile failed: {message}"));
    }
    Ok(shader)
}

fn shader_log(shader: u32) -> String {
    unsafe {
        let mut length = 0;
        gl::GetShaderiv(shader, gl::INFO_LOG_LENGTH, &mut length);
        if length <= 1 {
            return "unknown OpenGL shader error".to_owned();
        }
        let mut bytes = vec![0_u8; length as usize];
        gl::GetShaderInfoLog(
            shader,
            length,
            std::ptr::null_mut(),
            bytes.as_mut_ptr().cast(),
        );
        String::from_utf8_lossy(&bytes)
            .trim_end_matches('\0')
            .to_owned()
    }
}

fn program_log(program: u32) -> String {
    unsafe {
        let mut length = 0;
        gl::GetProgramiv(program, gl::INFO_LOG_LENGTH, &mut length);
        if length <= 1 {
            return "unknown OpenGL program error".to_owned();
        }
        let mut bytes = vec![0_u8; length as usize];
        gl::GetProgramInfoLog(
            program,
            length,
            std::ptr::null_mut(),
            bytes.as_mut_ptr().cast(),
        );
        String::from_utf8_lossy(&bytes)
            .trim_end_matches('\0')
            .to_owned()
    }
}

unsafe fn uniform(program: u32, name: &str) -> i32 {
    let name = CString::new(name).expect("uniform name");
    gl::GetUniformLocation(program, name.as_ptr())
}
unsafe fn uniform_1i(program: u32, name: &str, value: i32) {
    gl::Uniform1i(uniform(program, name), value);
}
unsafe fn uniform_1f(program: u32, name: &str, value: f32) {
    gl::Uniform1f(uniform(program, name), value);
}
unsafe fn uniform_2f(program: u32, name: &str, x: f32, y: f32) {
    gl::Uniform2f(uniform(program, name), x, y);
}
unsafe fn uniform_3f(program: u32, name: &str, x: f32, y: f32, z: f32) {
    gl::Uniform3f(uniform(program, name), x, y, z);
}
