//! Image preview widgets (single image, or before/after comparison).

use egui::{Color32, Rect, RichText, Sense, Vec2};

use crate::preview::{DecodedImage, ImagePair, Side};
use crate::repo::{Loaded, RepoTab};
use crate::ui::theme;

/// Transparent-background pattern behind the image.
fn checkerboard(ui: &egui::Ui, rect: Rect) {
    let dark = ui.visuals().dark_mode;
    let (a, b) = if dark {
        (Color32::from_gray(46), Color32::from_gray(58))
    } else {
        (Color32::from_gray(236), Color32::from_gray(250))
    };
    let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
    painter.rect_filled(rect, 0.0, a);
    const CELL: f32 = 8.0;
    let cols = (rect.width() / CELL).ceil() as usize;
    let rows = (rect.height() / CELL).ceil() as usize;
    if cols * rows > 40_000 {
        return;
    }
    for r in 0..rows {
        for c in (r % 2..cols).step_by(2) {
            let min = rect.min + Vec2::new(c as f32 * CELL, r as f32 * CELL);
            painter.rect_filled(Rect::from_min_size(min, Vec2::splat(CELL)), 0.0, b);
        }
    }
}

fn draw_image(ui: &mut egui::Ui, img: &mut DecodedImage, name: &str, max: Vec2) {
    let p = theme::pal(ui);
    ui.label(
        RichText::new(format!("{} · {} × {} · {}", img.format, img.width, img.height, crate::format::human_size(img.bytes as u64)))
            .small()
            .color(p.muted),
    );
    // Show at one image pixel per screen pixel; shrink to fit, magnify tiny icons.
    let ppp = ui.ctx().pixels_per_point();
    let natural = Vec2::new(img.width as f32, img.height as f32) / ppp;
    let fit = (max.x / natural.x).min(max.y / natural.y);
    let limit = if img.width.max(img.height) <= 128 { 4.0 } else { 1.0 };
    let size = natural * fit.min(limit).max(0.01);
    let Some(texture) = img.texture(ui.ctx(), name) else { return };
    let id = texture.id();
    let (rect, resp) = ui.allocate_exact_size(size, Sense::hover());
    checkerboard(ui, rect);
    ui.painter().image(id, rect, Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
    ui.painter().rect_stroke(rect, 0.0, egui::Stroke::new(1.0, p.border), egui::StrokeKind::Outside);
    resp.on_hover_text(format!("{} × {} px", img.width, img.height));
}

fn draw_side(ui: &mut egui::Ui, side: &mut Side, name: &str, missing: &str, max: Vec2) {
    let p = theme::pal(ui);
    match side {
        Some(Ok(img)) => draw_image(ui, img, name, max),
        Some(Err(e)) => {
            ui.label(RichText::new(e.as_str()).color(p.removed));
        }
        None => {
            ui.label(RichText::new(missing).color(p.muted));
        }
    }
}

/// Shows the preview for `key`, or a spinner while it loads. `max_height` limits embedded use.
/// `compare` labels a lone version as Added/Deleted (diff views); off for plain file views.
pub fn show(ui: &mut egui::Ui, tab: &mut RepoTab, key: &str, max_height: Option<f32>, compare: bool) {
    let Some(entry) = tab.image_previews.get_mut(key) else { return };
    let pair: &mut ImagePair = match entry {
        Loaded::Ready(pair) => pair,
        Loaded::Failed(e) => {
            ui.colored_label(theme::pal(ui).removed, e.as_str());
            return;
        }
        Loaded::Loading => {
            ui.spinner();
            return;
        }
    };
    let p = theme::pal(ui);
    let avail = ui.available_size();
    let height = max_height.unwrap_or(avail.y - 40.0).max(60.0);
    let both = pair.old.is_some() && pair.new.is_some();
    let scroll = egui::ScrollArea::both().id_salt(("image_preview", key)).auto_shrink([false, true]);
    scroll.show(ui, |ui| {
        if both {
            let col_w = ((avail.x - 24.0) / 2.0).max(80.0);
            ui.horizontal_top(|ui| {
                for (title, side, suffix) in [("Before", &mut pair.old, "old"), ("After", &mut pair.new, "new")] {
                    ui.allocate_ui(Vec2::new(col_w, height + 40.0), |ui| {
                        ui.vertical(|ui| {
                            ui.label(RichText::new(title).strong());
                            draw_side(ui, side, &format!("{key}:{suffix}"), "", Vec2::new(col_w, height));
                        });
                    });
                    ui.add_space(16.0);
                }
            });
        } else if pair.new.is_some() {
            if compare {
                ui.label(RichText::new("Added").strong().color(p.added));
            }
            draw_side(ui, &mut pair.new, &format!("{key}:new"), "", Vec2::new(avail.x - 8.0, height));
        } else if pair.old.is_some() {
            if compare {
                ui.label(RichText::new("Deleted").strong().color(p.removed));
            }
            draw_side(ui, &mut pair.old, &format!("{key}:old"), "", Vec2::new(avail.x - 8.0, height));
        } else {
            ui.label(RichText::new("Image not available").color(p.muted));
        }
    });
}
