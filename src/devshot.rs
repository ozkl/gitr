//! Development aid (feature `screenshot`): `GITR_SCREENSHOT=out.png` captures the window
//! after `GITR_SCREENSHOT_DELAY` seconds (default 3) and exits. `GITR_DEV` holds
//! comma-separated setup actions applied once the repository has loaded.

use std::sync::Arc;
use std::time::Instant;

pub struct DevShot {
    path: Option<String>,
    started: Instant,
    delay: f64,
    requested: bool,
    pub actions: Vec<String>,
    pub actions_applied: bool,
}

impl DevShot {
    pub fn from_env() -> Self {
        Self {
            path: std::env::var("GITR_SCREENSHOT").ok(),
            started: Instant::now(),
            delay: std::env::var("GITR_SCREENSHOT_DELAY")
                .ok()
                .and_then(|d| d.parse().ok())
                .unwrap_or(3.0),
            requested: false,
            actions: std::env::var("GITR_DEV")
                .map(|s| {
                    s.split(',')
                        .map(|a| a.trim().to_owned())
                        .filter(|a| !a.is_empty())
                        .collect()
                })
                .unwrap_or_default(),
            actions_applied: false,
        }
    }

    pub fn tick(&mut self, ctx: &egui::Context) {
        let Some(path) = self.path.clone() else {
            return;
        };
        ctx.request_repaint();
        if !self.requested && self.started.elapsed().as_secs_f64() > self.delay {
            self.requested = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
        }
        let image: Option<Arc<egui::ColorImage>> = ctx.input(|i| {
            i.raw.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(image) = image {
            let [w, h] = image.size;
            let buf = image::RgbaImage::from_raw(w as u32, h as u32, image.as_raw().to_vec())
                .expect("image size");
            buf.save(&path).expect("save screenshot");
            std::process::exit(0);
        }
    }
}
