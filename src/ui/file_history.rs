//! File history window: commits that touched a file, and the file's diff in each.

use egui::{Align2, FontId, Key, Pos2, Rect, RichText, Sense, Vec2};
use egui_phosphor::regular as icon;

use crate::git::{self, ChangeKind, HistoryEntry};
use crate::preview::Source;
use crate::repo::{Loaded, RepoTab};
use crate::ui::{diff_view, image_view, theme};

const ROW_HEIGHT: f32 = 68.0;

/// Shows the history window if one is open for this repository.
pub fn show(ctx: &egui::Context, tab: &mut RepoTab, font_size: f32) {
    let Some(h) = &tab.file_history else { return };
    let title = format!("History — {}", h.path);
    let id = egui::ViewportId::from_hash_of(("file_history", &tab.path));
    let builder = egui::ViewportBuilder::default()
        .with_title(title)
        .with_inner_size([1100.0, 720.0])
        .with_min_inner_size([600.0, 360.0])
        .with_icon(eframe::icon_data::from_png_bytes(crate::ICON_PNG).unwrap_or_default());
    ctx.show_viewport_immediate(id, builder, |ui, _class| {
        if ui.input(|i| i.viewport().close_requested() || i.key_pressed(Key::Escape)) {
            tab.file_history = None;
            return;
        }
        window(ui, tab, font_size);
    });
}

fn window(ui: &mut egui::Ui, tab: &mut RepoTab, font_size: f32) {
    let p = theme::pal(ui);
    let Some(h) = &tab.file_history else { return };
    let (dir, name) = theme::split_path(&h.path);
    let (dir, name, rev) = (dir.to_owned(), name.to_owned(), h.rev.clone());
    egui::Panel::top("history_header")
        .frame(egui::Frame::new().fill(p.sidebar).inner_margin(egui::Margin::symmetric(12, 8)))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("History").size(18.0).strong());
                ui.add_space(8.0);
                ui.label(RichText::new(icon::FILE).size(16.0).color(p.muted));
                ui.label(RichText::new(dir).size(15.0).color(p.muted));
                ui.label(RichText::new(name).size(15.0).strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let label = tab.refs.head_branch.clone().filter(|_| rev == "HEAD").unwrap_or_else(|| git::short(&rev).to_owned());
                    ui.label(RichText::new(format!("{} {label}", icon::GIT_BRANCH)).size(14.0));
                });
            });
        });

    // Arrow keys move through the commits.
    let nav = ui.input(|i| i32::from(i.key_pressed(Key::ArrowDown)) - i32::from(i.key_pressed(Key::ArrowUp)));
    if nav != 0 && !ui.ctx().egui_wants_keyboard_input() {
        if let Some(h) = &tab.file_history {
            if let Loaded::Ready(entries) = &h.entries {
                let next = h.selected.map_or(0, |s| (s as i32 + nav).clamp(0, entries.len() as i32 - 1) as usize);
                tab.select_history_entry(next);
            }
        }
    }

    egui::Panel::left("history_commits")
        .resizable(true)
        .default_size(380.0)
        .min_size(240.0)
        .frame(egui::Frame::new().fill(p.panel).inner_margin(egui::Margin::symmetric(6, 6)))
        .show(ui, |ui| commit_list(ui, tab));

    egui::CentralPanel::default()
        .frame(egui::Frame::new().fill(p.panel).inner_margin(egui::Margin::symmetric(8, 6)))
        .show(ui, |ui| diff_pane(ui, tab, font_size));
}

fn commit_list(ui: &mut egui::Ui, tab: &mut RepoTab) {
    let p = theme::pal(ui);
    let Some(h) = &mut tab.file_history else { return };
    let entries = match &h.entries {
        Loaded::Ready(e) => e.clone(),
        Loaded::Failed(e) => {
            ui.colored_label(p.removed, e.as_str());
            return;
        }
        Loaded::Loading => {
            ui.centered_and_justified(|ui| ui.spinner());
            return;
        }
    };
    if entries.is_empty() {
        ui.centered_and_justified(|ui| ui.label(RichText::new("No commits touch this file").color(p.muted)));
        return;
    }
    let selected = h.selected;
    let scroll_to = std::mem::take(&mut h.scroll_to_selected);
    let mut clicked = None;
    let mut save_as = None;
    let mut reveal = None;
    ui.label(RichText::new(format!("{} commit{}", entries.len(), if entries.len() == 1 { "" } else { "s" })).small().color(p.muted));
    ui.spacing_mut().item_spacing.y = 0.0;
    egui::ScrollArea::vertical().auto_shrink(false).show_rows(ui, ROW_HEIGHT, entries.len(), |ui, range| {
        for i in range {
            let e = &entries[i];
            let is_sel = selected == Some(i);
            let resp = entry_row(ui, e, is_sel);
            if is_sel && scroll_to {
                resp.scroll_to_me(None);
            }
            if resp.clicked() {
                clicked = Some(i);
            }
            resp.context_menu(|ui| {
                crate::ui::history::copy_menu_item(ui, &format!("{} Copy SHA", icon::COPY), e.commit.id.clone());
                crate::ui::history::copy_menu_item(ui, "Copy subject", e.commit.subject.clone());
                if ui.button(format!("{} Show in Commit Graph", icon::GIT_COMMIT)).clicked() {
                    reveal = Some(e.commit.id.clone());
                    ui.close();
                }
                if e.file.kind != ChangeKind::Deleted && ui.button(format!("{} Save This Version As…", icon::FLOPPY_DISK)).clicked() {
                    save_as = Some((e.commit.id.clone(), e.file.path.clone()));
                    ui.close();
                }
            });
        }
    });
    if let Some(i) = clicked {
        tab.select_history_entry(i);
    }
    if let Some((id, path)) = save_as {
        tab.save_file_as(id, path);
    }
    if let Some(id) = reveal {
        tab.reveal_commit(&id);
    }
}

fn entry_row(ui: &mut egui::Ui, e: &HistoryEntry, selected: bool) -> egui::Response {
    let p = theme::pal(ui);
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW_HEIGHT), Sense::click());
    if selected {
        ui.painter().rect_filled(rect.shrink2(Vec2::new(0.0, 1.0)), 5.0, p.selection);
    } else if resp.hovered() || resp.context_menu_opened() {
        ui.painter().rect_filled(rect.shrink2(Vec2::new(0.0, 1.0)), 5.0, p.hover);
    }
    let text = if selected { p.selection_text } else { ui.visuals().text_color() };
    let muted = if selected { p.selection_text.gamma_multiply(0.8) } else { p.muted };
    let left = rect.left() + 10.0;
    let right = rect.right() - 10.0;
    let painter = ui.painter().with_clip_rect(rect);

    // Line 1: avatar + author, date on the right.
    let y1 = rect.top() + 14.0;
    let avatar = Pos2::new(left + 8.0, y1);
    painter.circle_filled(avatar, 8.0, theme::hash_color(&e.commit.email));
    painter.text(avatar, Align2::CENTER_CENTER, theme::initials(&e.commit.author), FontId::proportional(8.0), egui::Color32::WHITE);
    let date = painter.text(Pos2::new(right, y1), Align2::RIGHT_CENTER, theme::format_time(e.commit.time), FontId::proportional(12.5), muted);
    painter.with_clip_rect(Rect::from_min_max(rect.min, Pos2::new(date.left() - 8.0, rect.bottom()))).text(
        Pos2::new(left + 22.0, y1),
        Align2::LEFT_CENTER,
        &e.commit.author,
        FontId::proportional(13.5),
        text,
    );

    // Line 2: subject, short SHA on the right.
    let y2 = rect.top() + 34.0;
    let sha = painter.text(Pos2::new(right, y2), Align2::RIGHT_CENTER, e.commit.short_id(), theme::mono(12.0), muted);
    painter.with_clip_rect(Rect::from_min_max(rect.min, Pos2::new(sha.left() - 8.0, rect.bottom()))).text(
        Pos2::new(left, y2),
        Align2::LEFT_CENTER,
        &e.commit.subject,
        FontId::proportional(13.5),
        text,
    );

    // Line 3: change marker and the file's path in that commit (renames show both).
    let y3 = rect.top() + 54.0;
    let badge = Rect::from_center_size(Pos2::new(left + 7.0, y3), Vec2::splat(14.0));
    painter.rect_filled(badge, 3.0, theme::change_color(&p, e.file.kind));
    painter.text(badge.center(), Align2::CENTER_CENTER, e.file.kind.letter(), theme::mono(10.0), egui::Color32::BLACK);
    let path = match &e.file.old_path {
        Some(old) if *old != e.file.path => format!("{old}  {}  {}", icon::ARROW_RIGHT, e.file.path),
        _ => e.file.path.clone(),
    };
    painter.text(Pos2::new(left + 20.0, y3), Align2::LEFT_CENTER, path, FontId::proportional(12.5), muted);

    resp.on_hover_text(format!("{}\n{} <{}>\n{}", e.commit.id, e.commit.author, e.commit.email, theme::format_time_long(e.commit.time)))
}

fn diff_pane(ui: &mut egui::Ui, tab: &mut RepoTab, font_size: f32) {
    let p = theme::pal(ui);
    let Some(h) = &tab.file_history else { return };
    let Loaded::Ready(entries) = &h.entries else { return };
    let Some(entry) = h.selected.and_then(|i| entries.get(i)).cloned() else {
        ui.centered_and_justified(|ui| ui.label(RichText::new("Select a commit").color(p.muted)));
        return;
    };
    let diff = h.diff.as_ref().map(|(_, d)| d);

    ui.horizontal(|ui| {
        theme::change_badge(ui, entry.file.kind);
        ui.label(RichText::new(&entry.file.path).strong());
        if let Some(Loaded::Ready(d)) = diff {
            ui.label(RichText::new(format!("+{}", d.added)).color(p.added));
            ui.label(RichText::new(format!("-{}", d.removed)).color(p.removed));
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(RichText::new(format!("{}  {}", entry.commit.short_id(), theme::format_time(entry.commit.time))).color(p.muted));
        });
    });
    ui.separator();

    if crate::preview::is_image_path(&entry.file.path) {
        let key = format!("hist:{}:{}", entry.commit.id, entry.file.path);
        let old = match (entry.commit.parents.first(), entry.file.kind) {
            (Some(parent), k) if k != ChangeKind::Added => {
                Some(Source::Commit(parent.clone(), entry.file.old_path.clone().unwrap_or_else(|| entry.file.path.clone())))
            }
            _ => None,
        };
        let new = (entry.file.kind != ChangeKind::Deleted).then(|| Source::Commit(entry.commit.id.clone(), entry.file.path.clone()));
        tab.request_images(&key, old, new);
        image_view::show(ui, tab, &key, None, true);
        return;
    }

    match diff {
        Some(Loaded::Ready(d)) => {
            let d = d.clone();
            let mut sel = Default::default();
            let mut last = None;
            diff_view::diff_view(
                ui,
                ("history_diff", &entry.commit.id, &entry.file.path),
                &d,
                diff_view::DiffMode::ReadOnly,
                diff_view::DiffState { selection: &mut sel, last_clicked: &mut last },
                font_size,
            );
        }
        Some(Loaded::Failed(e)) => {
            ui.colored_label(p.removed, e.as_str());
        }
        _ => {
            ui.centered_and_justified(|ui| ui.spinner());
        }
    }
}
