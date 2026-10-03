//! Unified diff renderer with hunk / line staging.

use std::collections::BTreeSet;

use egui::text::{LayoutJob, TextFormat};
use egui::{Align2, Color32, Pos2, Rect, RichText, Sense, Vec2};

use crate::git::diff::{FileDiff, LineKind, Selection};
use crate::repo::PatchAction;
use crate::ui::theme::{self, Palette};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffMode {
    ReadOnly,
    /// Working tree changes: can stage or discard.
    Unstaged,
    /// Index changes: can unstage.
    Staged,
}

#[derive(Debug, Clone)]
pub enum DiffRequest {
    Patch(Selection, PatchAction),
    /// Discard a hunk/lines: needs confirmation by the caller.
    ConfirmDiscard(Selection),
}

pub struct DiffState<'a> {
    pub selection: &'a mut BTreeSet<(usize, usize)>,
    pub last_clicked: &'a mut Option<(usize, usize)>,
}

#[derive(Clone, Copy)]
enum Row {
    Header(usize),
    Line(usize, usize),
}

fn row_at(diff: &FileDiff, mut index: usize) -> Option<Row> {
    for (h, hunk) in diff.hunks.iter().enumerate() {
        if index == 0 {
            return Some(Row::Header(h));
        }
        index -= 1;
        if index < hunk.lines.len() {
            return Some(Row::Line(h, index));
        }
        index -= hunk.lines.len();
    }
    None
}

fn row_count(diff: &FileDiff) -> usize {
    diff.hunks.iter().map(|h| h.lines.len() + 1).sum()
}

/// For a changed line, returns the byte range differing from its paired line
/// (removed lines pair with the added lines that follow them).
fn word_range(diff: &FileDiff, h: usize, l: usize) -> Option<(usize, usize)> {
    let lines = &diff.hunks[h].lines;
    let kind = lines[l].kind;
    if !matches!(kind, LineKind::Added | LineKind::Removed) {
        return None;
    }
    // Find the removed block start and the added block.
    let mut start = l;
    while start > 0 && matches!(lines[start - 1].kind, LineKind::Added | LineKind::Removed | LineKind::NoNewline) {
        start -= 1;
    }
    let removed: Vec<usize> = (start..lines.len())
        .take_while(|&i| lines[i].kind == LineKind::Removed || lines[i].kind == LineKind::NoNewline)
        .filter(|&i| lines[i].kind == LineKind::Removed)
        .collect();
    let after_removed = start + (start..lines.len()).take_while(|&i| matches!(lines[i].kind, LineKind::Removed | LineKind::NoNewline)).count();
    let added: Vec<usize> = (after_removed..lines.len())
        .take_while(|&i| lines[i].kind == LineKind::Added || lines[i].kind == LineKind::NoNewline)
        .filter(|&i| lines[i].kind == LineKind::Added)
        .collect();
    if removed.len() != added.len() || removed.is_empty() {
        return None;
    }
    let pos = if kind == LineKind::Removed {
        removed.iter().position(|&i| i == l)?
    } else {
        added.iter().position(|&i| i == l)?
    };
    let (a, b) = (&lines[removed[pos]].text, &lines[added[pos]].text);
    let prefix = a.bytes().zip(b.bytes()).take_while(|(x, y)| x == y).count();
    let prefix = floor_char_boundary(a, prefix.min(b.len()));
    let max_suffix = a.len().min(b.len()) - prefix;
    let suffix = a.bytes().rev().zip(b.bytes().rev()).take_while(|(x, y)| x == y).count().min(max_suffix);
    let this = &lines[l].text;
    let end = this.len() - suffix;
    let end = ceil_char_boundary(this, end);
    if prefix >= end || (prefix == 0 && end == this.len()) {
        return None;
    }
    Some((prefix, end))
}

fn floor_char_boundary(s: &str, mut i: usize) -> usize {
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

fn ceil_char_boundary(s: &str, mut i: usize) -> usize {
    while i < s.len() && !s.is_char_boundary(i) {
        i += 1;
    }
    i
}

/// Longest line prefix that is drawn. egui tessellates every glyph of a partly visible
/// galley, so a multi-megabyte single line (minified JSON, generated assets) would
/// otherwise exceed the GPU vertex buffer limit.
const MAX_LINE_CHARS: usize = 2000;

/// Byte index where display of `s` is cut off (`s.len()` if it fits).
fn display_cut(s: &str) -> usize {
    s.char_indices().nth(MAX_LINE_CHARS).map_or(s.len(), |(i, _)| i)
}

fn truncation_note(s: &str, cut: usize) -> Option<String> {
    (cut < s.len()).then(|| format!("  … {} more characters", s[cut..].chars().count()))
}

fn expand_tabs(s: &str) -> String {
    if s.contains('\t') { s.replace('\t', "    ") } else { s.to_owned() }
}

struct Geometry {
    font: egui::FontId,
    char_w: f32,
    gutter: f32,
    row_h: f32,
}

fn geometry(ui: &egui::Ui, diff: &FileDiff, font_size: f32) -> Geometry {
    let font = theme::mono(font_size);
    let char_w = ui.fonts_mut(|f| f.glyph_width(&font, '0'));
    let max_no = diff
        .hunks
        .iter()
        .flat_map(|h| h.lines.iter())
        .filter_map(|l| l.old_no.max(l.new_no))
        .max()
        .unwrap_or(1);
    let digits = max_no.to_string().len().max(3) as f32;
    Geometry {
        gutter: (digits * char_w + 10.0) * 2.0 + 18.0,
        row_h: (font_size * 1.6).round(),
        font,
        char_w,
    }
}

#[allow(clippy::too_many_arguments)]
fn paint_line(
    ui: &egui::Ui,
    rect: Rect,
    diff: &FileDiff,
    h: usize,
    l: usize,
    g: &Geometry,
    p: &Palette,
    selected: bool,
    text_color: Color32,
) {
    let painter = ui.painter();
    let line = &diff.hunks[h].lines[l];
    let bg = match line.kind {
        LineKind::Added => p.added_bg,
        LineKind::Removed => p.removed_bg,
        _ => Color32::TRANSPARENT,
    };
    painter.rect_filled(rect, 0.0, bg);
    let gutter_rect = Rect::from_min_size(rect.min, Vec2::new(g.gutter, rect.height()));
    if selected {
        painter.rect_filled(gutter_rect, 0.0, p.line_selected);
        painter.rect_filled(Rect::from_min_size(rect.min, Vec2::new(3.0, rect.height())), 0.0, p.selection);
    }
    let digits_w = (g.gutter - 18.0) / 2.0;
    let y = rect.center().y;
    if let Some(n) = line.old_no {
        painter.text(Pos2::new(rect.left() + digits_w - 4.0, y), Align2::RIGHT_CENTER, n.to_string(), g.font.clone(), p.line_no);
    }
    if let Some(n) = line.new_no {
        painter.text(Pos2::new(rect.left() + digits_w * 2.0 - 4.0, y), Align2::RIGHT_CENTER, n.to_string(), g.font.clone(), p.line_no);
    }
    let (marker, marker_color) = match line.kind {
        LineKind::Added => ("+", p.added),
        LineKind::Removed => ("-", p.removed),
        _ => ("", p.line_no),
    };
    painter.text(Pos2::new(rect.left() + digits_w * 2.0 + 4.0, y), Align2::LEFT_CENTER, marker, g.font.clone(), marker_color);

    let text_pos = Pos2::new(rect.left() + g.gutter, y);
    if line.kind == LineKind::NoNewline {
        painter.text(text_pos, Align2::LEFT_CENTER, &line.text, g.font.clone(), p.muted);
        return;
    }
    let fmt = |bg: Color32| TextFormat { font_id: g.font.clone(), color: text_color, background: bg, ..Default::default() };
    let mut job = LayoutJob::default();
    let text = &line.text;
    let cut = display_cut(text);
    match word_range(diff, h, l) {
        Some((a, b)) if a < cut => {
            let b = b.min(cut);
            let strong = if line.kind == LineKind::Added { p.added_word } else { p.removed_word };
            job.append(&expand_tabs(&text[..a]), 0.0, fmt(Color32::TRANSPARENT));
            job.append(&expand_tabs(&text[a..b]), 0.0, fmt(strong));
            job.append(&expand_tabs(&text[b..cut]), 0.0, fmt(Color32::TRANSPARENT));
        }
        _ => job.append(&expand_tabs(&text[..cut]), 0.0, fmt(Color32::TRANSPARENT)),
    }
    if let Some(note) = truncation_note(text, cut) {
        job.append(&note, 0.0, TextFormat { font_id: g.font.clone(), color: p.muted, ..Default::default() });
    }
    let galley = painter.layout_job(job);
    let pos = Pos2::new(text_pos.x, y - galley.size().y / 2.0);
    painter.galley(pos, galley, text_color);
}

fn content_width(diff: &FileDiff, g: &Geometry) -> f32 {
    let max_chars = diff
        .hunks
        .iter()
        .flat_map(|h| h.lines.iter())
        .map(|l| (l.text.len() + l.text.matches('\t').count() * 3).min(MAX_LINE_CHARS + 40))
        .max()
        .unwrap_or(0);
    g.gutter + max_chars as f32 * g.char_w + 40.0
}

/// Message shown instead of a diff (binary, empty, etc.). Returns true if handled.
fn special_cases(ui: &mut egui::Ui, diff: &FileDiff) -> bool {
    let p = theme::pal(ui);
    if let Some(notice) = &diff.notice {
        ui.centered_and_justified(|ui| ui.label(RichText::new(notice).color(p.muted)));
        return true;
    }
    if diff.binary {
        ui.centered_and_justified(|ui| ui.label(RichText::new("Binary file").color(p.muted)));
        return true;
    }
    if diff.hunks.is_empty() {
        let msg = diff
            .header
            .iter()
            .find(|h| h.starts_with("old mode") || h.starts_with("new mode") || h.starts_with("similarity") || h.starts_with("Directory"))
            .map(|h| h.as_str())
            .unwrap_or("No changes");
        ui.centered_and_justified(|ui| ui.label(RichText::new(msg).color(p.muted)));
        return true;
    }
    false
}

/// Full-size, virtualized diff view.
pub fn diff_view(
    ui: &mut egui::Ui,
    id_salt: impl std::hash::Hash + std::fmt::Debug,
    diff: &FileDiff,
    mode: DiffMode,
    state: DiffState<'_>,
    font_size: f32,
) -> Option<DiffRequest> {
    if special_cases(ui, diff) {
        return None;
    }
    let p = theme::pal(ui);
    let g = geometry(ui, diff, font_size);
    let total = row_count(diff);
    let width = content_width(diff, &g).max(ui.available_width());
    let text_color = ui.visuals().text_color();
    let can_partial = mode != DiffMode::ReadOnly && !diff.untracked;
    let mut request = None;

    // Toolbar for line selection.
    if can_partial && !state.selection.is_empty() {
        let hunk = state.selection.iter().next().map(|(h, _)| *h).unwrap_or(0);
        let lines: BTreeSet<usize> = state.selection.iter().filter(|(h, _)| *h == hunk).map(|(_, l)| *l).collect();
        let n = lines.len();
        egui::Frame::new().fill(p.hunk_bg).inner_margin(egui::Margin::symmetric(8, 4)).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(format!("{n} line{} selected", if n == 1 { "" } else { "s" }));
                match mode {
                    DiffMode::Unstaged => {
                        if ui.button(format!("{} Stage Lines", egui_phosphor::regular::PLUS)).clicked() {
                            request = Some(DiffRequest::Patch(Selection::Lines(hunk, lines.clone()), PatchAction::Stage));
                        }
                        if ui.button(format!("{} Discard Lines", egui_phosphor::regular::TRASH)).clicked() {
                            request = Some(DiffRequest::ConfirmDiscard(Selection::Lines(hunk, lines.clone())));
                        }
                    }
                    DiffMode::Staged => {
                        if ui.button(format!("{} Unstage Lines", egui_phosphor::regular::MINUS)).clicked() {
                            request = Some(DiffRequest::Patch(Selection::Lines(hunk, lines.clone()), PatchAction::Unstage));
                        }
                    }
                    DiffMode::ReadOnly => {}
                }
                if ui.button("Clear Selection").clicked() {
                    state.selection.clear();
                }
            });
        });
    }

    let shift = ui.input(|i| i.modifiers.shift);
    let toggle = ui.input(|i| i.modifiers.command);

    // Rows touch each other so added/removed blocks form continuous bands.
    ui.spacing_mut().item_spacing.y = 0.0;
    egui::ScrollArea::both()
        .id_salt(id_salt)
        .auto_shrink(false)
        .show_rows(ui, g.row_h, total, |ui, range| {
            ui.set_min_width(width);
            for index in range {
                let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, g.row_h), Sense::click());
                match row_at(diff, index) {
                    Some(Row::Header(h)) => {
                        ui.painter().rect_filled(rect, 0.0, p.hunk_bg);
                        let mut x = rect.left() + 6.0;
                        if can_partial {
                            let mut button = |ui: &mut egui::Ui, text: String| -> bool {
                                let galley = ui.painter().layout_no_wrap(text.clone(), egui::FontId::proportional(12.0), text_color);
                                let r = Rect::from_min_size(Pos2::new(x, rect.top() + 2.0), Vec2::new(galley.size().x + 12.0, rect.height() - 4.0));
                                x = r.right() + 6.0;
                                let resp = ui.interact(r, ui.id().with(("hunkbtn", h, &text)), Sense::click());
                                let fill = if resp.hovered() { p.selection } else { p.badge_bg };
                                ui.painter().rect(r, 4.0, fill, egui::Stroke::new(1.0, p.badge_border), egui::StrokeKind::Inside);
                                let color = if resp.hovered() { Color32::WHITE } else { text_color };
                                ui.painter().galley(Pos2::new(r.left() + 6.0, r.center().y - galley.size().y / 2.0), galley, color);
                                resp.clicked()
                            };
                            match mode {
                                DiffMode::Unstaged => {
                                    if button(ui, "Stage Hunk".into()) {
                                        request = Some(DiffRequest::Patch(Selection::Hunk(h), PatchAction::Stage));
                                    }
                                    if button(ui, "Discard Hunk".into()) {
                                        request = Some(DiffRequest::ConfirmDiscard(Selection::Hunk(h)));
                                    }
                                }
                                DiffMode::Staged => {
                                    if button(ui, "Unstage Hunk".into()) {
                                        request = Some(DiffRequest::Patch(Selection::Hunk(h), PatchAction::Unstage));
                                    }
                                }
                                DiffMode::ReadOnly => {}
                            }
                        }
                        ui.painter().text(
                            Pos2::new(x + 4.0, rect.center().y),
                            Align2::LEFT_CENTER,
                            &diff.hunks[h].header,
                            g.font.clone(),
                            p.hunk_text,
                        );
                    }
                    Some(Row::Line(h, l)) => {
                        let selected = state.selection.contains(&(h, l));
                        paint_line(ui, rect, diff, h, l, &g, &p, selected, text_color);
                        let kind = diff.hunks[h].lines[l].kind;
                        let selectable = can_partial && matches!(kind, LineKind::Added | LineKind::Removed);
                        if selectable && resp.hovered() {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                        }
                        if selectable && resp.clicked() {
                            select_line(diff, state.selection, state.last_clicked, (h, l), shift, toggle);
                        }
                    }
                    None => {}
                }
            }
        });
    request
}

fn select_line(
    diff: &FileDiff,
    selection: &mut BTreeSet<(usize, usize)>,
    last: &mut Option<(usize, usize)>,
    (h, l): (usize, usize),
    shift: bool,
    toggle: bool,
) {
    // Selections are limited to a single hunk.
    if selection.iter().any(|(sh, _)| *sh != h) {
        selection.clear();
    }
    match (*last, shift) {
        (Some((lh, ll)), true) if lh == h => {
            let (a, b) = if ll <= l { (ll, l) } else { (l, ll) };
            for i in a..=b {
                if matches!(diff.hunks[h].lines[i].kind, LineKind::Added | LineKind::Removed) {
                    selection.insert((h, i));
                }
            }
        }
        _ => {
            if toggle || selection.len() <= 1 || selection.contains(&(h, l)) {
                if !selection.remove(&(h, l)) {
                    selection.insert((h, l));
                }
            } else {
                selection.clear();
                selection.insert((h, l));
            }
        }
    }
    *last = Some((h, l));
}

/// Non-virtualized diff for embedding in an outer scroll area (commit tab).
/// `id_salt` must be unique among inline diffs shown at the same time (e.g. the file path).
pub fn diff_inline(ui: &mut egui::Ui, id_salt: impl std::hash::Hash + std::fmt::Debug, diff: &FileDiff, font_size: f32, max_rows: usize) {
    if special_cases(ui, diff) {
        return;
    }
    let p = theme::pal(ui);
    let g = geometry(ui, diff, font_size);
    let total = row_count(diff);
    let text_color = ui.visuals().text_color();
    let width = ui.available_width();
    egui::ScrollArea::horizontal().id_salt(("inline_diff", id_salt)).auto_shrink([false, true]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        let width = content_width(diff, &g).max(width);
        for index in 0..total.min(max_rows) {
            let (rect, _) = ui.allocate_exact_size(Vec2::new(width, g.row_h), Sense::hover());
            if !ui.is_rect_visible(rect) {
                continue;
            }
            match row_at(diff, index) {
                Some(Row::Header(h)) => {
                    ui.painter().rect_filled(rect, 0.0, p.hunk_bg);
                    ui.painter().text(
                        Pos2::new(rect.left() + 8.0, rect.center().y),
                        Align2::LEFT_CENTER,
                        &diff.hunks[h].header,
                        g.font.clone(),
                        p.hunk_text,
                    );
                }
                Some(Row::Line(h, l)) => paint_line(ui, rect, diff, h, l, &g, &p, false, text_color),
                None => {}
            }
        }
    });
    if total > max_rows {
        ui.label(RichText::new(format!("… {} more lines. Open the Changes tab to see the full diff.", total - max_rows)).color(p.muted));
    }
}

/// Plain text file viewer with line numbers (File Tree tab).
pub fn text_view(ui: &mut egui::Ui, id_salt: impl std::hash::Hash + std::fmt::Debug, text: &str, font_size: f32) {
    let p = theme::pal(ui);
    let lines: Vec<&str> = text.lines().collect();
    let font = theme::mono(font_size);
    let char_w = ui.fonts_mut(|f| f.glyph_width(&font, '0'));
    let row_h = (font_size * 1.6).round();
    let digits = lines.len().to_string().len().max(3) as f32;
    let gutter = digits * char_w + 16.0;
    let max_chars = lines.iter().map(|l| l.len().min(MAX_LINE_CHARS + 40)).max().unwrap_or(0);
    let width = (gutter + max_chars as f32 * char_w + 40.0).max(ui.available_width());
    let text_color = ui.visuals().text_color();
    ui.spacing_mut().item_spacing.y = 0.0;
    egui::ScrollArea::both().id_salt(id_salt).auto_shrink(false).show_rows(ui, row_h, lines.len(), |ui, range| {
        ui.set_min_width(width);
        for i in range {
            let (rect, _) = ui.allocate_exact_size(Vec2::new(width, row_h), Sense::hover());
            let y = rect.center().y;
            ui.painter().text(Pos2::new(rect.left() + gutter - 8.0, y), Align2::RIGHT_CENTER, (i + 1).to_string(), font.clone(), p.line_no);
            let line = lines[i];
            let cut = display_cut(line);
            let mut shown = expand_tabs(&line[..cut]);
            if let Some(note) = truncation_note(line, cut) {
                shown.push_str(&note);
            }
            ui.painter().text(Pos2::new(rect.left() + gutter, y), Align2::LEFT_CENTER, shown, font.clone(), text_color);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::diff::parse;

    #[test]
    fn word_ranges_for_paired_lines() {
        let d = parse("@@ -1,1 +1,1 @@\n-let x = 1;\n+let x = 42;\n");
        assert_eq!(word_range(&d, 0, 0), Some((8, 9)));
        assert_eq!(word_range(&d, 0, 1), Some((8, 10)));
    }

    #[test]
    fn row_mapping() {
        let d = parse("@@ -1,1 +1,1 @@\n-a\n+b\n@@ -10,1 +10,1 @@\n c\n");
        assert_eq!(row_count(&d), 5);
        assert!(matches!(row_at(&d, 0), Some(Row::Header(0))));
        assert!(matches!(row_at(&d, 2), Some(Row::Line(0, 1))));
        assert!(matches!(row_at(&d, 3), Some(Row::Header(1))));
        assert!(matches!(row_at(&d, 4), Some(Row::Line(1, 0))));
    }
}
