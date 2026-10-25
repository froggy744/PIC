//! OpenGL perspective Cover Flow shared by albums and the photo screensaver.
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::ffi::{c_void, CString};
use std::rc::Rc;
use std::sync::OnceLock;

use gtk::prelude::*;
use gtk::{gdk, gsk};
use gtk4 as gtk;
use libadwaita as adw;
use libloading::Library;

pub(crate) const VISIBLE_RADIUS: usize = 4;
const SIDE_ANGLE: f64 = 60.0_f64.to_radians();
const ALBUM_SIDE_ANGLE: f64 = 32.0_f64.to_radians();

// GTK 4.10 deprecated computed style color lookups without providing a
// replacement for resolving theme-defined background colors. Keep this
// compatibility call isolated so the rest of the app builds warning-free.
#[allow(deprecated)]
pub(crate) fn lookup_theme_color(widget: &gtk::Widget, name: &str) -> Option<gdk::RGBA> {
    let style = widget.style_context();
    style.lookup_color(name)
}

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
    coverflow_pose_profile(index, focus, count, viewport, false)
}

/// Album-index variant: compact gallery sizing while the fullscreen
/// screensaver retains its original Cover Flow proportions.
pub(crate) fn album_coverflow_pose(
    index: usize,
    focus: f64,
    count: usize,
    viewport: (f64, f64),
) -> Option<CoverflowPose> {
    coverflow_pose_profile(index, focus, count, viewport, true)
}

fn coverflow_pose_profile(
    index: usize,
    focus: f64,
    count: usize,
    viewport: (f64, f64),
    compact: bool,
) -> Option<CoverflowPose> {
    let (vw, vh) = viewport;
    if count == 0 || index >= count || vw <= 0.0 || vh <= 0.0 {
        return None;
    }
    let offset = index as f64 - focus.clamp(0.0, (count - 1) as f64);
    let distance = offset.abs();
    let base_height = if compact {
        (vh * 0.48).min(vw * 0.19).clamp(105.0, 245.0)
    } else {
        (vh * 0.60).min(vw * 0.38).max(1.0)
    };
    let scale = if compact {
        if distance < 1.0 {
            1.20 - 0.16 * distance
        } else {
            1.04
        }
    } else if distance < 1.0 {
        1.18 - 0.24 * distance
    } else {
        0.94 * 0.84_f64.powf(distance - 1.0)
    };
    let card_height = base_height * scale;
    let card_width = base_height * scale;
    let side_distance = (distance - 1.0).max(0.0);
    let rotation_y = -offset.signum()
        * if compact { ALBUM_SIDE_ANGLE } else { SIDE_ANGLE }
        * distance.min(1.0);
    let (center_y, center_z, opacity, visible_radius, lateral) = if compact {
        let side_width = base_height * 1.04 * ALBUM_SIDE_ANGLE.cos();
        let gap = (base_height * 0.22).clamp(36.0, 56.0);
        let first_step = card_width * 0.5 + side_width * 0.5 + gap;
        let lateral = if distance <= 1.0 {
            offset * first_step
        } else {
            offset.signum() * (first_step + (distance - 1.0) * (side_width + gap))
        };
        (
            vh * 0.49,
            -side_distance * base_height * 0.08,
            (1.0 - side_distance * 0.025).clamp(0.90, 1.0),
            VISIBLE_RADIUS as f64 + 0.5,
            lateral,
        )
    } else {
        if distance > VISIBLE_RADIUS as f64 + 0.5 {
            return None;
        }
        let lateral = if distance <= 1.0 {
            offset * 0.94 * base_height
        } else {
            offset.signum() * (0.94 + side_distance * 0.38) * base_height
        };
        (
            vh * 0.40,
            -side_distance * base_height * 0.28,
            (1.0 - distance * 0.16).clamp(0.48, 1.0),
            VISIBLE_RADIUS as f64 + 0.5,
            lateral,
        )
    };
    if distance > visible_radius {
        return None;
    }
    let camera_distance = vh * 3.4;
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
    if compact && (bounds.2 <= 0.0 || bounds.0 >= vw) {
        return None;
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
    focus_velocity: f64,
    last_tick: i64,
    animating: bool,
    items: Vec<CoverflowItem>,
    target_items: Vec<CoverflowItem>,
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
        scene.target = focus.clamp(0.0, count.saturating_sub(1) as f64);
        scene.error = None;
        if first || !animate || !self.area.settings().is_gtk_enable_animations() {
            scene.focus = scene.target;
            scene.focus_velocity = 0.0;
            scene.animating = false;
            scene.items = items;
            scene.target_items = scene.items.clone();
        } else {
            scene.target_items = items;
            let mut retained: HashMap<usize, CoverflowItem> = scene
                .items
                .iter()
                .filter(|item| {
                    (item.index as f64 - scene.focus).abs()
                        <= VISIBLE_RADIUS as f64 + 0.5
                })
                .cloned()
                .map(|item| (item.index, item))
                .collect();
            for item in &scene.target_items {
                retained.insert(item.index, item.clone());
            }
            scene.items = retained.into_values().collect();
            if (scene.target - scene.focus).abs() <= 0.0001
                && scene.focus_velocity.abs() <= 0.001
            {
                scene.focus = scene.target;
                scene.focus_velocity = 0.0;
                scene.animating = false;
                scene.items = scene.target_items.clone();
            } else if !scene.animating {
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
                    } as f64
                        / 1_000_000.0;
                    let omega = 8.0_f64;
                    let acceleration = (scene.target - scene.focus) * omega * omega
                        - 2.0 * omega * scene.focus_velocity;
                    scene.focus_velocity += acceleration * dt;
                    scene.focus += scene.focus_velocity * dt;
                    if (scene.target - scene.focus).abs() < 0.001
                        && scene.focus_velocity.abs() < 0.01
                    {
                        scene.focus = scene.target;
                        scene.focus_velocity = 0.0;
                        scene.animating = false;
                        scene.items = scene.target_items.clone();
                        widget.queue_render();
                        glib::ControlFlow::Break
                    } else {
                        widget.queue_render();
                        glib::ControlFlow::Continue
                    }
                });
            }
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
        let dark = adw::StyleManager::default().is_dark();
        let background = lookup_theme_color(area.upcast_ref(), "theme_bg_color")
            .or_else(|| lookup_theme_color(area.upcast_ref(), "window_bg_color"))
            .unwrap_or_else(|| {
                if dark {
                    gdk::RGBA::new(0.12, 0.12, 0.12, 1.0)
                } else {
                    gdk::RGBA::new(0.97, 0.97, 0.97, 1.0)
                }
            });
        gl::ClearColor(background.red(), background.green(), background.blue(), 1.0);
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
        let album_flow = area.has_css_class("albums-coverflow-canvas");
        let radius = if area.has_css_class("corner-style-rounded") {
            0.08_f32
        } else if area.has_css_class("corner-style-balanced") {
            0.045_f32
        } else {
            0.0
        };
        uniform_1f(resources.program, "uCornerRadius", radius);
        uniform_1f(resources.program, "uReflectionHeight", if album_flow { 0.36 } else { 0.32 });
        uniform_1f(resources.program, "uReflectionOpacity", if album_flow { 0.50 } else { 0.20 });
        for item in draw_order {
            let pose = if album_flow {
                album_coverflow_pose(item.index, scene.focus, scene.count, logical_viewport)
            } else {
                coverflow_pose(item.index, scene.focus, scene.count, logical_viewport)
            };
            let Some(pose) = pose
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
uniform float uCornerRadius;
uniform float uReflectionHeight;
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
    if (uReflection != 0) {
        y = -uCardSize.y * 0.5 - (corner.y + 0.5) * uCardSize.y * uReflectionHeight;
        uv.y = mix(1.0, 1.0 - uReflectionHeight, corner.y + 0.5);
    }
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
    vReflectFade = uReflection != 0 ? clamp(corner.y + 0.5, 0.0, 1.0) : 0.0;
}"#;

const FRAGMENT_SHADER: &str = r#"#version 150
uniform sampler2D uTexture;
uniform int uReflection;
uniform float uOpacity;
uniform float uCornerRadius;
uniform float uReflectionOpacity;
in vec2 vTexCoord;
in float vReflectFade;
out vec4 outColor;
void main() {
    vec2 uv = vTexCoord;
    vec2 p = uv - vec2(0.5);
    vec2 q = abs(p) - vec2(0.5 - uCornerRadius);
    float rounded_distance = length(max(q, vec2(0.0))) + min(max(q.x, q.y), 0.0) - uCornerRadius;
    float corner_coverage = 1.0;
    if (uCornerRadius > 0.0) {
        float edge_width = max(fwidth(rounded_distance), 0.0001);
        corner_coverage = 1.0 - smoothstep(-edge_width, edge_width, rounded_distance);
    }
    vec4 color = texture(uTexture, uv);
    color.a *= corner_coverage;
    if (uReflection != 0) {
        color.rgb *= 0.90;
        color.a *= uOpacity * uReflectionOpacity * pow(max(0.0, 1.0 - vReflectFade), 1.5);
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
