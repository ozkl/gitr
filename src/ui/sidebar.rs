//! Repository sidebar: views, branches, remotes, tags, stashes, submodules.

use std::collections::BTreeMap;

use egui::{Align2, FontId, Pos2, RichText, Sense, Vec2};
use egui_phosphor::regular as icon;

use crate::git::Branch;
use crate::repo::{RepoTab, View};
use crate::ui::Ctx;
use crate::ui::dialogs::Dialog;
use crate::ui::history::copy_menu_item;
use crate::ui::theme;

fn s(v: &str) -> String {
    v.to_owned()
}

struct Row<'a> {
    icon: &'a str,
    text: &'a str,
    selected: bool,
    bold: bool,
    right: Option<String>,
    indent: f32,
}

fn row(ui: &mut egui::Ui, r: Row<'_>) -> egui::Response {
    let p = theme::pal(ui);
    let (rect, resp) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 22.0), Sense::click());
    if r.selected {
        ui.painter().rect_filled(rect, 4.0, p.selection);
    } else if resp.hovered() || resp.context_menu_opened() {
        ui.painter().rect_filled(rect, 4.0, p.hover);
    }
    let color = if r.selected {
        p.selection_text
    } else {
        ui.visuals().text_color()
    };
    let x = rect.left() + 6.0 + r.indent;
    ui.painter().text(
        Pos2::new(x, rect.center().y),
        Align2::LEFT_CENTER,
        r.icon,
        FontId::proportional(14.0),
        if r.selected { color } else { p.muted },
    );
    let font = FontId::proportional(13.5);
    let mut right_w = 0.0;
    if let Some(right) = &r.right {
        let g = ui.painter().layout_no_wrap(
            right.clone(),
            FontId::proportional(11.5),
            if r.selected { color } else { p.muted },
        );
        right_w = g.size().x + 8.0;
        ui.painter().galley(
            Pos2::new(rect.right() - right_w, rect.center().y - g.size().y / 2.0),
            g,
            color,
        );
    }
    let g = ui.painter().layout_no_wrap(r.text.to_owned(), font, color);
    let clip = egui::Rect::from_min_max(
        Pos2::new(x + 20.0, rect.top()),
        Pos2::new(rect.right() - right_w - 4.0, rect.bottom()),
    );
    let pos = Pos2::new(x + 20.0, rect.center().y - g.size().y / 2.0);
    let painter = ui.painter().with_clip_rect(clip);
    painter.galley(pos, g.clone(), color);
    if r.bold {
        // Faux bold: draw again with a small offset.
        painter.galley(pos + Vec2::new(0.5, 0.0), g, color);
    }
    resp
}

fn matches(filter: &str, name: &str) -> bool {
    filter.is_empty() || name.to_lowercase().contains(filter)
}

pub fn sidebar(ui: &mut egui::Ui, tab: &mut RepoTab, cx: &mut Ctx) {
    let p = theme::pal(ui);
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.add_space(4.0);
        ui.label(RichText::new(&tab.name).size(15.0).strong());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.menu_button(icon::DOTS_THREE_CIRCLE, |ui| repo_menu(ui, tab, cx));
        });
    });
    ui.add_space(4.0);
    ui.spacing_mut().item_spacing.y = 1.0;

    let changes = tab.status.total();
    let r = row(
        ui,
        Row {
            icon: icon::TRAY,
            text: "Local Changes",
            selected: tab.view == View::LocalChanges,
            bold: false,
            right: (changes > 0).then(|| changes.to_string()),
            indent: 0.0,
        },
    );
    if r.clicked() {
        tab.view = View::LocalChanges;
    }
    let r = row(
        ui,
        Row {
            icon: icon::LIST,
            text: "All Commits",
            selected: tab.view == View::History,
            bold: false,
            right: None,
            indent: 0.0,
        },
    );
    if r.clicked() {
        tab.view = View::History;
    }
    ui.add_space(6.0);
    ui.separator();
    ui.add(
        egui::TextEdit::singleline(&mut tab.sidebar_filter)
            .hint_text(format!("{} Filter", icon::MAGNIFYING_GLASS))
            .desired_width(ui.available_width()),
    );
    ui.add_space(4.0);
    let filter = tab.sidebar_filter.trim().to_lowercase();
    let filtering = !filter.is_empty();

    egui::ScrollArea::vertical()
        .id_salt("sidebar_scroll")
        .auto_shrink(false)
        .show(ui, |ui| {
            // Branches
            let branches: Vec<Branch> = tab
                .refs
                .branches
                .iter()
                .filter(|b| matches(&filter, &b.name))
                .cloned()
                .collect();
            section(
                ui,
                "Branches",
                branches.len(),
                true,
                filtering,
                None,
                |ui| {
                    branch_tree(ui, tab, cx, &branches, None);
                },
            );

            // Remotes
            let remotes = tab.refs.remotes.clone();
            let add_remote = section(
                ui,
                "Remotes",
                remotes.len(),
                true,
                filtering,
                Some("Add remote…"),
                |ui| {
                    for remote in &remotes {
                        let branches: Vec<Branch> = remote
                            .branches
                            .iter()
                            .filter(|b| matches(&filter, &b.name))
                            .cloned()
                            .collect();
                        if filtering && branches.is_empty() {
                            continue;
                        }
                        let id = ui.make_persistent_id(("remote", &remote.name));
                        let state = collapsing_state(ui, id, false, filtering);
                        let header = state.show_header(ui, |ui| {
                            let r = row(
                                ui,
                                Row {
                                    icon: icon::CLOUD,
                                    text: &remote.name,
                                    selected: false,
                                    bold: false,
                                    right: None,
                                    indent: 0.0,
                                },
                            );
                            let r = if remote.url.is_empty() {
                                r.on_hover_text("No URL configured")
                            } else {
                                // Show which signed-in GitHub account this remote uses, once known.
                                match crate::github::cached_account_for_url(&remote.url) {
                                    Some(login) => r.on_hover_text(format!(
                                        "{}\nGitHub account: @{login}",
                                        remote.url
                                    )),
                                    None => r.on_hover_text(&remote.url),
                                }
                            };
                            r.context_menu(|ui| {
                                remote_menu(ui, tab, cx, &remote.name, &remote.url)
                            });
                            if r.clicked() {
                                ui.ctx()
                                    .data_mut(|d| d.insert_temp(id.with("toggle"), true));
                            }
                        });
                        header.body(|ui| branch_tree(ui, tab, cx, &branches, Some(&remote.name)));
                    }
                },
            );
            if add_remote {
                *cx.dialog = Some(Dialog::AddRemote {
                    name: String::new(),
                    url: String::new(),
                });
            }

            // Tags
            let tags: Vec<_> = tab
                .refs
                .tags
                .iter()
                .filter(|t| matches(&filter, &t.name))
                .cloned()
                .collect();
            section(ui, "Tags", tags.len(), false, filtering, None, |ui| {
                for tag in &tags {
                    let selected = tab.selected.as_deref() == Some(tag.target.as_str())
                        && tab.view == View::History;
                    let r = row(
                        ui,
                        Row {
                            icon: icon::TAG,
                            text: &tag.name,
                            selected,
                            bold: false,
                            right: None,
                            indent: 0.0,
                        },
                    );
                    if r.clicked() {
                        tab.reveal_commit(&tag.target);
                    }
                    r.context_menu(|ui| {
                        if ui.button(format!("Checkout '{}'", tag.name)).clicked() {
                            tab.checkout(&format!("refs/tags/{}", tag.name));
                            ui.close();
                        }
                        if ui
                            .button(format!("{} Create Branch from tag…", icon::GIT_BRANCH))
                            .clicked()
                        {
                            *cx.dialog = Some(Dialog::CreateBranch {
                                start: tag.target.clone(),
                                start_label: tag.name.clone(),
                                name: String::new(),
                                checkout: true,
                            });
                            ui.close();
                        }
                        if let Some(remote) = tab.default_remote() {
                            if ui
                                .button(format!("{} Push to '{remote}'", icon::ARROW_UP))
                                .clicked()
                            {
                                tab.git(
                                    format!("Push tag {}", tag.name),
                                    vec![
                                        s("push"),
                                        remote.clone(),
                                        format!("refs/tags/{}", tag.name),
                                    ],
                                );
                                ui.close();
                            }
                        }
                        ui.separator();
                        if ui.button(format!("{} Delete tag…", icon::TRASH)).clicked() {
                            let mut cmds = vec![vec![s("tag"), s("-d"), tag.name.clone()]];
                            let text = match tab.default_remote() {
                                Some(r) => {
                                    cmds.push(vec![
                                        s("push"),
                                        r.clone(),
                                        format!(":refs/tags/{}", tag.name),
                                    ]);
                                    format!("Delete tag '{}' locally and from '{r}'?", tag.name)
                                }
                                None => format!("Delete tag '{}'?", tag.name),
                            };
                            *cx.dialog = Some(Dialog::confirm(
                                "Delete Tag",
                                text,
                                "Delete",
                                format!("Delete tag {}", tag.name),
                                cmds,
                            ));
                            ui.close();
                        }
                        copy_menu_item(ui, &format!("{} Copy name", icon::COPY), tag.name.clone());
                    });
                }
            });

            // Stashes
            let stashes = tab.refs.stashes.clone();
            section(ui, "Stashes", stashes.len(), true, filtering, None, |ui| {
                for st in stashes.iter().filter(|st| matches(&filter, &st.message)) {
                    let selected = tab.selected.as_deref() == Some(st.id.as_str());
                    let r = row(
                        ui,
                        Row {
                            icon: icon::ARCHIVE,
                            text: &st.message,
                            selected,
                            bold: false,
                            right: None,
                            indent: 0.0,
                        },
                    )
                    .on_hover_text(format!(
                        "{} — {}",
                        st.refname,
                        theme::format_time(st.time)
                    ));
                    if r.clicked() {
                        // Stashes are not in the graph; show their details directly.
                        tab.select_commit(st.id.clone());
                    }
                    if r.double_clicked() {
                        tab.git(
                            "Apply stash",
                            vec![s("stash"), s("apply"), st.refname.clone()],
                        );
                    }
                    r.context_menu(|ui| {
                        if ui.button("Apply Stash").clicked() {
                            tab.git(
                                "Apply stash",
                                vec![s("stash"), s("apply"), st.refname.clone()],
                            );
                            ui.close();
                        }
                        if ui.button("Pop Stash").clicked() {
                            tab.git("Pop stash", vec![s("stash"), s("pop"), st.refname.clone()]);
                            ui.close();
                        }
                        ui.separator();
                        if ui
                            .button(format!("{} Delete Stash…", icon::TRASH))
                            .clicked()
                        {
                            *cx.dialog = Some(Dialog::confirm(
                                "Delete Stash",
                                format!("Delete '{}'? This cannot be undone.", st.message),
                                "Delete",
                                "Drop stash",
                                vec![vec![s("stash"), s("drop"), st.refname.clone()]],
                            ));
                            ui.close();
                        }
                    });
                }
            });

            // Submodules
            let subs = tab.refs.submodules.clone();
            if !subs.is_empty() {
                section(ui, "Submodules", subs.len(), false, filtering, None, |ui| {
                    for sm in subs.iter().filter(|sm| matches(&filter, &sm.path)) {
                        let right = match sm.status {
                            '-' => Some("not initialized".to_owned()),
                            '+' => Some("modified".to_owned()),
                            'U' => Some("conflict".to_owned()),
                            _ => None,
                        };
                        let r = row(
                            ui,
                            Row {
                                icon: icon::PACKAGE,
                                text: &sm.path,
                                selected: false,
                                bold: false,
                                right,
                                indent: 0.0,
                            },
                        );
                        if r.double_clicked() {
                            cx.open_repo = Some(tab.path.join(&sm.path));
                        }
                        r.context_menu(|ui| {
                            if ui.button("Open Submodule").clicked() {
                                cx.open_repo = Some(tab.path.join(&sm.path));
                                ui.close();
                            }
                            if ui.button("Update Submodule").clicked() {
                                tab.git(
                                    "Update submodule",
                                    vec![
                                        s("submodule"),
                                        s("update"),
                                        s("--init"),
                                        s("--recursive"),
                                        s("--"),
                                        sm.path.clone(),
                                    ],
                                );
                                ui.close();
                            }
                            copy_menu_item(
                                ui,
                                &format!("{} Copy path", icon::COPY),
                                sm.path.clone(),
                            );
                        });
                    }
                });
            }
            let _ = p;
        });
}

/// Loads a collapsing state, applying a toggle requested by a header click last frame.
fn collapsing_state(
    ui: &egui::Ui,
    id: egui::Id,
    default_open: bool,
    force_open: bool,
) -> egui::collapsing_header::CollapsingState {
    let mut state = egui::collapsing_header::CollapsingState::load_with_default_open(
        ui.ctx(),
        id,
        default_open,
    );
    if ui
        .ctx()
        .data_mut(|d| d.remove_temp::<bool>(id.with("toggle")).is_some())
    {
        state.toggle(ui);
    }
    if force_open {
        state.set_open(true);
    }
    state
}

/// Collapsible sidebar section. With `add_hint`, a "+" button is shown on the right of the
/// header; returns whether it was clicked.
fn section(
    ui: &mut egui::Ui,
    title: &str,
    count: usize,
    default_open: bool,
    force_open: bool,
    add_hint: Option<&str>,
    body: impl FnOnce(&mut egui::Ui),
) -> bool {
    let p = theme::pal(ui);
    let id = ui.make_persistent_id(("section", title));
    let state = collapsing_state(ui, id, default_open, force_open);
    ui.add_space(4.0);
    let mut add_clicked = false;
    let header = state.show_header(ui, |ui| {
        let r = ui.add(
            egui::Label::new(
                RichText::new(format!("{title}  "))
                    .strong()
                    .color(ui.visuals().text_color()),
            )
            .sense(Sense::click())
            .selectable(false),
        );
        ui.label(RichText::new(count.to_string()).small().color(p.muted));
        if r.clicked() {
            ui.ctx()
                .data_mut(|d| d.insert_temp(id.with("toggle"), true));
        }
        if let Some(hint) = add_hint {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let b = egui::Button::new(RichText::new(icon::PLUS).color(p.muted))
                    .frame_when_inactive(false)
                    .small();
                if ui.add(b).on_hover_text(hint).clicked() {
                    add_clicked = true;
                }
            });
        }
    });
    header.body(body);
    add_clicked
}

#[derive(Default)]
struct Folder {
    folders: BTreeMap<String, Folder>,
    branches: Vec<(String, Branch)>,
}

fn branch_tree(
    ui: &mut egui::Ui,
    tab: &mut RepoTab,
    cx: &mut Ctx,
    branches: &[Branch],
    remote: Option<&str>,
) {
    let mut root = Folder::default();
    for b in branches {
        let mut node = &mut root;
        let mut parts: Vec<&str> = b.name.split('/').collect();
        let leaf = parts.pop().unwrap_or_default();
        for part in parts {
            node = node.folders.entry(part.to_owned()).or_default();
        }
        node.branches.push((leaf.to_owned(), b.clone()));
    }
    folder(ui, tab, cx, &root, remote, 0, "");
}

fn folder(
    ui: &mut egui::Ui,
    tab: &mut RepoTab,
    cx: &mut Ctx,
    node: &Folder,
    remote: Option<&str>,
    depth: usize,
    prefix: &str,
) {
    for (name, child) in &node.folders {
        let path = format!("{prefix}{name}/");
        let id = ui.make_persistent_id(("branch_folder", remote, &path));
        let state = collapsing_state(ui, id, false, !tab.sidebar_filter.is_empty());
        let header = state.show_header(ui, |ui| {
            let r = row(
                ui,
                Row {
                    icon: icon::FOLDER,
                    text: name,
                    selected: false,
                    bold: false,
                    right: None,
                    indent: 0.0,
                },
            );
            if r.clicked() {
                ui.ctx()
                    .data_mut(|d| d.insert_temp(id.with("toggle"), true));
            }
        });
        header.body(|ui| folder(ui, tab, cx, child, remote, depth + 1, &path));
    }
    for (leaf, b) in &node.branches {
        let full = match remote {
            Some(r) => format!("{r}/{}", b.name),
            None => b.name.clone(),
        };
        let selected =
            tab.selected.as_deref() == Some(b.target.as_str()) && tab.view == View::History;
        let mut right = String::new();
        if b.ahead > 0 {
            right.push_str(&format!("{}{} ", icon::ARROW_UP, b.ahead));
        }
        if b.behind > 0 {
            right.push_str(&format!("{}{}", icon::ARROW_DOWN, b.behind));
        }
        if b.upstream_gone {
            right.push_str("gone");
        }
        let r = row(
            ui,
            Row {
                icon: if b.is_head {
                    icon::CHECK
                } else {
                    icon::GIT_BRANCH
                },
                text: leaf,
                selected,
                bold: b.is_head,
                right: (!right.is_empty()).then(|| right.trim().to_owned()),
                indent: 0.0,
            },
        );
        let r = match &b.upstream {
            Some(u) => r.on_hover_text(format!("{full} {} {u}", icon::ARROW_RIGHT)),
            None => r.on_hover_text(&full),
        };
        if r.clicked() {
            tab.reveal_commit(&b.target);
        }
        if r.double_clicked() && !b.is_head {
            match remote {
                Some(_) => tab.checkout_remote(&full),
                None => tab.checkout(&b.name),
            }
        }
        r.context_menu(|ui| branch_menu(ui, tab, cx, b, remote));
    }
}

fn branch_menu(
    ui: &mut egui::Ui,
    tab: &mut RepoTab,
    cx: &mut Ctx,
    b: &Branch,
    remote: Option<&str>,
) {
    let head = tab
        .refs
        .head_branch
        .clone()
        .unwrap_or_else(|| "HEAD".into());
    let full = match remote {
        Some(r) => format!("{r}/{}", b.name),
        None => b.name.clone(),
    };
    if !b.is_head
        && ui
            .button(format!("{} Checkout '{full}'", icon::GIT_BRANCH))
            .clicked()
    {
        match remote {
            Some(_) => tab.checkout_remote(&full),
            None => tab.checkout(&b.name),
        }
        ui.close();
    }
    if !b.is_head {
        if ui
            .button(format!("{} Merge '{full}' into '{head}'…", icon::GIT_MERGE))
            .clicked()
        {
            *cx.dialog = Some(Dialog::Merge {
                source: full.clone(),
                no_ff: false,
                squash: false,
            });
            ui.close();
        }
        if ui
            .button(format!("Rebase '{head}' onto '{full}'"))
            .clicked()
        {
            *cx.dialog = Some(Dialog::confirm(
                "Rebase",
                format!("Rebase '{head}' onto '{full}'?"),
                "Rebase",
                format!("Rebase onto {full}"),
                vec![vec![s("rebase"), full.clone()]],
            ));
            ui.close();
        }
    }
    ui.separator();
    if remote.is_none() {
        if b.is_head {
            if ui.button(format!("{} Pull…", icon::ARROW_DOWN)).clicked() {
                *cx.dialog = Some(Dialog::pull(tab));
                ui.close();
            }
            if ui.button(format!("{} Push…", icon::ARROW_UP)).clicked() {
                *cx.dialog = Some(Dialog::push(tab));
                ui.close();
            }
        } else if let Some(up) = &b.upstream {
            // Fast-forward a non-checked-out branch to its upstream.
            if b.behind > 0
                && b.ahead == 0
                && ui
                    .button(format!("{} Fast-forward to '{up}'", icon::ARROW_DOWN))
                    .clicked()
            {
                if let Some((r, rb)) = up.split_once('/') {
                    tab.git(
                        format!("Fast-forward {}", b.name),
                        vec![s("fetch"), r.to_owned(), format!("{rb}:{}", b.name)],
                    );
                }
                ui.close();
            }
            if ui
                .button(format!("{} Push '{}'", icon::ARROW_UP, b.name))
                .clicked()
            {
                if let Some((r, rb)) = up.split_once('/') {
                    tab.git(
                        format!("Push {}", b.name),
                        vec![
                            s("push"),
                            r.to_owned(),
                            format!("refs/heads/{}:refs/heads/{rb}", b.name),
                        ],
                    );
                }
                ui.close();
            }
        }
    }
    if ui
        .button(format!("{} Create Branch from '{full}'…", icon::GIT_BRANCH))
        .clicked()
    {
        *cx.dialog = Some(Dialog::CreateBranch {
            start: full.clone(),
            start_label: full.clone(),
            name: String::new(),
            checkout: true,
        });
        ui.close();
    }
    if ui.button(format!("{} Create Tag…", icon::TAG)).clicked() {
        *cx.dialog = Some(Dialog::CreateTag {
            target: b.target.clone(),
            name: String::new(),
            message: String::new(),
            push: false,
        });
        ui.close();
    }
    ui.separator();
    match remote {
        None => {
            if ui.button(format!("{} Rename…", icon::PENCIL)).clicked() {
                *cx.dialog = Some(Dialog::RenameBranch {
                    old: b.name.clone(),
                    name: b.name.clone(),
                });
                ui.close();
            }
            if let Some(up) = &b.upstream {
                if ui.button(format!("Unset upstream ({up})")).clicked() {
                    tab.git(
                        "Unset upstream",
                        vec![s("branch"), s("--unset-upstream"), b.name.clone()],
                    );
                    ui.close();
                }
            }
            if !b.is_head
                && ui
                    .button(format!("{} Delete '{}'…", icon::TRASH, b.name))
                    .clicked()
            {
                *cx.dialog = Some(Dialog::confirm(
                    "Delete Branch",
                    format!(
                        "Delete local branch '{}'? Unmerged commits will be lost.",
                        b.name
                    ),
                    "Delete",
                    format!("Delete branch {}", b.name),
                    vec![vec![s("branch"), s("-D"), b.name.clone()]],
                ));
                ui.close();
            }
        }
        Some(r) => {
            if ui
                .button(format!("{} Delete '{full}' from remote…", icon::TRASH))
                .clicked()
            {
                *cx.dialog = Some(Dialog::confirm(
                    "Delete Remote Branch",
                    format!("Delete branch '{}' from remote '{r}'?", b.name),
                    "Delete",
                    format!("Delete {full}"),
                    vec![vec![s("push"), r.to_owned(), s("--delete"), b.name.clone()]],
                ));
                ui.close();
            }
        }
    }
    ui.separator();
    copy_menu_item(ui, &format!("{} Copy name", icon::COPY), full.clone());
    if let Some(url) = remote
        .and_then(|r| tab.refs.remotes.iter().find(|x| x.name == r))
        .and_then(|r| crate::platform::web_url(&r.url))
    {
        if ui
            .button(format!("{} Open on remote", icon::GLOBE))
            .clicked()
        {
            crate::platform::open_url(&format!("{url}/tree/{}", b.name));
            ui.close();
        }
    }
}

fn remote_menu(ui: &mut egui::Ui, tab: &mut RepoTab, cx: &mut Ctx, name: &str, url: &str) {
    if ui
        .button(format!("{} Fetch '{name}'", icon::ARROWS_CLOCKWISE))
        .clicked()
    {
        tab.git(
            format!("Fetch {name}"),
            vec![s("fetch"), s("--prune"), name.to_owned()],
        );
        ui.close();
    }
    if let Some(web) = crate::platform::web_url(url) {
        if ui
            .button(format!("{} Open in browser", icon::GLOBE))
            .clicked()
        {
            crate::platform::open_url(&web);
            ui.close();
        }
    }
    copy_menu_item(ui, &format!("{} Copy URL", icon::COPY), url.to_owned());
    ui.separator();
    if ui.button(format!("{} Edit…", icon::PENCIL)).clicked() {
        *cx.dialog = Some(Dialog::EditRemote {
            old_name: name.to_owned(),
            old_url: url.to_owned(),
            name: name.to_owned(),
            url: url.to_owned(),
        });
        ui.close();
    }
    if ui
        .button(format!("{} Remove remote…", icon::TRASH))
        .clicked()
    {
        *cx.dialog = Some(Dialog::confirm(
            "Remove Remote",
            format!("Remove remote '{name}' ({url})?"),
            "Remove",
            format!("Remove remote {name}"),
            vec![vec![s("remote"), s("remove"), name.to_owned()]],
        ));
        ui.close();
    }
}

fn repo_menu(ui: &mut egui::Ui, tab: &mut RepoTab, cx: &mut Ctx) {
    if ui
        .button(format!("{} Repository Settings…", icon::GEAR))
        .clicked()
    {
        *cx.dialog = Some(Dialog::repo_settings(tab));
        ui.close();
    }
    ui.separator();
    if ui
        .button(format!("{} Show in file manager", icon::FOLDER_OPEN))
        .clicked()
    {
        tab.open_in_file_manager(None);
        ui.close();
    }
    if ui
        .button(format!("{} Open in terminal", icon::TERMINAL))
        .clicked()
    {
        crate::platform::open_terminal(&tab.path);
        ui.close();
    }
    copy_menu_item(
        ui,
        &format!("{} Copy path", icon::COPY),
        tab.path.display().to_string(),
    );
    ui.separator();
    if ui.button(format!("{} Add Remote…", icon::PLUS)).clicked() {
        *cx.dialog = Some(Dialog::AddRemote {
            name: String::new(),
            url: String::new(),
        });
        ui.close();
    }
    if ui.button("Update submodules").clicked() {
        tab.git(
            "Update submodules",
            vec![s("submodule"), s("update"), s("--init"), s("--recursive")],
        );
        ui.close();
    }
    if ui.button("Garbage collect").clicked() {
        tab.git("Garbage collect", vec![s("gc")]);
        ui.close();
    }
}
