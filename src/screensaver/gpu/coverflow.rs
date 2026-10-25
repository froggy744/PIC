//! Fullscreen photo Cover Flow, driven by the existing screensaver player.
use super::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, graphene, gsk};

mod imp {
    use super::*;
    #[derive(Default)]
    pub struct CoverFlowCanvas {
        pub scene: RefCell<Vec<(usize, gsk::RenderNode)>>,
        pub focus: Cell<f64>,
        pub count: Cell<usize>,
    }
    #[glib::object_subclass]
    impl ObjectSubclass for CoverFlowCanvas {
        const NAME: &'static str = "PicScreensaverCoverFlowCanvas";
        type Type = super::CoverFlowCanvas;
        type ParentType = gtk::Widget;
    }
    impl ObjectImpl for CoverFlowCanvas {}
    impl WidgetImpl for CoverFlowCanvas {
        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let widget = self.obj();
            let viewport = (widget.width() as f64, widget.height() as f64);
            let mut scene = self.scene.borrow().clone();
            scene.sort_by(|a, b| {
                let left = (a.0 as f64 - self.focus.get()).abs();
                let right = (b.0 as f64 - self.focus.get()).abs();
                right.total_cmp(&left)
            });
            let shadow = gdk::RGBA::new(0.0, 0.0, 0.0, 0.58);
            for (index, node) in scene {
                if let Some(pose) = crate::coverflow::coverflow_pose(
                    index,
                    self.focus.get(),
                    self.count.get(),
                    viewport,
                ) {
                    crate::coverflow::paint_cover(snapshot, &node, pose, 0.85, &shadow);
                }
            }
        }
    }
}
glib::wrapper! {
    pub struct CoverFlowCanvas(ObjectSubclass<imp::CoverFlowCanvas>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}
impl CoverFlowCanvas {
    fn new() -> Self {
        let canvas: Self = glib::Object::new();
        canvas.set_hexpand(true);
        canvas.set_vexpand(true);
        canvas.set_can_target(false);
        canvas
    }
    fn set_scene(&self, count: usize, focus: f64, scene: Vec<(usize, gsk::RenderNode)>) {
        self.imp().count.set(count);
        self.imp().focus.set(focus);
        *self.imp().scene.borrow_mut() = scene;
        self.queue_draw();
    }
}

fn photo_node(texture: &gdk::Texture) -> gsk::RenderNode {
    let snapshot = gtk::Snapshot::new();
    let outer = graphene::Rect::new(0.0, 0.0, 300.0, 300.0);
    let inside = graphene::Rect::new(7.0, 7.0, 286.0, 286.0);
    snapshot.append_color(&gdk::RGBA::new(0.9, 0.9, 0.88, 1.0), &outer);
    snapshot.append_color(&gdk::RGBA::new(0.045, 0.05, 0.06, 1.0), &inside);
    let iw = texture.width().max(1) as f32;
    let ih = texture.height().max(1) as f32;
    let scale = (inside.width() / iw).min(inside.height() / ih);
    let width = iw * scale;
    let height = ih * scale;
    snapshot.append_scaled_texture(
        texture,
        gsk::ScalingFilter::Trilinear,
        &graphene::Rect::new(
            inside.x() + (inside.width() - width) * 0.5,
            inside.y() + (inside.height() - height) * 0.5,
            width,
            height,
        ),
    );
    snapshot.to_node().expect("photo cover node")
}

pub(super) struct CoverFlow {
    pub canvas: CoverFlowCanvas,
    nodes: HashMap<usize, (u64, gsk::RenderNode)>,
    last_focus: Option<f64>,
    last_count: usize,
    last_size: (i32, i32),
}
impl CoverFlow {
    pub fn new(root: &gtk::Fixed) -> Self {
        let canvas = CoverFlowCanvas::new();
        root.put(&canvas, 0.0, 0.0);
        Self {
            canvas,
            nodes: HashMap::new(),
            last_focus: None,
            last_count: 0,
            last_size: (0, 0),
        }
    }
    pub fn sync(&mut self, player: &Player, width: f64, height: f64) {
        let size = (width.round() as i32, height.round() as i32);
        let resized = size != self.last_size;
        if resized {
            self.canvas.set_size_request(size.0, size.1);
            self.last_size = size;
        }
        let animated = self.canvas.settings().is_gtk_enable_animations();
        let focus = if animated {
            match player.previous {
                Some(previous) if previous + 1 == player.current && player.progress() < 1.0 => {
                    previous as f64 + smootherstep(player.progress())
                }
                _ => player.current as f64,
            }
        } else {
            player.current as f64
        };
        let visible = coverflow_neighbor_indices(player.current, player.count);
        let mut changed = self.last_count != player.count
            || self
                .last_focus
                .is_none_or(|old| (old - focus).abs() > 0.0001)
            || resized;
        self.nodes.retain(|index, _| visible.contains(index));
        for index in visible.clone() {
            if let Some(surface) = player.frames.get(&index) {
                let replace = self
                    .nodes
                    .get(&index)
                    .is_none_or(|(revision, _)| *revision != surface.revision);
                if replace {
                    self.nodes
                        .insert(index, (surface.revision, photo_node(&surface.texture)));
                    changed = true;
                }
            }
        }
        if changed {
            let scene = visible
                .filter_map(|index| {
                    self.nodes
                        .get(&index)
                        .map(|(_, node)| (index, node.clone()))
                })
                .collect();
            self.canvas.set_scene(player.count, focus, scene);
            self.last_focus = Some(focus);
            self.last_count = player.count;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires a GTK display; run with --ignored --test-threads=1"]
    fn photo_coverflow_paints_a_fading_reflection_and_moves_focus() {
        libadwaita::init().unwrap();
        let mut player = Player::new(Style::CoverFlow, 3, 5, false, 6);
        for index in 0..3 {
            let mut data = vec![255_u8; 64 * 64 * 4];
            for pixel in data.chunks_exact_mut(4) {
                pixel[0] = 70 + index as u8 * 50;
                pixel[1] = 100;
                pixel[2] = 170;
            }
            player.receive(ImageResult {
                index,
                high_quality: true,
                pixels: Some(Pixels {
                    width: 64,
                    height: 64,
                    data,
                }),
            });
        }
        let mut stage = Stage::new(Style::CoverFlow, 0, PicasaTransition::Crossfade);
        let window = gtk::Window::builder()
            .decorated(false)
            .default_width(900)
            .default_height(600)
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
        stage.sync(&player, 1_000_000);
        for _ in 0..20 {
            while context.pending() {
                context.iteration(false);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let canvas = &stage.coverflow.as_ref().unwrap().canvas;
        assert!(canvas.width() > 0 && canvas.height() > 0);
        let snapshot = gtk::Snapshot::new();
        canvas.imp().snapshot(&snapshot);
        let node = snapshot.to_node().expect("photo Cover Flow should paint");
        let width = canvas.width() as usize;
        let height = canvas.height() as usize;
        let texture = window.renderer().unwrap().render_texture(
            &node,
            Some(&graphene::Rect::new(0.0, 0.0, width as f32, height as f32)),
        );
        let mut pixels = vec![0_u8; width * height * 4];
        texture.download(&mut pixels, width * 4);
        let alpha = |x: usize, y: usize| pixels[(y * width + x) * 4 + 3];
        let pose =
            crate::coverflow::coverflow_pose(0, 0.0, 3, (width as f64, height as f64)).unwrap();
        let x = (pose.x + pose.width * 0.5) as usize;
        let top = (pose.y + pose.height + 9.0) as usize;
        let bottom = (pose.y + pose.height + pose.height * 0.4) as usize;
        assert!(alpha(x, top) > alpha(x, bottom));
        let largest_step = (top + 1..bottom)
            .map(|y| alpha(x, y).abs_diff(alpha(x, y - 1)))
            .max()
            .unwrap_or(0);
        assert!(
            largest_step < 12,
            "reflection has visible bands: {largest_step}"
        );
        player.previous = Some(0);
        player.current = 1;
        player.switched_at = Instant::now() - Duration::from_millis(450);
        stage.sync(&player, 1_016_667);
        let focus = stage.coverflow.as_ref().unwrap().canvas.imp().focus.get();
        assert!(focus > 0.0 && focus < 1.0);
        window.close();
    }
}
