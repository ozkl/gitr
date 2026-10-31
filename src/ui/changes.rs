//! Local changes: staging area, commit box and working-tree diff.

use egui::{Key, RichText, Sense, Vec2};
use egui_phosphor::regular as icon;

use crate::git::{ChangeKind, FileChange};
use crate::repo::{Loaded, RepoTab};
use crate::ui::Ctx;
use crate::ui::dialogs::Dialog;
use crate::ui::diff_view::{self, DiffMode, DiffRequest, DiffState};
use crate::ui::history::copy_menu_item;
use crate::ui::theme;

fn s(v: &str) -> String {
    v.to_owned()
}

pub fn changes_view(ui: &mut egui::Ui, tab: &mut RepoTab, cx: &mut Ctx) {
    if let Some(state) = tab.refs.state {
        let p = theme::pal(ui);
        egui::Frame::new()
            .fill(p.warning_bg)
            .inner_margin(egui::Margin::symmetric(10, 6))
            .corner_radius(4)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!("{} {}", icon::WARNING, state.label())).strong(),
                    );
                    let conflicts = tab
                        .status
                        .unstaged
                        .iter()
                        .filter(|f| f.kind == ChangeKind::Conflicted)
                        .count();
                    if conflicts > 0 {
                        ui.label(format!(
                            "— {conflicts} conflicted file{}",
                            if conflicts == 1 { "" } else { "s" }
                        ));
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let cmd = state.command();
                        if ui.button("Abort").clicked() {
                            *cx.dialog = Some(Dialog::confirm(
                                format!("Abort {cmd}"),
                                format!(
                                    "Abort the {cmd} in progress and restore the previous state?"
                                ),
                                "Abort",
                                format!("Abort {cmd}"),
                                vec![vec![s(cmd), s("--abort")]],
                            ));
                        }
                        if cmd != "merge" && ui.button("Skip").clicked() {
                            tab.git(format!("Skip ({cmd})"), vec![s(cmd), s("--skip")]);
                        }
                        if ui
                            .add_enabled(conflicts == 0, egui::Button::new("Continue"))
                            .clicked()
                        {
                            if cmd == "merge" && !tab.commit_subject.trim().is_empty() {
                                // Use the message shown in the commit box (it may be edited).
                                tab.commit(false);
                            } else if cmd == "merge" {
                                // `--cleanup=strip` drops git's "# Conflicts:" comment lines,
                                // which `--no-edit` would otherwise keep in the message.
                                tab.git(
                                    "Continue merge",
                                    vec![
                                        s("-c"),
                                        s("core.editor=true"),
                                        s("commit"),
                                        s("--no-edit"),
                                        s("--cleanup=strip"),
                                    ],
                                );
                            } else {
                                tab.git(
                                    format!("Continue {cmd}"),
                                    vec![s("-c"), s("core.editor=true"), s(cmd), s("--continue")],
                                );
                            }
                        }
                    });
                });
            });
        ui.add_space(4.0);
    }

    // Keyboard: arrows move (Shift extends), ⌘/Ctrl+A selects the whole list.
    if cx.dialog.is_none() && !ui.ctx().egui_wants_keyboard_input() {
        let (up, down, shift, all) = ui.input(|i| {
            (
                i.key_pressed(Key::ArrowUp),
                i.key_pressed(Key::ArrowDown),
                i.modifiers.shift,
                i.modifiers.command && i.key_pressed(Key::A),
            )
        });
        if up || down {
            tab.move_change_selection(if up { -1 } else { 1 }, shift);
        }
        if all {
            if let Some((staged, _)) = tab.selected_change.clone() {
                tab.select_all_changes(staged);
            }
        }
    }

    egui::Panel::left("changes_left")
        .resizable(true)
        .default_size(380.0)
        .min_size(240.0)
        .frame(egui::Frame::new().inner_margin(egui::Margin {
            left: 0,
            right: 6,
            top: 0,
            bottom: 0,
        }))
        .show(ui, |ui| {
            egui::Panel::bottom("commit_box")
                .frame(egui::Frame::new())
                .resizable(false)
                .show(ui, |ui| commit_box(ui, tab, cx));
            egui::Panel::top("changes_filter")
                .frame(egui::Frame::new())
                .resizable(false)
                .show(ui, |ui| filter_bar(ui, tab));
            // Start every run with an even split. For the first few frames the panel is pinned
            // to exactly half (the layout around it, e.g. the commit box, is only measured
            // after it has been drawn once); that also replaces the divider position egui
            // saved last time. After that the user's dragging is kept for the session.
            static SETTLE_FRAMES: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);
            let settling = SETTLE_FRAMES.load(std::sync::atomic::Ordering::Relaxed) < 4;
            let half = (ui.available_height() / 2.0).max(80.0);
            let mut panel = egui::Panel::top("unstaged_panel")
                .frame(egui::Frame::new())
                .resizable(true);
            if settling {
                SETTLE_FRAMES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                ui.ctx().request_repaint();
                panel = panel.exact_size(half);
            } else {
                panel = panel.default_size(half).min_size(80.0);
            }
            panel.show(ui, |ui| file_list(ui, tab, cx, false));
            egui::CentralPanel::no_frame().show(ui, |ui| file_list(ui, tab, cx, true));
        });

    egui::CentralPanel::no_frame().show(ui, |ui| diff_panel(ui, tab, cx));
}

/// Always-visible filter field that narrows both file lists.
fn filter_bar(ui: &mut egui::Ui, tab: &mut RepoTab) {
    ui.add_space(4.0);
    let before = tab.changes_filter.clone();
    ui.horizontal(|ui| {
        let clear_w = if tab.changes_filter.is_empty() {
            0.0
        } else {
            28.0
        };
        let edit = egui::TextEdit::singleline(&mut tab.changes_filter)
            .hint_text(format!("{} Filter", icon::MAGNIFYING_GLASS))
            .desired_width(ui.available_width() - clear_w);
        let r = ui.add(edit).on_hover_text("Filter files (⌘/Ctrl+F)");
        if std::mem::take(&mut tab.focus_changes_filter) {
            r.request_focus();
        }
        // Esc clears the filter (egui already drops focus on Esc).
        if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Escape)) {
            tab.changes_filter.clear();
        }
        if !tab.changes_filter.is_empty()
            && ui
                .small_button(icon::X)
                .on_hover_text("Clear filter")
                .clicked()
        {
            tab.changes_filter.clear();
        }
    });
    if tab.changes_filter != before {
        tab.apply_changes_filter();
    }
    ui.add_space(2.0);
}

fn file_list(ui: &mut egui::Ui, tab: &mut RepoTab, cx: &mut Ctx, staged: bool) {
    let p = theme::pal(ui);
    let total = if staged {
        tab.status.staged.len()
    } else {
        tab.status.unstaged.len()
    };
    let list = list_cache(tab, staged, cx.settings.changes_tree_view);
    let files = &list.files;
    let filtered = files.len() != total;
    // Selected files of this list that can be (un)staged; conflicts are resolved explicitly.
    let in_this_list = tab
        .selected_change
        .as_ref()
        .is_some_and(|(s, _)| *s == staged);
    let target_count = if in_this_list {
        tab.selected_changes
            .iter()
            .filter(|p| staged || !tab.status.conflicts.contains_key(*p))
            .count()
    } else {
        0
    };
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        let title = if staged {
            "Staged Changes"
        } else {
            "Unstaged Changes"
        };
        let count = if filtered {
            format!("{} of {total}", files.len())
        } else {
            total.to_string()
        };
        ui.label(RichText::new(format!("{title} ({count})")).strong());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.menu_button(icon::LIST, |ui| {
                let tree = &mut cx.settings.changes_tree_view;
                if ui
                    .add(egui::Button::selectable(!*tree, "View as List"))
                    .clicked()
                {
                    *tree = false;
                    ui.close();
                }
                if ui
                    .add(egui::Button::selectable(*tree, "View as Tree"))
                    .clicked()
                {
                    *tree = true;
                    ui.close();
                }
                ui.separator();
                let paths = || files.iter().map(|f| f.path.clone()).collect::<Vec<_>>();
                if staged {
                    let label = if filtered {
                        format!("{} Unstage {} Shown", icon::MINUS, files.len())
                    } else {
                        format!("{} Unstage All", icon::MINUS)
                    };
                    if ui
                        .add_enabled(!files.is_empty(), egui::Button::new(label))
                        .clicked()
                    {
                        tab.unstage(paths());
                        ui.close();
                    }
                } else {
                    let label = if filtered {
                        format!("{} Stage {} Shown", icon::PLUS, files.len())
                    } else {
                        format!("{} Stage All", icon::PLUS)
                    };
                    if ui
                        .add_enabled(!files.is_empty(), egui::Button::new(label))
                        .clicked()
                    {
                        tab.stage(paths());
                        ui.close();
                    }
                    let label = if filtered {
                        format!("{} Discard {} Shown…", icon::TRASH, files.len())
                    } else {
                        format!("{} Discard All…", icon::TRASH)
                    };
                    if ui
                        .add_enabled(!files.is_empty(), egui::Button::new(label))
                        .clicked()
                    {
                        *cx.dialog = Some(Dialog::Discard {
                            files: files.clone(),
                        });
                        ui.close();
                    }
                }
            });
            // Stage / unstage the selected files.
            let verb = if staged { "Unstage" } else { "Stage" };
            let label = if target_count > 1 {
                format!("{verb} ({target_count})")
            } else {
                verb.to_owned()
            };
            let hint = match target_count {
                0 => {
                    "Select files first (⌘/Ctrl-click or Shift-click to select several)".to_owned()
                }
                1 => format!("{verb} the selected file"),
                n => format!("{verb} {n} selected files"),
            };
            if ui
                .add_enabled(target_count > 0, egui::Button::new(label))
                .on_hover_text(hint)
                .clicked()
            {
                let targets: Vec<String> = tab
                    .selected_files(staged)
                    .into_iter()
                    .filter(|f| staged || f.kind != ChangeKind::Conflicted)
                    .map(|f| f.path)
                    .collect();
                if staged {
                    tab.unstage(targets);
                } else {
                    tab.stage(targets);
                }
            }
        });
    });
    ui.add_space(2.0);
    if files.is_empty() {
        ui.add_space(8.0);
        ui.vertical_centered(|ui| {
            let msg = if filtered {
                "No matching files"
            } else if staged {
                "No staged changes"
            } else {
                "No unstaged changes"
            };
            ui.label(RichText::new(msg).color(p.muted));
        });
        return;
    }

    // Only the visible rows are laid out, so lists with many thousands of files stay fast.
    let mut rows = Rows::default();
    ui.spacing_mut().item_spacing.y = 0.0;
    egui::ScrollArea::vertical()
        .id_salt(("files", staged))
        .auto_shrink(false)
        .show_rows(ui, ROW_H, list.rows.len(), |ui, range| {
            for row in &list.rows[range] {
                match row {
                    ListRow::Dir {
                        label,
                        path,
                        depth,
                        open,
                        files: under,
                    } => dir_row(
                        ui,
                        tab,
                        cx,
                        &list,
                        staged,
                        (label, path, *depth, *open),
                        under.clone(),
                        &mut rows,
                    ),
                    ListRow::File {
                        index,
                        indent,
                        name_only,
                    } => file_row(
                        ui,
                        tab,
                        cx,
                        &files[*index],
                        staged,
                        *name_only,
                        *indent,
                        &mut rows,
                    ),
                }
            }
            rows.paint_selection(ui, p.selection);
        });
}

const ROW_H: f32 = 24.0;

/// A file list laid out as rows; cached on the tab while its inputs stay the same.
pub struct ListCache {
    key: ListKey,
    /// The files that pass the filter.
    files: Vec<FileChange>,
    /// Indices into `files`, sorted by path: the files inside a folder form one run.
    by_path: Vec<usize>,
    rows: Vec<ListRow>,
}

#[derive(PartialEq)]
struct ListKey {
    status: u64,
    filter: String,
    tree: bool,
    /// Hash of the collapsed folders of this list.
    collapsed: u64,
}

enum ListRow {
    Dir {
        label: String,
        /// Folder path with a trailing slash.
        path: String,
        depth: usize,
        open: bool,
        /// Range of `by_path` with the files inside the folder.
        files: std::ops::Range<usize>,
    },
    File {
        index: usize,
        indent: f32,
        name_only: bool,
    },
}

impl ListCache {
    fn paths_under(&self, range: std::ops::Range<usize>) -> Vec<String> {
        self.by_path[range]
            .iter()
            .map(|&i| self.files[i].path.clone())
            .collect()
    }
}

fn list_cache(tab: &mut RepoTab, staged: bool, tree: bool) -> std::sync::Arc<ListCache> {
    use std::hash::{Hash, Hasher};
    let mut collapsed: Vec<&String> = tab
        .collapsed_dirs
        .iter()
        .filter(|(s, _)| *s == staged)
        .map(|(_, p)| p)
        .collect();
    collapsed.sort();
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    collapsed.hash(&mut hasher);
    let key = ListKey {
        status: tab.status_version,
        filter: tab.changes_filter.clone(),
        tree,
        collapsed: hasher.finish(),
    };
    let i = usize::from(staged);
    if let Some(cache) = &tab.change_lists[i]
        && cache.key == key
    {
        return cache.clone();
    }
    let files = tab.visible_changes(staged);
    let mut by_path: Vec<usize> = (0..files.len()).collect();
    by_path.sort_by(|&a, &b| files[a].path.cmp(&files[b].path));
    let mut rows = Vec::new();
    if tree {
        let root = build_tree(&files);
        flatten(tab, &files, &by_path, &root, staged, "", 0, &mut rows);
    } else {
        rows.extend((0..files.len()).map(|index| ListRow::File {
            index,
            indent: 0.0,
            name_only: false,
        }));
    }
    tab.change_order[i] = rows
        .iter()
        .filter_map(|r| match r {
            ListRow::File { index, .. } => Some(files[*index].path.clone()),
            ListRow::Dir { .. } => None,
        })
        .collect();
    let cache = std::sync::Arc::new(ListCache {
        key,
        files,
        by_path,
        rows,
    });
    tab.change_lists[i] = Some(cache.clone());
    cache
}

/// Collects rows while a file list is drawn.
#[derive(Default)]
struct Rows {
    /// Reserved background shapes of selected rows, filled in afterwards so that
    /// adjacent selected rows join into one block.
    selected: Vec<(egui::layers::ShapeIdx, egui::Rect)>,
}

impl Rows {
    fn paint_selection(&self, ui: &egui::Ui, color: egui::Color32) {
        const R: u8 = 4;
        let touches = |a: &egui::Rect, b: &egui::Rect| (a.bottom() - b.top()).abs() < 0.5;
        for (i, (idx, rect)) in self.selected.iter().enumerate() {
            let join_above = i > 0 && touches(&self.selected[i - 1].1, rect);
            let join_below = self
                .selected
                .get(i + 1)
                .is_some_and(|(_, next)| touches(rect, next));
            let top = if join_above { 0 } else { R };
            let bottom = if join_below { 0 } else { R };
            let radius = egui::CornerRadius {
                nw: top,
                ne: top,
                sw: bottom,
                se: bottom,
            };
            ui.painter()
                .set(*idx, egui::epaint::RectShape::filled(*rect, radius, color));
        }
    }
}

const INDENT: f32 = 16.0;

#[derive(Default)]
struct Dir {
    dirs: std::collections::BTreeMap<String, Dir>,
    /// Indices into the file list.
    files: Vec<usize>,
}

fn build_tree(files: &[FileChange]) -> Dir {
    let mut root = Dir::default();
    for (i, f) in files.iter().enumerate() {
        let mut node = &mut root;
        let (dir, _) = theme::split_path(&f.path);
        for part in dir.split('/').filter(|p| !p.is_empty()) {
            node = node.dirs.entry(part.to_owned()).or_default();
        }
        node.files.push(i);
    }
    root
}

/// Appends the rows of `node` (folders first, then files) in display order.
#[allow(clippy::too_many_arguments)]
fn flatten(
    tab: &RepoTab,
    files: &[FileChange],
    by_path: &[usize],
    node: &Dir,
    staged: bool,
    prefix: &str,
    depth: usize,
    rows: &mut Vec<ListRow>,
) {
    for (name, child) in &node.dirs {
        // Collapse chains of single folders into one row ("Assets/_BallSlot/NodeGraph").
        let mut label = name.clone();
        let mut child = child;
        while child.files.is_empty() && child.dirs.len() == 1 {
            let (n, c) = child.dirs.iter().next().unwrap();
            label = format!("{label}/{n}");
            child = c;
        }
        let path = format!("{prefix}{label}/");
        let open = !tab.collapsed_dirs.contains(&(staged, path.clone()));
        let start = by_path.partition_point(|&i| files[i].path.as_str() < path.as_str());
        let len = by_path[start..].partition_point(|&i| files[i].path.starts_with(&path));
        rows.push(ListRow::Dir {
            label,
            path: path.clone(),
            depth,
            open,
            files: start..start + len,
        });
        if open {
            flatten(tab, files, by_path, child, staged, &path, depth + 1, rows);
        }
    }
    for &index in &node.files {
        let indent = depth as f32 * INDENT
            + if depth > 0 || !node.dirs.is_empty() {
                18.0
            } else {
                0.0
            };
        rows.push(ListRow::File {
            index,
            indent,
            name_only: true,
        });
    }
}

#[allow(clippy::too_many_arguments)]
fn dir_row(
    ui: &mut egui::Ui,
    tab: &mut RepoTab,
    cx: &mut Ctx,
    list: &ListCache,
    staged: bool,
    (label, path, depth, open): (&str, &str, usize, bool),
    under: std::ops::Range<usize>,
    rows: &mut Rows,
) {
    let p = theme::pal(ui);
    let key = (staged, path.to_owned());
    // The selection is sorted, so the selected paths inside this folder are one run too.
    let n = under.len();
    let all_selected = n > 0
        && tab
            .selected_change
            .as_ref()
            .is_some_and(|(s, _)| *s == staged)
        && tab
            .selected_changes
            .range::<str, _>((std::ops::Bound::Included(path), std::ops::Bound::Unbounded))
            .take_while(|s| s.starts_with(path))
            .take(n)
            .count()
            == n;
    let (rect, resp) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW_H), Sense::click());
    if all_selected {
        rows.selected
            .push((ui.painter().add(egui::Shape::Noop), rect));
    } else if resp.hovered() || resp.context_menu_opened() {
        ui.painter().rect_filled(rect, 3.0, p.hover);
    }
    let x = rect.left() + 4.0 + depth as f32 * INDENT;
    let y = rect.center().y;
    let caret = if open {
        icon::CARET_DOWN
    } else {
        icon::CARET_RIGHT
    };
    let painter = ui.painter().with_clip_rect(rect);
    let (icon_color, text_color) = if all_selected {
        (p.selection_text, p.selection_text)
    } else {
        (p.muted, ui.visuals().text_color())
    };
    painter.text(
        egui::pos2(x + 6.0, y),
        egui::Align2::CENTER_CENTER,
        caret,
        egui::FontId::proportional(12.0),
        icon_color,
    );
    painter.text(
        egui::pos2(x + 22.0, y),
        egui::Align2::CENTER_CENTER,
        if open {
            icon::FOLDER_OPEN
        } else {
            icon::FOLDER
        },
        egui::FontId::proportional(15.0),
        icon_color,
    );
    painter.text(
        egui::pos2(x + 34.0, y),
        egui::Align2::LEFT_CENTER,
        label,
        egui::FontId::proportional(13.5),
        text_color,
    );
    // The caret toggles the folder; the rest of the row selects the files inside it.
    let on_caret = resp
        .interact_pointer_pos()
        .is_some_and(|pos| pos.x < x + 14.0);
    if (resp.clicked() && on_caret) || resp.double_clicked() {
        if open {
            tab.collapsed_dirs.insert(key);
        } else {
            tab.collapsed_dirs.remove(&key);
        }
    } else if resp.clicked() || (resp.secondary_clicked() && !all_selected) {
        let add = resp.clicked() && ui.input(|i| i.modifiers.command || i.modifiers.shift);
        tab.select_changes(staged, list.paths_under(under.clone()), add);
    }
    resp.context_menu(|ui| {
        if staged {
            if ui
                .button(format!("{} Unstage Folder", icon::MINUS))
                .clicked()
            {
                tab.unstage(list.paths_under(under.clone()));
                ui.close();
            }
        } else {
            if ui.button(format!("{} Stage Folder", icon::PLUS)).clicked() {
                tab.stage(list.paths_under(under.clone()));
                ui.close();
            }
            if ui
                .button(format!("{} Discard Folder…", icon::TRASH))
                .clicked()
            {
                let files = list.by_path[under.clone()]
                    .iter()
                    .map(|&i| list.files[i].clone())
                    .collect();
                *cx.dialog = Some(Dialog::Discard { files });
                ui.close();
            }
        }
        ui.separator();
        if ui
            .button(format!("{} Show in file manager", icon::FOLDER_OPEN))
            .clicked()
        {
            tab.open_in_file_manager(Some(path.trim_end_matches('/')));
            ui.close();
        }
        copy_menu_item(
            ui,
            &format!("{} Copy path", icon::COPY),
            path.trim_end_matches('/').to_owned(),
        );
    });
}

#[allow(clippy::too_many_arguments)]
fn file_row(
    ui: &mut egui::Ui,
    tab: &mut RepoTab,
    cx: &mut Ctx,
    file: &FileChange,
    staged: bool,
    name_only: bool,
    indent: f32,
    rows: &mut Rows,
) {
    let p = theme::pal(ui);
    let selected = tab.is_change_selected(staged, &file.path);
    let (rect, resp) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW_H), Sense::click());
    if selected {
        rows.selected
            .push((ui.painter().add(egui::Shape::Noop), rect));
    } else if resp.hovered() || resp.context_menu_opened() {
        ui.painter().rect_filled(rect, 3.0, p.hover);
    }
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect.shrink2(Vec2::new(4.0, 0.0)))
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    child.add_space(indent);
    theme::change_badge(&mut child, file.kind);
    if tab.status.lfs.contains(&file.path) {
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
    job.append(
        name,
        0.0,
        egui::TextFormat {
            color: text_color,
            ..Default::default()
        },
    );
    if !dir.is_empty() && !name_only {
        job.append(
            dir.trim_end_matches('/'),
            8.0,
            egui::TextFormat {
                color: muted,
                ..Default::default()
            },
        );
    }
    child.add(egui::Label::new(job).truncate().selectable(false));
    let resp = if name_only {
        resp.on_hover_text(&file.path)
    } else {
        resp
    };

    if resp.clicked() {
        let modifiers = ui.input(|i| i.modifiers);
        tab.click_change(staged, file.path.clone(), modifiers);
    }
    if resp.double_clicked() {
        if staged {
            tab.unstage(vec![file.path.clone()]);
        } else if file.kind != ChangeKind::Conflicted {
            tab.stage(vec![file.path.clone()]);
        }
    }
    if resp.secondary_clicked() && !selected {
        // Right-clicking outside the selection selects that file, like in Finder / Explorer.
        tab.select_change(staged, file.path.clone());
    }
    if selected && tab.selected_changes.len() > 1 {
        resp.context_menu(|ui| {
            let group = tab.selected_files(staged);
            multi_menu(ui, tab, cx, &group, staged)
        });
    } else {
        resp.context_menu(|ui| file_menu(ui, tab, cx, file, staged));
    }
}

/// Context menu / actions for several selected files.
fn multi_menu(
    ui: &mut egui::Ui,
    tab: &mut RepoTab,
    cx: &mut Ctx,
    files: &[FileChange],
    staged: bool,
) {
    let n = files.len();
    let paths: Vec<String> = files.iter().map(|f| f.path.clone()).collect();
    if staged {
        if ui
            .button(format!("{} Unstage {n} Files", icon::MINUS))
            .clicked()
        {
            tab.unstage(paths.clone());
            ui.close();
        }
    } else {
        let stageable: Vec<String> = files
            .iter()
            .filter(|f| f.kind != ChangeKind::Conflicted)
            .map(|f| f.path.clone())
            .collect();
        if ui
            .add_enabled(
                !stageable.is_empty(),
                egui::Button::new(format!("{} Stage {} Files", icon::PLUS, stageable.len())),
            )
            .clicked()
        {
            tab.stage(stageable);
            ui.close();
        }
        if ui
            .button(format!("{} Discard {n} Files…", icon::TRASH))
            .clicked()
        {
            *cx.dialog = Some(Dialog::Discard {
                files: files.to_vec(),
            });
            ui.close();
        }
    }
    ui.separator();
    copy_menu_item(ui, &format!("{} Copy paths", icon::COPY), paths.join("\n"));
}

fn file_menu(ui: &mut egui::Ui, tab: &mut RepoTab, cx: &mut Ctx, file: &FileChange, staged: bool) {
    if staged {
        if ui.button(format!("{} Unstage", icon::MINUS)).clicked() {
            tab.unstage(vec![file.path.clone()]);
            ui.close();
        }
    } else {
        if file.kind == ChangeKind::Conflicted {
            conflict_actions(ui, tab, file);
            ui.separator();
        }
        if ui.button(format!("{} Stage", icon::PLUS)).clicked() {
            tab.stage(vec![file.path.clone()]);
            ui.close();
        }
        if ui
            .button(format!("{} Discard Changes…", icon::TRASH))
            .clicked()
        {
            *cx.dialog = Some(Dialog::Discard {
                files: vec![file.clone()],
            });
            ui.close();
        }
        if file.kind == ChangeKind::Untracked {
            if ui.button("Add to .gitignore").clicked() {
                let path = file.path.clone();
                tab.run_task(
                    "Ignore file",
                    crate::repo::OpKind::Generic,
                    Box::new(move |repo| {
                        use std::io::Write;
                        let gitignore = repo.join(".gitignore");
                        let needs_newline = std::fs::read(&gitignore)
                            .map(|b| !b.is_empty() && !b.ends_with(b"\n"))
                            .unwrap_or(false);
                        let mut f = std::fs::OpenOptions::new()
                            .create(true)
                            .append(true)
                            .open(&gitignore)
                            .map_err(|e| e.to_string())?;
                        let line = format!("{}/{path}\n", if needs_newline { "\n" } else { "" });
                        f.write_all(line.as_bytes()).map_err(|e| e.to_string())?;
                        Ok(String::new())
                    }),
                );
                ui.close();
            }
        }
    }
    ui.separator();
    if file.kind != ChangeKind::Untracked && tab.refs.head_id.is_some() {
        let path = file
            .old_path
            .clone()
            .filter(|_| file.kind == ChangeKind::Renamed)
            .unwrap_or_else(|| file.path.clone());
        if ui
            .button(format!("{} History…", icon::CLOCK_COUNTER_CLOCKWISE))
            .clicked()
        {
            tab.open_file_history("HEAD", &path);
            ui.close();
        }
    }
    if ui.button("Open file").clicked() {
        crate::platform::open_file(&tab.path.join(&file.path));
        ui.close();
    }
    if ui
        .button(format!("{} Show in file manager", icon::FOLDER_OPEN))
        .clicked()
    {
        tab.open_in_file_manager(Some(&file.path));
        ui.close();
    }
    copy_menu_item(ui, &format!("{} Copy path", icon::COPY), file.path.clone());
    copy_menu_item(
        ui,
        "Copy full path",
        tab.path.join(&file.path).display().to_string(),
    );
}

fn conflict_actions(ui: &mut egui::Ui, tab: &mut RepoTab, file: &FileChange) {
    let path = file.path.clone();
    if ui.button("Use local version (ours)").clicked() {
        tab.take_conflict_side(&path, true);
        ui.close();
    }
    if ui.button("Use remote version (theirs)").clicked() {
        tab.take_conflict_side(&path, false);
        ui.close();
    }
    if tab
        .status
        .conflicts
        .get(&path)
        .is_some_and(|k| k.has_markers())
        && ui.button("Merge in Gitr…").clicked()
    {
        tab.select_change(false, path.clone());
        if let Err(e) = tab.open_merge_editor(&path) {
            tab.notices.push(crate::repo::Notice {
                title: "Cannot open merge editor".into(),
                text: e,
                error: true,
            });
        }
        ui.close();
    }
    if ui.button("Open in external merge tool").clicked() {
        tab.git(
            format!("Merge tool {path}"),
            vec![s("mergetool"), s("--no-prompt"), s("--"), path.clone()],
        );
        ui.close();
    }
    ui.separator();
    if ui
        .button("Mark as resolved")
        .on_hover_text("Stage the file as it is now in the working tree")
        .clicked()
    {
        tab.stage(vec![path.clone()]);
        ui.close();
    }
}

fn commit_box(ui: &mut egui::Ui, tab: &mut RepoTab, cx: &mut Ctx) {
    let p = theme::pal(ui);
    ui.add_space(6.0);
    let w = ui.available_width();
    let subject = ui.add(
        egui::TextEdit::singleline(&mut tab.commit_subject)
            .hint_text("Commit subject")
            .desired_width(w)
            .font(egui::TextStyle::Body),
    );
    if cx.focus_commit {
        subject.request_focus();
    }
    let len = tab.commit_subject.chars().count();
    if len > 72 {
        ui.label(
            RichText::new(format!("Subject is {len} characters (72 recommended)"))
                .small()
                .color(p.modified),
        );
    }
    ui.add(
        egui::TextEdit::multiline(&mut tab.commit_body)
            .hint_text("Description")
            .desired_width(w)
            .desired_rows(4),
    );
    // Who the commit will be attributed to; click to change it for this repository.
    let (text, color) = match &tab.refs.identity {
        Some(id) => (format!("{} {id}", icon::USER), p.muted),
        None => (
            format!(
                "{} No author identity set — click to configure",
                icon::WARNING
            ),
            p.modified,
        ),
    };
    let r = ui
        .add(
            egui::Label::new(RichText::new(text).small().color(color))
                .truncate()
                .sense(Sense::click()),
        )
        .on_hover_text("Change the author for this repository");
    if r.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    if r.clicked() {
        *cx.dialog = Some(Dialog::repo_settings(tab));
    }
    ui.add_space(4.0);
    let unresolved = tab.status.conflicts.len();
    let can_commit = (!tab.status.staged.is_empty()
        || tab.amend
        || tab.refs.state == Some(crate::git::RepoState::Merging))
        && !tab.commit_subject.trim().is_empty()
        && unresolved == 0;
    if unresolved > 0 {
        ui.label(
            RichText::new(format!(
                "{} Resolve {unresolved} conflicted file{} before committing",
                icon::WARNING,
                if unresolved == 1 { "" } else { "s" }
            ))
            .small()
            .color(p.modified),
        );
    }
    let mut commit = false;
    let mut and_push = false;
    ui.horizontal(|ui| {
        let before = tab.amend;
        ui.checkbox(&mut tab.amend, "Amend");
        if tab.amend && !before && tab.commit_subject.trim().is_empty() {
            if let Some(msg) = tab.head_message() {
                let (subject, body) = msg.split_once('\n').unwrap_or((&msg, ""));
                tab.commit_subject = subject.to_owned();
                tab.commit_body = body.trim().to_owned();
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_enabled_ui(can_commit, |ui| {
                ui.menu_button(icon::CARET_DOWN, |ui| {
                    if ui.button("Commit and Push").clicked() {
                        commit = true;
                        and_push = true;
                        ui.close();
                    }
                });
            });
            let label = if tab.amend {
                "Amend Last Commit"
            } else {
                "Commit"
            };
            let n = tab.status.staged.len();
            let text = if tab.amend || n == 0 {
                label.to_owned()
            } else {
                format!("{label} {n} file{}", if n == 1 { "" } else { "s" })
            };
            let button = if can_commit {
                egui::Button::new(RichText::new(text).color(egui::Color32::WHITE)).fill(p.selection)
            } else {
                egui::Button::new(text)
            };
            if ui
                .add_enabled(can_commit, button)
                .on_hover_text("⌘/Ctrl + Enter")
                .clicked()
            {
                commit = true;
            }
        });
    });
    if can_commit && ui.input(|i| i.modifiers.command && i.key_pressed(Key::Enter)) {
        commit = true;
    }
    if commit {
        tab.commit(and_push);
    }
    ui.add_space(4.0);
}

fn diff_panel(ui: &mut egui::Ui, tab: &mut RepoTab, cx: &mut Ctx) {
    let p = theme::pal(ui);
    let Some((staged, path)) = tab.selected_change.clone() else {
        ui.centered_and_justified(|ui| {
            let msg = if tab.status.is_clean() {
                "Working tree clean"
            } else {
                "Select a file to view changes"
            };
            ui.label(RichText::new(msg).color(p.muted));
        });
        return;
    };
    let Some(file) = tab.change_file(staged, &path).cloned() else {
        return;
    };
    let group = tab.selected_files(staged);
    if group.len() > 1 {
        selection_summary(ui, tab, cx, &group, staged);
        return;
    }

    ui.horizontal(|ui| {
        theme::change_badge(ui, file.kind);
        ui.label(RichText::new(&file.path).strong());
        if tab.status.lfs.contains(&file.path) {
            theme::lfs_badge(ui, false);
        }
        let conflicted = file.kind == ChangeKind::Conflicted;
        // Images and conflicts have their own views; the text-diff controls don't apply.
        let is_image = crate::preview::is_image_path(&file.path) || conflicted;
        if let (Some((_, Loaded::Ready(d))), false) = (&tab.change_diff, is_image) {
            ui.label(RichText::new(format!("+{}", d.added)).color(p.added));
            ui.label(RichText::new(format!("-{}", d.removed)).color(p.removed));
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let mut reload = false;
            if !is_image {
                reload |= ui
                    .toggle_value(&mut tab.ignore_whitespace, "Ignore whitespace")
                    .changed();
                if ui
                    .small_button(icon::PLUS)
                    .on_hover_text("More context")
                    .clicked()
                {
                    tab.diff_context = (tab.diff_context + 2).min(100);
                    reload = true;
                }
                ui.label(format!("{} lines", tab.diff_context));
                if ui
                    .small_button(icon::MINUS)
                    .on_hover_text("Less context")
                    .clicked()
                {
                    tab.diff_context = tab.diff_context.saturating_sub(2);
                    reload = true;
                }
                ui.separator();
            }
            if staged {
                if ui.button(format!("{} Unstage File", icon::MINUS)).clicked() {
                    tab.unstage(vec![file.path.clone()]);
                }
            } else {
                if !conflicted && ui.button(format!("{} Discard", icon::TRASH)).clicked() {
                    *cx.dialog = Some(Dialog::Discard {
                        files: vec![file.clone()],
                    });
                }
                if conflicted {
                    ui.menu_button("Resolve", |ui| conflict_actions(ui, tab, &file));
                } else if ui.button(format!("{} Stage File", icon::PLUS)).clicked() {
                    tab.stage(vec![file.path.clone()]);
                }
            }
            if reload {
                tab.line_selection.clear();
                tab.reload_change_diff();
            }
        });
    });
    ui.separator();

    if file.kind == ChangeKind::Conflicted {
        crate::ui::conflict::panel(ui, tab, cx, &file);
        return;
    }

    if crate::preview::is_image_path(&file.path) && file.kind != ChangeKind::Conflicted {
        use crate::preview::Source;
        let path = file.path.clone();
        let (old, new) = if staged {
            let old = (file.kind != ChangeKind::Added).then(|| {
                Source::Commit(
                    "HEAD".into(),
                    file.old_path.clone().unwrap_or_else(|| path.clone()),
                )
            });
            (
                old,
                (file.kind != ChangeKind::Deleted).then(|| Source::Index(path.clone())),
            )
        } else {
            let old = (file.kind != ChangeKind::Untracked).then(|| Source::Index(path.clone()));
            (
                old,
                (file.kind != ChangeKind::Deleted).then(|| Source::Working(path.clone())),
            )
        };
        let key = format!("work:{staged}:{path}");
        tab.request_images(&key, old, new);
        crate::ui::image_view::show(ui, tab, &key, None, true);
        return;
    }

    let diff = match &tab.change_diff {
        Some((_, Loaded::Ready(d))) => d.clone(),
        Some((_, Loaded::Failed(e))) => {
            ui.colored_label(p.removed, e);
            return;
        }
        _ => {
            ui.centered_and_justified(|ui| ui.spinner());
            return;
        }
    };
    // Partial staging on a whitespace-insensitive diff would produce patches that do not apply.
    let mode = if tab.ignore_whitespace || file.kind == ChangeKind::Conflicted {
        DiffMode::ReadOnly
    } else if staged {
        DiffMode::Staged
    } else {
        DiffMode::Unstaged
    };
    let request = diff_view::diff_view(
        ui,
        ("work_diff", staged, &path),
        &diff,
        mode,
        DiffState {
            selection: &mut tab.line_selection,
            last_clicked: &mut tab.last_clicked_line,
        },
        cx.settings.diff_font_size,
    );
    match request {
        Some(DiffRequest::Patch(sel, action)) => tab.apply_partial(sel, action),
        Some(DiffRequest::ConfirmDiscard(sel)) => *cx.dialog = Some(Dialog::DiscardPatch(sel)),
        None => {}
    }
}

/// Shown instead of a diff when several files are selected.
fn selection_summary(
    ui: &mut egui::Ui,
    tab: &mut RepoTab,
    cx: &mut Ctx,
    files: &[FileChange],
    staged: bool,
) {
    let p = theme::pal(ui);
    ui.vertical_centered(|ui| {
        ui.add_space((ui.available_height() * 0.25).max(20.0));
        ui.label(RichText::new(icon::FILES).size(40.0).color(p.muted));
        ui.label(
            RichText::new(format!("{} files selected", files.len()))
                .size(18.0)
                .strong(),
        );
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            let w = if staged { 120.0 } else { 240.0 };
            ui.add_space(((ui.available_width() - w) / 2.0).max(0.0));
            if staged {
                if ui.button(format!("{} Unstage", icon::MINUS)).clicked() {
                    tab.unstage(files.iter().map(|f| f.path.clone()).collect());
                }
            } else {
                let stageable: Vec<String> = files
                    .iter()
                    .filter(|f| f.kind != ChangeKind::Conflicted)
                    .map(|f| f.path.clone())
                    .collect();
                if ui
                    .add_enabled(
                        !stageable.is_empty(),
                        egui::Button::new(format!("{} Stage", icon::PLUS)),
                    )
                    .clicked()
                {
                    tab.stage(stageable);
                }
                if ui.button(format!("{} Discard…", icon::TRASH)).clicked() {
                    *cx.dialog = Some(Dialog::Discard {
                        files: files.to_vec(),
                    });
                }
            }
        });
        ui.add_space(12.0);
        egui::ScrollArea::vertical()
            .max_height(ui.available_height() - 20.0)
            .show(ui, |ui| {
                for f in files.iter().take(500) {
                    ui.horizontal(|ui| {
                        ui.add_space(((ui.available_width() - 360.0) / 2.0).max(0.0));
                        theme::change_badge(ui, f.kind);
                        ui.label(RichText::new(&f.path).color(p.muted));
                    });
                }
            });
        ui.add_space(6.0);
        ui.label(
            RichText::new("⌘/Ctrl-click to toggle, Shift-click to select a range")
                .small()
                .color(p.muted),
        );
    });
}
