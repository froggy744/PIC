//! Fullscreen photo Cover Flow, driven by the existing screensaver player.
use super::*;
use gtk::{gdk, graphene, gsk};

const COVER_TEXTURE_HEIGHT: f32 = 1600.0;

fn photo_node(texture: &gdk::Texture) -> gsk::RenderNode {
    let snapshot = gtk::Snapshot::new();
    let aspect_ratio = texture.width().max(1) as f32 / texture.height().max(1) as f32;
    let bounds = graphene::Rect::new(
        0.0,
        0.0,
        COVER_TEXTURE_HEIGHT * aspect_ratio,
        COVER_TEXTURE_HEIGHT,
    );
    snapshot.append_scaled_texture(texture, gsk::ScalingFilter::Trilinear, &bounds);
    snapshot.to_node().expect("photo cover node")
}

pub(super) struct CoverFlow {
    pub canvas: crate::coverflow::CoverflowGlArea,
    nodes: HashMap<usize, (u64, gsk::RenderNode)>,
    last_focus: Option<f64>,
    last_count: usize,
    last_size: (i32, i32),
}
impl CoverFlow {
    pub fn new(root: &gtk::Fixed) -> Self {
        let canvas = crate::coverflow::CoverflowGlArea::new("screensaver-coverflow-canvas", 1);
        canvas.widget().add_css_class("coverflow-album-style");
        canvas.widget().add_css_class("screensaver-coverflow-canvas");
        root.put(canvas.widget(), 0.0, 0.0);
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
            self.canvas.widget().set_size_request(size.0, size.1);
            self.last_size = size;
        }
        let animated = self.canvas.widget().settings().is_gtk_enable_animations();
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
                if self
                    .nodes
                    .get(&index)
                    .is_none_or(|(revision, _)| *revision != surface.revision)
                {
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
                        .map(|(revision, node)| crate::coverflow::CoverflowItem {
                            index,
                            revision: *revision,
                            aspect_ratio: player.frames[&index].texture.width().max(1) as f64
                                / player.frames[&index].texture.height().max(1) as f64,
                            node: node.clone(),
                        })
                })
                .collect();
            self.canvas.set_scene(player.count, focus, scene, false);
            self.last_focus = Some(focus);
            self.last_count = player.count;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coverflow::coverflow_pose;

    #[test]
    fn classic_cover_flow_keeps_full_height_cards_and_yaws_the_sides() {
        let viewport = (1200.0, 700.0);
        let left = coverflow_pose(1, 2.0, 5, viewport).unwrap();
        let center = coverflow_pose(2, 2.0, 5, viewport).unwrap();
        let right = coverflow_pose(3, 2.0, 5, viewport).unwrap();
        assert!((center.rotation_y).abs() < 1e-9);
        assert!((left.rotation_y.to_degrees() - 60.0).abs() < 1e-9);
        assert!((right.rotation_y.to_degrees() + 60.0).abs() < 1e-9);
        assert_eq!(left.card_height, center.card_height);
        assert_eq!(right.card_height, center.card_height);
        assert!(
            left.width < center.width * 0.75,
            "left card should narrow through perspective"
        );
        assert!(
            right.width < center.width * 0.75,
            "right card should narrow through perspective"
        );
        assert!((center.x + center.width / 2.0 - viewport.0 / 2.0).abs() < 1.0);
    }

    #[test]
    #[ignore = "requires GTK/OpenGL display; run under Xvfb with --ignored"]
    fn screensaver_coverflow_renders_in_a_live_gtk_window() {
        libadwaita::init().unwrap();
        let mut player = Player::new(Style::CoverFlow, 5, 5, false, 6);
        for index in 0..5 {
            let mut data = vec![255_u8; 96 * 96 * 4];
            for pixel in data.chunks_exact_mut(4) {
                pixel[0] = 50 + index as u8 * 35;
                pixel[1] = 100 + index as u8 * 15;
                pixel[2] = 210 - index as u8 * 25;
            }
            player.receive(ImageResult {
                index,
                high_quality: true,
                pixels: Some(Pixels {
                    width: 96,
                    height: 96,
                    data,
                }),
            });
        }
        let mut stage = Stage::new(Style::CoverFlow, 5, PicasaTransition::Crossfade);
        let window = gtk::Window::builder()
            .title("PIC Screensaver Cover Flow Validation")
            .decorated(false)
            .default_width(1000)
            .default_height(700)
            .child(&stage.root)
            .build();
        window.present();
        let context = glib::MainContext::default();
        for _ in 0..30 {
            while context.pending() {
                context.iteration(false);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        stage.sync(&player, 1_000_000);
        for _ in 0..30 {
            while context.pending() {
                context.iteration(false);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let canvas = &stage.coverflow.as_ref().unwrap().canvas;
        assert!(canvas.widget().width() > 0 && canvas.widget().height() > 0);
        let id = std::process::Command::new("xdotool")
            .args([
                "search",
                "--onlyvisible",
                "--name",
                "^PIC Screensaver Cover Flow Validation$",
            ])
            .output()
            .expect("find the screensaver GTK window");
        assert!(id.status.success());
        let id = String::from_utf8_lossy(&id.stdout)
            .lines()
            .last()
            .unwrap()
            .to_owned();
        let path = std::env::temp_dir().join("pic-screensaver-coverflow-live.png");
        let capture = std::process::Command::new("import")
            .args(["-window", &id, path.to_str().unwrap()])
            .status()
            .expect("capture the live screensaver GLArea");
        assert!(capture.success());
        assert!(std::fs::metadata(path).unwrap().len() > 10_000);
        window.close();
    }
}
