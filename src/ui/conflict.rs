//! Merge conflict panel and the built-in merge editor.

use egui::{Color32, Pos2, Rect, RichText, Sense, Stroke, Vec2};
use egui_phosphor::regular as icon;

use crate::git::conflict::{Choice, ConflictKind, Segment};
use crate::git::{FileChange, RepoState};
use crate::repo::RepoTab;
use crate::ui::Ctx;
use crate::ui::theme::{self, Palette};

fn ours_color(p: &Palette) -> Color32 {
    p.renamed
}

fn theirs_color(p: &Palette) -> Color32 {
    p.modified
}

fn operation_title(state: Option<RepoState>) -> &'static str {
    match state {
        Some(RepoState::Rebasing) => "Rebase Conflict",
        Some(RepoState::CherryPicking) => "Cherry-pick Conflict",
        Some(RepoState::Reverting) => "Revert Conflict",
        _ => "Merge Conflict",
    }
}

/// Shown instead of a diff for a conflicted file (or the merge editor when it is open).
pub fn panel(ui: &mut egui::Ui, tab: &mut RepoTab, cx: &mut Ctx, file: &FileChange) {
    if tab
        .merge_editor
        .as_ref()
        .is_some_and(|e| e.path == file.path)
    {
        merge_editor(ui, tab, cx);
        return;
    }
    let p = theme::pal(ui);
    let kind = tab
        .status
        .conflicts
        .get(&file.path)
        .copied()
        .unwrap_or(ConflictKind::BothModified);
    let (ours, theirs) = kind.sides();
    let sides = tab.refs.sides.clone().unwrap_or_default();
    let width = ui.available_width().min(760.0);

    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        ui.vertical_centered(|ui| {
            ui.set_max_width(width);
            ui.add_space(24.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new(icon::WARNING).size(52.0).color(p.modified));
                ui.add_space(10.0);
                ui.vertical(|ui| {
                    ui.label(RichText::new(operation_title(tab.refs.state)).size(20.0).strong());
                    ui.add_space(4.0);
                    let text = match kind {
                        ConflictKind::DeletedByUs | ConflictKind::DeletedByThem => {
                            "One side deleted this file and the other changed it. Keep the changed file or accept the deletion."
                        }
                        ConflictKind::BothDeleted => "Both sides deleted this file.",
                        _ => "This file was changed on both sides. Choose the local (ours) or remote (theirs) version, or merge the changes.",
                    };
                    ui.add(egui::Label::new(RichText::new(text).color(p.muted)).wrap());
                });
            });
            ui.add_space(24.0);

            // Local / Remote cards.
            let card_w = (width - 24.0) / 2.0 - 10.0;
            let mut take = None;
            ui.horizontal_top(|ui| {
                for (is_ours, title, who, change, color) in [
                    (true, "Local", sides.ours.as_str(), ours, ours_color(&p)),
                    (false, "Remote", sides.theirs.as_str(), theirs, theirs_color(&p)),
                ] {
                    egui::Frame::new()
                        .stroke(Stroke::new(1.5, color))
                        .corner_radius(10)
                        .inner_margin(egui::Margin::same(14))
                        .show(ui, |ui| {
                            ui.set_width(card_w - 30.0);
                            ui.vertical(|ui| {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(title).size(16.0).strong().color(color));
                                ui.label(RichText::new(if is_ours { "(ours)" } else { "(theirs)" }).color(p.muted));
                            });
                            ui.add_space(4.0);
                            ui.add(egui::Label::new(RichText::new(format!("{} {}", icon::GIT_BRANCH, if who.is_empty() { "—" } else { who }))).truncate());
                            ui.add(egui::Label::new(RichText::new(format!("{} {}", icon::FILE, file.path)).color(p.muted)).truncate());
                            let change_color = if change.exists() { p.muted } else { p.removed };
                            ui.label(RichText::new(change.label()).small().color(change_color));
                            ui.add_space(8.0);
                            let label = match (is_ours, change.exists()) {
                                (true, true) => "Use Local Version",
                                (false, true) => "Use Remote Version",
                                (true, false) => "Accept Local Deletion",
                                (false, false) => "Accept Remote Deletion",
                            };
                            if ui.add(egui::Button::new(RichText::new(label).color(Color32::WHITE)).fill(color.gamma_multiply(0.85))).clicked() {
                                take = Some(is_ours);
                            }
                            });
                        });
                    ui.add_space(20.0);
                }
            });
            if let Some(ours) = take {
                tab.take_conflict_side(&file.path, ours);
            }

            ui.add_space(20.0);
            let mut error = None;
            if kind.has_markers() {
                let merge = egui::Button::new(RichText::new(format!("{} Merge in Gitr", icon::GIT_MERGE)).size(14.5)).min_size(Vec2::new(240.0, 30.0));
                if ui.add(merge).on_hover_text("Pick local, remote or both for each conflicting part").clicked() {
                    if let Err(e) = tab.open_merge_editor(&file.path) {
                        error = Some(e);
                    }
                }
                ui.add_space(6.0);
            }
            let tool = crate::git::config_value(&tab.path, "merge.tool");
            let external = egui::Button::new(format!("{} Merge in External Tool", icon::ARROW_SQUARE_OUT)).min_size(Vec2::new(240.0, 30.0));
            let hint = match &tool {
                Some(t) => format!("Opens {t} (git mergetool)"),
                None => "Set merge.tool in your git config to use an external merge tool".to_owned(),
            };
            if ui.add_enabled(tool.is_some(), external).on_hover_text(hint.clone()).on_disabled_hover_text(hint).clicked() {
                tab.git(format!("Merge tool {}", file.path), vec!["mergetool".into(), "--no-prompt".into(), "--".into(), file.path.clone()]);
            }
            if let Some(e) = error {
                tab.notices.push(crate::repo::Notice { title: "Cannot open merge editor".into(), text: e, error: true });
            }
            let _ = cx;
        });
    });
}

fn choice_label(c: Choice) -> &'static str {
    match c {
        Choice::Ours => "Local",
        Choice::Theirs => "Remote",
        Choice::OursThenTheirs => "Local + Remote",
        Choice::TheirsThenOurs => "Remote + Local",
    }
}

/// Paints lines of code on a tinted background with a colored left bar.
fn code_block(ui: &mut egui::Ui, text: &str, bg: Color32, bar: Color32, font_size: f32) {
    let font = theme::mono(font_size);
    let row_h = (font_size * 1.6).round();
    let color = ui.visuals().text_color();
    let lines: Vec<&str> = if text.is_empty() {
        vec![""]
    } else {
        text.lines().collect()
    };
    for line in lines {
        let (rect, _) =
            ui.allocate_exact_size(Vec2::new(ui.available_width(), row_h), Sense::hover());
        ui.painter().rect_filled(rect, 0.0, bg);
        ui.painter().rect_filled(
            Rect::from_min_size(rect.min, Vec2::new(3.0, row_h)),
            0.0,
            bar,
        );
        let shown: String = line.replace('\t', "    ").chars().take(2000).collect();
        ui.painter().with_clip_rect(rect).text(
            Pos2::new(rect.left() + 10.0, rect.center().y),
            egui::Align2::LEFT_CENTER,
            shown,
            font.clone(),
            color,
        );
    }
}

fn merge_editor(ui: &mut egui::Ui, tab: &mut RepoTab, cx: &mut Ctx) {
    let p = theme::pal(ui);
    let font_size = cx.settings.diff_font_size;
    let sides = tab.refs.sides.clone().unwrap_or_default();
    let Some(editor) = tab.merge_editor.as_mut() else {
        return;
    };
    let total = editor
        .segments
        .iter()
        .filter(|s| matches!(s, Segment::Conflict(_)))
        .count();
    let resolved = editor
        .segments
        .iter()
        .filter(|s| matches!(s, Segment::Conflict(h) if h.choice.is_some()))
        .count();
    let mut save = false;
    let mut cancel = false;

    ui.horizontal(|ui| {
        ui.label(RichText::new(format!("{} Merging", icon::GIT_MERGE)).strong());
        let color = if resolved == total {
            p.added
        } else {
            p.modified
        };
        ui.label(
            RichText::new(format!(
                "{resolved} of {total} conflict{} resolved",
                if total == 1 { "" } else { "s" }
            ))
            .color(color),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let done = resolved == total;
            let save_btn = if done {
                egui::Button::new(RichText::new("Save & Mark Resolved").color(Color32::WHITE))
                    .fill(p.selection)
            } else {
                egui::Button::new("Save & Mark Resolved")
            };
            if ui
                .add_enabled(done, save_btn)
                .on_disabled_hover_text("Resolve every conflict first")
                .clicked()
            {
                save = true;
            }
            if ui.button("Cancel").clicked() {
                cancel = true;
            }
            ui.separator();
            // Jump to the next / previous unresolved conflict.
            let unresolved: Vec<usize> = editor
                .segments
                .iter()
                .filter(|s| matches!(s, Segment::Conflict(_)))
                .enumerate()
                .filter(|(_, s)| matches!(s, Segment::Conflict(h) if h.choice.is_none()))
                .map(|(i, _)| i)
                .collect();
            if ui
                .add_enabled(!unresolved.is_empty(), egui::Button::new(icon::CARET_DOWN))
                .on_hover_text("Next unresolved conflict")
                .clicked()
            {
                editor.scroll_to = unresolved.first().copied();
            }
        });
    });
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(format!("■ Local (ours): {}", sides.ours))
                .small()
                .color(ours_color(&p)),
        );
        ui.add_space(12.0);
        ui.label(
            RichText::new(format!("■ Remote (theirs): {}", sides.theirs))
                .small()
                .color(theirs_color(&p)),
        );
    });
    ui.separator();

    let ours_bg = ours_color(&p).gamma_multiply(0.18);
    let theirs_bg = theirs_color(&p).gamma_multiply(0.18);
    let base_bg = p.muted.gamma_multiply(0.15);
    let scroll_to = editor.scroll_to.take();
    egui::ScrollArea::both()
        .id_salt(("merge_editor", &editor.path))
        .auto_shrink(false)
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            let count = editor.segments.len();
            let mut conflict_no = 0;
            for (si, segment) in editor.segments.iter_mut().enumerate() {
                match segment {
                    Segment::Text(text) => {
                        let lines: Vec<&str> = text.lines().collect();
                        // Show a little context around conflicts; fold long unchanged stretches.
                        let (head, tail) = (
                            if si == 0 { 0 } else { 3 },
                            if si + 1 == count { 0 } else { 3 },
                        );
                        if lines.len() > head + tail + 2 {
                            code_block(
                                ui,
                                &lines[..head].join("\n"),
                                Color32::TRANSPARENT,
                                Color32::TRANSPARENT,
                                font_size,
                            );
                            let hidden = lines.len() - head - tail;
                            ui.add_space(2.0);
                            ui.label(
                                RichText::new(format!(
                                    "      {} {hidden} unchanged lines",
                                    icon::DOTS_THREE
                                ))
                                .small()
                                .color(p.muted),
                            );
                            ui.add_space(2.0);
                            code_block(
                                ui,
                                &lines[lines.len() - tail..].join("\n"),
                                Color32::TRANSPARENT,
                                Color32::TRANSPARENT,
                                font_size,
                            );
                        } else if !lines.is_empty() {
                            code_block(
                                ui,
                                text.trim_end_matches('\n'),
                                Color32::TRANSPARENT,
                                Color32::TRANSPARENT,
                                font_size,
                            );
                        }
                    }
                    Segment::Conflict(hunk) => {
                        let index = conflict_no;
                        conflict_no += 1;
                        ui.add_space(6.0);
                        let frame = egui::Frame::new()
                            .stroke(Stroke::new(
                                1.0,
                                if hunk.choice.is_some() {
                                    p.added
                                } else {
                                    p.modified
                                },
                            ))
                            .corner_radius(6)
                            .inner_margin(egui::Margin::same(6))
                            .show(ui, |ui| {
                                ui.spacing_mut().item_spacing.y = 4.0;
                                ui.horizontal(|ui| {
                                    ui.label(
                                        RichText::new(format!("Conflict {} of {total}", index + 1))
                                            .strong(),
                                    );
                                    ui.add_space(8.0);
                                    for c in [
                                        Choice::Ours,
                                        Choice::Theirs,
                                        Choice::OursThenTheirs,
                                        Choice::TheirsThenOurs,
                                    ] {
                                        if ui
                                            .add(egui::Button::selectable(
                                                hunk.choice == Some(c),
                                                choice_label(c),
                                            ))
                                            .clicked()
                                        {
                                            hunk.choice = Some(c);
                                        }
                                    }
                                    if hunk.choice.is_some() && ui.small_button("Reset").clicked() {
                                        hunk.choice = None;
                                    }
                                });
                                ui.spacing_mut().item_spacing.y = 0.0;
                                match hunk.resolved_text() {
                                    Some(result) => {
                                        ui.label(
                                            RichText::new(format!(
                                                "Result ({})",
                                                choice_label(hunk.choice.unwrap())
                                            ))
                                            .small()
                                            .color(p.added),
                                        );
                                        code_block(
                                            ui,
                                            result.trim_end_matches('\n'),
                                            p.added_bg,
                                            p.added,
                                            font_size,
                                        );
                                    }
                                    None => {
                                        ui.label(
                                            RichText::new(format!("Local (ours) — {}", sides.ours))
                                                .small()
                                                .color(ours_color(&p)),
                                        );
                                        code_block(
                                            ui,
                                            hunk.ours.trim_end_matches('\n'),
                                            ours_bg,
                                            ours_color(&p),
                                            font_size,
                                        );
                                        if let Some(base) = &hunk.base {
                                            ui.add_space(4.0);
                                            ui.label(
                                                RichText::new("Base (common ancestor)")
                                                    .small()
                                                    .color(p.muted),
                                            );
                                            code_block(
                                                ui,
                                                base.trim_end_matches('\n'),
                                                base_bg,
                                                p.muted,
                                                font_size,
                                            );
                                        }
                                        ui.add_space(4.0);
                                        ui.label(
                                            RichText::new(format!(
                                                "Remote (theirs) — {}",
                                                sides.theirs
                                            ))
                                            .small()
                                            .color(theirs_color(&p)),
                                        );
                                        code_block(
                                            ui,
                                            hunk.theirs.trim_end_matches('\n'),
                                            theirs_bg,
                                            theirs_color(&p),
                                            font_size,
                                        );
                                    }
                                }
                            });
                        if scroll_to == Some(index) {
                            ui.scroll_to_rect(frame.response.rect, Some(egui::Align::Center));
                        }
                        ui.add_space(6.0);
                    }
                }
            }
        });

    if cancel {
        tab.merge_editor = None;
    } else if save {
        tab.save_merge();
    }
}
