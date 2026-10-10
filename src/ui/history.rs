//! Commit list with graph, and the commit details panel.

use egui::epaint::CubicBezierShape;
use egui::{Align2, Color32, FontId, Pos2, Rect, RichText, Sense, Shape, Stroke, Vec2};
use egui_phosphor::regular as icon;

use crate::git::{self, CommitDetails, FileChange, RefKind, RefLabel};
use crate::repo::{DetailTab, Loaded, RepoTab, View};
use crate::ui::dialogs::{Dialog, ResetMode};
use crate::ui::theme::{self, LANE_WIDTH, ROW_HEIGHT, lane_color};
use crate::ui::{Ctx, diff_view};

fn s(v: &str) -> String {
    v.to_owned()
}

/// Which commits to show (all, or those matching the search).
fn visible_rows(tab: &RepoTab) -> Option<Vec<usize>> {
    let q = tab.search.trim().to_lowercase();
    if q.is_empty() {
        return None;
    }
    Some(
        tab.commits
            .iter()
            .enumerate()
            .filter(|(_, c)| {
                c.subject.to_lowercase().contains(&q)
                    || c.author.to_lowercase().contains(&q)
                    || c.email.to_lowercase().contains(&q)
                    || c.id.starts_with(&q)
                    || tab
                        .refs
                        .labels
                        .get(&c.id)
                        .is_some_and(|ls| ls.iter().any(|l| l.name.to_lowercase().contains(&q)))
            })
            .map(|(i, _)| i)
            .collect(),
    )
}

pub fn commit_list(ui: &mut egui::Ui, tab: &mut RepoTab, cx: &mut Ctx) {
    let p = theme::pal(ui);

    // Search bar.
    ui.horizontal(|ui| {
        ui.label(RichText::new(icon::MAGNIFYING_GLASS).color(p.muted));
        let r = ui.add(
            egui::TextEdit::singleline(&mut tab.search)
                .hint_text("Search commits (message, author, SHA, ref)")
                .desired_width(ui.available_width() - 90.0),
        );
        if cx.focus_search {
            r.request_focus();
        }
        if !tab.search.is_empty() && ui.button(icon::X).on_hover_text("Clear search").clicked() {
            tab.search.clear();
        }
        if !tab.log_loaded {
            ui.spinner();
        }
    });
    ui.add_space(2.0);

    if let Some(err) = &tab.load_error {
        ui.colored_label(p.removed, err);
    }

    let filtered = visible_rows(tab);
    let dirty = !tab.status.is_clean() && filtered.is_none();
    let offset = usize::from(dirty);
    let count = filtered.as_ref().map_or(tab.commits.len(), Vec::len) + offset;

    if tab.log_loaded && tab.commits.is_empty() && !dirty {
        ui.centered_and_justified(|ui| ui.label(RichText::new("No commits yet").color(p.muted)));
        return;
    }

    // Keyboard navigation.
    let nav = if !ui.ctx().egui_wants_keyboard_input() && cx.dialog.is_none() {
        ui.input(|i| {
            if i.key_pressed(egui::Key::ArrowDown) {
                1
            } else if i.key_pressed(egui::Key::ArrowUp) {
                -1
            } else {
                0
            }
        })
    } else {
        0
    };
    let pos_of = |_tab: &RepoTab, commit_idx: usize| -> Option<usize> {
        match &filtered {
            Some(rows) => rows.iter().position(|&r| r == commit_idx),
            None => Some(commit_idx + offset),
        }
    };
    let n_commits = tab.commits.len();
    let commit_at = |pos: usize| -> Option<usize> {
        if pos < offset {
            return None;
        }
        match &filtered {
            Some(rows) => rows.get(pos - offset).copied(),
            None => Some(pos - offset).filter(|&i| i < n_commits),
        }
    };
    if nav != 0 && count > offset {
        // The "uncommitted changes" row is reached by clicking; arrows move between commits.
        let current = tab
            .selected
            .as_ref()
            .and_then(|id| tab.index_of.get(id))
            .and_then(|&i| pos_of(tab, i));
        let next = match current {
            Some(c) => (c as i64 + nav).clamp(offset as i64, count as i64 - 1) as usize,
            None => offset,
        };
        if let Some(ci) = commit_at(next) {
            let id = tab.commits[ci].id.clone();
            tab.select_commit(id);
        }
        tab.scroll_to_selected = true;
    }

    let graph_width = if filtered.is_some() {
        LANE_WIDTH + 8.0
    } else {
        let lanes = tab.graph.iter().map(|r| r.width).max().unwrap_or(1) as f32;
        (lanes * LANE_WIDTH + 8.0).min(ui.available_width() * 0.35)
    };

    ui.spacing_mut().item_spacing.y = 0.0;
    let mut scroll = egui::ScrollArea::vertical()
        .id_salt("commit_list")
        .auto_shrink(false);
    if tab.scroll_to_selected {
        tab.scroll_to_selected = false;
        let target = if tab.view == View::LocalChanges && dirty {
            Some(0)
        } else {
            tab.selected
                .as_ref()
                .and_then(|id| tab.index_of.get(id))
                .and_then(|&i| pos_of(tab, i))
        };
        if let Some(t) = target {
            let row_h = ROW_HEIGHT;
            let view_h = ui.available_height();
            let current = ui
                .ctx()
                .data(|d| d.get_temp::<f32>(egui::Id::new(("commit_list_offset", &tab.path))))
                .unwrap_or(0.0);
            let y = t as f32 * row_h;
            if y < current || y + row_h > current + view_h {
                scroll = scroll.vertical_scroll_offset((y - view_h / 2.0).max(0.0));
            }
        }
    }

    let author_w = 150.0;
    let sha_w = 70.0;
    let date_w = 150.0;

    let out = scroll.show_rows(ui, ROW_HEIGHT, count, |ui, range| {
        let full_w = ui.available_width();
        let mut edges: Vec<Shape> = Vec::new();
        let mut nodes: Vec<Shape> = Vec::new();
        let mut row_rects: Vec<(usize, Rect)> = Vec::new();
        let text_color = ui.visuals().text_color();
        let right_cols = author_w + sha_w + date_w;
        let show_right = full_w > graph_width + right_cols + 200.0;

        for pos in range.clone() {
            let (rect, resp) =
                ui.allocate_exact_size(Vec2::new(full_w, ROW_HEIGHT), Sense::click());
            row_rects.push((pos, rect));
            let lane_x = |lane: u16| rect.left() + 10.0 + lane as f32 * LANE_WIDTH;

            if dirty && pos == 0 {
                // "Uncommitted changes" pseudo row.
                let selected = tab.view == View::LocalChanges;
                if selected {
                    ui.painter().rect_filled(rect, 4.0, p.selection);
                } else if resp.hovered() {
                    ui.painter().rect_filled(rect, 4.0, p.hover);
                }
                let head_lane = tab
                    .refs
                    .head_id
                    .as_ref()
                    .and_then(|h| tab.index_of.get(h))
                    .map(|&i| tab.graph[i].lane)
                    .unwrap_or(0);
                let c = Pos2::new(lane_x(head_lane), rect.center().y);
                nodes.push(Shape::circle_filled(c, 5.0, ui.visuals().panel_fill));
                nodes.push(Shape::circle_stroke(c, 4.5, Stroke::new(1.5, p.muted)));
                if tab.refs.head_id.as_ref().and_then(|h| tab.index_of.get(h)) == Some(&0) {
                    let below = Pos2::new(c.x, c.y + ROW_HEIGHT);
                    edges.push(dashed(c, below, p.muted));
                }
                let color = if selected {
                    p.selection_text
                } else {
                    text_color
                };
                ui.painter().text(
                    Pos2::new(rect.left() + graph_width + 4.0, rect.center().y),
                    Align2::LEFT_CENTER,
                    format!("Uncommitted changes ({})", tab.status.total()),
                    FontId::proportional(13.5),
                    color,
                );
                if resp.clicked() {
                    tab.view = View::LocalChanges;
                }
                continue;
            }

            let Some(ci) = commit_at(pos) else { continue };
            let commit = &tab.commits[ci];
            let row = &tab.graph[ci];
            let selected =
                tab.selected.as_deref() == Some(commit.id.as_str()) && tab.view == View::History;
            if selected {
                ui.painter().rect_filled(rect, 4.0, p.selection);
            } else if resp.hovered() || resp.context_menu_opened() {
                ui.painter().rect_filled(rect, 4.0, p.hover);
            }

            // Graph.
            let center = Pos2::new(lane_x(row.lane), rect.center().y);
            if filtered.is_none() {
                for e in &row.up {
                    let from = Pos2::new(lane_x(e.from), rect.center().y - ROW_HEIGHT);
                    let to = Pos2::new(lane_x(e.to), rect.center().y);
                    edges.push(edge(from, to, lane_color(e.color)));
                }
            }
            let color = lane_color(row.color);
            let is_head = tab.refs.head_id.as_deref() == Some(commit.id.as_str());
            if row.is_merge {
                nodes.push(Shape::circle_filled(center, 4.0, color));
            } else {
                nodes.push(Shape::circle_filled(center, 5.5, color));
                if is_head {
                    nodes.push(Shape::circle_filled(center, 2.5, ui.visuals().panel_fill));
                }
            }

            // Message with ref badges. Commits outside the checked-out history are dimmed;
            // the checked-out commit itself is bold.
            let current = tab.in_head.get(ci).copied().unwrap_or(true);
            let text_col = if selected {
                p.selection_text
            } else if current {
                text_color
            } else {
                p.muted
            };
            let muted = if selected {
                p.selection_text.gamma_multiply(0.85)
            } else if current {
                p.muted
            } else {
                p.line_no
            };
            // The bundled fonts have no bold weight; draw twice with a slight offset.
            let bold = |painter: &egui::Painter,
                        pos: Pos2,
                        galley: std::sync::Arc<egui::Galley>,
                        color: Color32| {
                if is_head {
                    painter.galley(pos + Vec2::new(0.6, 0.0), galley.clone(), color);
                }
                painter.galley(pos, galley, color);
            };
            let msg_right = if show_right {
                rect.right() - right_cols
            } else {
                rect.right() - 4.0
            };
            let mut x = rect.left() + graph_width + 4.0;
            if let Some(labels) = tab.refs.labels.get(&commit.id) {
                for badge in badges(labels) {
                    x = paint_badge(ui, &badge, Pos2::new(x, rect.center().y), msg_right, color)
                        + 5.0;
                    if x >= msg_right {
                        break;
                    }
                }
            }
            let clip = Rect::from_min_max(
                Pos2::new(x, rect.top()),
                Pos2::new(msg_right - 8.0, rect.bottom()),
            );
            let galley = ui.painter().layout_no_wrap(
                commit.subject.clone(),
                FontId::proportional(13.5),
                text_col,
            );
            bold(
                &ui.painter().with_clip_rect(clip),
                Pos2::new(x, rect.center().y - galley.size().y / 2.0),
                galley,
                text_col,
            );

            if show_right {
                let mut cx0 = rect.right() - right_cols;
                let clip_text =
                    |ui: &egui::Ui, x: f32, w: f32, text: &str, color: Color32, font: FontId| {
                        let r = Rect::from_min_max(
                            Pos2::new(x, rect.top()),
                            Pos2::new(x + w - 8.0, rect.bottom()),
                        );
                        let galley = ui.painter().layout_no_wrap(text.to_owned(), font, color);
                        let pos = Pos2::new(x, rect.center().y - galley.size().y / 2.0);
                        bold(&ui.painter().with_clip_rect(r), pos, galley, color);
                    };
                clip_text(
                    ui,
                    cx0,
                    author_w,
                    &commit.author,
                    text_col,
                    FontId::proportional(13.0),
                );
                cx0 += author_w;
                clip_text(ui, cx0, sha_w, commit.short_id(), muted, theme::mono(12.5));
                cx0 += sha_w;
                clip_text(
                    ui,
                    cx0,
                    date_w,
                    &theme::format_time(commit.time),
                    muted,
                    FontId::proportional(13.0),
                );
            }

            let id = commit.id.clone();
            if resp.clicked() {
                tab.select_commit(id.clone());
            }
            resp.context_menu(|ui| commit_menu(ui, tab, cx, &id));
        }

        // Edges into the row just below the visible range.
        if filtered.is_none() {
            if let Some(&(last_pos, last_rect)) = row_rects.last() {
                if let Some(ci) = commit_at(last_pos + 1) {
                    let y_to = last_rect.center().y + ROW_HEIGHT;
                    for e in &tab.graph[ci].up {
                        let from = Pos2::new(
                            last_rect.left() + 10.0 + e.from as f32 * LANE_WIDTH,
                            last_rect.center().y,
                        );
                        let to =
                            Pos2::new(last_rect.left() + 10.0 + e.to as f32 * LANE_WIDTH, y_to);
                        edges.push(edge(from, to, lane_color(e.color)));
                    }
                }
            }
        }
        let painter = ui.painter();
        painter.extend(edges);
        painter.extend(nodes);
    });
    let offset_y = out.state.offset.y;
    ui.ctx()
        .data_mut(|d| d.insert_temp(egui::Id::new(("commit_list_offset", &tab.path)), offset_y));
}

fn edge(from: Pos2, to: Pos2, color: Color32) -> Shape {
    let stroke = Stroke::new(2.0, color);
    if (from.x - to.x).abs() < 0.5 {
        return Shape::line_segment([from, to], stroke);
    }
    let mid = (from.y + to.y) / 2.0;
    CubicBezierShape::from_points_stroke(
        [from, Pos2::new(from.x, mid), Pos2::new(to.x, mid), to],
        false,
        Color32::TRANSPARENT,
        stroke,
    )
    .into()
}

fn dashed(from: Pos2, to: Pos2, color: Color32) -> Shape {
    Shape::dashed_line(&[from, to], Stroke::new(1.5, color), 3.0, 3.0)
        .into_iter()
        .collect::<Vec<_>>()
        .into()
}

struct Badge {
    text: String,
    kind: RefKind,
    is_head: bool,
    has_remote: bool,
}

/// Merges `main` and `origin/main` on the same commit into one badge.
fn badges(labels: &[RefLabel]) -> Vec<Badge> {
    let mut out: Vec<Badge> = Vec::new();
    for l in labels {
        match l.kind {
            RefKind::RemoteBranch => {
                let short = l.name.split_once('/').map(|(_, b)| b).unwrap_or(&l.name);
                if labels
                    .iter()
                    .any(|o| o.kind == RefKind::LocalBranch && o.name == short)
                {
                    continue;
                }
                out.push(Badge {
                    text: l.name.clone(),
                    kind: l.kind,
                    is_head: false,
                    has_remote: true,
                });
            }
            RefKind::LocalBranch => {
                let has_remote = labels.iter().any(|o| {
                    o.kind == RefKind::RemoteBranch
                        && o.name.split_once('/').is_some_and(|(_, b)| b == l.name)
                });
                out.push(Badge {
                    text: l.name.clone(),
                    kind: l.kind,
                    is_head: l.is_head,
                    has_remote,
                });
            }
            _ => out.push(Badge {
                text: l.name.clone(),
                kind: l.kind,
                is_head: l.is_head,
                has_remote: false,
            }),
        }
    }
    // Current branch first, then local, remote, tags.
    out.sort_by_key(|b| (!b.is_head, b.kind as u8));
    out
}

fn paint_badge(ui: &egui::Ui, badge: &Badge, left_center: Pos2, max_x: f32, lane: Color32) -> f32 {
    let p = theme::pal(ui);
    let text_color = ui.visuals().text_color();
    let mut text = String::new();
    if badge.has_remote {
        text.push_str(icon::CLOUD);
        text.push(' ');
    }
    match badge.kind {
        RefKind::Tag => {
            text.push_str(icon::TAG);
            text.push(' ');
        }
        RefKind::Head => {
            text.push_str(icon::GIT_COMMIT);
            text.push(' ');
        }
        _ if badge.is_head => {
            text.push_str(icon::CHECK);
            text.push(' ');
        }
        _ => {}
    }
    text.push_str(&badge.text);
    let font = FontId::proportional(12.0);
    let galley = ui.painter().layout_no_wrap(text, font, text_color);
    let w = (galley.size().x + 12.0).min((max_x - left_center.x).max(0.0));
    if w < 16.0 {
        return left_center.x;
    }
    let rect = Rect::from_min_size(
        Pos2::new(left_center.x, left_center.y - 9.0),
        Vec2::new(w, 18.0),
    );
    let (fill, stroke) = match badge.kind {
        RefKind::Tag => (p.badge_bg, p.tag),
        RefKind::RemoteBranch => (p.badge_bg, p.badge_border),
        // Opaque, so the badge reads the same on a selected (blue) row.
        _ if badge.is_head => (theme::mix(ui.visuals().panel_fill, lane, 0.35), lane),
        _ => (p.badge_bg, lane.gamma_multiply(0.8)),
    };
    ui.painter().rect(
        rect,
        4.0,
        fill,
        Stroke::new(1.0, stroke),
        egui::StrokeKind::Inside,
    );
    ui.painter().with_clip_rect(rect.shrink(1.0)).galley(
        Pos2::new(rect.left() + 6.0, left_center.y - galley.size().y / 2.0),
        galley,
        text_color,
    );
    rect.right()
}

pub fn copy_menu_item(ui: &mut egui::Ui, label: &str, text: String) {
    if ui.button(label).clicked() {
        ui.ctx().copy_text(text);
        ui.close();
    }
}

fn commit_menu(ui: &mut egui::Ui, tab: &mut RepoTab, cx: &mut Ctx, id: &str) {
    let short = git::short(id).to_owned();
    let head = tab
        .refs
        .head_branch
        .clone()
        .unwrap_or_else(|| "HEAD".into());
    let branches: Vec<RefLabel> = tab
        .refs
        .labels
        .get(id)
        .map(|ls| {
            ls.iter()
                .filter(|l| matches!(l.kind, RefKind::LocalBranch | RefKind::RemoteBranch))
                .cloned()
                .collect()
        })
        .unwrap_or_default();

    for b in &branches {
        if b.is_head {
            continue;
        }
        let name = b.name.clone();
        if ui
            .button(format!("{} Checkout '{name}'", icon::GIT_BRANCH))
            .clicked()
        {
            if b.kind == RefKind::RemoteBranch {
                tab.checkout_remote(&name);
            } else {
                tab.checkout(&name);
            }
            ui.close();
        }
    }
    if ui
        .button(format!(
            "{} Checkout commit {short} (detached)",
            icon::GIT_COMMIT
        ))
        .clicked()
    {
        tab.checkout(id);
        ui.close();
    }
    ui.separator();
    if ui
        .button(format!("{} Create Branch here…", icon::GIT_BRANCH))
        .clicked()
    {
        *cx.dialog = Some(Dialog::CreateBranch {
            start: id.to_owned(),
            start_label: short.clone(),
            name: String::new(),
            checkout: true,
        });
        ui.close();
    }
    if ui
        .button(format!("{} Create Tag here…", icon::TAG))
        .clicked()
    {
        *cx.dialog = Some(Dialog::CreateTag {
            target: id.to_owned(),
            name: String::new(),
            message: String::new(),
            push: false,
        });
        ui.close();
    }
    ui.separator();
    for b in branches.iter().filter(|b| !b.is_head) {
        if ui
            .button(format!(
                "{} Merge '{}' into '{head}'…",
                icon::GIT_MERGE,
                b.name
            ))
            .clicked()
        {
            *cx.dialog = Some(Dialog::Merge {
                source: b.name.clone(),
                no_ff: false,
                squash: false,
            });
            ui.close();
        }
        if ui
            .button(format!("Rebase '{head}' onto '{}'", b.name))
            .clicked()
        {
            tab.git(
                format!("Rebase onto {}", b.name),
                vec![s("rebase"), b.name.clone()],
            );
            ui.close();
        }
    }
    if branches.is_empty()
        && ui
            .button(format!("{} Merge commit into '{head}'…", icon::GIT_MERGE))
            .clicked()
    {
        // A shortened id keeps git's "Merge commit '…'" message readable.
        *cx.dialog = Some(Dialog::Merge {
            source: id[..id.len().min(12)].to_owned(),
            no_ff: false,
            squash: false,
        });
        ui.close();
    }
    if ui.button(format!("Rebase '{head}' onto {short}")).clicked() {
        *cx.dialog = Some(Dialog::confirm(
            "Rebase",
            format!("Rebase '{head}' onto {short}?"),
            "Rebase",
            "Rebase",
            vec![vec![s("rebase"), id.to_owned()]],
        ));
        ui.close();
    }
    if ui.button("Cherry-pick commit").clicked() {
        tab.git(
            format!("Cherry-pick {short}"),
            vec![s("cherry-pick"), id.to_owned()],
        );
        ui.close();
    }
    if ui.button("Revert commit").clicked() {
        tab.git(
            format!("Revert {short}"),
            vec![s("revert"), s("--no-edit"), id.to_owned()],
        );
        ui.close();
    }
    ui.menu_button(format!("Reset '{head}' to here"), |ui| {
        for (label, mode) in [
            ("Soft", ResetMode::Soft),
            ("Mixed", ResetMode::Mixed),
            ("Hard…", ResetMode::Hard),
        ] {
            if ui.button(label).clicked() {
                if mode == ResetMode::Hard {
                    *cx.dialog = Some(Dialog::Reset {
                        target: id.to_owned(),
                        mode,
                    });
                } else {
                    let flag = if mode == ResetMode::Soft {
                        "--soft"
                    } else {
                        "--mixed"
                    };
                    tab.git("Reset", vec![s("reset"), s(flag), id.to_owned()]);
                }
                ui.close();
            }
        }
    });
    ui.separator();
    copy_menu_item(ui, &format!("{} Copy SHA", icon::COPY), id.to_owned());
    copy_menu_item(ui, "Copy short SHA", short.clone());
    if let Some(c) = tab.index_of.get(id).map(|&i| &tab.commits[i]) {
        copy_menu_item(ui, "Copy subject", c.subject.clone());
    }
    if let Some(url) = tab
        .default_remote()
        .and_then(|r| tab.refs.remotes.iter().find(|x| x.name == r))
        .and_then(|r| crate::platform::web_url(&r.url))
    {
        if ui
            .button(format!("{} Open on remote", icon::GLOBE))
            .clicked()
        {
            crate::platform::open_url(&format!("{url}/commit/{id}"));
            ui.close();
        }
    }
}

// ---------------------------------------------------------------------------
// Details panel

pub fn details_panel(ui: &mut egui::Ui, tab: &mut RepoTab, cx: &mut Ctx) {
    ui.horizontal(|ui| {
        let before = tab.detail_tab;
        for (t, label) in [
            (DetailTab::Commit, "Commit"),
            (DetailTab::Changes, "Changes"),
            (DetailTab::FileTree, "File Tree"),
        ] {
            ui.selectable_value(&mut tab.detail_tab, t, label);
        }
        if tab.detail_tab != before && tab.detail_tab == DetailTab::FileTree {
            tab.load_tree();
        }
    });
    ui.separator();

    let details = match &tab.details {
        None => {
            ui.centered_and_justified(|ui| {
                ui.label(RichText::new("No commit selected").color(theme::pal(ui).muted))
            });
            return;
        }
        Some(Loaded::Loading) => {
            ui.centered_and_justified(|ui| ui.spinner());
            return;
        }
        Some(Loaded::Failed(e)) => {
            ui.colored_label(theme::pal(ui).removed, e);
            return;
        }
        Some(Loaded::Ready(d)) => d.clone(),
    };

    match tab.detail_tab {
        DetailTab::Commit => commit_tab(ui, tab, cx, &details),
        DetailTab::Changes => changes_tab(ui, tab, cx, &details),
        DetailTab::FileTree => file_tree_tab(ui, tab, cx),
    }
}

fn commit_header(ui: &mut egui::Ui, tab: &mut RepoTab, d: &CommitDetails) {
    let p = theme::pal(ui);
    ui.horizontal_top(|ui| {
        ui.add_space(4.0);
        theme::avatar(ui, &d.author, &d.author_email, 44.0);
        ui.add_space(8.0);
        ui.vertical(|ui| {
            egui::Grid::new("commit_meta")
                .num_columns(2)
                .spacing([10.0, 3.0])
                .show(ui, |ui| {
                    let key = |ui: &mut egui::Ui, t: &str| {
                        ui.label(RichText::new(t).small().color(p.muted))
                    };
                    key(ui, "AUTHOR");
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(&d.author).strong());
                        ui.label(RichText::new(&d.author_email).color(p.muted));
                        ui.label(
                            RichText::new(theme::format_time_long(d.author_time)).color(p.muted),
                        );
                    });
                    ui.end_row();
                    if d.committer != d.author || d.commit_time != d.author_time {
                        key(ui, "COMMITTER");
                        ui.horizontal(|ui| {
                            ui.label(&d.committer);
                            ui.label(RichText::new(&d.committer_email).color(p.muted));
                            ui.label(
                                RichText::new(theme::format_time_long(d.commit_time))
                                    .color(p.muted),
                            );
                        });
                        ui.end_row();
                    }
                    if let Some(labels) = tab.refs.labels.get(&d.id).cloned() {
                        key(ui, "REFS");
                        ui.horizontal_wrapped(|ui| {
                            for l in labels {
                                let prefix = match l.kind {
                                    RefKind::Tag => icon::TAG,
                                    RefKind::RemoteBranch => icon::CLOUD,
                                    _ => icon::GIT_BRANCH,
                                };
                                egui::Frame::new()
                                    .stroke(Stroke::new(1.0, p.badge_border))
                                    .corner_radius(4)
                                    .inner_margin(egui::Margin::symmetric(5, 1))
                                    .show(ui, |ui| ui.label(format!("{prefix} {}", l.name)));
                            }
                        });
                        ui.end_row();
                    }
                    key(ui, "SHA");
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(&d.id).monospace());
                        if ui
                            .small_button(icon::COPY)
                            .on_hover_text("Copy SHA")
                            .clicked()
                        {
                            ui.ctx().copy_text(d.id.clone());
                        }
                    });
                    ui.end_row();
                    if !d.parents.is_empty() {
                        key(
                            ui,
                            if d.parents.len() > 1 {
                                "PARENTS"
                            } else {
                                "PARENT"
                            },
                        );
                        ui.horizontal(|ui| {
                            for parent in &d.parents {
                                if ui
                                    .link(RichText::new(git::short(parent)).monospace())
                                    .clicked()
                                {
                                    tab.reveal_commit(parent);
                                }
                            }
                        });
                        ui.end_row();
                    }
                });
        });
    });
    ui.add_space(6.0);
    ui.separator();
    let (subject, body) = d.message.split_once('\n').unwrap_or((&d.message, ""));
    ui.add(egui::Label::new(RichText::new(subject).size(16.0).strong()).wrap());
    let body = body.trim();
    if !body.is_empty() {
        ui.add_space(4.0);
        ui.add(egui::Label::new(body).wrap().selectable(true));
    }
    ui.separator();
}

fn file_row(ui: &mut egui::Ui, file: &FileChange, selected: bool, lfs: bool) -> egui::Response {
    let p = theme::pal(ui);
    let (rect, resp) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 22.0), Sense::click());
    if selected {
        ui.painter().rect_filled(rect, 3.0, p.selection);
    } else if resp.hovered() {
        ui.painter().rect_filled(rect, 3.0, p.hover);
    }
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect.shrink2(Vec2::new(4.0, 0.0)))
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    theme::change_badge(&mut child, file.kind);
    if lfs {
        theme::lfs_badge(&mut child, selected);
    }
    let (dir, name) = theme::split_path(&file.path);
    let text_color = if selected {
        p.selection_text
    } else {
        child.visuals().text_color()
    };
    let muted = if selected {
        p.selection_text.gamma_multiply(0.8)
    } else {
        p.muted
    };
    let mut job = egui::text::LayoutJob::default();
    if let Some(old) = &file.old_path {
        job.append(
            &format!("{old} {} ", icon::ARROW_RIGHT),
            0.0,
            egui::TextFormat {
                color: muted,
                ..Default::default()
            },
        );
    }
    job.append(
        dir,
        0.0,
        egui::TextFormat {
            color: muted,
            ..Default::default()
        },
    );
    job.append(
        name,
        0.0,
        egui::TextFormat {
            color: text_color,
            ..Default::default()
        },
    );
    child.add(egui::Label::new(job).truncate().selectable(false));
    resp
}

fn file_menu(ui: &mut egui::Ui, tab: &mut RepoTab, file: &FileChange, commit: &str) {
    if file.kind != crate::git::ChangeKind::Deleted
        && ui
            .button(format!("{} History…", icon::CLOCK_COUNTER_CLOCKWISE))
            .clicked()
    {
        tab.open_file_history(commit, &file.path);
        ui.close();
    }
    copy_menu_item(ui, &format!("{} Copy path", icon::COPY), file.path.clone());
    if ui
        .button(format!("{} Show in file manager", icon::FOLDER_OPEN))
        .clicked()
    {
        tab.open_in_file_manager(Some(&file.path));
        ui.close();
    }
    if ui.button("Open file").clicked() {
        crate::platform::open_file(&tab.path.join(&file.path));
        ui.close();
    }
    ui.separator();
    if ui.button("Checkout this version of the file").clicked() {
        tab.git(
            format!("Checkout {}", file.path),
            vec![s("checkout"), commit.to_owned(), s("--"), file.path.clone()],
        );
        ui.close();
    }
}

fn commit_tab(ui: &mut egui::Ui, tab: &mut RepoTab, cx: &mut Ctx, d: &CommitDetails) {
    let p = theme::pal(ui);
    egui::ScrollArea::vertical()
        .id_salt("commit_tab")
        .auto_shrink(false)
        .show(ui, |ui| {
            commit_header(ui, tab, d);
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(format!(
                        "{} changed file{}",
                        d.files.len(),
                        if d.files.len() == 1 { "" } else { "s" }
                    ))
                    .color(p.muted),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let all_open = !d.files.is_empty()
                        && d.files.iter().all(|f| tab.expanded_files.contains(&f.path));
                    if ui
                        .small_button(if all_open {
                            "Collapse All"
                        } else {
                            "Expand All"
                        })
                        .clicked()
                    {
                        if all_open {
                            tab.expanded_files.clear();
                        } else {
                            for f in d.files.iter().take(100) {
                                tab.expanded_files.insert(f.path.clone());
                                tab.load_commit_diff(f);
                            }
                        }
                    }
                });
            });
            for file in &d.files {
                let open = tab.expanded_files.contains(&file.path);
                ui.horizontal(|ui| {
                    let caret = if open {
                        icon::CARET_DOWN
                    } else {
                        icon::CARET_RIGHT
                    };
                    ui.label(RichText::new(caret).color(p.muted));
                    let resp = file_row(ui, file, false, d.lfs.contains(&file.path));
                    if resp.clicked() {
                        if open {
                            tab.expanded_files.remove(&file.path);
                        } else {
                            tab.expanded_files.insert(file.path.clone());
                            tab.load_commit_diff(file);
                        }
                    }
                    resp.context_menu(|ui| file_menu(ui, tab, file, &d.id));
                });
                if open && crate::preview::is_image_path(&file.path) {
                    let key = request_commit_image(tab, d, file);
                    egui::Frame::new()
                        .stroke(Stroke::new(1.0, p.border))
                        .corner_radius(4)
                        .inner_margin(6)
                        .show(ui, |ui| {
                            crate::ui::image_view::show(ui, tab, &key, Some(260.0), true);
                        });
                    ui.add_space(4.0);
                } else if open {
                    match tab.commit_diffs.get(&file.path) {
                        Some(Loaded::Ready(diff)) => {
                            egui::Frame::new()
                                .stroke(Stroke::new(1.0, p.border))
                                .corner_radius(4)
                                .inner_margin(2)
                                .show(ui, |ui| {
                                    diff_view::diff_inline(
                                        ui,
                                        (&d.id, &file.path),
                                        diff,
                                        cx.settings.diff_font_size,
                                        1500,
                                    );
                                });
                        }
                        Some(Loaded::Failed(e)) => {
                            ui.colored_label(p.removed, e);
                        }
                        _ => {
                            ui.spinner();
                        }
                    }
                    ui.add_space(4.0);
                }
            }
        });
}

/// Requests a before/after image preview for a file changed in commit `d`; returns its key.
fn request_commit_image(tab: &mut RepoTab, d: &CommitDetails, file: &FileChange) -> String {
    use crate::git::ChangeKind;
    use crate::preview::Source;
    let key = format!("commit:{}:{}", d.id, file.path);
    let old = match (d.parents.first(), file.kind) {
        (Some(parent), k) if k != ChangeKind::Added => Some(Source::Commit(
            parent.clone(),
            file.old_path.clone().unwrap_or_else(|| file.path.clone()),
        )),
        _ => None,
    };
    let new =
        (file.kind != ChangeKind::Deleted).then(|| Source::Commit(d.id.clone(), file.path.clone()));
    tab.request_images(&key, old, new);
    key
}

fn diff_options(ui: &mut egui::Ui, tab: &mut RepoTab) -> bool {
    let mut changed = false;
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        changed |= ui
            .toggle_value(&mut tab.ignore_whitespace, "Ignore whitespace")
            .changed();
        if ui
            .small_button(icon::PLUS)
            .on_hover_text("More context")
            .clicked()
        {
            tab.diff_context = (tab.diff_context + 2).min(100);
            changed = true;
        }
        ui.label(format!("{} lines", tab.diff_context));
        if ui
            .small_button(icon::MINUS)
            .on_hover_text("Less context")
            .clicked()
        {
            tab.diff_context = tab.diff_context.saturating_sub(2);
            changed = true;
        }
    });
    changed
}

fn changes_tab(ui: &mut egui::Ui, tab: &mut RepoTab, cx: &mut Ctx, d: &CommitDetails) {
    if tab.commit_file.is_none() {
        if let Some(f) = d.files.first() {
            tab.commit_file = Some(f.path.clone());
        }
    }
    egui::Panel::left("commit_files")
        .resizable(true)
        .default_size(280.0)
        .min_size(160.0)
        .show(ui, |ui| {
            egui::ScrollArea::vertical()
                .id_salt("commit_files_scroll")
                .auto_shrink(false)
                .show(ui, |ui| {
                    for file in &d.files {
                        let selected = tab.commit_file.as_deref() == Some(file.path.as_str());
                        let resp = file_row(ui, file, selected, d.lfs.contains(&file.path));
                        if resp.clicked() {
                            tab.commit_file = Some(file.path.clone());
                        }
                        resp.context_menu(|ui| file_menu(ui, tab, file, &d.id));
                    }
                });
        });
    egui::CentralPanel::no_frame().show(ui, |ui| {
        let Some(file) = tab
            .commit_file
            .as_ref()
            .and_then(|p| d.files.iter().find(|f| f.path == *p))
            .cloned()
        else {
            return;
        };
        tab.load_commit_diff(&file);
        ui.horizontal(|ui| {
            theme::change_badge(ui, file.kind);
            ui.label(RichText::new(&file.path).strong());
            if d.lfs.contains(&file.path) {
                theme::lfs_badge(ui, false);
            }
            let is_image = crate::preview::is_image_path(&file.path);
            if let (Some(Loaded::Ready(diff)), false) = (tab.commit_diffs.get(&file.path), is_image)
            {
                ui.label(RichText::new(format!("+{}", diff.added)).color(theme::pal(ui).added));
                ui.label(RichText::new(format!("-{}", diff.removed)).color(theme::pal(ui).removed));
            }
            if !is_image && diff_options(ui, tab) {
                tab.reload_commit_diffs();
            }
        });
        ui.separator();
        if crate::preview::is_image_path(&file.path) {
            let key = request_commit_image(tab, &d, &file);
            crate::ui::image_view::show(ui, tab, &key, None, true);
            return;
        }
        match tab.commit_diffs.get(&file.path) {
            Some(Loaded::Ready(diff)) => {
                let diff = diff.clone();
                let mut sel = Default::default();
                let mut last = None;
                diff_view::diff_view(
                    ui,
                    ("commit_diff", &d.id, &file.path),
                    &diff,
                    diff_view::DiffMode::ReadOnly,
                    diff_view::DiffState {
                        selection: &mut sel,
                        last_clicked: &mut last,
                    },
                    cx.settings.diff_font_size,
                );
            }
            Some(Loaded::Failed(e)) => {
                ui.colored_label(theme::pal(ui).removed, e);
            }
            _ => {
                ui.spinner();
            }
        }
    });
}

#[derive(Default)]
struct TreeNode {
    dirs: std::collections::BTreeMap<String, TreeNode>,
    files: Vec<(String, String)>,
}

fn build_tree(paths: &[String]) -> TreeNode {
    let mut root = TreeNode::default();
    for path in paths {
        let mut node = &mut root;
        let mut parts: Vec<&str> = path.split('/').collect();
        let name = parts.pop().unwrap_or_default();
        for part in parts {
            node = node.dirs.entry(part.to_owned()).or_default();
        }
        node.files.push((name.to_owned(), path.clone()));
    }
    root
}

fn show_tree(
    ui: &mut egui::Ui,
    node: &TreeNode,
    selected: Option<&str>,
    clicked: &mut Option<String>,
    save_as: &mut Option<String>,
    history: &mut Option<String>,
    lfs: &std::collections::HashSet<String>,
    depth: usize,
) {
    let p = theme::pal(ui);
    for (name, child) in &node.dirs {
        egui::CollapsingHeader::new(format!("{} {name}", icon::FOLDER))
            .id_salt((depth, name))
            .default_open(depth == 0 && node.dirs.len() + node.files.len() < 12)
            .show(ui, |ui| {
                show_tree(
                    ui,
                    child,
                    selected,
                    clicked,
                    save_as,
                    history,
                    lfs,
                    depth + 1,
                )
            });
    }
    for (name, path) in &node.files {
        let is_sel = selected == Some(path.as_str());
        let text = RichText::new(format!("{} {name}", icon::FILE));
        let text = if is_sel {
            text.color(p.selection_text)
        } else {
            text
        };
        let resp = ui
            .horizontal(|ui| {
                let resp =
                    ui.add(egui::Button::selectable(is_sel, text).frame_when_inactive(false));
                if lfs.contains(path) {
                    theme::lfs_badge(ui, false);
                }
                resp
            })
            .inner;
        if resp.clicked() {
            *clicked = Some(path.clone());
        }
        resp.context_menu(|ui| {
            copy_menu_item(ui, &format!("{} Copy Path", icon::COPY), path.clone());
            if ui
                .button(format!("{} History…", icon::CLOCK_COUNTER_CLOCKWISE))
                .clicked()
            {
                *history = Some(path.clone());
                ui.close();
            }
            if ui
                .button(format!("{} Save As…", icon::FLOPPY_DISK))
                .clicked()
            {
                *save_as = Some(path.clone());
                ui.close();
            }
        });
    }
}

fn file_tree_tab(ui: &mut egui::Ui, tab: &mut RepoTab, cx: &mut Ctx) {
    tab.load_tree();
    let paths = match &tab.tree {
        Some((_, Loaded::Ready(paths))) => paths.clone(),
        Some((_, Loaded::Failed(e))) => {
            ui.colored_label(theme::pal(ui).removed, e);
            return;
        }
        _ => {
            ui.spinner();
            return;
        }
    };
    let tree = build_tree(&paths);
    let mut clicked = None;
    let mut save_as = None;
    let mut history = None;
    let selected = tab.tree_file.as_ref().map(|(p, _)| p.clone());
    egui::Panel::left("file_tree")
        .resizable(true)
        .default_size(280.0)
        .min_size(160.0)
        .show(ui, |ui| {
            egui::ScrollArea::both()
                .id_salt("file_tree_scroll")
                .auto_shrink(false)
                .show(ui, |ui| {
                    show_tree(
                        ui,
                        &tree,
                        selected.as_deref(),
                        &mut clicked,
                        &mut save_as,
                        &mut history,
                        &tab.tree_lfs,
                        0,
                    );
                });
        });
    if let Some(path) = clicked {
        tab.load_tree_file(path);
    }
    if let (Some(path), Some(id)) = (save_as, tab.selected.clone()) {
        tab.save_file_as(id, path);
    }
    if let (Some(path), Some(id)) = (history, tab.selected.clone()) {
        tab.open_file_history(&id, &path);
    }
    egui::CentralPanel::no_frame().show(ui, |ui| match &tab.tree_file {
        Some((path, Loaded::Ready(bytes))) => {
            ui.horizontal(|ui| {
                ui.label(RichText::new(path).strong());
                if tab.tree_lfs.contains(path) || crate::git::lfs::is_pointer(bytes) {
                    theme::lfs_badge(ui, false);
                }
                let size = crate::git::lfs::pointer_size(bytes).unwrap_or(bytes.len() as u64);
                ui.label(RichText::new(crate::format::human_size(size)).color(theme::pal(ui).muted));
            });
            ui.separator();
            if crate::preview::is_image_path(path) {
                if let Some(id) = tab.selected.clone() {
                    let (path, key) = (path.clone(), format!("tree:{id}:{path}"));
                    tab.request_images(&key, None, Some(crate::preview::Source::Commit(id, path)));
                    crate::ui::image_view::show(ui, tab, &key, None, false);
                }
            } else if crate::git::lfs::is_pointer(bytes) {
                let p = theme::pal(ui);
                ui.add_space(8.0);
                ui.label(RichText::new(format!("{} Stored in Git LFS", icon::CLOUD)).strong());
                ui.label(RichText::new("The repository contains a pointer to this file. Right-click it and choose Save As… to get the real content.").color(p.muted));
                ui.add_space(6.0);
                ui.label(RichText::new(String::from_utf8_lossy(bytes).trim()).monospace().small().color(p.muted));
            } else if bytes.len() > crate::git::diff::MAX_DIFF_BYTES {
                ui.label(RichText::new(crate::git::diff::too_large_notice(bytes.len())).color(theme::pal(ui).muted));
            } else if bytes.iter().take(8000).any(|b| *b == 0) {
                ui.label(RichText::new(format!("Binary file ({})", crate::format::human_size(bytes.len() as u64))).color(theme::pal(ui).muted));
            } else {
                let text = String::from_utf8_lossy(bytes).into_owned();
                diff_view::text_view(ui, ("tree_file", path), &text, cx.settings.diff_font_size);
            }
        }
        Some((_, Loaded::Failed(e))) => {
            ui.colored_label(theme::pal(ui).removed, e);
        }
        Some((_, Loaded::Loading)) => {
            ui.spinner();
        }
        None => {
            ui.centered_and_justified(|ui| ui.label(RichText::new("Select a file").color(theme::pal(ui).muted)));
        }
    });
}
