//! Colors, fonts and small formatting helpers shared by all views.

use egui::{Color32, FontFamily, FontId, Visuals};

pub const ROW_HEIGHT: f32 = 24.0;
pub const LANE_WIDTH: f32 = 14.0;

const LANES: [Color32; 10] = [
    Color32::from_rgb(0xf5, 0x9e, 0x0b), // amber
    Color32::from_rgb(0x3b, 0x82, 0xf6), // blue
    Color32::from_rgb(0x22, 0xc5, 0x5e), // green
    Color32::from_rgb(0xa8, 0x55, 0xf7), // purple
    Color32::from_rgb(0xef, 0x44, 0x44), // red
    Color32::from_rgb(0x06, 0xb6, 0xd4), // cyan
    Color32::from_rgb(0xec, 0x48, 0x99), // pink
    Color32::from_rgb(0x84, 0xcc, 0x16), // lime
    Color32::from_rgb(0xf9, 0x73, 0x16), // orange
    Color32::from_rgb(0x63, 0x66, 0xf1), // indigo
];

pub fn lane_color(i: u16) -> Color32 {
    LANES[i as usize % LANES.len()]
}

#[derive(Clone, Copy)]
pub struct Palette {
    pub selection: Color32,
    pub selection_text: Color32,
    pub hover: Color32,
    pub muted: Color32,
    pub sidebar: Color32,
    pub panel: Color32,
    pub border: Color32,
    pub added_bg: Color32,
    pub removed_bg: Color32,
    pub added_word: Color32,
    pub removed_word: Color32,
    pub hunk_bg: Color32,
    pub hunk_text: Color32,
    pub line_no: Color32,
    pub line_selected: Color32,
    pub added: Color32,
    pub removed: Color32,
    pub modified: Color32,
    pub renamed: Color32,
    pub conflict: Color32,
    pub badge_bg: Color32,
    pub badge_border: Color32,
    pub tag: Color32,
    pub warning_bg: Color32,
}

pub fn palette(dark: bool) -> Palette {
    if dark {
        Palette {
            selection: Color32::from_rgb(0x0b, 0x5c, 0xd5),
            selection_text: Color32::WHITE,
            hover: Color32::from_rgb(0x2c, 0x2d, 0x30),
            muted: Color32::from_rgb(0x9a, 0x9c, 0xa3),
            sidebar: Color32::from_rgb(0x23, 0x24, 0x27),
            panel: Color32::from_rgb(0x1b, 0x1c, 0x1e),
            border: Color32::from_rgb(0x36, 0x37, 0x3b),
            added_bg: Color32::from_rgb(0x1d, 0x33, 0x24),
            removed_bg: Color32::from_rgb(0x3d, 0x20, 0x22),
            added_word: Color32::from_rgb(0x2a, 0x5e, 0x39),
            removed_word: Color32::from_rgb(0x74, 0x2e, 0x32),
            hunk_bg: Color32::from_rgb(0x24, 0x2a, 0x36),
            hunk_text: Color32::from_rgb(0x8f, 0xa8, 0xd8),
            line_no: Color32::from_rgb(0x6b, 0x6e, 0x76),
            line_selected: Color32::from_rgb(0x2b, 0x44, 0x7a),
            added: Color32::from_rgb(0x3f, 0xb9, 0x50),
            removed: Color32::from_rgb(0xf8, 0x51, 0x49),
            modified: Color32::from_rgb(0xe3, 0xb3, 0x41),
            renamed: Color32::from_rgb(0x58, 0xa6, 0xff),
            conflict: Color32::from_rgb(0xff, 0x7b, 0x72),
            badge_bg: Color32::from_rgb(0x2d, 0x2f, 0x33),
            badge_border: Color32::from_rgb(0x55, 0x58, 0x5f),
            tag: Color32::from_rgb(0xd2, 0xa8, 0xff),
            warning_bg: Color32::from_rgb(0x4a, 0x3b, 0x12),
        }
    } else {
        Palette {
            selection: Color32::from_rgb(0x1a, 0x6f, 0xe8),
            selection_text: Color32::WHITE,
            hover: Color32::from_rgb(0xea, 0xec, 0xf0),
            muted: Color32::from_rgb(0x6a, 0x6e, 0x76),
            sidebar: Color32::from_rgb(0xf1, 0xf2, 0xf4),
            panel: Color32::from_rgb(0xfb, 0xfb, 0xfc),
            border: Color32::from_rgb(0xd5, 0xd8, 0xdd),
            added_bg: Color32::from_rgb(0xe6, 0xf6, 0xe9),
            removed_bg: Color32::from_rgb(0xfc, 0xe9, 0xea),
            added_word: Color32::from_rgb(0xab, 0xe4, 0xb6),
            removed_word: Color32::from_rgb(0xf6, 0xb8, 0xbd),
            hunk_bg: Color32::from_rgb(0xe9, 0xef, 0xfa),
            hunk_text: Color32::from_rgb(0x3a, 0x5a, 0x9a),
            line_no: Color32::from_rgb(0x9a, 0x9e, 0xa6),
            line_selected: Color32::from_rgb(0xc9, 0xdb, 0xfb),
            added: Color32::from_rgb(0x1a, 0x7f, 0x37),
            removed: Color32::from_rgb(0xcf, 0x22, 0x2e),
            modified: Color32::from_rgb(0xb0, 0x7d, 0x00),
            renamed: Color32::from_rgb(0x09, 0x69, 0xda),
            conflict: Color32::from_rgb(0xcf, 0x22, 0x2e),
            badge_bg: Color32::from_rgb(0xff, 0xff, 0xff),
            badge_border: Color32::from_rgb(0xb8, 0xbc, 0xc4),
            tag: Color32::from_rgb(0x82, 0x50, 0xdf),
            warning_bg: Color32::from_rgb(0xff, 0xf3, 0xc4),
        }
    }
}

pub fn pal(ui: &egui::Ui) -> Palette {
    palette(ui.visuals().dark_mode)
}

pub fn mono(size: f32) -> FontId {
    FontId::new(size, FontFamily::Monospace)
}

pub fn apply_style(ctx: &egui::Context) {
    for dark in [true, false] {
        let p = palette(dark);
        let mut visuals = if dark {
            Visuals::dark()
        } else {
            Visuals::light()
        };
        visuals.panel_fill = p.panel;
        visuals.window_fill = if dark {
            Color32::from_rgb(0x25, 0x26, 0x29)
        } else {
            Color32::WHITE
        };
        visuals.extreme_bg_color = if dark {
            Color32::from_rgb(0x15, 0x16, 0x18)
        } else {
            Color32::WHITE
        };
        visuals.faint_bg_color = if dark {
            Color32::from_rgb(0x21, 0x22, 0x25)
        } else {
            Color32::from_rgb(0xf4, 0xf5, 0xf7)
        };
        visuals.selection.bg_fill = p.selection;
        visuals.selection.stroke.color = p.selection_text;
        visuals.hyperlink_color = p.renamed;
        visuals.widgets.noninteractive.bg_stroke.color = p.border;
        let theme = if dark {
            egui::Theme::Dark
        } else {
            egui::Theme::Light
        };
        ctx.set_visuals_of(theme, visuals);
        ctx.style_mut_of(theme, |style| {
            style.spacing.item_spacing = egui::vec2(6.0, 4.0);
            style.spacing.button_padding = egui::vec2(8.0, 3.0);
            style.spacing.interact_size.y = 22.0;
            style.spacing.scroll.floating = true;
            style
                .text_styles
                .insert(egui::TextStyle::Body, FontId::proportional(13.5));
            style
                .text_styles
                .insert(egui::TextStyle::Button, FontId::proportional(13.5));
            style
                .text_styles
                .insert(egui::TextStyle::Small, FontId::proportional(11.5));
            style
                .text_styles
                .insert(egui::TextStyle::Monospace, mono(12.5));
            style
                .text_styles
                .insert(egui::TextStyle::Heading, FontId::proportional(18.0));
        });
    }
}

pub fn install_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
    ctx.set_fonts(fonts);
}

pub fn format_time(ts: i64) -> String {
    use chrono::TimeZone;
    match chrono::Local.timestamp_opt(ts, 0).single() {
        Some(t) => t.format("%-d %b %Y at %H:%M").to_string(),
        None => String::new(),
    }
}

pub fn format_time_long(ts: i64) -> String {
    use chrono::TimeZone;
    match chrono::Local.timestamp_opt(ts, 0).single() {
        Some(t) => t.format("%-d %B %Y at %H:%M:%S %Z").to_string(),
        None => String::new(),
    }
}

/// Stable color derived from a string (for avatars).
pub fn hash_color(s: &str) -> Color32 {
    let mut h: u32 = 2166136261;
    for b in s.bytes() {
        h = (h ^ b as u32).wrapping_mul(16777619);
    }
    let hue = (h % 360) as f32 / 360.0;
    egui::ecolor::Hsva::new(hue, 0.55, 0.75, 1.0).into()
}

pub fn initials(name: &str) -> String {
    let mut parts = name.split_whitespace().filter_map(|w| w.chars().next());
    let first = parts.next();
    let last = parts.last();
    [first, last]
        .into_iter()
        .flatten()
        .flat_map(char::to_uppercase)
        .collect()
}

/// Paints a round avatar with initials.
pub fn avatar(ui: &mut egui::Ui, name: &str, email: &str, size: f32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
    let painter = ui.painter();
    painter.circle_filled(rect.center(), size / 2.0, hash_color(email));
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        initials(name),
        FontId::proportional(size * 0.42),
        Color32::WHITE,
    );
    resp
}

pub fn change_color(p: &Palette, kind: crate::git::ChangeKind) -> Color32 {
    use crate::git::ChangeKind::*;
    match kind {
        Added | Untracked => p.added,
        Deleted => p.removed,
        Renamed | Copied => p.renamed,
        Conflicted => p.conflict,
        Modified | TypeChanged => p.modified,
    }
}

/// Small colored square with the change letter (M/A/D/R…).
pub fn change_badge(ui: &mut egui::Ui, kind: crate::git::ChangeKind) {
    let p = pal(ui);
    let color = change_color(&p, kind);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(16.0, 16.0), egui::Sense::hover());
    ui.painter().rect_filled(rect.shrink(1.0), 3.0, color);
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        kind.letter(),
        FontId::new(11.0, FontFamily::Monospace),
        Color32::BLACK,
    );
}

/// Splits a path into (directory with trailing slash, file name).
pub fn split_path(path: &str) -> (&str, &str) {
    match path.rfind('/') {
        Some(i) => (&path[..=i], &path[i + 1..]),
        None => ("", path),
    }
}

/// Small "LFS" pill marking files tracked with Git LFS.
pub fn lfs_badge(ui: &mut egui::Ui, selected: bool) -> egui::Response {
    let color = if selected {
        Color32::WHITE
    } else {
        Color32::from_rgb(0x2d, 0xd4, 0xbf)
    };
    let galley = ui
        .painter()
        .layout_no_wrap("LFS".to_owned(), FontId::proportional(9.5), color);
    let size = egui::vec2(galley.size().x + 8.0, 14.0);
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::hover());
    ui.painter().rect_stroke(
        rect,
        3.0,
        egui::Stroke::new(1.0, color),
        egui::StrokeKind::Inside,
    );
    ui.painter()
        .galley(rect.center() - galley.size() / 2.0, galley, color);
    resp.on_hover_text("Tracked with Git LFS")
}
