//! Continuous GSK camera transforms with retained outgoing and upgrade textures.
use super::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, graphene, gsk};

const UPGRADE_SECONDS: f64 = 0.35;

#[derive(Clone)]
struct Photo {
    id: usize,
    revision: u64,
    texture: gdk::Texture,
    upgrade: Option<(gdk::Texture, f64)>,
    born: f64,
}

#[derive(Default)]
struct Scene {
    current: Option<Photo>,
    outgoing: Option<Photo>,
    clock: f64,
    last_frame: Option<i64>,
    duration: f64,
    transition: f64,
    ken: bool,
    matrix: bool,
    rain: bool,
}

fn camera_fraction(age: f64, interval: f64, transition: f64) -> f64 {
    (age / (interval + transition).max(0.2)).clamp(0.0, 1.0)
}

mod imp {
    use super::*;
    #[derive(Default)]
    pub struct CinemaCanvas {
        pub(super) scene: RefCell<Scene>,
    }
    #[glib::object_subclass]
    impl ObjectSubclass for CinemaCanvas {
        const NAME: &'static str = "PicCinemaCanvas";
        type Type = super::CinemaCanvas;
        type ParentType = gtk::Widget;
    }
    impl ObjectImpl for CinemaCanvas {}
    impl WidgetImpl for CinemaCanvas {
        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let obj = self.obj();
            let (w, h) = (obj.width() as f64, obj.height() as f64);
            let scene = self.scene.borrow();
            if scene.rain {
                snapshot.append_color(
                    &gdk::RGBA::new(0.0, 0.012, 0.004, 1.0),
                    &graphene::Rect::new(0.0, 0.0, w as f32, h as f32),
                );
                let cr = snapshot.append_cairo(&graphene::Rect::new(0., 0., w as f32, h as f32));
                super::super::super::matrix::draw_rain(&cr, w, h, scene.clock);
                return;
            }
            let Some(current) = &scene.current else {
                return;
            };
            snapshot.push_clip(&graphene::Rect::new(0.0, 0.0, w as f32, h as f32));
            if let Some(old) = &scene.outgoing {
                paint(snapshot, old, &scene, w, h);
            }
            let fade = if scene.outgoing.is_some() {
                smootherstep((scene.clock - current.born) / scene.transition)
            } else {
                1.0
            };
            snapshot.push_opacity(fade);
            // Fade the whole viewport, including letterboxing in Fullscreen Fade.
            snapshot.append_color(
                &if scene.matrix {
                    gdk::RGBA::new(0.0, 0.015, 0.005, 1.0)
                } else {
                    gdk::RGBA::new(0.035, 0.043, 0.063, 1.0)
                },
                &graphene::Rect::new(0.0, 0.0, w as f32, h as f32),
            );
            paint(snapshot, current, &scene, w, h);
            snapshot.pop();
            snapshot.pop();
        }
    }
}
glib::wrapper! {
    pub struct CinemaCanvas(ObjectSubclass<imp::CinemaCanvas>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

fn paint(snapshot: &gtk::Snapshot, photo: &Photo, scene: &Scene, w: f64, h: f64) {
    let (scale, x, y) = if scene.ken {
        cinematic_motion(
            photo.id,
            camera_fraction(scene.clock - photo.born, scene.duration, scene.transition),
        )
    } else {
        (1.0, 0.0, 0.0)
    };
    snapshot.save();
    snapshot.translate(&graphene::Point::new(
        (w * 0.5 + x) as f32,
        (h * 0.5 + y) as f32,
    ));
    // Guarantee pan overscan even on small viewports.
    let scale = if scene.ken {
        scale.max(1.0 + 56.0 / w).max(1.0 + 40.0 / h)
    } else {
        scale
    };
    snapshot.scale(scale as f32, scale as f32);
    let draw = |texture: &gdk::Texture, snapshot: &gtk::Snapshot| {
        let (tw, th) = (texture.width() as f64, texture.height() as f64);
        let fit = if scene.ken {
            (w / tw).max(h / th)
        } else {
            (w / tw).min(h / th)
        };
        let (dw, dh) = (tw * fit, th * fit);
        snapshot.append_scaled_texture(
            texture,
            gsk::ScalingFilter::Trilinear,
            &graphene::Rect::new((-dw * 0.5) as f32, (-dh * 0.5) as f32, dw as f32, dh as f32),
        );
    };
    draw(&photo.texture, snapshot);
    if let Some((texture, since)) = &photo.upgrade {
        snapshot.push_opacity(smootherstep((scene.clock - since) / UPGRADE_SECONDS));
        draw(texture, snapshot);
        snapshot.pop();
    }
    snapshot.restore();
}

pub(super) struct Cinema {
    canvas: CinemaCanvas,
    size: (i32, i32),
}
impl Cinema {
    pub fn new(root: &gtk::Fixed) -> Self {
        let canvas: CinemaCanvas = glib::Object::new();
        canvas.set_can_target(false);
        root.put(&canvas, 0.0, 0.0);
        Self {
            canvas,
            size: (0, 0),
        }
    }
    pub fn sync(&mut self, player: &Player, w: f64, h: f64, frame_time: i64, ken: bool) {
        let size = (w.round() as i32, h.round() as i32);
        if self.size != size {
            self.size = size;
            self.canvas.set_size_request(size.0, size.1);
        }
        let mut scene = self.canvas.imp().scene.borrow_mut();
        let was_rain = scene.rain;
        let previous_tick = (scene.clock * 30.0) as u64;
        let previous_photo = scene.current.as_ref().map(|p| (p.id, p.revision));
        scene.sync(player, frame_time, ken);
        let redraw = !scene.rain
            || !was_rain
            || previous_tick != (scene.clock * 30.0) as u64
            || previous_photo != scene.current.as_ref().map(|p| (p.id, p.revision));
        drop(scene);
        if redraw {
            self.canvas.queue_draw();
        }
    }
}

impl Scene {
    fn sync(&mut self, player: &Player, frame_time: i64, ken: bool) {
        let dt = self
            .last_frame
            .map(|last| ((frame_time - last) as f64 / 1e6).clamp(0.0, 0.05))
            .unwrap_or(0.0);
        self.last_frame = Some(frame_time);
        if player.paused_at.is_none() {
            self.clock += dt;
        }
        self.duration = player.interval.as_secs_f64();
        self.transition = player.transition.as_secs_f64().max(0.2);
        self.ken = ken;
        self.matrix = player.style.matrix();
        self.rain = player.style == Style::MatrixRain;
        let clock = self.clock;
        if let Some(surface) = player.frames.get(&player.current) {
            if self.current.as_ref().is_none_or(|p| p.id != player.current) {
                self.outgoing = self.current.take();
                self.current = Some(Photo {
                    id: player.current,
                    revision: surface.revision,
                    texture: surface.texture.clone(),
                    upgrade: None,
                    born: clock,
                });
            } else if let Some(photo) = self.current.as_mut() {
                if photo.revision != surface.revision {
                    photo.revision = surface.revision;
                    photo.upgrade = Some((surface.texture.clone(), clock));
                }
            }
        }
        if let Some(photo) = self.current.as_mut() {
            if photo
                .upgrade
                .as_ref()
                .is_some_and(|(_, since)| clock - since >= UPGRADE_SECONDS)
            {
                photo.texture = photo.upgrade.take().unwrap().0;
            }
        }
        if self
            .current
            .as_ref()
            .is_some_and(|p| clock - p.born >= self.transition)
        {
            self.outgoing = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
    fn matrix_styles_render_green_move_and_pause() {
        libadwaita::init().unwrap();
        let context = glib::MainContext::default();
        for style in [Style::MatrixRain, Style::MatrixKenBurns] {
            let mut player = Player::new(
                style,
                if style == Style::MatrixRain { 0 } else { 2 },
                6,
                false,
                6,
            );
            let mut data = Vec::new();
            for y in 0..360 {
                for x in 0..640 {
                    let color = if y > 230 - ((x as f64 / 90.).sin() * 65.) as i32 {
                        [70, 150, 45, 255]
                    } else {
                        [(x / 3) as u8, (y / 2 + 60) as u8, 210, 255]
                    };
                    data.extend_from_slice(&color);
                }
            }
            if style == Style::MatrixKenBurns {
                let source = if let Ok(path) = std::env::var("PIC_MATRIX_PREVIEW_PHOTO") {
                    let image = image::open(path)
                        .unwrap()
                        .resize(1920, 1200, image::imageops::FilterType::Lanczos3)
                        .to_rgba8();
                    texture_pixels(image)
                } else {
                    Pixels {
                        width: 640,
                        height: 360,
                        data,
                    }
                };
                let pixels = super::super::super::matrix::prepare(source, style);
                player.receive(ImageResult {
                    index: 0,
                    high_quality: true,
                    pixels: Some(pixels),
                });
            }
            let mut stage = Stage::new(style, 0, PicasaTransition::Crossfade);
            let window = gtk::Window::builder()
                .decorated(false)
                .default_width(1280)
                .default_height(720)
                .child(&stage.root)
                .build();
            window.present();
            for _ in 0..20 {
                while context.pending() {
                    context.iteration(false);
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            let capture = |stage: &Stage| {
                let snapshot = gtk::Snapshot::new();
                stage
                    .cinema
                    .as_ref()
                    .unwrap()
                    .canvas
                    .imp()
                    .snapshot(&snapshot);
                let node = snapshot.to_node().unwrap();
                let texture = window
                    .renderer()
                    .unwrap()
                    .render_texture(&node, Some(&graphene::Rect::new(0., 0., 1280., 720.)));
                let mut data = vec![0; 1280 * 720 * 4];
                texture.download(&mut data, 1280 * 4);
                (data, texture)
            };
            stage.sync(&player, 1_000_000);
            while context.pending() {
                context.iteration(false);
            }
            let (first, _) = capture(&stage);
            for tick in 1..=60 {
                stage.sync(&player, 1_000_000 + tick * 16_667);
            }
            let (moving, texture) = capture(&stage);
            assert_ne!(first, moving, "{} did not animate", style.label());
            assert!(
                moving.chunks_exact(4).all(|p| p[1] >= p[0] && p[1] >= p[2]),
                "non-green pixels"
            );
            if let Ok(dir) = std::env::var("PIC_MATRIX_PREVIEW_DIR") {
                texture
                    .save_to_png(std::path::Path::new(&dir).join(format!("{}.png", style.key())))
                    .unwrap();
            }
            player.pause_or_resume();
            stage.sync(&player, 2_500_000);
            assert_eq!(moving, capture(&stage).0, "pause changed the image");
            let scene = stage.cinema.as_ref().unwrap().canvas.imp().scene.borrow();
            assert_eq!(scene.rain, style == Style::MatrixRain);
            if style == Style::MatrixRain {
                assert!(scene.current.is_none());
            }
            assert_eq!(scene.ken, style == Style::MatrixKenBurns);
            drop(scene);
            window.close();
        }
    }

    #[test]
    #[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
    fn canvas_renders_portraits_and_crossfades_at_viewport_size() {
        libadwaita::init().unwrap();
        let mut player = Player::new(Style::IPhoto, 2, 6, false, 6);
        for (index, color) in [(0, [255, 0, 0, 255]), (1, [0, 0, 255, 255])] {
            player.receive(ImageResult {
                index,
                high_quality: true,
                pixels: Some(Pixels {
                    width: 2,
                    height: 4,
                    data: color.repeat(8),
                }),
            });
        }
        let mut stage = Stage::new(Style::IPhoto, 0, PicasaTransition::Crossfade);
        let window = gtk::Window::builder()
            .default_width(640)
            .default_height(360)
            .child(&stage.root)
            .build();
        window.present();
        let context = glib::MainContext::default();
        for _ in 0..20 {
            while context.pending() {
                context.iteration(false);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let mut time = 1_000_000;
        for frame in 0..100 {
            if frame == 10 {
                player.current = 1;
            }
            time += 16_667;
            stage.sync(&player, time);
            while context.pending() {
                context.iteration(false);
            }
            if [5, 35, 95].contains(&frame) {
                let canvas = &stage.cinema.as_ref().unwrap().canvas;
                let snapshot = gtk::Snapshot::new();
                // Snapshot directly so rendering does not depend on compositor frame delivery.
                canvas.imp().snapshot(&snapshot);
                let node = snapshot.to_node().expect("cinema should paint");
                let renderer = window.renderer().unwrap();
                let texture = renderer
                    .render_texture(&node, Some(&graphene::Rect::new(0.0, 0.0, 640.0, 360.0)));
                let mut pixels = vec![0; 640 * 360 * 4];
                texture.download(&mut pixels, 640 * 4);
                // Native download channel order can vary; red + blue must fill the edge.
                let edge = &pixels[(180 * 640 + 1) * 4..][..4];
                assert!(
                    u16::from(edge[0]) + u16::from(edge[2]) > 220,
                    "portrait left a bar: {edge:?}"
                );
                if frame == 35 {
                    assert!(edge[0] > 20 && edge[2] > 20, "missing crossfade: {edge:?}");
                }
            }
        }
        window.close();
    }

    fn load(player: &mut Player, high_quality: bool) {
        player.receive(ImageResult {
            index: 0,
            high_quality,
            pixels: Some(Pixels {
                width: 1,
                height: 1,
                data: vec![255; 4],
            }),
        });
    }
    #[test]
    fn upgrade_blends_without_restarting_motion_and_pause_freezes_it() {
        let mut player = Player::new(Style::IPhoto, 2, 6, false, 6);
        let mut scene = Scene::default();
        load(&mut player, false);
        scene.sync(&player, 1_000_000, true);
        let born = scene.current.as_ref().unwrap().born;
        load(&mut player, true);
        scene.sync(&player, 1_016_667, true);
        assert!(scene.current.as_ref().unwrap().upgrade.is_some());
        player.pause_or_resume();
        let paused = scene.clock;
        scene.sync(&player, 2_000_000, true);
        assert_eq!(scene.clock, paused);
        player.pause_or_resume();
        for frame in 1..=30 {
            scene.sync(&player, 2_000_000 + frame * 16_667, true);
        }
        let photo = scene.current.as_ref().unwrap();
        assert!(photo.upgrade.is_none());
        assert_eq!(photo.born, born);
        assert!(scene.outgoing.is_none());
    }

    #[test]
    fn slow_frames_are_clamped_and_outgoing_photo_survives_player_cleanup() {
        let mut player = Player::new(Style::IPhoto, 2, 6, false, 6);
        let mut scene = Scene::default();
        load(&mut player, true);
        scene.sync(&player, 1_000_000, true);
        scene.sync(&player, 2_000_000, true);
        assert!((scene.clock - 0.05).abs() < 1e-9);
        player.receive(ImageResult {
            index: 1,
            high_quality: true,
            pixels: Some(Pixels {
                width: 1,
                height: 1,
                data: vec![255; 4],
            }),
        });
        player.current = 1;
        player.previous = None;
        player.frames.remove(&0);
        scene.sync(&player, 2_016_667, true);
        assert_eq!(scene.outgoing.as_ref().unwrap().id, 0);
        for frame in 2..=60 {
            scene.sync(&player, 2_000_000 + frame * 16_667, true);
        }
        assert!(scene.outgoing.is_none());
        assert_eq!(scene.current.as_ref().unwrap().id, 1);
    }

    #[test]
    fn camera_keeps_moving_through_the_outgoing_fade() {
        let before = camera_fraction(6.0, 6.0, 0.9);
        let during = camera_fraction(6.45, 6.0, 0.9);
        assert!(before < during && during < 1.0);
        assert_eq!(camera_fraction(6.9, 6.0, 0.9), 1.0);
    }
    #[test]
    fn long_slides_move_on_every_frame() {
        let a = cinematic_motion(0, camera_fraction(15.0, 30.0, 0.9));
        let b = cinematic_motion(0, camera_fraction(15.0 + 1.0 / 60.0, 30.0, 0.9));
        assert!(b.0 > a.0 && b.1 > a.1 && b.2 < a.2);
    }
}
